use reqwest::Url;

use super::filename_policy::{decode_percent_encoded, sanitize_filename_leaf};
use crate::model::DownloadCompatibility;

const FALLBACK_DOWNLOAD_NAME: &str = "download";
const QUERY_FILENAME_KEYS: [&str; 6] = [
    "filename",
    "file",
    "download",
    "attachment",
    "name",
    "title",
];

#[derive(Clone)]
pub(super) struct FilenameCandidate {
    pub(super) name: String,
    source: &'static str,
    classification: &'static str,
    generic: bool,
}

pub(super) fn clean_mime_type(value: Option<&str>) -> Option<String> {
    value
        .map(|raw| {
            raw.split(';')
                .next()
                .unwrap_or(raw)
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|raw| !raw.is_empty())
}

pub(super) fn has_confident_name_hint(
    original_url: &str,
    final_url: &str,
    content_disposition: Option<&str>,
) -> bool {
    if extract_filename_from_content_disposition(content_disposition).is_some() {
        return true;
    }
    best_url_filename_candidate(final_url, true)
        .or_else(|| best_url_filename_candidate(original_url, false))
        .is_some_and(|candidate| !candidate.generic)
}

pub(super) fn resolve_suggested_name(
    original_url: &str,
    final_url: &str,
    content_disposition: Option<&str>,
    mime_type: Option<&str>,
    html_hint_name: Option<&str>,
) -> (String, DownloadCompatibility, Vec<String>) {
    let mut warnings = Vec::new();
    let header_candidate =
        extract_filename_from_content_disposition(content_disposition).map(|name| {
            FilenameCandidate {
                name,
                source: "content-disposition",
                classification: "header-derived",
                generic: false,
            }
        });
    let query_candidate = choose_preferred_candidate(
        best_url_filename_candidate(final_url, true),
        best_url_filename_candidate(original_url, false),
    );
    let path_candidate = choose_preferred_candidate(
        best_url_path_candidate(final_url, true),
        best_url_path_candidate(original_url, false),
    );
    let html_candidate = html_hint_name.and_then(sanitize_filename_leaf).map(|name| {
        let generic = is_generic_route_candidate(&name);
        FilenameCandidate {
            name,
            source: "html-wrapper",
            classification: "wrapper-derived",
            generic,
        }
    });
    let specific_path_candidate = path_candidate
        .clone()
        .filter(|candidate| !candidate.generic && usable_extension(&candidate.name).is_some());

    let mut primary = header_candidate
        .clone()
        .or_else(|| query_candidate.clone())
        .or(specific_path_candidate)
        .or_else(|| html_candidate.clone())
        .or_else(|| path_candidate.clone())
        .unwrap_or(FilenameCandidate {
            name: FALLBACK_DOWNLOAD_NAME.to_string(),
            source: "fallback",
            classification: "fallback",
            generic: true,
        });

    if primary.generic && header_candidate.is_none() && html_candidate.is_none() {
        warnings.push(
            "Filename came from a generic download endpoint, so the final file name was inferred conservatively."
                .to_string(),
        );
    }

    if !has_usable_extension(&primary.name) {
        if let Some(extension) = header_candidate
            .iter()
            .chain(query_candidate.iter())
            .chain(html_candidate.iter())
            .chain(path_candidate.iter())
            .filter_map(|candidate| usable_extension(&candidate.name))
            .next()
        {
            primary.name = append_extension(&primary.name, extension);
            primary.classification = "url-extended";
        } else if let Some(extension) = preferred_extension_from_mime(mime_type) {
            primary.name = append_extension(&primary.name, extension);
            primary.classification = "mime-extended";
            if let Some(mime) = mime_type {
                warnings.push(format!(
                    "File extension was inferred from the reported content type ({mime})."
                ));
            }
        } else if primary.source == "fallback" {
            primary.name = format!("{FALLBACK_DOWNLOAD_NAME}.bin");
            warnings.push(
                "Remote host did not advertise a usable file name or extension; using a generic fallback."
                    .to_string(),
            );
        }
    }

    let redirect_chain = if original_url != final_url {
        vec![original_url.to_string(), final_url.to_string()]
    } else {
        Vec::new()
    };

    (
        primary.name,
        DownloadCompatibility {
            redirect_chain,
            filename_source: Some(primary.source.to_string()),
            classification: Some(primary.classification.to_string()),
            wrapper_detected: false,
            direct_url_recovered: false,
            browser_interstitial_only: false,
            request_referer: None,
            request_cookies: None,
            request_method: Default::default(),
            request_form_fields: Vec::new(),
        },
        warnings,
    )
}

pub(super) fn best_url_filename_candidate(
    url: &str,
    is_final_url: bool,
) -> Option<FilenameCandidate> {
    let parsed = Url::parse(url).ok()?;

    if let Some(response_content_disposition) = parsed.query_pairs().find_map(|(key, value)| {
        if key
            .as_ref()
            .eq_ignore_ascii_case("response-content-disposition")
        {
            extract_filename_from_content_disposition(Some(value.as_ref()))
        } else {
            None
        }
    }) {
        return Some(FilenameCandidate {
            name: response_content_disposition,
            source: if is_final_url {
                "response-content-disposition"
            } else {
                "original-response-content-disposition"
            },
            classification: "header-derived",
            generic: false,
        });
    }

    for key in QUERY_FILENAME_KEYS {
        if let Some(value) = parsed
            .query_pairs()
            .find_map(|(candidate_key, candidate_value)| {
                if candidate_key.as_ref().eq_ignore_ascii_case(key) {
                    let trimmed = candidate_value.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        sanitize_filename_leaf(trimmed)
                    }
                } else {
                    None
                }
            })
        {
            return Some(FilenameCandidate {
                generic: is_generic_route_candidate(&value),
                name: value,
                source: if is_final_url {
                    "query-parameter"
                } else {
                    "original-query-parameter"
                },
                classification: "url-derived",
            });
        }
    }

    None
}

pub(super) fn best_url_path_candidate(url: &str, is_final_url: bool) -> Option<FilenameCandidate> {
    let parsed = Url::parse(url).ok()?;
    let segment = parsed.path_segments().and_then(|mut segments| {
        segments
            .rfind(|segment| !segment.trim().is_empty())
            .map(ToString::to_string)
    })?;
    let name = sanitize_filename_leaf(&segment)?;
    Some(FilenameCandidate {
        generic: is_generic_route_candidate(&name),
        name,
        source: if is_final_url {
            "final-url"
        } else {
            "original-url"
        },
        classification: "url-derived",
    })
}

pub(super) fn extract_filename_from_content_disposition(value: Option<&str>) -> Option<String> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }

    let mut filename_star = None;
    let mut filename = None;

    for part in split_header_parameters(raw) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();

        match key.as_str() {
            "filename*" => {
                if let Some(decoded) = decode_rfc5987_filename(value) {
                    filename_star = Some(decoded);
                }
            }
            "filename" => {
                if let Some(decoded) = decode_basic_filename(value) {
                    filename = Some(decoded);
                }
            }
            _ => {}
        }
    }

    filename_star
        .or(filename)
        .and_then(|value| sanitize_filename_leaf(&value))
}

pub(super) fn usable_extension(name: &str) -> Option<&str> {
    let (_, extension) = name.rsplit_once('.')?;
    let normalized = extension.trim();
    if normalized.is_empty() || normalized.len() > 10 {
        return None;
    }
    if !normalized
        .chars()
        .all(|value| value.is_ascii_alphanumeric())
    {
        return None;
    }
    if is_server_route_extension(normalized) {
        return None;
    }
    Some(normalized)
}

pub(super) fn query_response_content_disposition(url: &str) -> Option<String> {
    Url::parse(url).ok().and_then(|parsed| {
        parsed.query_pairs().find_map(|(key, value)| {
            if key
                .as_ref()
                .eq_ignore_ascii_case("response-content-disposition")
            {
                Some(value.into_owned())
            } else {
                None
            }
        })
    })
}

pub(super) fn query_response_content_type(url: &str) -> Option<String> {
    Url::parse(url).ok().and_then(|parsed| {
        parsed.query_pairs().find_map(|(key, value)| {
            if key.as_ref().eq_ignore_ascii_case("response-content-type") {
                Some(value.into_owned())
            } else {
                None
            }
        })
    })
}

fn has_usable_extension(name: &str) -> bool {
    usable_extension(name).is_some()
}

fn choose_preferred_candidate(
    primary: Option<FilenameCandidate>,
    secondary: Option<FilenameCandidate>,
) -> Option<FilenameCandidate> {
    match (primary, secondary) {
        (Some(primary), Some(secondary)) => {
            if score_filename_candidate(&secondary) > score_filename_candidate(&primary) {
                Some(secondary)
            } else {
                Some(primary)
            }
        }
        (Some(primary), None) => Some(primary),
        (None, Some(secondary)) => Some(secondary),
        (None, None) => None,
    }
}

fn score_filename_candidate(candidate: &FilenameCandidate) -> i32 {
    let mut score = 0_i32;
    if usable_extension(&candidate.name).is_some() {
        score += 120;
    }
    if !candidate.generic {
        score += 40;
    }
    score + (candidate.name.len().min(96) as i32 / 8)
}

fn split_header_parameters(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;

    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                parts.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }

    parts.push(value[start..].trim());
    parts
}

fn decode_rfc5987_filename(value: &str) -> Option<String> {
    let trimmed = strip_surrounding_quotes(value);
    let encoded = trimmed
        .split_once("''")
        .map(|(_, remainder)| remainder)
        .unwrap_or(trimmed);
    decode_percent_encoded(encoded)
}

fn decode_basic_filename(value: &str) -> Option<String> {
    let unquoted = unescape_quoted_value(strip_surrounding_quotes(value));
    if unquoted.contains('%') {
        decode_percent_encoded(&unquoted).or(Some(unquoted))
    } else {
        Some(unquoted)
    }
}

fn strip_surrounding_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|trimmed| trimmed.strip_suffix('"'))
        .unwrap_or(value)
}

fn unescape_quoted_value(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            result.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        result.push(character);
    }
    result
}

fn append_extension(name: &str, extension: &str) -> String {
    let trimmed = name.trim_end_matches('.');
    if trimmed.is_empty() {
        format!("{FALLBACK_DOWNLOAD_NAME}.{extension}")
    } else {
        format!("{trimmed}.{extension}")
    }
}

fn preferred_extension_from_mime(mime_type: Option<&str>) -> Option<&'static str> {
    let normalized = clean_mime_type(mime_type)?;
    match normalized.as_str() {
        "application/vnd.android.package-archive" => Some("apk"),
        "application/vnd.apple.installer+xml" => Some("pkg"),
        "application/vnd.ms-excel" => Some("xls"),
        "application/vnd.ms-powerpoint" => Some("ppt"),
        "application/vnd.ms-word" | "application/msword" => Some("doc"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => Some("pptx"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some("xlsx"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Some("docx"),
        "application/x-7z-compressed" => Some("7z"),
        "application/x-debian-package" => Some("deb"),
        "application/x-msdownload" => Some("exe"),
        "application/x-rar-compressed" => Some("rar"),
        "application/x-rpm" => Some("rpm"),
        "application/epub+zip" => Some("epub"),
        "application/gzip" => Some("gz"),
        "application/json" => Some("json"),
        "application/pdf" => Some("pdf"),
        "application/vnd.rar" => Some("rar"),
        "application/x-tar" => Some("tar"),
        "application/xml" => Some("xml"),
        "application/zip" => Some("zip"),
        "audio/aac" => Some("aac"),
        "audio/flac" => Some("flac"),
        "audio/m4a" | "audio/mp4" => Some("m4a"),
        "audio/mpeg" => Some("mp3"),
        "audio/ogg" => Some("ogg"),
        "audio/wav" | "audio/wave" => Some("wav"),
        "image/gif" => Some("gif"),
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/svg+xml" => Some("svg"),
        "image/webp" => Some("webp"),
        "text/csv" => Some("csv"),
        "text/plain" => Some("txt"),
        "video/mp4" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        "video/webm" => Some("webm"),
        "video/x-matroska" => Some("mkv"),
        _ => None,
    }
}

#[allow(clippy::match_like_matches_macro)]
fn is_generic_route_candidate(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return true;
    }
    let stem = trimmed
        .split('.')
        .next()
        .unwrap_or(trimmed)
        .trim()
        .to_ascii_lowercase();
    let generic_stem = match stem.as_str() {
        "download" | "file" | "get" | "dl" | "attachment" | "fetch" | "view" | "open"
        | "content" | "asset" | "index" | "default" => true,
        _ => false,
    };
    let generic_route_extension = if let Some((_, extension)) = trimmed.rsplit_once('.') {
        is_server_route_extension(extension.trim())
    } else {
        false
    };
    generic_stem || generic_route_extension
}

#[allow(clippy::match_like_matches_macro)]
fn is_server_route_extension(extension: &str) -> bool {
    match extension.to_ascii_lowercase().as_str() {
        "php" | "asp" | "aspx" | "ashx" | "axd" | "jsp" | "jspx" | "cgi" | "fcgi" | "do"
        | "action" => true,
        _ => false,
    }
}
