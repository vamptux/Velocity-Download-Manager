use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::{sync::Notify, task::JoinHandle};
use ts_rs::TS;

pub mod disk;
pub mod disk_pool;
mod download_identity;
mod engine_log;
mod file_ops;
mod filename_policy;
mod helpers;
mod host_planner;
mod http_helpers;
pub mod http_pool;
mod integrity;
mod operations;
mod persistence;
mod persistence_queue;
mod probe;
mod probe_cache;
mod probe_filename;
mod probe_html;
mod probe_html_cache;
mod probe_html_patterns;
mod progress;
mod runtime;
mod runtime_bootstrap;
mod runtime_dispatch;
mod runtime_race;
mod runtime_ramp;
mod runtime_recovery;
mod runtime_state;
pub mod runtime_transfer;
mod runtime_unknown_size;
pub mod scheduler;
pub mod segmentation;
mod settings_policy;
mod wake_lock;

use download_identity::{
    apply_detected_extension, classify_category, extract_host, join_target_path,
    suggested_name_from_url,
};
use engine_log::append_download_log;
use file_ops::{
    acquire_temp_transfer_lock, finalize_download_file, open_in_file_manager,
    query_available_space, reset_temp_file_path,
};
use helpers::{
    clear_download_terminal_state, format_bytes_compact, next_queue_position, non_empty,
    normalize_queue_positions, queue_positions_are_normalized, reset_download_progress,
    reset_download_transient_state, unix_epoch_millis,
};
use host_planner::{
    apply_host_telemetry, effective_average_throughput_bytes_per_second, effective_average_ttfb_ms,
    effective_connection_target_for_scope, host_diagnostics_summary_for_scope,
    initial_target_connections_for_scope, profile_warning_for_scope,
};
use integrity::{
    apply_integrity_result, compute_checksum, mark_integrity_failure, mark_integrity_verifying,
    normalize_checksum_spec, reset_integrity_for_expected,
};
use persistence::{load_registry_snapshot, snapshot_path};
use persistence_queue::{PersistPriority, SnapshotPersistQueue};
use probe::{normalize_protocol_label, probe_download_headers_with_context};
use probe_cache::{
    append_probe_cache_warning, apply_scope_range_validation_failure,
    cached_probe_to_download_probe, fresh_probe_capabilities, fresh_recent_probe,
    probe_cache_stale, probe_scope_key, record_probe_failure, scoped_hard_no_range,
    scoped_probe_failures, store_recent_probe, update_profile_probe_cache,
};
use runtime_dispatch::{RuntimeDispatchPlan, plan_runtime_dispatch};
use runtime_state::clear_runtime_checkpoint;
use runtime_transfer::TokenBucketRateLimiter;
use segmentation::{SegmentPlanningHints, compute_segments_with_hints};
use settings_policy::sanitize_engine_settings;
use wake_lock::WakeLockController;

use crate::model::{
    AddDownloadArgs, ChecksumSpec, DownloadCapabilities, DownloadCompatibility,
    DownloadDiagnostics, DownloadFailureKind, DownloadIntegrity, DownloadLogEntry,
    DownloadLogLevel, DownloadRecord, DownloadRequestField, DownloadRequestMethod,
    DownloadRuntimeCheckpoint, DownloadSegment, DownloadSegmentStatus, DownloadStatus,
    EngineSettings, HostProfile, HostTelemetryArgs, ProbeDownloadArgs, ProbeResult, QueueState,
    RegistrySnapshot, ReorderDirection, TrafficMode,
};

const DEFAULT_QUEUE: &str = "default";
const RUNTIME_SEGMENT_RETRY_BUDGET: u32 = 6;
const DOWNLOAD_UPSERT_EVENT: &str = "downloads://upsert";
const DOWNLOAD_UPSERT_ROW_EVENT: &str = "downloads://upsert-row";
const DOWNLOAD_REMOVE_EVENT: &str = "downloads://remove";
const DOWNLOAD_PROGRESS_DIFF_EVENT: &str = "downloads://progress-diff";
const DOWNLOAD_COMPLETED_EVENT: &str = "downloads://completed";
const ENGINE_BOOTSTRAP_EVENT: &str = "engine://bootstrap";
const ENGINE_SETTINGS_EVENT: &str = "engine://settings";
const TRAFFIC_MODE_LOW_BUFFER_BYTES: usize = 512 * 1024;
const TRAFFIC_MODE_MEDIUM_BUFFER_BYTES: usize = 1024 * 1024;
const TRAFFIC_MODE_HIGH_BUFFER_BYTES: usize = 2 * 1024 * 1024;
const TRAFFIC_MODE_MAX_BUFFER_BYTES: usize = 4 * 1024 * 1024;
const RUNTIME_CHUNK_BUFFER_FLOOR_BYTES: usize = 256 * 1024;
const RUNTIME_CHUNK_BUFFER_CEILING_BYTES: usize = 8 * 1024 * 1024;
const RATE_LIMITER_MIN_BURST_BYTES: u64 = 512 * 1024;

#[derive(Clone)]
pub struct EngineState {
    inner: Arc<EngineInner>,
}

struct EngineInner {
    app: AppHandle,
    registry: Mutex<RegistrySnapshot>,
    snapshot_path: PathBuf,
    snapshot_writer: SnapshotPersistQueue,
    bootstrap: Mutex<BootstrapRuntimeState>,
    bootstrap_notify: Notify,
    progress_sync: Mutex<ProgressSyncState>,
    runtime_tasks: Mutex<BTreeMap<String, JoinHandle<()>>>,
    integrity_tasks: Mutex<BTreeMap<String, JoinHandle<()>>>,
    download_limiters: Mutex<BTreeMap<String, Arc<TokenBucketRateLimiter>>>,
    host_limiters: Mutex<BTreeMap<String, Arc<TokenBucketRateLimiter>>>,
    wake_lock: Mutex<WakeLockController>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineBootstrapState {
    pub ready: bool,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateSnapshot {
    pub downloads: Vec<DownloadRecord>,
    pub settings: EngineSettings,
    pub queue_state: QueueState,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateRowSnapshot {
    pub downloads: Vec<DownloadRecord>,
    pub settings: EngineSettings,
    pub queue_state: QueueState,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupSnapshot {
    pub bootstrap: EngineBootstrapState,
    pub settings: EngineSettings,
    pub queue_state: QueueState,
    pub active_downloads: Vec<DownloadRecord>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDetailSnapshot {
    pub id: String,
    pub engine_log: Vec<DownloadLogEntry>,
    pub runtime_checkpoint: DownloadRuntimeCheckpoint,
}

#[derive(Default)]
struct BootstrapRuntimeState {
    ready: bool,
    error: Option<String>,
    running: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadRemovedEvent {
    id: String,
}

#[derive(Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
struct DownloadCompletedEvent {
    id: String,
    name: String,
    target_path: String,
    save_path: String,
}

#[derive(Default)]
struct ProgressSyncState {
    by_download: BTreeMap<String, ProgressSyncEntry>,
}

struct ProgressSyncEntry {
    last_emit_ms: i64,
    last_downloaded: i64,
    last_speed: u64,
    last_time_left: Option<u64>,
    last_status: DownloadStatus,
    last_writer_backpressure: bool,
    last_target_connections: u32,
    segment_downloaded: BTreeMap<u32, i64>,
    segment_status: BTreeMap<u32, DownloadSegmentStatus>,
}

impl Default for ProgressSyncEntry {
    fn default() -> Self {
        Self {
            last_emit_ms: 0,
            last_downloaded: 0,
            last_speed: 0,
            last_time_left: None,
            last_status: DownloadStatus::Stopped,
            last_writer_backpressure: false,
            last_target_connections: 0,
            segment_downloaded: BTreeMap::new(),
            segment_status: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
struct DownloadProgressDiffEvent {
    id: String,
    downloaded: i64,
    speed: u64,
    time_left: Option<u64>,
    status: DownloadStatus,
    writer_backpressure: bool,
    target_connections: u32,
    segments: Vec<SegmentProgressDiff>,
}

#[derive(Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct SegmentProgressDiff {
    id: u32,
    downloaded: i64,
    status: DownloadSegmentStatus,
}

fn apply_host_feedback_to_registry(
    registry: &mut RegistrySnapshot,
    host: &str,
    payload: &HostTelemetryArgs,
) {
    let settings = registry.settings.clone();
    let profile = registry.host_profiles.entry(host.to_string()).or_default();
    apply_host_telemetry(profile, payload);
    let profile_snapshot = profile.clone();
    for download in &mut registry.downloads {
        if download.host != host {
            continue;
        }
        apply_download_host_profile(download, &settings, Some(&profile_snapshot));
    }
}

pub(super) fn download_scope_key(download: &DownloadRecord) -> String {
    probe_scope_key(
        &download.url,
        &download.compatibility.request_method,
        &download.compatibility.request_form_fields,
    )
}

pub(super) fn apply_download_host_profile(
    download: &mut DownloadRecord,
    settings: &EngineSettings,
    host_profile: Option<&HostProfile>,
) {
    let scope_key = download_scope_key(download);
    download.host_average_ttfb_ms = effective_average_ttfb_ms(host_profile, Some(&scope_key));
    download.host_average_throughput_bytes_per_second =
        effective_average_throughput_bytes_per_second(host_profile, Some(&scope_key));
    download.host_max_connections = host_profile.and_then(|profile| profile.max_connections);
    download.host_cooldown_until = host_profile.and_then(|profile| profile.cooldown_until);
    let now = unix_epoch_millis();
    let hard_no_range = scoped_hard_no_range(host_profile, &scope_key, now);
    if hard_no_range && !has_live_transfer_state(download) {
        download.capabilities.range_supported = false;
        download.capabilities.resumable = false;
        download.capabilities.segmented = false;
    }
    if let Some(protocol) = host_profile.and_then(|profile| profile.negotiated_protocol.as_deref())
    {
        download.host_protocol = Some(normalize_protocol_label(protocol).to_string());
    }
    download.host_diagnostics = host_diagnostics_summary_for_scope(host_profile, Some(&scope_key));
    download.host_diagnostics.hard_no_range = hard_no_range;
    if download.host_diagnostics.lock_reason.is_none()
        && scoped_probe_failures(host_profile, &scope_key, now) > 0
    {
        download.host_diagnostics.lock_reason = Some("probe-failures".to_string());
    }
    let requested = 16;
    download.max_connections =
        effective_connection_target_for_scope(requested, settings, host_profile, Some(&scope_key));
    download.target_connections = reconcile_download_target_connections(
        download,
        settings,
        download.max_connections,
        host_profile,
    );
    ensure_segment_plan(download, settings);
}

fn build_token_bucket_limiter(
    rate_bytes_per_second: Option<u64>,
) -> Option<Arc<TokenBucketRateLimiter>> {
    let rate = rate_bytes_per_second?;
    let burst = token_bucket_burst_bytes(rate);
    Some(Arc::new(TokenBucketRateLimiter::new(rate, burst)))
}

fn token_bucket_burst_bytes(rate_bytes_per_second: u64) -> u64 {
    rate_bytes_per_second
        .saturating_mul(2)
        .max(RATE_LIMITER_MIN_BURST_BYTES)
}

fn host_token_bucket_rate(profile: Option<&HostProfile>) -> Option<u64> {
    let profile = profile?;
    let base = profile.average_throughput_bytes_per_second?;
    if profile
        .cooldown_until
        .is_none_or(|deadline| deadline <= unix_epoch_millis())
    {
        return None;
    }

    let mut adapted = base.max(512 * 1024);
    if profile.throttle_events > 0 {
        adapted = adapted.saturating_div(2).max(512 * 1024);
    }
    if profile.timeout_events > 0 {
        adapted = adapted.saturating_mul(3).saturating_div(4).max(512 * 1024);
    }

    Some(adapted)
}

pub(super) fn effective_download_speed_limit(
    download: &DownloadRecord,
    settings: &EngineSettings,
) -> Option<u64> {
    download
        .speed_limit_bytes_per_second
        .filter(|value| *value > 0)
        .or(settings
            .speed_limit_bytes_per_second
            .filter(|value| *value > 0))
}

fn planned_download_size(size: i64) -> Option<u64> {
    u64::try_from(size).ok().filter(|value| *value > 0)
}

pub(super) fn runtime_chunk_buffer_size_with_pressure(
    traffic_mode: &TrafficMode,
    throughput_bytes_per_second: u64,
    queue_utilization_percent: usize,
) -> usize {
    let base = runtime_chunk_buffer_base_size(traffic_mode);
    let floor = runtime_chunk_buffer_floor(base);
    let ceiling = runtime_chunk_buffer_ceiling(base);

    if throughput_bytes_per_second == 0 {
        return base.clamp(floor, ceiling);
    }

    // When the disk queue is under pressure, increase the buffer size (target_window_ms).
    // This results in fewer, larger I/O operations, reducing lock contention and I/O thrashing.
    let target_window_ms = match queue_utilization_percent {
        85..=100 => 128_u64,
        70..=84 => 96_u64,
        55..=69 => 64_u64,
        0..=34 => 32_u64,
        _ => 48_u64,
    };
    let target = throughput_bytes_per_second
        .saturating_mul(target_window_ms)
        .div_ceil(1_000);
    usize::try_from(target)
        .unwrap_or(usize::MAX)
        .clamp(floor, ceiling)
}

fn runtime_chunk_buffer_base_size(traffic_mode: &TrafficMode) -> usize {
    match traffic_mode {
        TrafficMode::Low => TRAFFIC_MODE_LOW_BUFFER_BYTES,
        TrafficMode::Medium => TRAFFIC_MODE_MEDIUM_BUFFER_BYTES,
        TrafficMode::High => TRAFFIC_MODE_HIGH_BUFFER_BYTES,
        TrafficMode::Max => TRAFFIC_MODE_MAX_BUFFER_BYTES,
    }
}

fn runtime_chunk_buffer_floor(base: usize) -> usize {
    base.saturating_div(4).max(RUNTIME_CHUNK_BUFFER_FLOOR_BYTES)
}

fn runtime_chunk_buffer_ceiling(base: usize) -> usize {
    base.saturating_mul(2).clamp(
        runtime_chunk_buffer_floor(base),
        RUNTIME_CHUNK_BUFFER_CEILING_BYTES,
    )
}

pub(super) fn compatibility_request_context_supports_segmented_transfer(
    download: &DownloadRecord,
) -> bool {
    http_helpers::request_context_supports_segmented_transfer(
        &download.compatibility.request_method,
        &download.compatibility.request_form_fields,
    )
}

fn has_live_transfer_state(download: &DownloadRecord) -> bool {
    download.downloaded > 0
        || download.segments.iter().any(|segment| {
            segment.downloaded > 0 || segment.status == DownloadSegmentStatus::Downloading
        })
}

pub(super) fn reconcile_download_target_connections(
    download: &DownloadRecord,
    settings: &EngineSettings,
    effective_max_connections: u32,
    host_profile: Option<&HostProfile>,
) -> u32 {
    let effective_max_connections = effective_max_connections.max(1);
    if !download.capabilities.range_supported
        || !download.capabilities.resumable
        || download.size <= 0
        || !compatibility_request_context_supports_segmented_transfer(download)
    {
        return 1;
    }
    if has_live_transfer_state(download) {
        return download
            .target_connections
            .max(1)
            .min(effective_max_connections);
    }

    let scope_key = download_scope_key(download);
    initial_target_connections_for_scope(
        effective_max_connections,
        settings,
        host_profile,
        Some(&scope_key),
        planned_download_size(download.size),
    )
}

fn should_use_segmented_mode(download: &DownloadRecord) -> bool {
    download.capabilities.range_supported
        && download.capabilities.resumable
        && download.size > 0
        && download.max_connections > 1
        && compatibility_request_context_supports_segmented_transfer(download)
}

fn build_segment_plan(
    size: u64,
    target_connections: u32,
    settings: &EngineSettings,
    throughput_hint_bytes_per_second: Option<u64>,
    ttfb_hint_ms: Option<u64>,
) -> Vec<DownloadSegment> {
    compute_segments_with_hints(
        size,
        target_connections.max(1),
        settings.min_segment_size_bytes.max(1),
        RUNTIME_SEGMENT_RETRY_BUDGET,
        SegmentPlanningHints {
            throughput_bytes_per_second: throughput_hint_bytes_per_second,
            ttfb_ms: ttfb_hint_ms,
            target_chunk_time_seconds: settings.target_chunk_time_seconds,
        },
    )
}

pub(super) fn ensure_segment_plan(download: &mut DownloadRecord, settings: &EngineSettings) {
    let segmented = should_use_segmented_mode(download);
    download.capabilities.segmented = segmented;
    if !segmented || download.downloaded > 0 {
        return;
    }
    let size = download.size as u64;
    let has_progress = download.segments.iter().any(|segment| {
        segment.downloaded > 0
            || matches!(
                segment.status,
                DownloadSegmentStatus::Downloading | DownloadSegmentStatus::Finished
            )
    });
    if has_progress {
        return;
    }
    download.segments = build_segment_plan(
        size,
        download.target_connections,
        settings,
        download.host_average_throughput_bytes_per_second,
        download.host_average_ttfb_ms,
    );
}

fn classify_runtime_task_failure_kind(error: &str) -> DownloadFailureKind {
    let normalized = error.to_ascii_lowercase();
    if normalized.starts_with("http ") {
        return DownloadFailureKind::Http;
    }
    if normalized.contains("range") || normalized.contains("validation") {
        return DownloadFailureKind::Validation;
    }
    if normalized.contains("disk")
        || normalized.contains("file")
        || normalized.contains("volume")
        || normalized.contains("space")
        || normalized.contains("reserve")
        || normalized.contains("rename")
        || normalized.contains("sync")
    {
        return DownloadFailureKind::FileSystem;
    }
    DownloadFailureKind::Network
}

fn restore_registry_snapshot(mut snapshot: RegistrySnapshot) -> RegistrySnapshot {
    snapshot.settings = sanitize_engine_settings(snapshot.settings);
    let queue_running = snapshot.queue_running;
    let settings = &snapshot.settings;
    let host_profiles = &snapshot.host_profiles;
    let downloads = &mut snapshot.downloads;
    if !queue_positions_are_normalized(downloads) {
        normalize_queue_positions(downloads);
    }
    for download in downloads {
        if matches!(download.status, DownloadStatus::Downloading) {
            download.status = if queue_running {
                DownloadStatus::Queued
            } else {
                DownloadStatus::Stopped
            };
        }
        let host_profile = host_profiles.get(&download.host);
        apply_download_host_profile(download, settings, host_profile);
        reset_download_transient_state(download);
        for segment in &mut download.segments {
            if segment.status == DownloadSegmentStatus::Downloading {
                segment.status = DownloadSegmentStatus::Pending;
            }
            if segment.retry_budget == 0 {
                segment.retry_budget = RUNTIME_SEGMENT_RETRY_BUDGET;
            }
        }
        ensure_segment_plan(download, settings);
    }

    snapshot
}

fn load_persisted_registry_snapshot(path: &Path) -> Result<RegistrySnapshot, String> {
    let snapshot = load_registry_snapshot(path)?.unwrap_or_default();
    Ok(restore_registry_snapshot(snapshot))
}

impl EngineState {
    pub fn new(app: AppHandle) -> Self {
        let base_path = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("vdm"));
        let state_path = snapshot_path(&base_path);
        Self {
            inner: Arc::new(EngineInner {
                app,
                registry: Mutex::new(RegistrySnapshot::default()),
                snapshot_writer: SnapshotPersistQueue::new(state_path.clone()),
                snapshot_path: state_path,
                bootstrap: Mutex::new(BootstrapRuntimeState::default()),
                bootstrap_notify: Notify::new(),
                progress_sync: Mutex::new(ProgressSyncState::default()),
                runtime_tasks: Mutex::new(BTreeMap::new()),
                integrity_tasks: Mutex::new(BTreeMap::new()),
                download_limiters: Mutex::new(BTreeMap::new()),
                host_limiters: Mutex::new(BTreeMap::new()),
                wake_lock: Mutex::new(WakeLockController::default()),
            }),
        }
    }

    pub fn spawn_bootstrap(&self) {
        let bootstrap_state = match self.inner.bootstrap.lock() {
            Ok(mut bootstrap) => {
                if bootstrap.running {
                    return;
                }
                bootstrap.ready = false;
                bootstrap.error = None;
                bootstrap.running = true;
                EngineBootstrapState {
                    ready: false,
                    error: None,
                }
            }
            Err(_) => {
                return;
            }
        };

        let _ = self.inner.app.emit(ENGINE_BOOTSTRAP_EVENT, bootstrap_state);

        let engine = self.clone();
        tauri::async_runtime::spawn(async move {
            let snapshot_path = engine.inner.snapshot_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                load_persisted_registry_snapshot(&snapshot_path)
            })
            .await
            .map_err(|error| format!("Engine bootstrap task failed: {error}"))
            .and_then(|result| result);
            engine.finish_bootstrap(result);
        });
    }

    pub fn get_bootstrap_state(&self) -> EngineBootstrapState {
        self.inner
            .bootstrap
            .lock()
            .map(|state| EngineBootstrapState {
                ready: state.ready,
                error: state.error.clone(),
            })
            .unwrap_or_else(|_| EngineBootstrapState {
                ready: true,
                error: Some("Engine bootstrap state lock was poisoned.".to_string()),
            })
    }

    async fn await_bootstrap(&self) {
        loop {
            let notified = {
                let Ok(state) = self.inner.bootstrap.lock() else {
                    return;
                };
                if state.ready {
                    return;
                }
                self.inner.bootstrap_notify.notified()
            };
            notified.await;
        }
    }

    fn finish_bootstrap(&self, snapshot: Result<RegistrySnapshot, String>) {
        let mut bootstrap_error = None;
        match snapshot {
            Ok(snapshot) => match self.inner.registry.lock() {
                Ok(mut registry) => {
                    *registry = snapshot;
                }
                Err(_) => {
                    bootstrap_error =
                        Some("Engine state lock was poisoned during bootstrap.".to_string());
                }
            },
            Err(error) => bootstrap_error = Some(error),
        }

        let state = match self.inner.bootstrap.lock() {
            Ok(mut bootstrap) => {
                bootstrap.ready = true;
                bootstrap.error = bootstrap_error.clone();
                bootstrap.running = false;
                EngineBootstrapState {
                    ready: bootstrap.ready,
                    error: bootstrap.error.clone(),
                }
            }
            Err(_) => EngineBootstrapState {
                ready: true,
                error: bootstrap_error
                    .or_else(|| Some("Engine bootstrap state lock was poisoned.".to_string())),
            },
        };

        self.inner.bootstrap_notify.notify_waiters();
        let _ = self.inner.app.emit(ENGINE_BOOTSTRAP_EVENT, state);
        if let Ok(registry) = self.inner.registry.lock() {
            self.emit_engine_settings(&registry.settings);
        }
    }

    fn registry_guard(&self) -> Result<MutexGuard<'_, RegistrySnapshot>, String> {
        self.inner
            .registry
            .lock()
            .map_err(|_| "Engine state lock was poisoned.".to_string())
    }

    fn persist_registry(&self, registry: &RegistrySnapshot) -> Result<(), String> {
        self.persist_registry_with_priority(registry, PersistPriority::Deferred)
    }

    fn persist_registry_flush(&self, registry: &RegistrySnapshot) -> Result<(), String> {
        self.persist_registry_with_priority(registry, PersistPriority::Flush)
    }

    fn persist_registry_with_priority(
        &self,
        registry: &RegistrySnapshot,
        priority: PersistPriority,
    ) -> Result<(), String> {
        self.inner.snapshot_writer.persist(registry, priority)
    }

    fn retain_wake_lock(&self) {
        if let Ok(mut wake_lock) = self.inner.wake_lock.lock() {
            wake_lock.retain();
        }
    }

    fn apply_runtime_dispatch_plan(&self, plan: RuntimeDispatchPlan, min_emit_interval_ms: u32) {
        plan.apply(
            |download| {
                self.emit_download_upsert(download);
                self.emit_download_progress_diff_if_due(download, min_emit_interval_ms);
            },
            |id| {
                self.spawn_download_runtime(id);
            },
        );
    }

    fn release_wake_lock(&self) {
        if let Ok(mut wake_lock) = self.inner.wake_lock.lock() {
            wake_lock.release();
        }
    }

    fn host_rate_limiter(
        &self,
        host: &str,
        rate_bytes_per_second: Option<u64>,
    ) -> Option<Arc<TokenBucketRateLimiter>> {
        let rate = rate_bytes_per_second?;
        let burst = token_bucket_burst_bytes(rate);
        let Ok(mut limiters) = self.inner.host_limiters.lock() else {
            return build_token_bucket_limiter(Some(rate));
        };
        if let Some(existing) = limiters.get(host) {
            existing.reconfigure(rate, burst);
            return Some(Arc::clone(existing));
        }
        let limiter = build_token_bucket_limiter(Some(rate))?;
        limiters.insert(host.to_string(), Arc::clone(&limiter));
        Some(limiter)
    }

    fn download_rate_limiter(
        &self,
        download_id: &str,
        rate_bytes_per_second: Option<u64>,
    ) -> Arc<TokenBucketRateLimiter> {
        let rate = rate_bytes_per_second.unwrap_or(0);
        let burst = token_bucket_burst_bytes(rate);
        let Ok(mut limiters) = self.inner.download_limiters.lock() else {
            return Arc::new(TokenBucketRateLimiter::new(rate, burst));
        };
        if let Some(existing) = limiters.get(download_id) {
            existing.reconfigure(rate, burst);
            return Arc::clone(existing);
        }

        let limiter = Arc::new(TokenBucketRateLimiter::new(rate, burst));
        limiters.insert(download_id.to_string(), Arc::clone(&limiter));
        limiter
    }

    fn reconfigure_download_rate_limiter(
        &self,
        download_id: &str,
        rate_bytes_per_second: Option<u64>,
    ) {
        let rate = rate_bytes_per_second.unwrap_or(0);
        let burst = token_bucket_burst_bytes(rate);
        if let Ok(limiters) = self.inner.download_limiters.lock()
            && let Some(existing) = limiters.get(download_id)
        {
            existing.reconfigure(rate, burst);
        }
    }

    fn clear_download_rate_limiter(&self, download_id: &str) {
        if let Ok(mut limiters) = self.inner.download_limiters.lock() {
            limiters.remove(download_id);
        }
    }

    fn compact_download_for_row(download: &DownloadRecord) -> DownloadRecord {
        let mut row = download.clone();
        row.engine_log.clear();
        row.runtime_checkpoint = DownloadRuntimeCheckpoint::default();
        row
    }

    fn emit_download_upsert(&self, download: &DownloadRecord) {
        let _ = self.inner.app.emit(DOWNLOAD_UPSERT_EVENT, download);
        let row = Self::compact_download_for_row(download);
        let _ = self.inner.app.emit(DOWNLOAD_UPSERT_ROW_EVENT, row);
    }

    fn emit_engine_settings(&self, settings: &EngineSettings) {
        let _ = self.inner.app.emit(ENGINE_SETTINGS_EVENT, settings);
    }

    fn emit_download_progress_diff_if_due(&self, download: &DownloadRecord, min_interval_ms: u32) {
        let now = unix_epoch_millis();
        let Ok(mut sync) = self.inner.progress_sync.lock() else {
            return;
        };
        let entry = sync
            .by_download
            .entry(download.id.clone())
            .or_insert_with(ProgressSyncEntry::default);

        let mut changed_segments = Vec::new();
        let mut changed_segment_updates = Vec::new();
        for segment in &download.segments {
            let downloaded_changed = entry
                .segment_downloaded
                .get(&segment.id)
                .copied()
                .unwrap_or(-1)
                != segment.downloaded;
            let status_changed = entry
                .segment_status
                .get(&segment.id)
                .cloned()
                .unwrap_or(DownloadSegmentStatus::Pending)
                != segment.status;
            if downloaded_changed || status_changed {
                let status = segment.status.clone();
                changed_segments.push(SegmentProgressDiff {
                    id: segment.id,
                    downloaded: segment.downloaded,
                    status: status.clone(),
                });
                changed_segment_updates.push((segment.id, segment.downloaded, status));
            }
        }

        let download_changed = entry.last_downloaded != download.downloaded
            || entry.last_speed != download.speed
            || entry.last_time_left != download.time_left
            || entry.last_status != download.status
            || entry.last_writer_backpressure != download.writer_backpressure
            || entry.last_target_connections != download.target_connections;
        if changed_segments.is_empty() && !download_changed {
            return;
        }

        let elapsed_ms = now.saturating_sub(entry.last_emit_ms);
        let force_emit = entry.last_status != download.status
            || !matches!(download.status, DownloadStatus::Downloading);
        if !force_emit && elapsed_ms < i64::from(min_interval_ms) {
            return;
        }

        let event = DownloadProgressDiffEvent {
            id: download.id.clone(),
            downloaded: download.downloaded,
            speed: download.speed,
            time_left: download.time_left,
            status: download.status.clone(),
            writer_backpressure: download.writer_backpressure,
            target_connections: download.target_connections,
            segments: changed_segments,
        };
        let _ = self.inner.app.emit(DOWNLOAD_PROGRESS_DIFF_EVENT, event);

        entry.last_emit_ms = now;
        entry.last_downloaded = download.downloaded;
        entry.last_speed = download.speed;
        entry.last_time_left = download.time_left;
        entry.last_status = download.status.clone();
        entry.last_writer_backpressure = download.writer_backpressure;
        entry.last_target_connections = download.target_connections;
        for (segment_id, downloaded, status) in changed_segment_updates {
            entry.segment_downloaded.insert(segment_id, downloaded);
            entry.segment_status.insert(segment_id, status);
        }
        if entry.segment_downloaded.len() > download.segments.len()
            || entry.segment_status.len() > download.segments.len()
        {
            let active_segment_ids: HashSet<u32> =
                download.segments.iter().map(|segment| segment.id).collect();
            entry
                .segment_downloaded
                .retain(|segment_id, _| active_segment_ids.contains(segment_id));
            entry
                .segment_status
                .retain(|segment_id, _| active_segment_ids.contains(segment_id));
        }
    }

    fn emit_download_removed(&self, id: &str) {
        let _ = self.inner.app.emit(
            DOWNLOAD_REMOVE_EVENT,
            DownloadRemovedEvent { id: id.to_string() },
        );
    }

    fn emit_download_completed(&self, download: &DownloadRecord) {
        let _ = self.inner.app.emit(
            DOWNLOAD_COMPLETED_EVENT,
            DownloadCompletedEvent {
                id: download.id.clone(),
                name: download.name.clone(),
                target_path: download.target_path.clone(),
                save_path: download.save_path.clone(),
            },
        );
    }

    fn trigger_download_completion_actions(&self, download: &DownloadRecord) {
        self.emit_download_completed(download);
        if !download.open_folder_on_completion {
            return;
        }

        let engine = self.clone();
        let id = download.id.clone();
        tauri::async_runtime::spawn(async move {
            let _ = engine.open_download_folder(&id).await;
        });
    }

    async fn run_checksum_verification(&self, id: &str) -> Result<(), String> {
        let (target_path, expected) = {
            let mut registry = self.registry_guard()?;
            let Some(download) = registry
                .downloads
                .iter_mut()
                .find(|download| download.id == id)
            else {
                return Ok(());
            };
            let Some(expected) = download.integrity.expected.clone() else {
                return Ok(());
            };
            if !matches!(download.status, DownloadStatus::Finished) {
                return Ok(());
            }

            mark_integrity_verifying(&mut download.integrity);
            append_download_log(
                download,
                DownloadLogLevel::Info,
                "integrity.verify-started",
                "Started checksum verification for the completed file.",
            );

            let response = download.clone();
            self.persist_registry(&registry)?;
            drop(registry);
            self.emit_download_upsert(&response);
            (PathBuf::from(response.target_path), expected)
        };

        let actual_result = compute_checksum(target_path, expected.clone()).await;

        let mut registry = self.registry_guard()?;
        let Some(download) = registry
            .downloads
            .iter_mut()
            .find(|download| download.id == id)
        else {
            return Ok(());
        };
        if !matches!(download.status, DownloadStatus::Finished) {
            return Ok(());
        }
        if download.integrity.expected.as_ref() != Some(&expected) {
            return Ok(());
        }

        match actual_result {
            Ok(actual) => {
                let matched = actual == expected.value;
                let detail = if matched {
                    format!(
                        "Verified {} checksum for the completed file.",
                        expected.value
                    )
                } else {
                    format!(
                        "Checksum mismatch: expected {} but computed {}.",
                        expected.value, actual
                    )
                };
                apply_integrity_result(
                    &mut download.integrity,
                    actual,
                    matched,
                    unix_epoch_millis(),
                );
                append_download_log(
                    download,
                    if matched {
                        DownloadLogLevel::Info
                    } else {
                        DownloadLogLevel::Warn
                    },
                    if matched {
                        "integrity.verified"
                    } else {
                        "integrity.mismatch"
                    },
                    detail.clone(),
                );
                if !matched
                    && !download
                        .diagnostics
                        .warnings
                        .iter()
                        .any(|value| value == &detail)
                {
                    download.diagnostics.warnings.push(detail);
                }
            }
            Err(error) => {
                let detail = format!("Checksum verification failed: {error}");
                mark_integrity_failure(&mut download.integrity, &error);
                append_download_log(
                    download,
                    DownloadLogLevel::Error,
                    "integrity.verify-failed",
                    detail.clone(),
                );
                if !download
                    .diagnostics
                    .warnings
                    .iter()
                    .any(|value| value == &detail)
                {
                    download.diagnostics.warnings.push(detail);
                }
            }
        }

        let response = download.clone();
        self.persist_registry(&registry)?;
        drop(registry);
        self.emit_download_upsert(&response);
        Ok(())
    }

    fn record_runtime_task_failure(&self, id: &str, error: &str) {
        let Ok(mut registry) = self.registry_guard() else {
            return;
        };
        let Some(download) = registry
            .downloads
            .iter_mut()
            .find(|download| download.id == id)
        else {
            return;
        };
        if matches!(
            download.status,
            DownloadStatus::Finished | DownloadStatus::Paused | DownloadStatus::Stopped
        ) {
            return;
        }

        download.status = DownloadStatus::Error;
        reset_download_transient_state(download);
        download.error_message = Some(error.to_string());
        if download.diagnostics.failure_kind.is_none() {
            download.diagnostics.failure_kind = Some(classify_runtime_task_failure_kind(error));
        }
        download.diagnostics.restart_required = download.downloaded > 0;
        download
            .diagnostics
            .terminal_reason
            .get_or_insert_with(|| "Transfer runtime terminated before completion.".to_string());

        let response = download.clone();
        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let min_emit_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        let _ = self.persist_registry(&registry);
        drop(registry);
        self.emit_download_upsert(&response);
        self.emit_download_progress_diff_if_due(&response, min_emit_interval_ms);
        self.apply_runtime_dispatch_plan(dispatch_plan, min_emit_interval_ms);
    }

    fn abort_integrity_task(&self, id: &str) {
        if let Ok(mut tasks) = self.inner.integrity_tasks.lock()
            && let Some(handle) = tasks.remove(id)
        {
            handle.abort();
        }
    }

    fn abort_runtime_task(&self, id: &str) {
        if let Ok(mut tasks) = self.inner.runtime_tasks.lock()
            && let Some(handle) = tasks.remove(id)
        {
            handle.abort();
        }
    }

    fn abort_all_runtime_tasks(&self) {
        if let Ok(mut tasks) = self.inner.runtime_tasks.lock() {
            for handle in std::mem::take(&mut *tasks).into_values() {
                handle.abort();
            }
        }
    }

    fn spawn_checksum_verification(&self, id: String) {
        if let Ok(tasks) = self.inner.integrity_tasks.lock()
            && tasks.contains_key(&id)
        {
            return;
        }

        let engine = self.clone();
        let task_id = id.clone();
        let handle = tokio::spawn(async move {
            let _ = engine.run_checksum_verification(&task_id).await;
            if let Ok(mut tasks) = engine.inner.integrity_tasks.lock() {
                tasks.remove(&task_id);
            }
        });
        match self.inner.integrity_tasks.lock() {
            Ok(mut tasks) => {
                tasks.insert(id, handle);
            }
            Err(_) => {
                drop(handle);
            }
        }
    }

    fn spawn_download_runtime(&self, id: String) {
        if let Ok(tasks) = self.inner.runtime_tasks.lock()
            && tasks.contains_key(&id)
        {
            return;
        }
        let engine = self.clone();
        let task_id = id.clone();
        let handle = tokio::spawn(async move {
            if let Err(error) = engine.run_download_runtime(&task_id).await {
                engine.record_runtime_task_failure(&task_id, &error);
            }
            if let Ok(mut tasks) = engine.inner.runtime_tasks.lock() {
                tasks.remove(&task_id);
            }
            engine.clear_download_rate_limiter(&task_id);
        });
        match self.inner.runtime_tasks.lock() {
            Ok(mut tasks) => {
                tasks.insert(id, handle);
            }
            Err(_) => {
                drop(handle);
            }
        }
    }
}
