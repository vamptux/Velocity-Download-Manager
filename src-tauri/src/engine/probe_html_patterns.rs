use std::sync::LazyLock;

use regex::Regex;

pub(super) static HTML_ATTRIBUTE_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)(?:href|src|data-url|data-href|data-download-url|data-direct-url|data-file-url)\s*=\s*["']([^"'<>]+)["']"#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_JSON_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)"(?:downloadUrl|directLink|contentUrl|fileUrl|downloadLink|url)"\s*:\s*"([^"]+)""#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_SCRIPT_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)(?:downloadUrl|directLink|contentUrl|fileUrl|downloadLink)\s*[:=]\s*["']([^"']+)["']"#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_LOCATION_ASSIGN_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)(?:(?:window|document|top)\.)?location(?:\.href)?\s*=\s*["']([^"']+)["']"#)
        .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_LOCATION_CALL_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)location\.(?:assign|replace)\(\s*["']([^"']+)["']\s*\)"#)
        .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_META_REFRESH_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)<meta[^>]+http-equiv\s*=\s*["']refresh["'][^>]+content\s*=\s*["'][^"']*url=([^"'>]+)["'][^>]*>"#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_META_TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)<meta[^>]+(?:property|name)\s*=\s*["'](?:og:title|twitter:title|title)["'][^>]+content\s*=\s*["']([^"'>]{1,260})["']"#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<title[^>]*>([^<]{1,260})</title>"#).unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_FILENAME_JSON_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)"(?:fileName|filename|file_name|downloadName|download_name|displayName|display_name|suggestedName|name|title)"\s*:\s*"([^"]{1,260})""#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_FILENAME_SCRIPT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)(?:fileName|filename|file_name|downloadName|download_name|displayName|display_name|suggestedName|name|title)\s*[:=]\s*["']([^"']{1,260})["']"#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_FILENAME_ATTR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)(?:data-filename|data-file-name|data-name|download)\s*=\s*["']([^"'<>]{1,260})["']"#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_CONFIG_SCRIPT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)<script[^>]+src\s*=\s*["']([^"'<>]*(?:config|runtime-config|app-config|site-config)\.js[^"'<>]*)["']"#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_APPDATA_API_SERVER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)(?:appdata\.apiServer|(?:["'](?:apiServer|apiBase|apiBaseUrl)["']|(?:apiServer|apiBase|apiBaseUrl)))\s*[:=]\s*["']([^"']+)["']"#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_APPDATA_WT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?is)(?:appdata\.wt|(?:["'](?:wt|websiteToken|websiteTokenSeed)["']|(?:wt|websiteToken|websiteTokenSeed)))\s*[:=]\s*["']([^"']+)["']"#,
    )
    .unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_FORM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<form\b([^>]*)>(.*?)</form>"#).unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_FORM_METHOD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)\bmethod\s*=\s*["']([^"']+)["']"#).unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_FORM_ACTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)\baction\s*=\s*["']([^"']+)["']"#).unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_INPUT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<input\b([^>]*)>"#).unwrap_or_else(|_| unreachable!()));
pub(super) static HTML_INPUT_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)\bname\s*=\s*["']([^"']+)["']"#).unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_INPUT_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)\bvalue\s*=\s*["']([^"']*)["']"#).unwrap_or_else(|_| unreachable!())
});
pub(super) static HTML_INPUT_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)\btype\s*=\s*["']([^"']+)["']"#).unwrap_or_else(|_| unreachable!())
});
