use super::engine_log::append_download_log;
use super::runtime_state::clear_runtime_checkpoint;
use super::{
    clear_download_terminal_state, reset_download_progress, reset_download_transient_state,
};
use crate::model::{DownloadFailureKind, DownloadLogLevel, DownloadRecord, DownloadStatus};

pub(super) struct RuntimeErrorResolution {
    pub(super) retry_allowed: bool,
    pub(super) range_validation_failed: bool,
}

fn push_unique_diagnostic(values: &mut Vec<String>, message: impl Into<String>) {
    let message = message.into();
    if !values.iter().any(|value| value == &message) {
        values.push(message);
    }
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
