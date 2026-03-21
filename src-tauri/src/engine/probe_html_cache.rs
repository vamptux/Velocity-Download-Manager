use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const HTML_APP_GUEST_TOKEN_TTL_SECONDS: u64 = 10 * 60;
const HTML_APP_API_BACKOFF_BASE_SECONDS: u64 = 30;
const HTML_APP_API_BACKOFF_MAX_SECONDS: u64 = 5 * 60;

#[derive(Clone, Default)]
struct HtmlAppGuestTokenCacheEntry {
    token: Option<String>,
    token_expires_at: Option<u64>,
    backoff_until: Option<u64>,
    failure_count: u32,
}

static HTML_APP_GUEST_TOKEN_CACHE: LazyLock<Mutex<BTreeMap<String, HtmlAppGuestTokenCacheEntry>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub(super) fn cached_html_app_guest_token(api_base_url: &str) -> Result<Option<String>, String> {
    let now = unix_seconds();
    let cache = HTML_APP_GUEST_TOKEN_CACHE
        .lock()
        .map_err(|_| "Wrapper token cache lock failed.".to_string())?;
    let Some(entry) = cache.get(api_base_url) else {
        return Ok(None);
    };
    if let Some(backoff_until) = entry.backoff_until.filter(|value| *value > now) {
        return Err(format!(
            "Wrapper app API is cooling down after recent failures; retry in {} seconds.",
            backoff_until.saturating_sub(now)
        ));
    }
    Ok(match (&entry.token, entry.token_expires_at) {
        (Some(token), Some(expires_at)) if expires_at > now => Some(token.clone()),
        _ => None,
    })
}

pub(super) fn cache_html_app_guest_token(api_base_url: &str, token: &str) {
    let now = unix_seconds();
    if let Ok(mut cache) = HTML_APP_GUEST_TOKEN_CACHE.lock() {
        let entry = cache.entry(api_base_url.to_string()).or_default();
        entry.token = Some(token.to_string());
        entry.token_expires_at = Some(now.saturating_add(HTML_APP_GUEST_TOKEN_TTL_SECONDS));
        entry.backoff_until = None;
        entry.failure_count = 0;
    }
}

pub(super) fn invalidate_html_app_guest_token(api_base_url: &str) {
    if let Ok(mut cache) = HTML_APP_GUEST_TOKEN_CACHE.lock()
        && let Some(entry) = cache.get_mut(api_base_url)
    {
        entry.token = None;
        entry.token_expires_at = None;
    }
}

pub(super) fn record_html_app_api_failure(api_base_url: &str) {
    let now = unix_seconds();
    if let Ok(mut cache) = HTML_APP_GUEST_TOKEN_CACHE.lock() {
        let entry = cache.entry(api_base_url.to_string()).or_default();
        entry.failure_count = entry.failure_count.saturating_add(1);
        let exponent = entry.failure_count.saturating_sub(1).min(4);
        let backoff_seconds = HTML_APP_API_BACKOFF_BASE_SECONDS
            .saturating_mul(2u64.saturating_pow(exponent))
            .min(HTML_APP_API_BACKOFF_MAX_SECONDS);
        entry.backoff_until = Some(now.saturating_add(backoff_seconds));
        if entry.failure_count > 2 {
            entry.token = None;
            entry.token_expires_at = None;
        }
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
