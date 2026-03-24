use std::collections::BTreeMap;

use super::engine_log::append_download_log;
use super::scheduler::SegmentRuntimeSample;
use super::{
    EngineState, clear_download_terminal_state, reset_download_progress,
    reset_download_transient_state,
};
use crate::model::{
    DownloadFailureKind, DownloadLogLevel, DownloadRecord, DownloadRuntimeCheckpoint,
    DownloadRuntimeRaceState, DownloadRuntimeSegmentSample, DownloadSegmentStatus,
    DownloadStatus,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RuntimeRaceState {
    pub companion_segment_id: u32,
    pub slow_segment_id: u32,
    pub slow_baseline_downloaded: i64,
}

pub(super) struct RuntimeErrorResolution {
    pub(super) retry_allowed: bool,
    pub(super) range_validation_failed: bool,
}

pub(super) fn push_unique_diagnostic(values: &mut Vec<String>, message: impl Into<String>) {
    let message = message.into();
    if !values.iter().any(|existing| existing == &message) {
        values.push(message);
    }
}

pub(super) fn record_runtime_warning(
    engine: &EngineState,
    runtime_download: &mut DownloadRecord,
    code: &str,
    warning: String,
) {
    push_unique_diagnostic(&mut runtime_download.diagnostics.warnings, warning.clone());

    if let Ok(mut registry) = engine.registry_guard() {
        if let Some(download) = registry
            .downloads
            .iter_mut()
            .find(|download| download.id == runtime_download.id)
        {
            push_unique_diagnostic(&mut download.diagnostics.warnings, warning.clone());
            append_download_log(download, DownloadLogLevel::Warn, code, warning.clone());
            runtime_download.diagnostics = download.diagnostics.clone();
            runtime_download.engine_log = download.engine_log.clone();
        }
        let _ = engine.persist_registry(&registry);
    }
}

pub(super) fn clear_runtime_checkpoint(download: &mut DownloadRecord) {
    download.runtime_checkpoint = DownloadRuntimeCheckpoint::default();
}

pub(super) fn upsert_runtime_segment_sample(
    checkpoint: &mut DownloadRuntimeCheckpoint,
    sample: &SegmentRuntimeSample,
) {
    if let Some(existing) = checkpoint
        .segment_samples
        .iter_mut()
        .find(|value| value.segment_id == sample.segment_id)
    {
        existing.remaining_bytes = sample.remaining_bytes;
        existing.eta_seconds = sample.eta_seconds;
        existing.throughput_bytes_per_second = sample.throughput_bytes_per_second;
        return;
    }
    checkpoint
        .segment_samples
        .push(DownloadRuntimeSegmentSample {
            segment_id: sample.segment_id,
            remaining_bytes: sample.remaining_bytes,
            eta_seconds: sample.eta_seconds,
            throughput_bytes_per_second: sample.throughput_bytes_per_second,
            retry_attempts: 0,
            terminal_failure_reason: None,
        });
}

pub(super) fn upsert_runtime_segment_health(
    checkpoint: &mut DownloadRuntimeCheckpoint,
    segment_id: u32,
    retry_attempts: u32,
    terminal_failure_reason: Option<String>,
) {
    if let Some(existing) = checkpoint
        .segment_samples
        .iter_mut()
        .find(|value| value.segment_id == segment_id)
    {
        existing.retry_attempts = retry_attempts;
        existing.terminal_failure_reason = terminal_failure_reason;
        return;
    }
    checkpoint
        .segment_samples
        .push(DownloadRuntimeSegmentSample {
            segment_id,
            remaining_bytes: 0,
            eta_seconds: None,
            throughput_bytes_per_second: None,
            retry_attempts,
            terminal_failure_reason,
        });
}

pub(super) fn persist_runtime_races(
    checkpoint: &mut DownloadRuntimeCheckpoint,
    race_by_segment: &BTreeMap<u32, RuntimeRaceState>,
) {
    checkpoint.active_races.clear();
    for race in race_by_segment.values() {
        if race.slow_segment_id > race.companion_segment_id {
            continue;
        }
        checkpoint.active_races.push(DownloadRuntimeRaceState {
            slow_segment_id: race.slow_segment_id,
            companion_segment_id: race.companion_segment_id,
            slow_baseline_downloaded: race.slow_baseline_downloaded,
        });
    }
}

pub(super) fn restore_runtime_races(
    checkpoint: &DownloadRuntimeCheckpoint,
    pending_ids: &BTreeMap<u32, ()>,
) -> BTreeMap<u32, RuntimeRaceState> {
    let mut race_by_segment = BTreeMap::new();
    for race in &checkpoint.active_races {
        if pending_ids.contains_key(&race.slow_segment_id)
            && pending_ids.contains_key(&race.companion_segment_id)
        {
            race_by_segment.insert(
                race.slow_segment_id,
                RuntimeRaceState {
                    companion_segment_id: race.companion_segment_id,
                    slow_segment_id: race.slow_segment_id,
                    slow_baseline_downloaded: race.slow_baseline_downloaded,
                },
            );
            race_by_segment.insert(
                race.companion_segment_id,
                RuntimeRaceState {
                    companion_segment_id: race.slow_segment_id,
                    slow_segment_id: race.slow_segment_id,
                    slow_baseline_downloaded: race.slow_baseline_downloaded,
                },
            );
        }
    }
    race_by_segment
}

pub(super) fn resolve_runtime_race(
    download: &mut DownloadRecord,
    winner_id: u32,
    race: RuntimeRaceState,
    race_by_segment: &BTreeMap<u32, RuntimeRaceState>,
) {
    if winner_id == race.slow_segment_id {
        if let Some(challenger) = download
            .segments
            .iter_mut()
            .find(|value| value.id == race.companion_segment_id)
        {
            challenger.downloaded = 0;
            challenger.status = DownloadSegmentStatus::Finished;
        }
    } else if let Some(slow) = download
        .segments
        .iter_mut()
        .find(|value| value.id == race.slow_segment_id)
    {
        slow.downloaded = slow.downloaded.min(race.slow_baseline_downloaded);
        slow.status = DownloadSegmentStatus::Finished;
    }
    persist_runtime_races(&mut download.runtime_checkpoint, race_by_segment);
}

pub(super) fn classify_runtime_error(error: &str) -> (bool, bool, bool) {
    let normalized = error.to_ascii_lowercase();
    let throttle = normalized.contains("429") || normalized.contains("too many requests");
    let timeout = normalized.contains("timed out") || normalized.contains("timeout");
    let reset = normalized.contains("connection reset") || normalized.contains("reset by peer");
    (throttle, timeout, reset)
}

pub(super) fn runtime_control_flow_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("no longer active")
        || normalized.contains("segment-canceled")
        || normalized.contains("removed during transfer")
}

pub(super) fn runtime_validation_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("content-range") || normalized.contains("byte range")
}

pub(super) fn can_retry_after_range_validation_failure(download: &DownloadRecord) -> bool {
    download.size > 0
        && download.downloaded <= 0
        && download
            .segments
            .iter()
            .all(|segment| segment.downloaded <= 0)
}

pub(super) fn apply_guarded_single_stream_fallback(download: &mut DownloadRecord) {
    reset_download_progress(download);
    reset_download_transient_state(download);
    clear_download_terminal_state(download);
    clear_runtime_checkpoint(download);
    download.capabilities.range_supported = false;
    download.capabilities.resumable = false;
    download.capabilities.segmented = false;
    download.target_connections = 1;
    download.segments.clear();
    download.status = DownloadStatus::Queued;
    download.diagnostics.failure_kind = None;
    download.diagnostics.restart_required = false;
    push_unique_diagnostic(
        &mut download.diagnostics.warnings,
        "Host ignored the opening byte-range request; VDM is retrying this transfer in guarded single-stream mode.",
    );
    download.diagnostics.terminal_reason = Some(
        "Retrying in guarded single-stream mode after the host ignored the first range request."
            .to_string(),
    );
}

pub(super) fn reconcile_runtime_error(
    download: &mut DownloadRecord,
    error: &str,
    runtime_attempt: u32,
    validation_error: bool,
) -> RuntimeErrorResolution {
    if matches!(
        download.status,
        DownloadStatus::Paused | DownloadStatus::Stopped
    ) {
        return RuntimeErrorResolution {
            retry_allowed: false,
            range_validation_failed: false,
        };
    }
    if validation_error && can_retry_after_range_validation_failure(download) {
        apply_guarded_single_stream_fallback(download);
        append_download_log(
            download,
            DownloadLogLevel::Warn,
            "runtime.range-fallback",
            "Opening range validation failed before progress was committed; retrying in guarded single-stream mode.",
        );
        return RuntimeErrorResolution {
            retry_allowed: true,
            range_validation_failed: true,
        };
    }
    if validation_error {
        download.status = DownloadStatus::Error;
        download.error_message = Some(error.to_string());
        download.diagnostics.failure_kind = Some(DownloadFailureKind::Validation);
        download.diagnostics.restart_required = true;
        push_unique_diagnostic(
            &mut download.diagnostics.warnings,
            "Host range validation failed during transfer; follow-up attempts will stay in single-connection mode until a fresh probe proves byte-range support again.",
        );
        download.diagnostics.terminal_reason = Some(
            "Host returned an invalid byte-range response; segmented transfer stopped to avoid corruption."
                .to_string(),
        );
        append_download_log(
            download,
            DownloadLogLevel::Error,
            "runtime.range-validation-failed",
            format!("Segmented runtime stopped after invalid byte-range data: {error}"),
        );
        clear_runtime_checkpoint(download);
        return RuntimeErrorResolution {
            retry_allowed: false,
            range_validation_failed: true,
        };
    }
    if runtime_attempt >= 3 {
        download.status = DownloadStatus::Error;
        download.error_message = Some(error.to_string());
        download.diagnostics.failure_kind = Some(DownloadFailureKind::Network);
        download.diagnostics.restart_required = true;
        download.diagnostics.terminal_reason = Some("Runtime retries exhausted.".to_string());
        append_download_log(
            download,
            DownloadLogLevel::Error,
            "runtime.retries-exhausted",
            format!("Segmented runtime exhausted retries: {error}"),
        );
        clear_runtime_checkpoint(download);
        return RuntimeErrorResolution {
            retry_allowed: false,
            range_validation_failed: false,
        };
    }

    download.status = DownloadStatus::Queued;
    download.error_message = None;
    download.diagnostics.terminal_reason = Some(format!(
        "Recovering from transfer error (attempt {runtime_attempt})."
    ));
    append_download_log(
        download,
        DownloadLogLevel::Warn,
        "runtime.retrying",
        format!("Recovering from transfer error: {error}"),
    );
    RuntimeErrorResolution {
        retry_allowed: true,
        range_validation_failed: false,
    }
}