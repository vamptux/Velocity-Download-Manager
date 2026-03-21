use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use super::runtime::{
    RuntimeTelemetrySnapshot, RuntimeWakeLockGuard, UNKNOWN_SIZE_SPACE_CHECK_INTERVAL_BYTES,
    UNKNOWN_SIZE_SPACE_SAFETY_MARGIN_BYTES, disk_queue_under_pressure, median_runtime_ttfb,
    push_unique_diagnostic, record_runtime_warning, runtime_connection_reuse_hint,
    runtime_protocol_hint, take_disk_pool_error, wait_for_disk_pool_drain,
};
use super::runtime_recovery::classify_runtime_error;
use super::runtime_state::clear_runtime_checkpoint;
use super::runtime_transfer::{
    InitialResponseStream, TransferWorkerConfig, UnknownSizeStreamOptions,
    run_unknown_size_stream_worker,
};
use super::*;
use crate::model::{
    DownloadFailureKind, DownloadRecord, DownloadStatus, HostProfile, HostTelemetryArgs,
};

pub(super) fn classify_unknown_size_failure_kind(error: &str) -> DownloadFailureKind {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("disk write")
        || normalized.contains("disk space")
        || normalized.contains("volume ran out of space")
        || normalized.contains("syncing temp file")
        || normalized.contains("target location")
        || normalized.contains("moving temp file")
        || normalized.contains("copying completed temp file")
        || normalized.contains("restoring the previous target")
    {
        return DownloadFailureKind::FileSystem;
    }
    if normalized.starts_with("http ") {
        return DownloadFailureKind::Http;
    }
    DownloadFailureKind::Network
}

pub(super) async fn run_unknown_size_stream_runtime(
    engine: &EngineState,
    runtime_download: &mut DownloadRecord,
    http_pool: &http_pool::HttpPool,
    runtime_host_profile: Option<HostProfile>,
    initial_stream: Option<InitialResponseStream>,
    min_emit_interval_ms: u32,
    max_checkpoint_interval_ms: i64,
) -> Result<(), String> {
    if let Some(parent) = Path::new(&runtime_download.temp_path).parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed creating temp directory: {error}"))?;
    }

    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(&runtime_download.temp_path)
        .map_err(|error| format!("Failed opening temp file: {error}"))?;
    let output_file = Arc::new(file);
    let mut temp_file_lock =
        acquire_temp_transfer_lock(Arc::clone(&output_file), &runtime_download.temp_path);
    if let Some(warning) = temp_file_lock.warning.clone() {
        record_runtime_warning(engine, runtime_download, "runtime.temp-lock", warning);
    }
    let _wake_lock_guard = RuntimeWakeLockGuard::acquire(engine);
    let disk_pool = Arc::new(disk_pool::DiskPool::new(256));
    let checkpoint_clock = Arc::new(AtomicI64::new(unix_epoch_millis()));
    let runtime_telemetry: Arc<Mutex<RuntimeTelemetrySnapshot>> =
        Arc::new(Mutex::new(RuntimeTelemetrySnapshot::default()));
    let runtime_settings = engine.get_settings();
    let effective_download_limit =
        effective_download_speed_limit(&*runtime_download, &runtime_settings);
    let per_download_limiter =
        Some(engine.download_rate_limiter(&runtime_download.id, effective_download_limit));
    let per_host_limiter = engine.host_rate_limiter(
        &runtime_download.host,
        host_token_bucket_rate(runtime_host_profile.as_ref()),
    );
    let space_check_path = Path::new(&runtime_download.temp_path)
        .parent()
        .map(Path::to_path_buf)
        .or_else(|| Some(Path::new(&runtime_download.save_path).to_path_buf()));
    let config = TransferWorkerConfig {
        url: runtime_download.final_url.clone(),
        request_referer: runtime_download.compatibility.request_referer.clone(),
        request_cookies: runtime_download.compatibility.request_cookies.clone(),
        request_method: runtime_download.compatibility.request_method.clone(),
        request_form_fields: runtime_download.compatibility.request_form_fields.clone(),
        chunk_buffer_size: runtime_chunk_buffer_size_with_pressure(
            &runtime_download.traffic_mode,
            runtime_download.speed,
            disk_pool.queue_utilization_percent(),
        ),
        request_timeout_secs: 30,
        retry_budget: 4,
        backoff_base_ms: 180,
        backoff_max_ms: 3_000,
        per_download_limiter: per_download_limiter.clone(),
        per_host_limiter: per_host_limiter.clone(),
    };
    let download_id = runtime_download.id.clone();

    let stream_result = run_unknown_size_stream_worker(
        &config,
        UnknownSizeStreamOptions {
            starting_offset: 0,
            space_check_path,
            space_check_interval_bytes: UNKNOWN_SIZE_SPACE_CHECK_INTERVAL_BYTES,
            space_safety_margin_bytes: UNKNOWN_SIZE_SPACE_SAFETY_MARGIN_BYTES,
        },
        http_pool,
        &disk_pool,
        &output_file,
        initial_stream,
        |progress| {
            take_disk_pool_error(disk_pool.as_ref())?;
            let response = {
                let mut registry = engine.registry_guard()?;
                let Some(download) = registry
                    .downloads
                    .iter_mut()
                    .find(|download| download.id == download_id)
                else {
                    return Err("Download no longer active.".to_string());
                };

                if let Some(telemetry) = progress.telemetry.as_ref()
                    && let Ok(mut snapshot) = runtime_telemetry.lock()
                {
                    if let Some(ttfb_ms) = telemetry.ttfb_ms.filter(|value| *value > 0) {
                        snapshot.ttfb_samples_ms.push(ttfb_ms);
                    }
                    if let Some(reused) = telemetry.connection_reused {
                        if reused {
                            snapshot.reused_true = snapshot.reused_true.saturating_add(1);
                        } else {
                            snapshot.reused_false = snapshot.reused_false.saturating_add(1);
                        }
                    }
                    if let Some(protocol) = telemetry
                        .negotiated_protocol
                        .as_ref()
                        .filter(|value| !value.is_empty())
                    {
                        let counter = snapshot
                            .protocol_counts
                            .entry(protocol.clone())
                            .or_insert(0);
                        *counter = counter.saturating_add(1);
                    }
                }

                download.downloaded = progress.downloaded.max(0);
                if progress.throughput_bytes_per_second > 0 {
                    download.speed = progress.throughput_bytes_per_second;
                }
                download.time_left = None;
                let under_pressure = disk_queue_under_pressure(disk_pool.as_ref());
                if under_pressure && !download.writer_backpressure {
                    download.diagnostics.checkpoint_disk_pressure_events = download
                        .diagnostics
                        .checkpoint_disk_pressure_events
                        .saturating_add(1);
                }
                download.writer_backpressure = under_pressure;
                let now = unix_epoch_millis();
                let elapsed = now.saturating_sub(checkpoint_clock.load(Ordering::Relaxed));
                let should_flush = elapsed >= max_checkpoint_interval_ms;
                if should_flush {
                    download.diagnostics.checkpoint_flushes =
                        download.diagnostics.checkpoint_flushes.saturating_add(1);
                    checkpoint_clock.store(now, Ordering::Relaxed);
                } else {
                    download.diagnostics.checkpoint_skips =
                        download.diagnostics.checkpoint_skips.saturating_add(1);
                }
                let response = download.clone();
                if should_flush {
                    engine.persist_registry(&registry)?;
                }
                response
            };
            engine.emit_download_progress_diff_if_due(&response, min_emit_interval_ms);
            Ok(())
        },
    )
    .await;

    let drained = wait_for_disk_pool_drain(disk_pool.as_ref()).await;
    let sync_result = output_file
        .sync_data()
        .map_err(|error| format!("Failed syncing temp file before finalize: {error}"));
    let disk_error = take_disk_pool_error(disk_pool.as_ref());

    let stream_result = match stream_result {
        Ok(outcome) => {
            drained?;
            sync_result?;
            disk_error?;
            let final_len = output_file
                .metadata()
                .map_err(|error| format!("Failed reading temp file metadata: {error}"))?
                .len();
            if final_len != outcome.downloaded {
                return Err(format!(
                    "Unknown-size transfer wrote {} bytes but the temp file reports {} bytes after disk flush.",
                    outcome.downloaded, final_len
                ));
            }
            if let Some(reported) = outcome.reported_content_length
                && reported != final_len
            {
                return Err(format!(
                    "Unknown-size transfer reached EOF at {} bytes but the final stream reported Content-Length {}.",
                    final_len, reported
                ));
            }
            Ok(final_len)
        }
        Err(error) => {
            if let Err(drain_error) = drained {
                Err(drain_error)
            } else if let Err(sync_error) = sync_result {
                Err(sync_error)
            } else if let Err(disk_error) = disk_error {
                Err(disk_error)
            } else {
                Err(error)
            }
        }
    };

    let mut temp_lock_release_warning = temp_file_lock.release();

    drop(output_file);
    drop(disk_pool);

    match stream_result {
        Ok(final_len) => {
            let finalize_result =
                finalize_download_file(&runtime_download.temp_path, &runtime_download.target_path)?;
            let finalize_used_copy_fallback = finalize_result.used_copy_fallback;
            let mut finalize_warnings = finalize_result.warnings;
            if let Some(warning) = temp_lock_release_warning.take() {
                finalize_warnings.push(warning);
            }

            let runtime_telemetry_snapshot = runtime_telemetry
                .lock()
                .ok()
                .map(|value| value.clone())
                .unwrap_or_default();
            let final_len_i64 = i64::try_from(final_len).unwrap_or(i64::MAX);
            let mut registry = engine.registry_guard()?;
            let Some(download_index) = registry
                .downloads
                .iter()
                .position(|download| download.id == runtime_download.id)
            else {
                return Ok(());
            };

            let telemetry_payload = {
                let download = &mut registry.downloads[download_index];
                download.status = DownloadStatus::Finished;
                download.size = final_len_i64;
                download.downloaded = final_len_i64;
                download.speed = 0;
                download.time_left = Some(0);
                download.error_message = None;
                download.writer_backpressure = false;
                download.capabilities.segmented = false;
                download.target_connections = 1;
                download.validators.content_length = Some(final_len);
                download.diagnostics.failure_kind = None;
                download.diagnostics.restart_required = false;
                download.diagnostics.terminal_reason =
                    Some("Transfer completed through guarded single-stream mode.".to_string());
                push_unique_diagnostic(
                    &mut download.diagnostics.notes,
                    "Guarded single-stream transfer finished after live disk-space checks and post-write verification.",
                );
                if finalize_used_copy_fallback {
                    push_unique_diagnostic(
                        &mut download.diagnostics.notes,
                        "Finalization used a cross-volume copy fallback because the temp and target folders resolved to different volumes.",
                    );
                }
                for warning in finalize_warnings {
                    push_unique_diagnostic(&mut download.diagnostics.warnings, warning);
                }
                append_download_log(
                    download,
                    DownloadLogLevel::Info,
                    "runtime.finished",
                    "Completed the guarded single-stream transfer and finalized the file.",
                );
                if download.integrity.expected.is_some() {
                    append_download_log(
                        download,
                        DownloadLogLevel::Info,
                        "integrity.verify-pending",
                        "Triggering checksum verification after completion.",
                    );
                }
                clear_runtime_checkpoint(download);
                HostTelemetryArgs {
                    host: download.host.clone(),
                    scope_key: Some(download_scope_key(download)),
                    attempted_connections: Some(1),
                    sustained_gain_bytes_per_second: None,
                    throughput_bytes_per_second: None,
                    ttfb_ms: median_runtime_ttfb(&runtime_telemetry_snapshot),
                    negotiated_protocol: runtime_protocol_hint(&runtime_telemetry_snapshot)
                        .or_else(|| download.host_protocol.clone()),
                    connection_reused: runtime_connection_reuse_hint(&runtime_telemetry_snapshot),
                    throttle_event: false,
                    timeout_event: false,
                    reset_event: false,
                    range_validation_failed: false,
                }
            };
            apply_host_feedback_to_registry(
                &mut registry,
                &telemetry_payload.host,
                &telemetry_payload,
            );
            let dispatch_plan = plan_runtime_dispatch(&mut registry);
            let response = registry.downloads[download_index].clone();
            engine.persist_registry_flush(&registry)?;
            drop(registry);
            engine.emit_download_progress_diff_if_due(&response, min_emit_interval_ms);
            engine.emit_download_upsert(&response);
            engine.trigger_download_completion_actions(&response);
            engine.apply_runtime_dispatch_plan(dispatch_plan, min_emit_interval_ms);
            if response.integrity.expected.is_some() {
                engine.spawn_checksum_verification(response.id.clone());
            }
        }
        Err(error) => {
            let runtime_telemetry_snapshot = runtime_telemetry
                .lock()
                .ok()
                .map(|value| value.clone())
                .unwrap_or_default();
            let (throttle_event, timeout_event, reset_event) = classify_runtime_error(&error);
            let failure_kind = classify_unknown_size_failure_kind(&error);
            let mut registry = engine.registry_guard()?;
            let Some(download_index) = registry
                .downloads
                .iter()
                .position(|download| download.id == runtime_download.id)
            else {
                return Ok(());
            };

            let telemetry_payload = {
                let download = &mut registry.downloads[download_index];
                let restart_required = download.downloaded > 0;
                download.status = DownloadStatus::Error;
                download.error_message = Some(error.clone());
                download.speed = 0;
                download.time_left = None;
                download.writer_backpressure = false;
                download.diagnostics.failure_kind = Some(failure_kind);
                download.diagnostics.restart_required = restart_required;
                if restart_required {
                    push_unique_diagnostic(
                        &mut download.diagnostics.warnings,
                        "Guarded single-stream transfers cannot resume safely after partial progress. Use Restart to retry from byte 0.",
                    );
                }
                download.diagnostics.terminal_reason = Some(if restart_required {
                    "Guarded single-stream transfer stopped after partial progress and now requires a clean restart."
                        .to_string()
                } else {
                    "Guarded single-stream transfer failed before stable progress was established."
                        .to_string()
                });
                append_download_log(
                    download,
                    DownloadLogLevel::Error,
                    "runtime.failed",
                    format!("Guarded single-stream runtime failed: {error}"),
                );
                clear_runtime_checkpoint(download);
                HostTelemetryArgs {
                    host: download.host.clone(),
                    scope_key: Some(download_scope_key(download)),
                    attempted_connections: Some(1),
                    sustained_gain_bytes_per_second: None,
                    throughput_bytes_per_second: None,
                    ttfb_ms: median_runtime_ttfb(&runtime_telemetry_snapshot),
                    negotiated_protocol: runtime_protocol_hint(&runtime_telemetry_snapshot)
                        .or_else(|| download.host_protocol.clone()),
                    connection_reused: runtime_connection_reuse_hint(&runtime_telemetry_snapshot),
                    throttle_event,
                    timeout_event,
                    reset_event,
                    range_validation_failed: false,
                }
            };
            apply_host_feedback_to_registry(
                &mut registry,
                &telemetry_payload.host,
                &telemetry_payload,
            );
            let dispatch_plan = plan_runtime_dispatch(&mut registry);
            let response = registry.downloads[download_index].clone();
            engine.persist_registry_flush(&registry)?;
            drop(registry);
            engine.emit_download_progress_diff_if_due(&response, min_emit_interval_ms);
            engine.emit_download_upsert(&response);
            engine.apply_runtime_dispatch_plan(dispatch_plan, min_emit_interval_ms);
        }
    }

    Ok(())
}
