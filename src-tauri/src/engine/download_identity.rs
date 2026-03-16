use std::path::PathBuf;

use reqwest::Url;

use super::filename_policy::normalize_filename_hint;
use super::http_helpers::extract_url_host;
use crate::model::DownloadCategory;

pub(super) fn join_target_path(save_path: &str, name: &str) -> String {
    let mut path = PathBuf::from(save_path);
    path.push(name);
    path.to_string_lossy().to_string()
}

pub(super) fn suggested_name_from_url(url: &str) -> String {
    let candidate = Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .query_pairs()
                .find_map(
                    |(key, value)| match key.as_ref().to_ascii_lowercase().as_str() {
                        "filename" | "file" | "download" | "attachment" | "name" | "title" => {
                            let trimmed = value.trim();
                            if trimmed.is_empty() {
                                None
                            } else {
                                Some(normalize_filename_hint(trimmed))
                            }
                        }
                        _ => None,
                    },
                )
                .or_else(|| {
                    parsed.path_segments().and_then(|mut segments| {
                        segments
                            .rfind(|segment| !segment.trim().is_empty())
                            .map(|segment| normalize_filename_hint(segment.trim()))
                    })
                })
        })
        .unwrap_or_else(|| {
            normalize_filename_hint(
                url.split(['?', '#'])
                    .next()
                    .unwrap_or(url)
                    .rsplit('/')
                    .find(|segment| !segment.is_empty())
                    .unwrap_or("download.bin")
                    .trim(),
            )
        });

    if candidate.trim().is_empty() {
        "download.bin".to_string()
    } else {
        candidate
    }
}

pub(super) fn apply_detected_extension(name: &str, detected_name: Option<&str>) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return detected_name.unwrap_or("download.bin").to_string();
    }
    if filename_extension(trimmed).is_some() {
        return trimmed.to_string();
    }
    if let Some(extension) = detected_name.and_then(filename_extension) {
        return format!("{trimmed}.{extension}");
    }
    trimmed.to_string()
}

fn filename_extension(name: &str) -> Option<&str> {
    let (_, extension) = name.rsplit_once('.')?;
    let trimmed = extension.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|value| value.is_ascii_alphanumeric()) {
        Some(trimmed)
    } else {
        None
    }
}

pub(super) fn extract_host(url: &str) -> String {
    extract_url_host(url).unwrap_or_else(|| "unknown-host".to_string())
}

pub(super) fn classify_category(name: &str) -> DownloadCategory {
    let extension = name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => DownloadCategory::Compressed,
        "exe" | "msi" | "dmg" | "pkg" | "deb" | "rpm" | "apk" => DownloadCategory::Programs,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "flv" | "wmv" => DownloadCategory::Videos,
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" => DownloadCategory::Music,
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "svg" | "avif" | "tif" | "tiff"
        | "ico" | "heic" | "heif" => DownloadCategory::Pictures,
        _ => DownloadCategory::Documents,
    }
}

#[cfg(test)]
mod tests {
    use super::suggested_name_from_url;

    #[test]
    fn decodes_percent_encoded_url_filenames() {
        assert_eq!(
            suggested_name_from_url(
                "https://example.com/files/Rust%20v262%20Build%2003052026.part01.rar"
            ),
            "Rust v262 Build 03052026.part01.rar"
        );
    }

    #[test]
    fn strips_invisible_unicode_from_query_hints() {
        assert_eq!(
            suggested_name_from_url(
                "https://example.com/download?file=file%E2%80%8Bname%EF%BB%BF.zip"
            ),
            "filename.zip"
        );
    }
}
