use std::time::{Duration, Instant};

use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, ACCEPT_RANGES, CACHE_CONTROL,
    CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, LAST_MODIFIED, PRAGMA,
    RANGE,
};

use super::http_helpers::{
    apply_request_cookies, apply_request_payload, apply_request_referer,
    parse_content_range_bounds, request_context_supports_segmented_transfer,
};
use super::probe_filename::{
    clean_mime_type, has_confident_name_hint, query_response_content_disposition,
    query_response_content_type, resolve_suggested_name,
};
use super::probe_html::{
    is_html_interstitial_response, read_html_resolution_hint, resolve_html_app_api_download_url,
    urls_match_after_normalization, HtmlFollowUpMethod, HtmlFollowUpRequest, HtmlResolutionHint,
};
use crate::model::{
    DownloadCompatibility, DownloadRequestField, DownloadRequestMethod, ResumeValidators,
};

const HTTP_REDIRECT_LIMIT: usize = 10;
const PROBE_TIMEOUT_SECONDS: u64 = 12;
const PROBE_CONNECT_TIMEOUT_SECONDS: u64 = 6;
const PROBE_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36";
const PROBE_PRIMARY_LANGUAGE: &str = "en-US";
const PROBE_ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
const PROBE_DOWNLOAD_ACCEPT: &str = "*/*";
const PROBE_NAVIGATION_ACCEPT: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8";
const HTML_WRAPPER_RESOLVE_MAX_HOPS: u8 = 1;
const HTML_WRAPPER_SAME_URL_RETRY_LIMIT: u8 = 1;

#[derive(Clone)]
pub struct DownloadProbeData {
    pub final_url: String,
    pub size: Option<u64>,
    pub mime_type: Option<String>,
    pub negotiated_protocol: Option<String>,
    pub range_supported: bool,
    pub range_observation: RangeObservation,
    pub resumable: bool,
    pub validators: ResumeValidators,
    pub suggested_name: String,
    pub compatibility: DownloadCompatibility,
    pub warnings: Vec<String>,
}

pub(super) struct RuntimeBootstrapProbe {
    pub probe: DownloadProbeData,
    pub reusable_stream: Option<ProbeStreamHandoff>,
}

pub(super) struct ProbeStreamHandoff {
    pub response: reqwest::Response,
    pub ttfb_ms: u64,
    pub negotiated_protocol: Option<String>,
}

#[derive(Clone)]
struct ProbeStageMetadata {
    final_url: String,
    size: Option<u64>,
    mime_type: Option<String>,
    negotiated_protocol: Option<String>,
    range_observation: RangeObservation,
    validators: ResumeValidators,
    html_interstitial: bool,
    html_resolution: Option<HtmlResolutionHint>,
}

struct ProbeStageResult {
    metadata: ProbeStageMetadata,
    reusable_stream: Option<ProbeStreamHandoff>,
}

#[derive(Clone)]
struct ProbeRequestState {
    url: String,
    referer: Option<String>,
    cookies: Option<String>,
    method: DownloadRequestMethod,
    form_fields: Vec<DownloadRequestField>,
    resolved_wrapper_hop: u8,
    resolved_wrapper_page: bool,
    same_url_wrapper_retries: u8,
}

impl ProbeRequestState {
    fn new(
        original_url: &str,
        request_referer: Option<&str>,
        request_cookies: Option<&str>,
        request_method: &DownloadRequestMethod,
        request_form_fields: &[DownloadRequestField],
    ) -> Self {
        Self {
            url: original_url.to_string(),
            referer: request_referer.map(ToString::to_string),
            cookies: request_cookies.map(ToString::to_string),
            method: request_method.clone(),
            form_fields: request_form_fields.to_vec(),
            resolved_wrapper_hop: 0,
            resolved_wrapper_page: false,
            same_url_wrapper_retries: 0,
        }
    }

    fn exact_request_shape_allows_segmentation(&self) -> bool {
        request_context_supports_segmented_transfer(&self.method, self.form_fields.as_slice())
    }

    fn can_resolve_wrapper_hop(&self) -> bool {
        self.resolved_wrapper_hop < HTML_WRAPPER_RESOLVE_MAX_HOPS
    }

    fn retarget_to_direct_download(&mut self, url: &str, referer: String) {
        self.url = url.to_string();
        self.referer = Some(referer);
        self.method = DownloadRequestMethod::Get;
        self.form_fields.clear();
        self.resolved_wrapper_hop = self.resolved_wrapper_hop.saturating_add(1);
        self.resolved_wrapper_page = true;
    }

    fn retry_same_url_wrapper(&mut self, referer: String) {
        self.referer = Some(referer);
        self.method = DownloadRequestMethod::Get;
        self.form_fields.clear();
        self.same_url_wrapper_retries = self.same_url_wrapper_retries.saturating_add(1);
    }
}

struct ProbeAttemptState {
    failures: Vec<String>,
    app_api_resolution_failure: Option<String>,
    head_stage: Option<ProbeStageResult>,
    range_stage: Option<ProbeStageResult>,
    standard_get_stage: Option<ProbeStageResult>,
}

impl ProbeAttemptState {
    fn ordered_stage_results(&self) -> [Option<&ProbeStageResult>; 3] {
        [
            self.range_stage.as_ref(),
            self.standard_get_stage.as_ref(),
            self.head_stage.as_ref(),
        ]
    }

    fn ordered_stages(&self) -> [Option<&ProbeStageMetadata>; 3] {
        [
            self.range_stage.as_ref().map(|stage| &stage.metadata),
            self.standard_get_stage.as_ref().map(|stage| &stage.metadata),
            self.head_stage.as_ref().map(|stage| &stage.metadata),
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProbeMethod {
    Head,
    RangeGet,
    StandardGet,
    FormPost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeObservation {
    Supported,
    Unsupported,
    Unknown,
}

pub(super) fn configure_http_client_builder(
    builder: reqwest::ClientBuilder,
) -> reqwest::ClientBuilder {
    let mut default_headers = HeaderMap::new();
    default_headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static(PROBE_ACCEPT_LANGUAGE),
    );
    default_headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    default_headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));

    builder
        .redirect(reqwest::redirect::Policy::limited(HTTP_REDIRECT_LIMIT))
        .default_headers(default_headers)
        .user_agent(PROBE_USER_AGENT)
        .cookie_store(true)
        .referer(true)
}

pub(super) async fn probe_download_headers_with_context(
    url: &str,
    request_referer: Option<&str>,
    request_cookies: Option<&str>,
    request_method: &DownloadRequestMethod,
    request_form_fields: &[DownloadRequestField],
) -> Result<DownloadProbeData, String> {
    let client = configure_http_client_builder(reqwest::Client::builder())
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECONDS))
        .connect_timeout(Duration::from_secs(PROBE_CONNECT_TIMEOUT_SECONDS))
        .build()
        .map_err(|error| format!("Failed to create probe client: {error}"))?;

    Ok(probe_download_headers_internal(
        &client,
        url,
        request_referer,
        request_cookies,
        request_method,
        request_form_fields,
        false,
    )
    .await?
    .probe)
}

pub(super) async fn probe_runtime_bootstrap_with_client(
    client: &reqwest::Client,
    original_url: &str,
    request_referer: Option<&str>,
    request_cookies: Option<&str>,
    request_method: &DownloadRequestMethod,
    request_form_fields: &[DownloadRequestField],
    capture_stream: bool,
) -> Result<RuntimeBootstrapProbe, String> {
    probe_download_headers_internal(
        client,
        original_url,
        request_referer,
        request_cookies,
        request_method,
        request_form_fields,
        capture_stream,
    )
    .await
}

async fn probe_download_headers_internal(
    client: &reqwest::Client,
    original_url: &str,
    request_referer: Option<&str>,
    request_cookies: Option<&str>,
    request_method: &DownloadRequestMethod,
    request_form_fields: &[DownloadRequestField],
    capture_stream: bool,
) -> Result<RuntimeBootstrapProbe, String> {
    let mut request_state = ProbeRequestState::new(
        original_url,
        request_referer,
        request_cookies,
        request_method,
        request_form_fields,
    );

    loop {
        let mut attempts = run_probe_attempts(client, &request_state, capture_stream).await;
        if attempts.ordered_stage_results().iter().all(Option::is_none) {
            return Err(attempts.failures.into_iter().next().unwrap_or_else(|| {
                "Probe request failed before metadata could be collected.".to_string()
            }));
        }

        if let Some((follow_up_referer, follow_up_request)) =
            select_html_follow_up_candidate(&attempts, request_state.can_resolve_wrapper_hop())
        {
            let follow_up_method = request_method_from_html_follow_up(follow_up_request.method);
            let follow_up_fields = request_fields_from_pairs(&follow_up_request.fields);
            match send_html_follow_up_request(
                client,
                &follow_up_request,
                Some(follow_up_referer.as_str()),
                request_state.cookies.as_deref(),
            )
            .await
            {
                Ok(stage) => {
                    request_state.method = follow_up_method;
                    request_state.form_fields = follow_up_fields;
                    request_state.referer = Some(follow_up_referer.clone());
                    if !stage.metadata.html_interstitial {
                        request_state.resolved_wrapper_page = true;
                    }
                    if !stage.metadata.html_interstitial
                        && !urls_match_after_normalization(&request_state.url, &stage.metadata.final_url)
                    {
                        request_state.retarget_to_direct_download(
                            &stage.metadata.final_url,
                            follow_up_referer,
                        );
                        continue;
                    }
                    attempts.standard_get_stage = Some(stage);
                }
                Err(error) => attempts.failures.push(error),
            }
        }

        if let Some(hint) = attempts
            .ordered_stage_results()
            .iter()
            .flatten()
            .find_map(|stage| stage.metadata.html_resolution.as_ref())
            .cloned()
        {
            if request_state.can_resolve_wrapper_hop() {
                if let Some(direct_download_url) = hint.direct_download_url.as_deref() {
                    if !urls_match_after_normalization(&request_state.url, direct_download_url) {
                        request_state
                            .retarget_to_direct_download(direct_download_url, hint.request_referer.clone());
                        continue;
                    }
                }
            }

            let all_stages_html = attempts
                .ordered_stages()
                .iter()
                .flatten()
                .all(|stage| stage.html_interstitial);
            if request_state.can_resolve_wrapper_hop()
                && all_stages_html
                && hint.direct_download_url.is_none()
            {
                if let Some(app_api_hint) = hint.app_api.as_ref() {
                    let api_resolution_result = resolve_html_app_api_download_url(
                        client,
                        &request_state.url,
                        app_api_hint,
                        PROBE_USER_AGENT,
                        PROBE_PRIMARY_LANGUAGE,
                        PROBE_ACCEPT_LANGUAGE,
                    )
                    .await;
                    if let Err(error) = api_resolution_result {
                        attempts.app_api_resolution_failure = Some(error);
                    } else if let Ok(Some(api_resolution)) = api_resolution_result {
                        let _ = api_resolution.suggested_name.as_deref();
                        if api_resolution.request_cookies.is_some() {
                            request_state.cookies = api_resolution.request_cookies.clone();
                        }

                        if !urls_match_after_normalization(
                            &request_state.url,
                            &api_resolution.direct_download_url,
                        ) {
                            request_state.retarget_to_direct_download(
                                &api_resolution.direct_download_url,
                                hint.request_referer.clone(),
                            );
                            continue;
                        }
                    }
                }
            }

            let same_url_wrapper_hint = hint.suggested_name.is_some()
                || hint
                    .direct_download_url
                    .as_deref()
                    .is_some_and(|candidate| {
                        urls_match_after_normalization(&request_state.url, candidate)
                    });
            if all_stages_html
                && same_url_wrapper_hint
                && request_state.same_url_wrapper_retries < HTML_WRAPPER_SAME_URL_RETRY_LIMIT
            {
                request_state.retry_same_url_wrapper(hint.request_referer.clone());
                continue;
            }
        }

        return Ok(finalize_probe_result(
            original_url,
            &request_state,
            attempts,
            capture_stream,
        ));
    }
}

async fn run_probe_attempts(
    client: &reqwest::Client,
    request_state: &ProbeRequestState,
    capture_stream: bool,
) -> ProbeAttemptState {
    let mut failures = Vec::new();
    let exact_request_shape_allows_segmentation = request_state.exact_request_shape_allows_segmentation();
    let head_stage = if exact_request_shape_allows_segmentation {
        match send_probe_request(
            client,
            &request_state.url,
            ProbeMethod::Head,
            request_state.referer.as_deref(),
            request_state.cookies.as_deref(),
            &[][..],
            false,
        )
        .await
        {
            Ok(stage) => Some(stage),
            Err(error) => {
                failures.push(error);
                None
            }
        }
    } else {
        None
    };

    let range_stage = if exact_request_shape_allows_segmentation
        && should_try_range_probe(
            &request_state.url,
            head_stage.as_ref().map(|stage| &stage.metadata),
        )
    {
        match send_probe_request(
            client,
            &request_state.url,
            ProbeMethod::RangeGet,
            request_state.referer.as_deref(),
            request_state.cookies.as_deref(),
            &[][..],
            false,
        )
        .await
        {
            Ok(stage) => Some(stage),
            Err(error) => {
                failures.push(error);
                None
            }
        }
    } else {
        None
    };

    let standard_get_stage = if exact_request_shape_allows_segmentation {
        if should_try_standard_probe(
            head_stage.as_ref().map(|stage| &stage.metadata),
            range_stage.as_ref().map(|stage| &stage.metadata),
        ) {
            match send_probe_request(
                client,
                &request_state.url,
                ProbeMethod::StandardGet,
                request_state.referer.as_deref(),
                request_state.cookies.as_deref(),
                &[][..],
                capture_stream,
            )
            .await
            {
                Ok(stage) => Some(stage),
                Err(error) => {
                    failures.push(error);
                    None
                }
            }
        } else {
            None
        }
    } else {
        match send_probe_request(
            client,
            &request_state.url,
            probe_method_for_request_context(&request_state.method),
            request_state.referer.as_deref(),
            request_state.cookies.as_deref(),
            request_state.form_fields.as_slice(),
            capture_stream,
        )
        .await
        {
            Ok(stage) => Some(stage),
            Err(error) => {
                failures.push(error);
                None
            }
        }
    };

    ProbeAttemptState {
        failures,
        app_api_resolution_failure: None,
        head_stage,
        range_stage,
        standard_get_stage,
    }
}

fn select_html_follow_up_candidate(
    attempts: &ProbeAttemptState,
    can_resolve_wrapper_hop: bool,
) -> Option<(String, HtmlFollowUpRequest)> {
    let ordered_stage_results = attempts.ordered_stage_results();
    let all_stages_html = ordered_stage_results.iter().flatten().next().is_some()
        && ordered_stage_results
            .iter()
            .flatten()
            .all(|stage| stage.metadata.html_interstitial);
    if !can_resolve_wrapper_hop || !all_stages_html {
        return None;
    }
    ordered_stage_results.iter().flatten().find_map(|stage| {
        stage.metadata.html_resolution.as_ref().and_then(|hint| {
            hint.follow_up_request
                .as_ref()
                .map(|request| (hint.request_referer.clone(), request.clone()))
        })
    })
}

fn finalize_probe_result(
    original_url: &str,
    request_state: &ProbeRequestState,
    attempts: ProbeAttemptState,
    capture_stream: bool,
) -> RuntimeBootstrapProbe {
    let ordered_stages = attempts.ordered_stages();
    let ordered_stage_results = attempts.ordered_stage_results();
    let metadata_stages = [
        attempts
            .range_stage
            .as_ref()
            .map(|stage| &stage.metadata)
            .filter(|stage| !stage.html_interstitial),
        attempts
            .standard_get_stage
            .as_ref()
            .map(|stage| &stage.metadata)
            .filter(|stage| !stage.html_interstitial),
        attempts
            .head_stage
            .as_ref()
            .map(|stage| &stage.metadata)
            .filter(|stage| !stage.html_interstitial),
    ];
    let preferred_metadata_stages = if metadata_stages.iter().any(Option::is_some) {
        metadata_stages
    } else {
        [None, None, None]
    };

    let html_name_hint = ordered_stage_results.iter().flatten().find_map(|stage| {
        stage
            .metadata
            .html_resolution
            .as_ref()
            .and_then(|hint| hint.suggested_name.clone())
    });
    let final_url = ordered_stages
        .iter()
        .find_map(|stage| stage.map(|value| value.final_url.clone()))
        .unwrap_or_else(|| request_state.url.clone());
    let size = preferred_metadata_stages
        .iter()
        .find_map(|stage| stage.and_then(|value| value.size));
    let header_mime_type = preferred_metadata_stages.iter().find_map(|stage| {
        stage
            .and_then(|value| value.mime_type.as_deref())
            .and_then(|value| clean_mime_type(Some(value)))
    });
    let query_mime_type = query_response_content_type(&final_url)
        .or_else(|| query_response_content_type(original_url))
        .and_then(|value| clean_mime_type(Some(value.as_str())));
    let validator_content_type = header_mime_type.clone();
    let mime_type = validator_content_type.clone().or(query_mime_type);
    let content_disposition = preferred_metadata_stages
        .iter()
        .find_map(|stage| stage.and_then(|value| value.validators.content_disposition.clone()))
        .or_else(|| query_response_content_disposition(&final_url))
        .or_else(|| query_response_content_disposition(original_url));
    let mut range_observation = if preferred_metadata_stages.iter().any(Option::is_some) {
        strongest_range_observation(&preferred_metadata_stages)
    } else {
        RangeObservation::Unknown
    };
    let validators = ResumeValidators {
        etag: preferred_metadata_stages
            .iter()
            .find_map(|stage| stage.and_then(|value| value.validators.etag.clone())),
        last_modified: preferred_metadata_stages
            .iter()
            .find_map(|stage| stage.and_then(|value| value.validators.last_modified.clone())),
        content_length: size,
        content_type: validator_content_type,
        content_disposition: content_disposition.clone(),
    };
    let negotiated_protocol = ordered_stages
        .iter()
        .find_map(|stage| stage.and_then(|value| value.negotiated_protocol.clone()));
    let (suggested_name, mut compatibility, mut warnings) = resolve_suggested_name(
        original_url,
        &final_url,
        content_disposition.as_deref(),
        mime_type.as_deref(),
        html_name_hint.as_deref(),
    );

    let saw_html_interstitial = ordered_stages
        .iter()
        .flatten()
        .any(|stage| stage.html_interstitial);
    let only_html_interstitial = ordered_stages.iter().flatten().next().is_some()
        && ordered_stages
            .iter()
            .flatten()
            .all(|stage| stage.html_interstitial);
    let recovered_wrapper_response = request_state.resolved_wrapper_page
        || (request_state.same_url_wrapper_retries > 0 && !only_html_interstitial);

    compatibility.wrapper_detected = saw_html_interstitial;
    compatibility.direct_url_recovered = recovered_wrapper_response;
    compatibility.browser_interstitial_only = only_html_interstitial;
    compatibility.request_referer = request_state.referer.clone();
    compatibility.request_cookies = request_state.cookies.clone();
    compatibility.request_method = request_state.method.clone();
    compatibility.request_form_fields = request_state.form_fields.clone();

    let exact_request_shape_allows_segmentation = request_context_supports_segmented_transfer(
        &compatibility.request_method,
        &compatibility.request_form_fields,
    );
    if !exact_request_shape_allows_segmentation {
        range_observation = RangeObservation::Unknown;
    }
    let range_supported =
        matches!(range_observation, RangeObservation::Supported) && exact_request_shape_allows_segmentation;
    let resumable = range_supported && size.is_some_and(|value| value > 0);

    if recovered_wrapper_response {
        warnings.push(
            "Probe followed a browser wrapper page and recovered a direct download response."
                .to_string(),
        );
    }

    if only_html_interstitial {
        if html_name_hint.is_some() {
            warnings.push(
                "Probe only reached a browser wrapper page; VDM recovered a filename hint but still needs the transfer bootstrap to stabilize the final binary response."
                    .to_string(),
            );
        } else if attempts.app_api_resolution_failure.is_some() {
            warnings.push(
                "Probe detected an app-backed wrapper page, but its public resolver did not yield a stable direct file during metadata discovery. VDM will retry during transfer."
                    .to_string(),
            );
        } else {
            warnings.push(
                "Probe only reached a browser wrapper page and could not recover a stable direct-download response. VDM will retry during transfer, but some hosts require the final file URL instead of the share page."
                    .to_string(),
            );
        }
    } else if saw_html_interstitial {
        warnings.push(
            "One probe stage returned an HTML wrapper page; VDM ignored that response and kept safer metadata hints."
                .to_string(),
        );
    }

    if size.is_none() {
        warnings.push(
            "Remote host did not advertise a stable file size during probe; VDM will discover it while downloading."
                .to_string(),
        );
    }

    if !exact_request_shape_allows_segmentation {
        warnings.push(
            "This download requires a replayed POST or form request; VDM will keep it on guarded single-stream mode until byte-range support is proven on that exact request shape."
                .to_string(),
        );
    }

    if original_url != final_url {
        warnings.push("Probe redirected to a different final URL during metadata discovery.".to_string());
    }

    RuntimeBootstrapProbe {
        probe: DownloadProbeData {
            final_url,
            size,
            mime_type,
            negotiated_protocol,
            range_supported,
            range_observation,
            resumable,
            validators,
            suggested_name,
            compatibility,
            warnings,
        },
        reusable_stream: if capture_stream {
            attempts
                .standard_get_stage
                .and_then(|stage| stage.reusable_stream)
        } else {
            None
        },
    }
}

async fn send_probe_request(
    client: &reqwest::Client,
    url: &str,
    method: ProbeMethod,
    request_referer: Option<&str>,
    request_cookies: Option<&str>,
    request_form_fields: &[DownloadRequestField],
    capture_stream: bool,
) -> Result<ProbeStageResult, String> {
    let request_started = Instant::now();
    let request = match method {
        ProbeMethod::Head => client.head(url),
        ProbeMethod::RangeGet => client.get(url).header(RANGE, "bytes=0-0"),
        ProbeMethod::StandardGet => client.get(url),
        ProbeMethod::FormPost => client.post(url),
    };
    let response = apply_request_cookies(
        apply_request_referer(
            apply_request_payload(
                apply_probe_request_profile(request, method),
                &request_method_for_probe_method(method),
                request_form_fields,
            ),
            request_referer,
        ),
        request_cookies,
    )
    .send()
    .await
    .map_err(|error| {
        format!(
            "{} probe request failed: {error}",
            probe_method_label(method)
        )
    })?;

    let status_code = response.status().as_u16();
    if !response.status().is_success() {
        return Err(format!(
            "{} probe request returned HTTP {}.",
            probe_method_label(method),
            status_code
        ));
    }

    let ttfb_ms = request_started.elapsed().as_millis().min(u64::MAX as u128) as u64;

    Ok(parse_probe_stage(
        response,
        matches!(method, ProbeMethod::RangeGet),
        method,
        capture_stream,
        ttfb_ms,
    )
    .await)
}

async fn send_html_follow_up_request(
    client: &reqwest::Client,
    request: &HtmlFollowUpRequest,
    request_referer: Option<&str>,
    request_cookies: Option<&str>,
) -> Result<ProbeStageResult, String> {
    let request_started = Instant::now();
    let probe_method = match request.method {
        HtmlFollowUpMethod::Get => ProbeMethod::StandardGet,
        HtmlFollowUpMethod::Post => ProbeMethod::FormPost,
    };
    let builder = match request.method {
        HtmlFollowUpMethod::Get => {
            let builder = client.get(&request.url);
            if request.fields.is_empty() {
                builder
            } else {
                builder.query(&request.fields)
            }
        }
        HtmlFollowUpMethod::Post => client.post(&request.url).form(&request.fields),
    };
    let response = apply_request_cookies(
        apply_request_referer(
            apply_probe_request_profile(builder, probe_method),
            request_referer,
        ),
        request_cookies,
    )
    .send()
    .await
    .map_err(|error| {
        format!(
            "{} probe request failed: {error}",
            probe_method_label(probe_method)
        )
    })?;

    let status_code = response.status().as_u16();
    if !response.status().is_success() {
        return Err(format!(
            "{} probe request returned HTTP {}.",
            probe_method_label(probe_method),
            status_code
        ));
    }

    let ttfb_ms = request_started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    Ok(parse_probe_stage(response, false, probe_method, false, ttfb_ms).await)
}

fn apply_probe_request_profile(
    builder: reqwest::RequestBuilder,
    method: ProbeMethod,
) -> reqwest::RequestBuilder {
    match method {
        ProbeMethod::Head | ProbeMethod::RangeGet => builder.header(ACCEPT, PROBE_DOWNLOAD_ACCEPT),
        ProbeMethod::StandardGet | ProbeMethod::FormPost => {
            builder.header(ACCEPT, PROBE_NAVIGATION_ACCEPT)
        }
    }
}

async fn parse_probe_stage(
    mut response: reqwest::Response,
    used_range_request: bool,
    method: ProbeMethod,
    capture_stream: bool,
    ttfb_ms: u64,
) -> ProbeStageResult {
    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let version = response.version();
    let headers = response.headers();
    let accept_ranges = headers
        .get(ACCEPT_RANGES)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .map(|value| value.to_ascii_lowercase());
    let content_range = headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .map(ToString::to_string);
    let mime_type = clean_mime_type(header_to_string(headers.get(CONTENT_TYPE)).as_deref());
    let content_disposition = header_to_string(headers.get(CONTENT_DISPOSITION));
    let size = parse_total_size_from_content_range(content_range.as_deref()).or_else(|| {
        headers
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
    });
    let html_interstitial = is_html_interstitial_response(
        &final_url,
        mime_type.as_deref(),
        content_disposition.as_deref(),
    );
    let validators = ResumeValidators {
        etag: header_to_string(headers.get(ETAG)),
        last_modified: header_to_string(headers.get(LAST_MODIFIED)),
        content_length: size,
        content_type: mime_type.clone(),
        content_disposition: content_disposition.clone(),
    };
    let html_resolution = if html_interstitial && !matches!(method, ProbeMethod::Head) {
        read_html_resolution_hint(&final_url, &mut response).await
    } else {
        None
    };
    let negotiated_protocol = Some(protocol_label(version).to_string());

    ProbeStageResult {
        metadata: ProbeStageMetadata {
            final_url,
            size,
            mime_type,
            negotiated_protocol: negotiated_protocol.clone(),
            range_observation: classify_range_observation(
                status,
                used_range_request,
                accept_ranges.as_deref(),
                content_range.as_deref(),
            ),
            validators,
            html_interstitial,
            html_resolution,
        },
        reusable_stream: if capture_stream
            && matches!(method, ProbeMethod::StandardGet | ProbeMethod::FormPost)
            && !html_interstitial
        {
            Some(ProbeStreamHandoff {
                response,
                ttfb_ms,
                negotiated_protocol,
            })
        } else {
            None
        },
    }
}

fn probe_method_for_request_context(request_method: &DownloadRequestMethod) -> ProbeMethod {
    match request_method {
        DownloadRequestMethod::Get => ProbeMethod::StandardGet,
        DownloadRequestMethod::Post => ProbeMethod::FormPost,
    }
}

fn request_method_for_probe_method(method: ProbeMethod) -> DownloadRequestMethod {
    match method {
        ProbeMethod::FormPost => DownloadRequestMethod::Post,
        ProbeMethod::Head | ProbeMethod::RangeGet | ProbeMethod::StandardGet => {
            DownloadRequestMethod::Get
        }
    }
}

fn request_method_from_html_follow_up(method: HtmlFollowUpMethod) -> DownloadRequestMethod {
    match method {
        HtmlFollowUpMethod::Get => DownloadRequestMethod::Get,
        HtmlFollowUpMethod::Post => DownloadRequestMethod::Post,
    }
}

fn request_fields_from_pairs(fields: &[(String, String)]) -> Vec<DownloadRequestField> {
    fields
        .iter()
        .map(|(name, value)| DownloadRequestField {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}

fn should_try_range_probe(url: &str, head_stage: Option<&ProbeStageMetadata>) -> bool {
    let Some(stage) = head_stage else {
        return true;
    };
    stage.size.is_none()
        || stage.range_observation == RangeObservation::Unknown
        || !has_confident_name_hint(
            url,
            &stage.final_url,
            stage.validators.content_disposition.as_deref(),
        )
}

fn should_try_standard_probe(
    head_stage: Option<&ProbeStageMetadata>,
    range_stage: Option<&ProbeStageMetadata>,
) -> bool {
    let stages = [range_stage, head_stage];
    if stages.iter().all(Option::is_none) {
        return true;
    }
    !stages
        .iter()
        .flatten()
        .any(|stage| !stage.html_interstitial)
}

fn strongest_range_observation(stages: &[Option<&ProbeStageMetadata>; 3]) -> RangeObservation {
    if stages.iter().any(|stage| {
        stage.is_some_and(|value| value.range_observation == RangeObservation::Supported)
    }) {
        return RangeObservation::Supported;
    }
    if stages.iter().any(|stage| {
        stage.is_some_and(|value| value.range_observation == RangeObservation::Unsupported)
    }) {
        return RangeObservation::Unsupported;
    }
    RangeObservation::Unknown
}

fn header_to_string(value: Option<&reqwest::header::HeaderValue>) -> Option<String> {
    value
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn classify_range_observation(
    status: u16,
    used_range_request: bool,
    accept_ranges: Option<&str>,
    content_range: Option<&str>,
) -> RangeObservation {
    if status == 206 {
        return RangeObservation::Supported;
    }
    if content_range.is_some() {
        return RangeObservation::Supported;
    }
    if used_range_request && status == 200 {
        return RangeObservation::Unsupported;
    }
    if matches!(accept_ranges, Some("none")) {
        return RangeObservation::Unsupported;
    }
    RangeObservation::Unknown
}

fn parse_total_size_from_content_range(value: Option<&str>) -> Option<u64> {
    parse_content_range_bounds(value).and_then(|(_, _, total)| total)
}

fn probe_method_label(method: ProbeMethod) -> &'static str {
    match method {
        ProbeMethod::Head => "HEAD",
        ProbeMethod::RangeGet => "Range GET",
        ProbeMethod::StandardGet => "GET",
        ProbeMethod::FormPost => "Form POST",
    }
}

pub fn protocol_label(version: reqwest::Version) -> &'static str {
    match version {
        reqwest::Version::HTTP_3 => "http3",
        reqwest::Version::HTTP_2 => "http2",
        reqwest::Version::HTTP_11 => "http1.1",
        reqwest::Version::HTTP_10 => "http1.0",
        reqwest::Version::HTTP_09 => "http0.9",
        _ => "http-unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::super::probe_html::{
        extract_html_direct_download_url, extract_html_filename_hint,
        extract_html_follow_up_request, is_html_interstitial_response, HtmlFollowUpMethod,
    };
    use super::{
        classify_range_observation, should_try_standard_probe, ProbeStageMetadata, RangeObservation,
    };
    use crate::model::ResumeValidators;

    fn html_stage(final_url: &str) -> ProbeStageMetadata {
        ProbeStageMetadata {
            final_url: final_url.to_string(),
            size: None,
            mime_type: Some("text/html".to_string()),
            negotiated_protocol: Some("http2".to_string()),
            range_observation: RangeObservation::Unknown,
            validators: ResumeValidators::default(),
            html_interstitial: true,
            html_resolution: None,
        }
    }

    #[test]
    fn flags_generic_html_wrapper_responses() {
        assert!(is_html_interstitial_response(
            "https://example.com/download?id=42",
            Some("text/html; charset=utf-8"),
            None,
        ));
    }

    #[test]
    fn allows_named_html_file_responses() {
        assert!(!is_html_interstitial_response(
            "https://example.com/help/manual.html",
            Some("text/html"),
            None,
        ));
    }

    #[test]
    fn standard_probe_runs_after_html_only_preflight() {
        assert!(should_try_standard_probe(
            Some(&html_stage("https://example.com/download")),
            None,
        ));
    }

    #[test]
    fn accept_ranges_header_without_confirmed_range_response_stays_unknown() {
        assert_eq!(
            classify_range_observation(200, false, Some("bytes"), None),
            RangeObservation::Unknown
        );
    }

    #[test]
    fn range_get_200_overrides_accept_ranges_header_as_unsupported() {
        assert_eq!(
            classify_range_observation(200, true, Some("bytes"), None),
            RangeObservation::Unsupported
        );
    }

    #[test]
    fn extracts_direct_download_url_from_html_wrapper() {
        let html = r#"
                        <html>
                            <head>
                                <script>
                                    window.directLink = "https:\/\/cdn.example.com\/releases\/toolkit.zip";
                                </script>
                            </head>
                            <body>
                                <a href="/dist/app.js">app</a>
                                <a href="/download?id=42">Download</a>
                            </body>
                        </html>
                "#;

        assert_eq!(
            extract_html_direct_download_url("https://example.com/d/42", html).as_deref(),
            Some("https://cdn.example.com/releases/toolkit.zip")
        );
    }

    #[test]
    fn extracts_location_redirect_from_html_wrapper() {
        let html = r#"
                        <html>
                            <head>
                                <script>
                                    window.location.href = "https://cdn.example.com/files/toolkit.exe";
                                </script>
                            </head>
                        </html>
                "#;

        assert_eq!(
            extract_html_direct_download_url("https://example.com/share/toolkit", html).as_deref(),
            Some("https://cdn.example.com/files/toolkit.exe")
        );
    }

    #[test]
    fn extracts_post_form_follow_up_from_html_wrapper() {
        let html = r#"
                        <html>
                            <body>
                                <form method="post" action="/download?file=archive.part01.rar&amp;token=abc123">
                                    <input type="hidden" name="op" value="download2" />
                                    <input type="hidden" name="id" value="abc123" />
                                    <button type="submit">Continue</button>
                                </form>
                            </body>
                        </html>
                "#;

        let request = extract_html_follow_up_request("https://example.com/share/abc", html)
            .expect("expected follow-up request");

        assert_eq!(
            request.url,
            "https://example.com/download?file=archive.part01.rar&token=abc123"
        );
        assert_eq!(request.method, HtmlFollowUpMethod::Post);
        assert_eq!(
            request.fields,
            vec![
                ("op".to_string(), "download2".to_string()),
                ("id".to_string(), "abc123".to_string()),
            ]
        );
    }

    #[test]
    fn ignores_static_assets_when_extracting_wrapper_link() {
        let html = r#"
                        <html>
                            <body>
                                <a href="/dist/app.js">script</a>
                                <a href="/assets/logo.png">logo</a>
                                <a href="/download?file=archive.part01.rar">Download</a>
                            </body>
                        </html>
                "#;

        assert_eq!(
            extract_html_direct_download_url("https://example.com/share/abc", html).as_deref(),
            Some("https://example.com/download?file=archive.part01.rar")
        );
    }

    #[test]
    fn extracts_filename_hint_from_html_wrapper_title() {
        let html = r#"
                        <html>
                            <head>
                                <title>Human_Fall_Flat_Build_01152026.rar - Gofile</title>
                            </head>
                        </html>
                "#;

        assert_eq!(
            extract_html_filename_hint(html).as_deref(),
            Some("Human_Fall_Flat_Build_01152026.rar")
        );
    }
}
