use crate::model::{DownloadRequestField, DownloadRequestMethod};

pub(super) fn sanitize_request_fields(
    fields: Vec<DownloadRequestField>,
) -> Vec<DownloadRequestField> {
    fields
        .into_iter()
        .filter_map(|field| {
            let name = field.name.trim();
            if name.is_empty() {
                return None;
            }

            Some(DownloadRequestField {
                name: name.to_string(),
                value: field.value,
            })
        })
        .collect()
}

pub(super) fn request_context_supports_segmented_transfer(
    request_method: &DownloadRequestMethod,
    request_form_fields: &[DownloadRequestField],
) -> bool {
    matches!(request_method, DownloadRequestMethod::Get) && request_form_fields.is_empty()
}

pub(super) fn apply_request_payload(
    builder: reqwest::RequestBuilder,
    request_method: &DownloadRequestMethod,
    request_form_fields: &[DownloadRequestField],
) -> reqwest::RequestBuilder {
    let payload: Vec<(String, String)> = request_form_fields
        .iter()
        .map(|field| (field.name.clone(), field.value.clone()))
        .collect();

    match request_method {
        DownloadRequestMethod::Get => {
            if payload.is_empty() {
                builder
            } else {
                builder.query(&payload)
            }
        }
        DownloadRequestMethod::Post => {
            if payload.is_empty() {
                builder
            } else {
                builder.form(&payload)
            }
        }
    }
}

pub(super) fn apply_request_referer(
    builder: reqwest::RequestBuilder,
    request_referer: Option<&str>,
) -> reqwest::RequestBuilder {
    let Some(request_referer) = request_referer
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return builder;
    };

    builder.header(reqwest::header::REFERER, request_referer)
}

pub(super) fn apply_request_cookies(
    builder: reqwest::RequestBuilder,
    request_cookies: Option<&str>,
) -> reqwest::RequestBuilder {
    let Some(cookies) = request_cookies
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return builder;
    };

    builder.header(reqwest::header::COOKIE, cookies)
}

pub(super) fn parse_content_range_bounds(value: Option<&str>) -> Option<(u64, u64, Option<u64>)> {
    let raw = value?.trim();
    let range = raw.strip_prefix("bytes ").unwrap_or(raw);
    let (bounds, total) = range.split_once('/')?;
    let (start, end) = bounds.split_once('-')?;
    let total = if total.trim() == "*" {
        None
    } else {
        Some(total.trim().parse::<u64>().ok()?)
    };
    Some((
        start.trim().parse::<u64>().ok()?,
        end.trim().parse::<u64>().ok()?,
        total,
    ))
}

pub(super) fn extract_url_host(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .map(|host| host.trim().to_string())
        .filter(|host| !host.is_empty())
}

pub(super) fn origin_pool_key(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?.trim();
    if host.is_empty() {
        return None;
    }

    match parsed.port() {
        Some(port) => Some(format!("{}://{}:{}", parsed.scheme(), host, port)),
        None => Some(format!("{}://{}", parsed.scheme(), host)),
    }
}
