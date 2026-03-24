use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::{
    DownloadIntegrityAlgorithm, DownloadIntegrityStatus, DownloadRecord, DownloadSegmentStatus,
};

use super::RUNTIME_SEGMENT_RETRY_BUDGET;

pub(super) fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn reset_download_progress(download: &mut DownloadRecord) {
    download.downloaded = 0;
    download.integrity.computed_hash = None;
    download.integrity.algorithm = DownloadIntegrityAlgorithm::Sha256;
    download.integrity.status = DownloadIntegrityStatus::Unavailable;
    download.integrity.verified_at = None;
    download.integrity.last_error = None;
    for segment in &mut download.segments {
        segment.downloaded = 0;
        segment.retry_attempts = 0;
        if segment.retry_budget == 0 {
            segment.retry_budget = RUNTIME_SEGMENT_RETRY_BUDGET;
        }
        segment.status = DownloadSegmentStatus::Pending;
    }
}

pub(super) fn reset_download_transient_state(download: &mut DownloadRecord) {
    download.speed = 0;
    download.time_left = None;
    download.writer_backpressure = false;
}

pub(super) fn clear_download_terminal_state(download: &mut DownloadRecord) {
    download.error_message = None;
    download.diagnostics.terminal_reason = None;
}

pub(super) fn next_queue_position(downloads: &[DownloadRecord]) -> u32 {
    downloads
        .iter()
        .map(|download| download.queue_position)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

pub(super) fn queue_positions_are_normalized(downloads: &[DownloadRecord]) -> bool {
    downloads.iter().enumerate().all(|(index, download)| {
        download.queue_position == u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1)
    })
}

pub(super) fn normalize_queue_positions(downloads: &mut [DownloadRecord]) {
    downloads.sort_by(|left, right| {
        left.queue_position
            .cmp(&right.queue_position)
            .then_with(|| left.date_added.cmp(&right.date_added))
            .then_with(|| left.id.cmp(&right.id))
    });

    for (index, download) in downloads.iter_mut().enumerate() {
        download.queue_position = u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1);
    }
}

pub(super) fn unix_epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

pub(super) fn format_bytes_compact(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }

    let mut value = bytes as f64;
    let mut unit_index = 0usize;
    while value >= 1024.0 && unit_index < UNITS.len().saturating_sub(1) {
        value /= 1024.0;
        unit_index = unit_index.saturating_add(1);
    }

    if unit_index == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit_index])
    } else {
        format!("{value:.2} {}", UNITS[unit_index])
    }
}
