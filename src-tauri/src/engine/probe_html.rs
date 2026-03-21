use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::{
    header::{ACCEPT, ACCEPT_LANGUAGE, AUTHORIZATION, COOKIE, REFERER},
    Url,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::filename_policy::sanitize_filename_leaf;
use super::probe_filename::{
    best_url_filename_candidate, best_url_path_candidate,
    extract_filename_from_content_disposition, query_response_content_disposition,
    query_response_content_type, usable_extension,
};
use super::probe_html_cache::{
    cache_html_app_guest_token, cached_html_app_guest_token, invalidate_html_app_guest_token,
    record_html_app_api_failure,
};
use super::probe_html_patterns::{
    HTML_APPDATA_API_SERVER_RE, HTML_APPDATA_WT_RE, HTML_ATTRIBUTE_URL_RE, HTML_CONFIG_SCRIPT_RE,
    HTML_FILENAME_ATTR_RE, HTML_FILENAME_JSON_RE, HTML_FILENAME_SCRIPT_RE, HTML_FORM_ACTION_RE,
    HTML_FORM_METHOD_RE, HTML_FORM_RE, HTML_INPUT_NAME_RE, HTML_INPUT_RE, HTML_INPUT_TYPE_RE,
    HTML_INPUT_VALUE_RE, HTML_JSON_URL_RE, HTML_LOCATION_ASSIGN_URL_RE, HTML_LOCATION_CALL_URL_RE,
    HTML_META_REFRESH_URL_RE, HTML_META_TITLE_RE, HTML_SCRIPT_URL_RE, HTML_TITLE_RE,
};

const HTML_WRAPPER_BODY_SCAN_LIMIT_BYTES: usize = 256 * 1024;
const HTML_WRAPPER_PROGRESSIVE_SCAN_STEP_BYTES: usize = 32 * 1024;
const HTML_DOWNLOAD_CANDIDATE_MIN_SCORE: i32 = 40;
const HTML_FILENAME_CANDIDATE_MIN_SCORE: i32 = 90;
const HTML_FORM_FOLLOW_UP_MIN_SCORE: i32 = 100;
const HTML_APP_API_BUCKET_WINDOW_SECONDS: u64 = 4 * 60 * 60;
const HTML_APP_CONTENT_PAGE_SIZE: &str = "1000";

#[derive(Clone)]
pub(super) struct HtmlAppApiHint {
    pub config_url: Option<String>,
    pub inline_api_base_url: Option<String>,
    pub inline_website_token_seed: Option<String>,
    pub content_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HtmlFollowUpMethod {
    Get,
    Post,
}

#[derive(Clone)]
pub(super) struct HtmlFollowUpRequest {
    pub url: String,
    pub method: HtmlFollowUpMethod,
    pub fields: Vec<(String, String)>,
}

#[derive(Clone)]
pub(super) struct HtmlApiResolvedDownload {
    pub direct_download_url: String,
    pub suggested_name: Option<String>,
    pub request_cookies: Option<String>,
}

#[derive(Clone)]
struct HtmlAppApiConfig {
    api_base_url: String,
    website_token_seed: String,
}

struct HtmlAppContentRequest<'a> {
    api_base_url: &'a str,
    content_id: &'a str,
    guest_token: &'a str,
    website_token_seed: &'a str,
    user_agent: &'a str,
    primary_language: &'a str,
    accept_language_header: &'a str,
    page_url: &'a str,
}

#[derive(Debug, Deserialize)]
struct HtmlAppGuestAccountResponse {
    status: String,
    #[serde(default)]
    data: HtmlAppGuestAccountData,
}

#[derive(Debug, Default, Deserialize)]
struct HtmlAppGuestAccountData {
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HtmlAppContentResponse {
    status: String,
    #[serde(default)]
    data: HtmlAppContentData,
}

#[derive(Debug, Default, Deserialize)]
struct HtmlAppContentData {
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    children: BTreeMap<String, HtmlAppContentChild>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct HtmlAppContentChild {
    #[serde(default)]
    link: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Clone)]
pub(super) struct HtmlResolutionHint {
    pub direct_download_url: Option<String>,
    pub request_referer: String,
    pub suggested_name: Option<String>,
    pub app_api: Option<HtmlAppApiHint>,
    pub follow_up_request: Option<HtmlFollowUpRequest>,
}

pub(super) async fn read_html_resolution_hint(
    final_url: &str,
    response: &mut reqwest::Response,
) -> Option<HtmlResolutionHint> {
    let body = read_response_excerpt_until_resolution(
        final_url,
        response,
        HTML_WRAPPER_BODY_SCAN_LIMIT_BYTES,
    )
    .await;
    inspect_html_resolution_body(final_url, &body)
}

fn inspect_html_resolution_body(final_url: &str, body: &str) -> Option<HtmlResolutionHint> {
    let mut direct_download_url = extract_html_direct_download_url(final_url, body);
    let suggested_name = extract_html_filename_hint(body);
    let app_api = extract_html_app_api_hint(final_url, body);
    let follow_up_request = extract_html_follow_up_request(final_url, body);
    if follow_up_request.as_ref().is_some_and(|request| {
        matches!(request.method, HtmlFollowUpMethod::Post)
            && direct_download_url
                .as_deref()
                .is_some_and(|candidate| urls_match_after_normalization(candidate, &request.url))
    }) {
        direct_download_url = None;
    }
    if direct_download_url.is_none()
        && suggested_name.is_none()
        && app_api.is_none()
        && follow_up_request.is_none()
    {
        return None;
    }
    Some(HtmlResolutionHint {
        direct_download_url,
        request_referer: final_url.to_string(),
        suggested_name,
        app_api,
        follow_up_request,
    })
}

pub(super) async fn resolve_html_app_api_download_url(
    client: &reqwest::Client,
    page_url: &str,
    hint: &HtmlAppApiHint,
    user_agent: &str,
    browser_language: &str,
    accept_language_header: &str,
) -> Result<Option<HtmlApiResolvedDownload>, String> {
    let config = if let (Some(api_base_url), Some(website_token_seed)) = (
        hint.inline_api_base_url.clone(),
        hint.inline_website_token_seed.clone(),
    ) {
        HtmlAppApiConfig {
            api_base_url,
            website_token_seed,
        }
    } else {
        let Some(config_url) = hint.config_url.as_deref() else {
            return Ok(None);
        };
        let config_response = client
            .get(config_url)
            .header(ACCEPT, "application/javascript, text/javascript, */*;q=0.8")
            .header(ACCEPT_LANGUAGE, accept_language_header)
            .header(REFERER, page_url)
            .send()
            .await
            .map_err(|error| format!("Wrapper config request failed: {error}"))?;
        if !config_response.status().is_success() {
            record_html_app_api_failure(config_url);
            return Err(format!(
                "Wrapper config request returned HTTP {}.",
                config_response.status().as_u16()
            ));
        }
        let config_body = config_response
            .text()
            .await
            .map_err(|error| format!("Wrapper config body could not be read: {error}"))?;
        parse_html_app_api_config(config_url, &config_body)
            .ok_or_else(|| "Wrapper config did not expose a usable app API mapping.".to_string())?
    };
    let (mut guest_token, token_from_cache) =
        create_html_app_guest_token(client, &config.api_base_url).await?;
    let primary_language = browser_primary_language(browser_language);
    let mut content_response = send_html_app_content_request(
        client,
        &HtmlAppContentRequest {
            api_base_url: &config.api_base_url,
            content_id: &hint.content_id,
            guest_token: &guest_token,
            website_token_seed: &config.website_token_seed,
            user_agent,
            primary_language,
            accept_language_header,
            page_url,
        },
    )
    .await?;
    if token_from_cache && matches!(content_response.status().as_u16(), 401 | 403) {
        invalidate_html_app_guest_token(&config.api_base_url);
        let (fresh_guest_token, _fresh_token_from_cache) =
            create_html_app_guest_token(client, &config.api_base_url).await?;
        guest_token = fresh_guest_token;
        content_response = send_html_app_content_request(
            client,
            &HtmlAppContentRequest {
                api_base_url: &config.api_base_url,
                content_id: &hint.content_id,
                guest_token: &guest_token,
                website_token_seed: &config.website_token_seed,
                user_agent,
                primary_language,
                accept_language_header,
                page_url,
            },
        )
        .await?;
    }
    if !content_response.status().is_success() {
        let _ = token_from_cache;
        record_html_app_api_failure(&config.api_base_url);
        return Err(format!(
            "Wrapper app API content request returned HTTP {}.",
            content_response.status().as_u16()
        ));
    }

    let account_cookie = format!("accountToken={guest_token}");

    let content_body = content_response
        .text()
        .await
        .map_err(|error| format!("Wrapper app API content payload could not be read: {error}"))?;
    let content_payload = serde_json::from_str::<HtmlAppContentResponse>(&content_body)
        .map_err(|error| format!("Wrapper app API content payload was invalid: {error}"))?;
    if content_payload.status != "ok" {
        return Err(format!(
            "Wrapper app API content request returned {}.",
            content_payload.status
        ));
    }

    Ok(select_html_app_direct_download(
        &content_payload.data,
        &account_cookie,
    ))
}

async fn read_response_excerpt_until_resolution(
    final_url: &str,
    response: &mut reqwest::Response,
    max_bytes: usize,
) -> String {
    let mut buffer = Vec::with_capacity(max_bytes.min(16 * 1024));
    let mut next_scan_threshold = HTML_WRAPPER_PROGRESSIVE_SCAN_STEP_BYTES.min(max_bytes);

    while buffer.len() < max_bytes {
        let remaining = max_bytes.saturating_sub(buffer.len());
        let next = match response.chunk().await {
            Ok(next) => next,
            Err(_) => break,
        };
        let Some(next) = next else {
            break;
        };
        let take = next.len().min(remaining);
        buffer.extend_from_slice(&next[..take]);
        let reached_scan_threshold = buffer.len() >= next_scan_threshold || take < next.len();
        if reached_scan_threshold {
            let excerpt = String::from_utf8_lossy(&buffer);
            if inspect_html_resolution_body(final_url, excerpt.as_ref()).is_some() {
                return excerpt.into_owned();
            }
            next_scan_threshold = next_scan_threshold
                .saturating_add(HTML_WRAPPER_PROGRESSIVE_SCAN_STEP_BYTES)
                .min(max_bytes);
        }
        if take < next.len() {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

pub(super) fn extract_html_follow_up_request(
    page_url: &str,
    body: &str,
) -> Option<HtmlFollowUpRequest> {
    let page_url = Url::parse(page_url).ok()?;
    let mut best_candidate: Option<(i32, HtmlFollowUpRequest)> = None;

    for capture in HTML_FORM_RE.captures_iter(body) {
        let Some(attributes) = capture.get(1).map(|value| value.as_str()) else {
            continue;
        };
        let Some(inner_html) = capture.get(2).map(|value| value.as_str()) else {
            continue;
        };
        let action = HTML_FORM_ACTION_RE
            .captures(attributes)
            .and_then(|value| value.get(1).map(|match_value| match_value.as_str()))
            .and_then(normalize_embedded_html_url)
            .and_then(|raw| resolve_html_candidate_url(&page_url, &raw))
            .unwrap_or_else(|| page_url.to_string());
        let method = HTML_FORM_METHOD_RE
            .captures(attributes)
            .and_then(|value| value.get(1).map(|match_value| match_value.as_str()))
            .map(|value| value.trim().to_ascii_uppercase())
            .map(|value| {
                if value == "POST" {
                    HtmlFollowUpMethod::Post
                } else {
                    HtmlFollowUpMethod::Get
                }
            })
            .unwrap_or(HtmlFollowUpMethod::Get);
        let fields = extract_html_form_fields(inner_html);
        let score = score_html_follow_up_request(&page_url, &action, method, &fields);
        if score < HTML_FORM_FOLLOW_UP_MIN_SCORE {
            continue;
        }
        let candidate = HtmlFollowUpRequest {
            url: action,
            method,
            fields,
        };
        match best_candidate.as_ref() {
            Some((best_score, best_request))
                if *best_score > score
                    || (*best_score == score
                        && best_request.fields.len() >= candidate.fields.len()) => {}
            _ => best_candidate = Some((score, candidate)),
        }
    }

    best_candidate.map(|(_, request)| request)
}

pub(super) fn extract_html_direct_download_url(page_url: &str, body: &str) -> Option<String> {
    let page_url = Url::parse(page_url).ok()?;
    let mut best_candidate: Option<(i32, String)> = None;

    for raw in HTML_META_REFRESH_URL_RE
        .captures_iter(body)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str()))
        .chain(
            HTML_JSON_URL_RE
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str())),
        )
        .chain(
            HTML_SCRIPT_URL_RE
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str())),
        )
        .chain(
            HTML_LOCATION_ASSIGN_URL_RE
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str())),
        )
        .chain(
            HTML_LOCATION_CALL_URL_RE
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str())),
        )
        .chain(
            HTML_ATTRIBUTE_URL_RE
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str())),
        )
    {
        let Some(normalized) = normalize_embedded_html_url(raw) else {
            continue;
        };
        let Some(candidate) = resolve_html_candidate_url(&page_url, &normalized) else {
            continue;
        };
        let score = score_html_download_candidate(&page_url, &candidate);
        if score < HTML_DOWNLOAD_CANDIDATE_MIN_SCORE {
            continue;
        }
        match best_candidate.as_ref() {
            Some((best_score, best_url))
                if *best_score > score
                    || (*best_score == score && best_url.len() <= candidate.len()) => {}
            _ => best_candidate = Some((score, candidate)),
        }
    }

    best_candidate.map(|(_, url)| url)
}

pub(super) fn extract_html_filename_hint(body: &str) -> Option<String> {
    let mut best_candidate: Option<(i32, String)> = None;

    for raw in HTML_META_TITLE_RE
        .captures_iter(body)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str()))
        .chain(
            HTML_FILENAME_JSON_RE
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str())),
        )
        .chain(
            HTML_FILENAME_SCRIPT_RE
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str())),
        )
        .chain(
            HTML_FILENAME_ATTR_RE
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str())),
        )
        .chain(
            HTML_TITLE_RE
                .captures_iter(body)
                .filter_map(|capture| capture.get(1).map(|value| value.as_str())),
        )
    {
        let Some(candidate) = normalize_embedded_html_filename(raw) else {
            continue;
        };
        let score = score_html_filename_candidate(&candidate);
        if score < HTML_FILENAME_CANDIDATE_MIN_SCORE {
            continue;
        }
        match best_candidate.as_ref() {
            Some((best_score, best_name))
                if *best_score > score
                    || (*best_score == score && best_name.len() <= candidate.len()) => {}
            _ => best_candidate = Some((score, candidate)),
        }
    }

    best_candidate.map(|(_, candidate)| candidate)
}

fn extract_html_app_api_hint(page_url: &str, body: &str) -> Option<HtmlAppApiHint> {
    let page_url = Url::parse(page_url).ok()?;
    let content_id = extract_html_app_content_id(&page_url)?;
    let config_url = HTML_CONFIG_SCRIPT_RE
        .captures_iter(body)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str()))
        .find_map(|raw| {
            let normalized = normalize_embedded_html_url(raw)?;
            resolve_html_candidate_url(&page_url, &normalized)
        });
    let inline_config =
        parse_html_app_api_config(config_url.as_deref().unwrap_or(page_url.as_str()), body);

    if config_url.is_none() && inline_config.is_none() {
        return None;
    }

    Some(HtmlAppApiHint {
        config_url,
        inline_api_base_url: inline_config
            .as_ref()
            .map(|config| config.api_base_url.clone()),
        inline_website_token_seed: inline_config
            .as_ref()
            .map(|config| config.website_token_seed.clone()),
        content_id,
    })
}

fn extract_html_app_content_id(page_url: &Url) -> Option<String> {
    page_url.path_segments()?.rev().find_map(|segment| {
        let trimmed = segment.trim();
        if trimmed.is_empty() || usable_extension(trimmed).is_some() || trimmed.len() < 6 {
            return None;
        }
        trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
            .then(|| trimmed.to_string())
    })
}

fn parse_html_app_api_config(config_url: &str, body: &str) -> Option<HtmlAppApiConfig> {
    let api_server = HTML_APPDATA_API_SERVER_RE
        .captures(body)
        .and_then(|capture| {
            capture
                .get(1)
                .map(|value| value.as_str().trim().to_string())
        })
        .filter(|value| !value.is_empty())?;
    let website_token_seed = HTML_APPDATA_WT_RE
        .captures(body)
        .and_then(|capture| {
            capture
                .get(1)
                .map(|value| value.as_str().trim().to_string())
        })
        .filter(|value| !value.is_empty())?;
    let api_base_url = resolve_html_app_api_base_url(config_url, &api_server)?;

    Some(HtmlAppApiConfig {
        api_base_url,
        website_token_seed,
    })
}

fn resolve_html_app_api_base_url(config_url: &str, api_server: &str) -> Option<String> {
    if api_server.starts_with("http://") || api_server.starts_with("https://") {
        return Some(api_server.trim_end_matches('/').to_string());
    }
    if api_server.contains('.') {
        return Some(format!("https://{}", api_server.trim_end_matches('/')));
    }

    let parsed = Url::parse(config_url).ok()?;
    let host = parsed
        .host_str()?
        .strip_prefix("www.")
        .unwrap_or_else(|| parsed.host_str().unwrap_or_default());
    if host.is_empty() {
        return None;
    }

    Some(format!(
        "{}://{}.{}",
        parsed.scheme(),
        api_server.trim_end_matches('/'),
        host
    ))
}

async fn create_html_app_guest_token(
    client: &reqwest::Client,
    api_base_url: &str,
) -> Result<(String, bool), String> {
    if let Some(cached) = cached_html_app_guest_token(api_base_url)? {
        return Ok((cached, true));
    }

    let response = client
        .post(format!("{api_base_url}/accounts"))
        .header(ACCEPT, "application/json, text/plain, */*")
        .send()
        .await
        .map_err(|error| format!("Wrapper app API guest account request failed: {error}"))?;
    if !response.status().is_success() {
        record_html_app_api_failure(api_base_url);
        return Err(format!(
            "Wrapper app API guest account request returned HTTP {}.",
            response.status().as_u16()
        ));
    }

    let response_body = response.text().await.map_err(|error| {
        format!("Wrapper app API guest account payload could not be read: {error}")
    })?;
    let payload = serde_json::from_str::<HtmlAppGuestAccountResponse>(&response_body)
        .map_err(|error| format!("Wrapper app API guest account payload was invalid: {error}"))?;
    if payload.status != "ok" {
        record_html_app_api_failure(api_base_url);
        return Err(format!(
            "Wrapper app API guest account request returned {}.",
            payload.status
        ));
    }

    let Some(token) = payload
        .data
        .token
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        record_html_app_api_failure(api_base_url);
        return Err("Wrapper app API guest account request did not return a token.".to_string());
    };

    cache_html_app_guest_token(api_base_url, &token);
    Ok((token, false))
}

async fn send_html_app_content_request(
    client: &reqwest::Client,
    request: &HtmlAppContentRequest<'_>,
) -> Result<reqwest::Response, String> {
    let website_token = generate_html_app_website_token(
        request.user_agent,
        request.primary_language,
        request.guest_token,
        request.website_token_seed,
    );
    let content_url = format!("{}/contents/{}", request.api_base_url, request.content_id);
    client
        .get(&content_url)
        .query(&[
            ("contentFilter", ""),
            ("page", "1"),
            ("pageSize", HTML_APP_CONTENT_PAGE_SIZE),
            ("sortField", "createTime"),
            ("sortDirection", "-1"),
        ])
        .header(ACCEPT, "application/json, text/plain, */*")
        .header(ACCEPT_LANGUAGE, request.accept_language_header)
        .header(AUTHORIZATION, format!("Bearer {}", request.guest_token))
        .header(COOKIE, format!("accountToken={}", request.guest_token))
        .header("X-Website-Token", website_token)
        .header("X-BL", request.primary_language)
        .header(REFERER, request.page_url)
        .send()
        .await
        .map_err(|error| format!("Wrapper app API content request failed: {error}"))
}

fn browser_primary_language(browser_language: &str) -> &str {
    browser_language
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(browser_language)
}

fn generate_html_app_website_token(
    user_agent: &str,
    browser_language: &str,
    account_token: &str,
    website_token_seed: &str,
) -> String {
    let now_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let bucket = now_seconds / HTML_APP_API_BUCKET_WINDOW_SECONDS;

    generate_html_app_website_token_for_bucket(
        user_agent,
        browser_language,
        account_token,
        website_token_seed,
        bucket,
    )
}

fn generate_html_app_website_token_for_bucket(
    user_agent: &str,
    browser_language: &str,
    account_token: &str,
    website_token_seed: &str,
    bucket: u64,
) -> String {
    let raw = format!(
        "{user_agent}::{browser_language}::{account_token}::{bucket}::{website_token_seed}"
    );
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());

    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn select_html_app_direct_download(
    content: &HtmlAppContentData,
    account_cookie: &str,
) -> Option<HtmlApiResolvedDownload> {
    let cookies = Some(account_cookie.to_string());

    if content.kind.as_deref() != Some("folder")
        && let Some(direct_download_url) = content
            .link
            .as_deref()
            .and_then(normalize_html_app_direct_download_url)
        {
            return Some(HtmlApiResolvedDownload {
                direct_download_url,
                suggested_name: content.name.as_deref().and_then(sanitize_filename_leaf),
                request_cookies: cookies,
            });
        }

    let child = if content.children.len() == 1 {
        content.children.values().next()
    } else {
        content.children.values().max_by_key(|child| {
            let mut score = 0_i32;
            if child.kind.as_deref() == Some("file") {
                score += 50;
            }
            if child.name.as_deref().and_then(usable_extension).is_some() {
                score += 100;
            }
            score
        })
    }?;

    Some(HtmlApiResolvedDownload {
        direct_download_url: child
            .link
            .as_deref()
            .and_then(normalize_html_app_direct_download_url)?,
        suggested_name: child.name.as_deref().and_then(sanitize_filename_leaf),
        request_cookies: cookies,
    })
}

fn normalize_html_app_direct_download_url(value: &str) -> Option<String> {
    let parsed = Url::parse(value).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| parsed.to_string())
}

fn extract_html_form_fields(body: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();

    for input in HTML_INPUT_RE
        .captures_iter(body)
        .filter_map(|capture| capture.get(1).map(|value| value.as_str()))
    {
        let Some(name) = HTML_INPUT_NAME_RE
            .captures(input)
            .and_then(|value| value.get(1).map(|match_value| match_value.as_str()))
            .map(decode_basic_html_entities)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let input_type = HTML_INPUT_TYPE_RE
            .captures(input)
            .and_then(|value| value.get(1).map(|match_value| match_value.as_str()))
            .map(|value| value.trim().to_ascii_lowercase());
        if matches!(
            input_type.as_deref(),
            Some("file" | "password" | "checkbox" | "radio")
        ) {
            continue;
        }
        let value = HTML_INPUT_VALUE_RE
            .captures(input)
            .and_then(|capture| capture.get(1).map(|match_value| match_value.as_str()))
            .map(decode_basic_html_entities)
            .unwrap_or_default();
        fields.push((name, value));
    }

    fields
}

fn score_html_follow_up_request(
    page_url: &Url,
    action_url: &str,
    method: HtmlFollowUpMethod,
    fields: &[(String, String)],
) -> i32 {
    if matches!(method, HtmlFollowUpMethod::Get)
        && fields.is_empty()
        && urls_match_after_normalization(page_url.as_str(), action_url)
    {
        return i32::MIN;
    }

    let Ok(action) = Url::parse(action_url) else {
        return i32::MIN;
    };

    let mut score = 0_i32;
    if matches!(method, HtmlFollowUpMethod::Post) {
        score += 80;
    } else if !fields.is_empty() {
        score += 20;
    }
    if action.host_str() == page_url.host_str() {
        score += 10;
    }
    if has_file_like_url_hint(action_url) {
        score += 60;
    }
    let path = action.path().to_ascii_lowercase();
    if path.contains("/download")
        || path.contains("/direct")
        || path.contains("/dl")
        || path.contains("/file")
        || path.contains("/get")
    {
        score += 50;
    }
    if fields.len() >= 2 {
        score += 20;
    }
    for (name, value) in fields {
        let lowered = name.to_ascii_lowercase();
        if lowered.contains("token")
            || lowered.contains("csrf")
            || lowered.contains("download")
            || lowered == "op"
            || lowered == "id"
        {
            score += 15;
        }
        if !value.trim().is_empty() {
            score += 4;
        }
    }

    score
}

fn normalize_embedded_html_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches(['"', '\'', ' ']);
    if trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with("javascript:")
        || trimmed.starts_with("mailto:")
    {
        return None;
    }

    let normalized = trimmed
        .replace("\\\\/", "/")
        .replace("\\/", "/")
        .replace("\\u002F", "/")
        .replace("\\u002f", "/")
        .replace("\\u003A", ":")
        .replace("\\u003a", ":")
        .replace("\\u003D", "=")
        .replace("\\u003d", "=")
        .replace("\\u0026", "&");
    Some(decode_basic_html_entities(&normalized))
}

fn normalize_embedded_html_filename(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches(['"', '\'', ' ']);
    if trimmed.is_empty() {
        return None;
    }

    let normalized = decode_basic_html_entities(trimmed)
        .replace("\\u0026", "&")
        .replace("\\u003a", ":")
        .replace("\\u003A", ":")
        .replace("\\u002e", ".")
        .replace("\\u002E", ".")
        .replace("\\\"", "\"")
        .replace("\\'", "'");

    for candidate in html_filename_candidates(&normalized) {
        let Some(sanitized) = sanitize_filename_leaf(&candidate) else {
            continue;
        };
        let has_extension = usable_extension(&sanitized).is_some();
        if has_extension {
            return Some(sanitized);
        }
    }

    None
}

fn html_filename_candidates(value: &str) -> Vec<String> {
    let mut segments = vec![value.trim().to_string()];
    for separator in [" | ", " - ", " :: "] {
        let mut next = Vec::new();
        for segment in segments {
            if segment.contains(separator) {
                next.extend(
                    segment
                        .split(separator)
                        .map(str::trim)
                        .filter(|candidate| !candidate.is_empty())
                        .map(ToString::to_string),
                );
            } else {
                next.push(segment);
            }
        }
        segments = next;
    }
    segments
}

fn score_html_filename_candidate(candidate: &str) -> i32 {
    let mut score = 0_i32;
    if usable_extension(candidate).is_some() {
        score += 120;
    }
    if candidate.len() >= 10 {
        score += 10;
    }
    if candidate.contains(' ') || candidate.contains('_') || candidate.contains('-') {
        score += 5;
    }
    score
}

fn decode_basic_html_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&#38;", "&")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn resolve_html_candidate_url(page_url: &Url, raw: &str) -> Option<String> {
    if raw.starts_with("//") {
        return Some(format!("{}:{raw}", page_url.scheme()));
    }
    if let Ok(parsed) = Url::parse(raw) {
        return matches!(parsed.scheme(), "http" | "https").then(|| parsed.to_string());
    }
    page_url.join(raw).ok().map(|value| value.to_string())
}

fn score_html_download_candidate(page_url: &Url, candidate: &str) -> i32 {
    let Ok(candidate_url) = Url::parse(candidate) else {
        return i32::MIN;
    };
    if !matches!(candidate_url.scheme(), "http" | "https") {
        return i32::MIN;
    }
    if urls_match_after_normalization(page_url.as_str(), candidate) {
        return i32::MIN;
    }

    let mut score = 0_i32;
    if has_file_like_url_hint(candidate) {
        score += 140;
    }
    if best_url_filename_candidate(candidate, true).is_some() {
        score += 30;
    }
    if query_response_content_disposition(candidate).is_some() {
        score += 30;
    }
    if query_response_content_type(candidate).is_some() {
        score += 15;
    }
    if candidate_url.host_str() != page_url.host_str() {
        score += 20;
    }

    let path = candidate_url.path().to_ascii_lowercase();
    if path.contains("/download")
        || path.contains("/direct")
        || path.contains("/dl")
        || path.contains("/file")
        || path.contains("/get")
    {
        score += 40;
    }

    let asset_extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.trim_matches('/').to_ascii_lowercase());
    if matches!(
        asset_extension.as_deref(),
        Some(
            "js" | "css"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "svg"
                | "ico"
                | "woff"
                | "woff2"
                | "map"
                | "json"
        )
    ) {
        score -= 180;
    }
    if path.ends_with(".html") || path.ends_with(".htm") {
        score -= 100;
    }
    if path.contains("/assets/")
        || path.contains("/static/")
        || path.contains("/dist/")
        || path.contains("favicon")
    {
        score -= 100;
    }

    score
}

pub(super) fn urls_match_after_normalization(left: &str, right: &str) -> bool {
    normalize_url_without_fragment(left) == normalize_url_without_fragment(right)
}

fn normalize_url_without_fragment(value: &str) -> String {
    Url::parse(value)
        .map(|mut parsed| {
            parsed.set_fragment(None);
            parsed.to_string()
        })
        .unwrap_or_else(|_| value.trim().to_string())
}

pub(super) fn is_html_interstitial_response(
    final_url: &str,
    mime_type: Option<&str>,
    content_disposition: Option<&str>,
) -> bool {
    is_html_document_mime(mime_type)
        && !content_disposition_signals_download(content_disposition)
        && !has_file_like_url_hint(final_url)
}

fn content_disposition_signals_download(value: Option<&str>) -> bool {
    value.is_some_and(|raw| {
        let disposition = raw.split(';').next().unwrap_or(raw).trim();
        disposition.eq_ignore_ascii_case("attachment")
    }) || extract_filename_from_content_disposition(value).is_some()
}

fn has_file_like_url_hint(url: &str) -> bool {
    best_url_filename_candidate(url, true)
        .or_else(|| best_url_path_candidate(url, true))
        .is_some_and(|candidate| usable_extension(&candidate.name).is_some())
}

#[allow(clippy::match_like_matches_macro)]
fn is_html_document_mime(value: Option<&str>) -> bool {
    let normalized = value
        .map(|raw| {
            raw.split(';')
                .next()
                .unwrap_or(raw)
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|raw| !raw.is_empty());

    match normalized.as_deref() {
        Some("text/html") | Some("application/xhtml+xml") => true,
        _ => false,
    }
}
