use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinSet;

use super::probe::{DownloadProbeData, RangeObservation, probe_runtime_bootstrap_with_client};
use super::progress::{
    aggregate_runtime_throughput, estimate_time_left, recompute_download_speed,
    stabilized_segment_throughput,
};
use super::runtime_race::{
    attempt_runtime_queue_expansion, resolve_runtime_race_winner,
    restore_runtime_races_from_checkpoint,
};
use super::runtime_ramp::*;
use super::runtime_recovery::{
    classify_runtime_error, reconcile_runtime_error, runtime_control_flow_error,
    runtime_validation_error,
};
use super::runtime_state::{
    persist_runtime_races, upsert_runtime_segment_health, upsert_runtime_segment_sample,
};
use super::runtime_transfer::{
    InitialResponseStream, SegmentRuntimeControl, SegmentWorkerStart, TransferWorkerConfig,
    run_segment_worker,
};
use super::runtime_unknown_size::run_unknown_size_stream_runtime;
use super::scheduler::{SegmentRuntimeSample, SegmentScheduler};
use super::*;
use crate::model::{DownloadCompatibility, ResumeValidators};

const RUNTIME_ADAPTIVE_POLL_MS: u64 = 400;
pub(super) const UNKNOWN_SIZE_SPACE_CHECK_INTERVAL_BYTES: u64 = 8 * 1024 * 1024;
pub(super) const UNKNOWN_SIZE_SPACE_SAFETY_MARGIN_BYTES: u64 = 64 * 1024 * 1024;
const DISK_DRAIN_WAIT_POLL_MS: u64 = 25;
const DISK_DRAIN_WAIT_TIMEOUT_MS: u64 = 20_000;

pub(super) struct RuntimeWakeLockGuard {
    engine: EngineState,
    active: bool,
}

impl RuntimeWakeLockGuard {
    pub(super) fn acquire(engine: &EngineState) -> Self {
        engine.retain_wake_lock();
        Self {
            engine: engine.clone(),
            active: true,
        }
    }
}

impl Drop for RuntimeWakeLockGuard {
    fn drop(&mut self) {
        if self.active {
            self.engine.release_wake_lock();
            self.active = false;
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct RuntimeTelemetrySnapshot {
    pub(super) ttfb_samples_ms: Vec<u64>,
    pub(super) protocol_counts: BTreeMap<String, u32>,
    pub(super) reused_true: u32,
    pub(super) reused_false: u32,
}

struct RuntimeBootstrapOutcome {
    host_profile: Option<HostProfile>,
    initial_stream: Option<InitialResponseStream>,
    probe: Option<DownloadProbeData>,
}

struct RuntimeProbeTarget<'a> {
    url: &'a str,
    request_referer: Option<&'a str>,
    request_cookies: Option<&'a str>,
    request_method: DownloadRequestMethod,
    request_form_fields: &'a [DownloadRequestField],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuntimeRampSample {
    recorded_at_ms: i64,
    target_connections: u32,
    speed_bytes_per_second: u64,
    ttfb_ms: Option<u64>,
}

struct RuntimeRampState {
    samples: VecDeque<RuntimeRampSample>,
    last_change_at_ms: i64,
    last_change_speed: Option<u64>,
    last_changed_target: u32,
    negative_gain_windows: u32,
}

impl RuntimeRampState {
    fn new(now_ms: i64, current_target: u32) -> Self {
        Self {
            samples: VecDeque::new(),
            last_change_at_ms: now_ms,
            last_change_speed: None,
            last_changed_target: current_target.max(1),
            negative_gain_windows: 0,
        }
    }

    fn record_sample(
        &mut self,
        now_ms: i64,
        current_target: u32,
        speed_bytes_per_second: u64,
        ttfb_ms: Option<u64>,
    ) {
        let current_target = current_target.max(1);
        if self.last_changed_target != current_target {
            self.last_changed_target = current_target;
            self.last_change_at_ms = now_ms;
            self.last_change_speed = None;
            self.negative_gain_windows = 0;
        }

        let sample = RuntimeRampSample {
            recorded_at_ms: now_ms,
            target_connections: current_target,
            speed_bytes_per_second,
            ttfb_ms,
        };
        if let Some(last) = self.samples.back_mut()
            && last.target_connections == current_target
            && now_ms.saturating_sub(last.recorded_at_ms) < RUNTIME_RAMP_SAMPLE_MIN_INTERVAL_MS
        {
            *last = sample;
        } else {
            self.samples.push_back(sample);
        }

        let retain_after = now_ms.saturating_sub(RUNTIME_RAMP_HISTORY_RETENTION_MS);
        while matches!(self.samples.front(), Some(front) if front.recorded_at_ms < retain_after) {
            self.samples.pop_front();
        }
        while self.samples.len() > RUNTIME_RAMP_WINDOW_SAMPLES.saturating_mul(4) {
            self.samples.pop_front();
        }
    }

    fn stable_speed(&self, current_target: u32) -> Option<u64> {
        average_recent_u64(
            self.samples
                .iter()
                .rev()
                .filter(|sample| sample.target_connections == current_target)
                .map(|sample| sample.speed_bytes_per_second)
                .filter(|value| *value > 0)
                .take(RUNTIME_RAMP_WINDOW_SAMPLES),
        )
    }

    fn stable_ttfb_ms(&self, current_target: u32) -> Option<u64> {
        let mut samples: Vec<u64> = self
            .samples
            .iter()
            .rev()
            .filter(|sample| sample.target_connections == current_target)
            .filter_map(|sample| sample.ttfb_ms)
            .take(RUNTIME_RAMP_WINDOW_SAMPLES)
            .collect();
        if samples.is_empty() {
            return None;
        }
        samples.sort_unstable();
        Some(samples[samples.len() / 2])
    }

    fn sample_count(&self, current_target: u32) -> usize {
        self.samples
            .iter()
            .rev()
            .filter(|sample| sample.target_connections == current_target)
            .take(RUNTIME_RAMP_WINDOW_SAMPLES)
            .count()
    }

    fn marginal_gain(&self, current_target: u32) -> Option<i64> {
        if self.last_changed_target != current_target {
            return None;
        }

        let baseline = self.last_change_speed?;
        let stable_speed = self.stable_speed(current_target)?;
        Some(
            i64::try_from(stable_speed)
                .unwrap_or(i64::MAX)
                .saturating_sub(i64::try_from(baseline).unwrap_or(i64::MAX)),
        )
    }

    fn mark_change(&mut self, now_ms: i64, next_target: u32, baseline_speed: u64) {
        self.last_change_at_ms = now_ms;
        self.last_change_speed = Some(baseline_speed);
        self.last_changed_target = next_target.max(1);
        self.negative_gain_windows = 0;
    }

    fn reset_negative_gain_windows(&mut self) {
        self.negative_gain_windows = 0;
    }

    fn note_negative_gain_window(&mut self) {
        self.negative_gain_windows = self.negative_gain_windows.saturating_add(1);
    }
}

fn average_recent_u64(values: impl Iterator<Item = u64>) -> Option<u64> {
    let mut total = 0_u64;
    let mut count = 0_u64;
    for value in values {
        total = total.saturating_add(value);
        count = count.saturating_add(1);
    }
    if count == 0 {
        None
    } else {
        Some(total / count)
    }
}

pub(super) fn median_runtime_ttfb(snapshot: &RuntimeTelemetrySnapshot) -> Option<u64> {
    let mut samples = snapshot.ttfb_samples_ms.clone();
    if samples.is_empty() {
        return None;
    }
    samples.sort_unstable();
    Some(samples[samples.len() / 2])
}

pub(super) fn runtime_connection_reuse_hint(snapshot: &RuntimeTelemetrySnapshot) -> Option<bool> {
    let total = snapshot.reused_true.saturating_add(snapshot.reused_false);
    if total == 0 {
        return None;
    }
    Some(snapshot.reused_true >= snapshot.reused_false)
}

pub(super) fn runtime_protocol_hint(snapshot: &RuntimeTelemetrySnapshot) -> Option<String> {
    snapshot
        .protocol_counts
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(protocol, _)| protocol.clone())
}

fn resume_validators_compatible(saved: &ResumeValidators, fresh: &ResumeValidators) -> bool {
    if let (Some(left), Some(right)) = (&saved.etag, &fresh.etag)
        && left != right
    {
        return false;
    }
    if let (Some(left), Some(right)) = (&saved.last_modified, &fresh.last_modified)
        && left != right
    {
        return false;
    }
    if let (Some(left), Some(right)) = (saved.content_length, fresh.content_length)
        && left != right
    {
        return false;
    }
    true
}

fn needs_runtime_metadata_bootstrap(download: &DownloadRecord) -> bool {
    requires_wrapper_probe_refresh(download)
        || (download.downloaded <= 0
            && (download.size <= 0
                || download.validators.content_length.is_none()
                || (download.validators.etag.is_none()
                    && download.validators.last_modified.is_none())))
}

fn requires_wrapper_probe_refresh(download: &DownloadRecord) -> bool {
    download.url != download.final_url
        && (download.compatibility.wrapper_detected
            || download.compatibility.direct_url_recovered
            || download.compatibility.browser_interstitial_only)
}

fn runtime_probe_target(download: &DownloadRecord) -> RuntimeProbeTarget<'_> {
    if requires_wrapper_probe_refresh(download) {
        RuntimeProbeTarget {
            url: &download.url,
            request_referer: None,
            request_cookies: download.compatibility.request_cookies.as_deref(),
            request_method: DownloadRequestMethod::Get,
            request_form_fields: &[][..],
        }
    } else {
        RuntimeProbeTarget {
            url: &download.final_url,
            request_referer: download.compatibility.request_referer.as_deref(),
            request_cookies: download.compatibility.request_cookies.as_deref(),
            request_method: download.compatibility.request_method.clone(),
            request_form_fields: &download.compatibility.request_form_fields,
        }
    }
}

fn should_prefer_fresh_request_context(fresh: &DownloadCompatibility) -> bool {
    let exact_request_context = !super::http_helpers::request_context_supports_segmented_transfer(
        &fresh.request_method,
        &fresh.request_form_fields,
    );

    exact_request_context
        || fresh.direct_url_recovered
        || (!fresh.wrapper_detected && !fresh.browser_interstitial_only)
}

fn merge_resume_validators(saved: &ResumeValidators, fresh: &ResumeValidators) -> ResumeValidators {
    ResumeValidators {
        etag: fresh.etag.clone().or_else(|| saved.etag.clone()),
        last_modified: fresh
            .last_modified
            .clone()
            .or_else(|| saved.last_modified.clone()),
        content_length: fresh.content_length.or(saved.content_length),
        content_type: fresh
            .content_type
            .clone()
            .or_else(|| saved.content_type.clone()),
        content_disposition: fresh
            .content_disposition
            .clone()
            .or_else(|| saved.content_disposition.clone()),
    }
}

fn merge_download_compatibility(
    saved: &DownloadCompatibility,
    fresh: &DownloadCompatibility,
) -> DownloadCompatibility {
    let direct_url_recovered = saved.direct_url_recovered || fresh.direct_url_recovered;
    let prefer_fresh_request_context = should_prefer_fresh_request_context(fresh);

    DownloadCompatibility {
        redirect_chain: if !fresh.redirect_chain.is_empty() {
            fresh.redirect_chain.clone()
        } else {
            saved.redirect_chain.clone()
        },
        filename_source: fresh
            .filename_source
            .clone()
            .or_else(|| saved.filename_source.clone()),
        classification: fresh
            .classification
            .clone()
            .or_else(|| saved.classification.clone()),
        wrapper_detected: saved.wrapper_detected || fresh.wrapper_detected,
        direct_url_recovered,
        browser_interstitial_only: !direct_url_recovered
            && (saved.browser_interstitial_only || fresh.browser_interstitial_only),
        request_referer: if prefer_fresh_request_context {
            fresh
                .request_referer
                .clone()
                .or_else(|| saved.request_referer.clone())
        } else {
            saved
                .request_referer
                .clone()
                .or_else(|| fresh.request_referer.clone())
        },
        request_cookies: if prefer_fresh_request_context {
            fresh
                .request_cookies
                .clone()
                .or_else(|| saved.request_cookies.clone())
        } else {
            saved
                .request_cookies
                .clone()
                .or_else(|| fresh.request_cookies.clone())
        },
        request_method: if prefer_fresh_request_context {
            fresh.request_method.clone()
        } else {
            saved.request_method.clone()
        },
        request_form_fields: if prefer_fresh_request_context {
            fresh.request_form_fields.clone()
        } else {
            saved.request_form_fields.clone()
        },
    }
}

fn should_use_bootstrap_stream(download: &DownloadRecord) -> bool {
    download.downloaded <= 0 && (download.size <= 0 || !should_use_segmented_mode(download))
}

fn additional_reservation_bytes_required(download: &DownloadRecord) -> u64 {
    if download.size <= 0 {
        return 0;
    }

    let reserved_len = fs::metadata(&download.temp_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let target_size = download.size.max(0) as u64;

    target_size.saturating_sub(reserved_len.min(target_size))
}

fn ensure_known_size_reservation_capacity(download: &DownloadRecord) -> Result<(), String> {
    if download.size <= 0 {
        return Ok(());
    }

    let required_bytes = additional_reservation_bytes_required(download);
    if required_bytes == 0 {
        return Ok(());
    }

    let reserve_root = Path::new(&download.temp_path)
        .parent()
        .unwrap_or_else(|| Path::new(&download.save_path));
    let Some(available_space) = query_available_space(reserve_root) else {
        return Ok(());
    };

    if available_space >= required_bytes {
        return Ok(());
    }

    Err(format!(
        "Selected volume has {} free but VDM still needs {} to reserve the {} temp file before transfer can start.",
        format_bytes_compact(available_space),
        format_bytes_compact(required_bytes),
        format_bytes_compact(download.size.max(0) as u64),
    ))
}

fn reserve_known_size_temp_file(file: &std::fs::File, size: u64) -> Result<(), String> {
    super::disk::allocate_file(file, size).map_err(|error| {
        format!(
            "Failed reserving {} for the temp file: {error}",
            format_bytes_compact(size)
        )
    })
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

pub(super) fn disk_queue_under_pressure(disk_pool: &disk_pool::DiskPool) -> bool {
    disk_pool.under_pressure()
}

pub(super) fn disk_queue_blocks_new_supply(disk_pool: &disk_pool::DiskPool) -> bool {
    disk_pool.blocks_new_supply()
}

pub(super) fn disk_queue_parallelism_target(
    disk_pool: &disk_pool::DiskPool,
    requested_parallelism: u32,
) -> u32 {
    disk_pool.recommended_parallelism(requested_parallelism)
}

fn apply_bootstrap_capabilities(download: &mut DownloadRecord, bootstrap: &DownloadProbeData) {
    match bootstrap.range_observation {
        RangeObservation::Supported => {
            download.capabilities.range_supported = true;
            download.capabilities.resumable = bootstrap.resumable;
        }
        RangeObservation::Unsupported => {
            download.capabilities.range_supported = false;
            download.capabilities.resumable = false;
            download.capabilities.segmented = false;
        }
        RangeObservation::Unknown => {}
    }
}

fn requires_unknown_size_restart(download: &DownloadRecord) -> bool {
    download.size <= 0 && download.downloaded > 0
}

pub(super) fn take_disk_pool_error(disk_pool: &disk_pool::DiskPool) -> Result<(), String> {
    if let Some(error) = disk_pool.take_error() {
        return Err(format!("Disk write failed: {error}"));
    }
    Ok(())
}

pub(super) async fn wait_for_disk_pool_drain(
    disk_pool: &disk_pool::DiskPool,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        take_disk_pool_error(disk_pool)?;
        if disk_pool.pending_writes() == 0 {
            return Ok(());
        }
        if started.elapsed() >= Duration::from_millis(DISK_DRAIN_WAIT_TIMEOUT_MS) {
            return Err("Timed out waiting for queued disk writes to finish.".to_string());
        }
        tokio::time::sleep(Duration::from_millis(DISK_DRAIN_WAIT_POLL_MS)).await;
    }
}

async fn bootstrap_runtime_metadata(
    engine: &EngineState,
    runtime_download: &mut DownloadRecord,
    settings: &EngineSettings,
    http_pool: &http_pool::HttpPool,
    min_emit_interval_ms: u32,
) -> Result<RuntimeBootstrapOutcome, String> {
    if !needs_runtime_metadata_bootstrap(runtime_download) {
        return Ok(RuntimeBootstrapOutcome {
            host_profile: None,
            initial_stream: None,
            probe: None,
        });
    }

    let probe_target = runtime_probe_target(runtime_download);
    let client_lease = http_pool
        .get_client(probe_target.url)
        .ok_or_else(|| "Failed to acquire HTTP client for transfer bootstrap.".to_string())?;
    let bootstrap = probe_runtime_bootstrap_with_client(
        client_lease.client.as_ref(),
        probe_target.url,
        probe_target.request_referer,
        probe_target.request_cookies,
        &probe_target.request_method,
        probe_target.request_form_fields,
        should_use_bootstrap_stream(runtime_download),
    )
    .await
    .map_err(|error| format!("Transfer-start metadata bootstrap failed: {error}"))?;
    let pooled_client_reused = client_lease.reused_pool_client;
    let probe = bootstrap.probe;
    let probe_snapshot = probe.clone();

    let now = unix_epoch_millis();
    let bootstrap_host = extract_host(&probe.final_url);
    let scope_key = probe_scope_key(
        &runtime_download.url,
        &probe.compatibility.request_method,
        &probe.compatibility.request_form_fields,
    );
    let mut registry = engine.registry_guard()?;
    {
        let profile = registry
            .host_profiles
            .entry(bootstrap_host.clone())
            .or_default();
        update_profile_probe_cache(profile, &scope_key, &probe, now);
    }
    store_recent_probe(&mut registry, &scope_key, &bootstrap_host, &probe, now);
    let host_profile_snapshot = registry.host_profiles.get(&bootstrap_host).cloned();
    let Some(download) = registry
        .downloads
        .iter_mut()
        .find(|download| download.id == runtime_download.id)
    else {
        return Err("Download disappeared during transfer bootstrap.".to_string());
    };

    let size_was_unknown = download.size <= 0;
    let validators_were_incomplete = download.validators.content_length.is_none()
        || (download.validators.etag.is_none() && download.validators.last_modified.is_none());
    let final_url_changed = download.final_url != probe.final_url;

    download.host = bootstrap_host.clone();
    download.final_url = probe.final_url.clone();
    if let Some(size) = probe.size {
        download.size = i64::try_from(size).unwrap_or(i64::MAX);
    }
    if probe.mime_type.is_some() {
        download.content_type = probe.mime_type.clone();
    }
    download.validators = merge_resume_validators(&download.validators, &probe.validators);
    if let Some(protocol) = probe.negotiated_protocol.clone() {
        download.host_protocol = Some(protocol);
    }
    download.compatibility =
        merge_download_compatibility(&download.compatibility, &probe.compatibility);
    apply_bootstrap_capabilities(download, &probe);
    for warning in probe.warnings {
        push_unique_diagnostic(&mut download.diagnostics.warnings, warning);
    }
    if (size_was_unknown && download.size > 0)
        || (validators_were_incomplete
            && (download.validators.content_length.is_some()
                || download.validators.etag.is_some()
                || download.validators.last_modified.is_some()))
    {
        push_unique_diagnostic(
            &mut download.diagnostics.notes,
            "Live transfer bootstrap refreshed size and resume validators from the first stable download response.",
        );
    }
    if final_url_changed {
        push_unique_diagnostic(
            &mut download.diagnostics.warnings,
            "Transfer startup redirected to a different final URL while stabilizing metadata.",
        );
    }
    append_download_log(
        download,
        DownloadLogLevel::Info,
        "runtime.bootstrap",
        if final_url_changed {
            "Runtime bootstrap refreshed metadata and stabilized a new final URL."
        } else {
            "Runtime bootstrap refreshed transfer metadata from a stable download response."
        },
    );

    apply_download_host_profile(download, settings, host_profile_snapshot.as_ref());

    let response = download.clone();
    engine.persist_registry(&registry)?;
    drop(registry);
    engine.emit_download_upsert(&response);
    engine.emit_download_progress_diff_if_due(&response, min_emit_interval_ms);
    *runtime_download = response;
    Ok(RuntimeBootstrapOutcome {
        host_profile: host_profile_snapshot,
        initial_stream: bootstrap
            .reusable_stream
            .map(|stream| InitialResponseStream {
                response: stream.response,
                ttfb_ms: stream.ttfb_ms,
                connection_reused_hint: pooled_client_reused,
                negotiated_protocol: stream.negotiated_protocol,
            }),
        probe: Some(probe_snapshot),
    })
}

struct RuntimeSupplyAdjustment {
    desired_parallel: usize,
    appended_segments: Vec<DownloadSegment>,
    control_updates: Vec<(u32, i64)>,
    response: Option<DownloadRecord>,
    dispatch_plan: RuntimeDispatchPlan,
}

struct RuntimeSupplyInputs<'a> {
    settings: &'a EngineSettings,
    scheduler: &'a SegmentScheduler,
    runtime_samples: &'a [SegmentRuntimeSample],
    disk_pool: &'a disk_pool::DiskPool,
    ramp_state: &'a mut RuntimeRampState,
    median_ttfb_ms: Option<u64>,
}

fn cooldown_retry_delay_ms(cooldown_until: Option<i64>) -> Option<u64> {
    let until = cooldown_until?;
    let remaining_ms = until.saturating_sub(unix_epoch_millis());
    if remaining_ms <= 0 {
        None
    } else {
        Some(u64::try_from(remaining_ms).unwrap_or(u64::MAX))
    }
}

fn runtime_ramp_requirements(current_target: u32) -> (i64, u64) {
    if current_target <= 2 {
        (
            RUNTIME_RAMP_WARMUP_MIN_INTERVAL_MS,
            RUNTIME_RAMP_WARMUP_MIN_SPEED_BYTES_PER_SECOND,
        )
    } else {
        (
            RUNTIME_RAMP_MIN_INTERVAL_MS,
            RUNTIME_RAMP_MIN_SPEED_BYTES_PER_SECOND,
        )
    }
}

fn ramp_positive_gain_threshold(reference_throughput: u64) -> i64 {
    let relative_threshold = reference_throughput
        .saturating_mul(RUNTIME_RAMP_FAST_PATH_GAIN_PERCENT)
        .div_ceil(100);
    super::host_planner::ramp_gain_threshold_bytes_per_second(reference_throughput)
        .max(i64::try_from(relative_threshold).unwrap_or(i64::MAX))
}

fn ramp_negative_gain_threshold(reference_throughput: u64) -> i64 {
    let relative_threshold = reference_throughput
        .saturating_mul(RUNTIME_RAMP_NEGATIVE_GAIN_PERCENT)
        .div_ceil(100);
    super::host_planner::ramp_gain_threshold_bytes_per_second(reference_throughput)
        .max(i64::try_from(relative_threshold).unwrap_or(i64::MAX))
}

fn should_fast_path_ramp(
    current_target: u32,
    max_target: u32,
    sample_count: usize,
    stable_ttfb_ms: Option<u64>,
    last_change_speed: Option<u64>,
    sustained_gain: Option<i64>,
) -> bool {
    if current_target != 2
        || max_target < 4
        || sample_count < RUNTIME_RAMP_WINDOW_SAMPLES
        || stable_ttfb_ms.is_none_or(|ttfb_ms| ttfb_ms > RUNTIME_RAMP_FAST_PATH_MAX_TTFB_MS)
    {
        return false;
    }

    let Some(reference_throughput) = last_change_speed.filter(|value| *value > 0) else {
        return false;
    };
    let Some(gain) = sustained_gain else {
        return false;
    };

    gain >= ramp_positive_gain_threshold(reference_throughput)
}

fn should_proactively_downshift(
    current_target: u32,
    sample_count: usize,
    last_change_speed: Option<u64>,
    sustained_gain: Option<i64>,
) -> bool {
    if current_target <= 1 || sample_count < RUNTIME_RAMP_WINDOW_SAMPLES {
        return false;
    }

    let Some(reference_throughput) = last_change_speed.filter(|value| *value > 0) else {
        return false;
    };
    let Some(gain) = sustained_gain else {
        return false;
    };

    gain <= -ramp_negative_gain_threshold(reference_throughput)
}

fn adjust_runtime_segment_supply(
    engine: &EngineState,
    download_id: &str,
    inputs: RuntimeSupplyInputs<'_>,
) -> Result<RuntimeSupplyAdjustment, String> {
    let now = unix_epoch_millis();
    let RuntimeSupplyInputs {
        settings,
        scheduler,
        runtime_samples,
        disk_pool,
        ramp_state,
        median_ttfb_ms,
    } = inputs;
    let disk_supply_guard = disk_queue_blocks_new_supply(disk_pool);
    let mut registry = engine.registry_guard()?;
    let Some(download_index) = registry
        .downloads
        .iter()
        .position(|download| download.id == download_id)
    else {
        return Ok(RuntimeSupplyAdjustment {
            desired_parallel: 1,
            appended_segments: Vec::new(),
            control_updates: Vec::new(),
            response: None,
            dispatch_plan: RuntimeDispatchPlan::default(),
        });
    };

    let mut telemetry_payload: Option<HostTelemetryArgs> = None;
    {
        let download = &mut registry.downloads[download_index];
        let current_target = download.target_connections.max(1);
        let max_target = download.max_connections.max(1);
        let active_workers = download
            .segments
            .iter()
            .filter(|segment| matches!(segment.status, DownloadSegmentStatus::Downloading))
            .count() as u32;
        let queued_workers = download
            .segments
            .iter()
            .filter(|segment| matches!(segment.status, DownloadSegmentStatus::Pending))
            .count() as u32;
        let remaining_bytes = (download.size - download.downloaded).max(0) as u64;
        let required_window = settings
            .min_segment_size_bytes
            .saturating_mul(u64::from(current_target.saturating_add(1)));
        let speed = download.speed;
        ramp_state.record_sample(now, current_target, speed, median_ttfb_ms);
        let stable_speed = ramp_state.stable_speed(current_target).unwrap_or(speed);
        let stable_ttfb_ms = ramp_state.stable_ttfb_ms(current_target).or(median_ttfb_ms);
        let sample_count = ramp_state.sample_count(current_target);
        let sustained_gain = ramp_state.marginal_gain(current_target);
        let (ramp_interval_ms, ramp_speed_floor) = runtime_ramp_requirements(current_target);
        let can_ramp = now.saturating_sub(ramp_state.last_change_at_ms) >= ramp_interval_ms
            && download.capabilities.segmented
            && !disk_supply_guard
            && cooldown_retry_delay_ms(download.host_cooldown_until).is_none()
            && current_target < max_target
            && active_workers.saturating_add(queued_workers) >= current_target
            && remaining_bytes >= required_window
            && stable_speed >= ramp_speed_floor
            && disk_queue_parallelism_target(
                disk_pool,
                current_target.saturating_add(1).min(max_target),
            ) > current_target;

        if can_ramp {
            let fast_path = should_fast_path_ramp(
                current_target,
                max_target,
                sample_count,
                stable_ttfb_ms,
                ramp_state.last_change_speed,
                sustained_gain,
            );
            let next_target = current_target
                .saturating_add(if fast_path { 2 } else { 1 })
                .min(max_target);
            if next_target > current_target {
                download.target_connections = next_target;
                ramp_state.mark_change(now, next_target, stable_speed.max(1));
                append_download_log(
                    download,
                    DownloadLogLevel::Info,
                    if fast_path {
                        "runtime.ramp-fast-path"
                    } else {
                        "runtime.ramp-up"
                    },
                    format!(
                        "Raised live target connections from {} to {} after a stable ramp window around {}/s{}.",
                        current_target,
                        next_target,
                        format_bytes_compact(stable_speed.max(1)),
                        stable_ttfb_ms
                            .map(|ttfb_ms| format!(" with median TTFB {}ms", ttfb_ms))
                            .unwrap_or_default()
                    ),
                );
                telemetry_payload = Some(HostTelemetryArgs {
                    host: download.host.clone(),
                    scope_key: Some(download_scope_key(download)),
                    attempted_connections: Some(next_target),
                    sustained_gain_bytes_per_second: sustained_gain,
                    throughput_bytes_per_second: Some(stable_speed).filter(|value| *value > 0),
                    ttfb_ms: stable_ttfb_ms,
                    negotiated_protocol: download.host_protocol.clone(),
                    connection_reused: None,
                    throttle_event: false,
                    timeout_event: false,
                    reset_event: false,
                    range_validation_failed: false,
                });
            }
        } else if !disk_supply_guard
            && cooldown_retry_delay_ms(download.host_cooldown_until).is_none()
            && active_workers.saturating_add(queued_workers) >= current_target
            && should_proactively_downshift(
                current_target,
                sample_count,
                ramp_state.last_change_speed,
                sustained_gain,
            )
        {
            ramp_state.note_negative_gain_window();
            if ramp_state.negative_gain_windows >= RUNTIME_RAMP_NEGATIVE_GAIN_WINDOWS {
                let next_target = current_target.saturating_sub(1).max(1);
                if next_target < current_target {
                    let lost_gain = sustained_gain.unwrap_or_default().unsigned_abs();
                    download.target_connections = next_target;
                    ramp_state.mark_change(now, next_target, stable_speed.max(1));
                    append_download_log(
                        download,
                        DownloadLogLevel::Warn,
                        "runtime.downshift",
                        format!(
                            "Reduced live target connections from {} to {} after the last ramp window lost about {}/s of aggregate throughput.",
                            current_target,
                            next_target,
                            format_bytes_compact(lost_gain)
                        ),
                    );
                    telemetry_payload = Some(HostTelemetryArgs {
                        host: download.host.clone(),
                        scope_key: Some(download_scope_key(download)),
                        attempted_connections: Some(current_target),
                        sustained_gain_bytes_per_second: sustained_gain,
                        throughput_bytes_per_second: Some(stable_speed).filter(|value| *value > 0),
                        ttfb_ms: stable_ttfb_ms,
                        negotiated_protocol: download.host_protocol.clone(),
                        connection_reused: None,
                        throttle_event: false,
                        timeout_event: false,
                        reset_event: false,
                        range_validation_failed: false,
                    });
                }
            }
        } else {
            ramp_state.reset_negative_gain_windows();
        }
    }

    if let Some(payload) = telemetry_payload.as_ref() {
        apply_host_feedback_to_registry(&mut registry, &payload.host, payload);
    }
    let dispatch_plan = if telemetry_payload.is_some() {
        plan_runtime_dispatch(&mut registry)
    } else {
        RuntimeDispatchPlan::default()
    };

    let mut adjustment = RuntimeSupplyAdjustment {
        desired_parallel: 1,
        appended_segments: Vec::new(),
        control_updates: Vec::new(),
        response: None,
        dispatch_plan,
    };
    {
        let download = &mut registry.downloads[download_index];
        let requested_parallel = download
            .target_connections
            .max(1)
            .min(download.max_connections.max(1));
        let target_parallel = disk_queue_parallelism_target(disk_pool, requested_parallel);
        adjustment.desired_parallel = usize::try_from(target_parallel)
            .unwrap_or(usize::MAX)
            .max(1);

        if download.capabilities.segmented && !disk_supply_guard {
            let refill = scheduler.fill_idle_slots(
                &mut download.segments,
                runtime_samples,
                download.size.max(0) as u64,
                adjustment.desired_parallel,
            );
            adjustment.control_updates = refill.control_updates;
            adjustment.appended_segments = refill.appended_segments;
        }

        if telemetry_payload.is_some() || !adjustment.appended_segments.is_empty() {
            adjustment.response = Some(download.clone());
        }
    }

    if adjustment.response.is_some() {
        engine.persist_registry(&registry)?;
    }

    Ok(adjustment)
}

impl EngineState {
    pub(super) async fn run_download_runtime(&self, id: &str) -> Result<(), String> {
        let (
            mut runtime_download,
            settings,
            _queue_running,
            min_emit_interval_ms,
            max_checkpoint_interval_ms,
            mut runtime_host_profile,
        ) = {
            let mut registry = self.registry_guard()?;
            let settings = registry.settings.clone();
            let queue_running = registry.queue_running;
            let min_emit_interval_ms = settings.segment_checkpoint_min_interval_ms;
            let max_checkpoint_interval_ms =
                settings.segment_checkpoint_max_interval_ms.max(100) as i64;
            let host_profile = registry
                .downloads
                .iter()
                .find(|download| download.id == id)
                .and_then(|download| registry.host_profiles.get(&download.host).cloned());
            let Some(download) = registry
                .downloads
                .iter_mut()
                .find(|download| download.id == id)
            else {
                return Ok(());
            };
            if matches!(
                download.status,
                DownloadStatus::Finished | DownloadStatus::Paused | DownloadStatus::Stopped
            ) {
                return Ok(());
            }
            if !queue_running && !download.manual_start_requested {
                return Ok(());
            }
            ensure_segment_plan(download, &settings);
            download.status = DownloadStatus::Downloading;
            download.manual_start_requested = false;
            download.error_message = None;
            download.diagnostics.failure_kind = None;
            download.diagnostics.restart_required = false;
            append_download_log(
                download,
                DownloadLogLevel::Info,
                "runtime.started",
                "Started the transfer runtime.",
            );
            let cloned = download.clone();
            self.persist_registry(&registry)?;
            (
                cloned,
                settings,
                queue_running,
                min_emit_interval_ms,
                max_checkpoint_interval_ms,
                host_profile,
            )
        };
        self.emit_download_upsert(&runtime_download);

        let http_pool = http_pool::HttpPool::new();
        let mut bootstrap_error: Option<String> = None;
        let mut bootstrap_initial_stream: Option<InitialResponseStream> = None;
        let mut bootstrap_probe: Option<DownloadProbeData> = None;
        if needs_runtime_metadata_bootstrap(&runtime_download) {
            match bootstrap_runtime_metadata(
                self,
                &mut runtime_download,
                &settings,
                &http_pool,
                min_emit_interval_ms,
            )
            .await
            {
                Ok(outcome) => {
                    if let Some(profile) = outcome.host_profile {
                        runtime_host_profile = Some(profile);
                    }
                    bootstrap_initial_stream = outcome.initial_stream;
                    bootstrap_probe = outcome.probe;
                }
                Err(error) => bootstrap_error = Some(error),
            }
        }

        if runtime_download.downloaded > 0 {
            let validation_probe = if let Some(probe) = bootstrap_probe.clone() {
                Some(probe)
            } else {
                let probe_target = runtime_probe_target(&runtime_download);
                match http_pool.get_client(probe_target.url) {
                    Some(client_lease) => probe_runtime_bootstrap_with_client(
                        client_lease.client.as_ref(),
                        probe_target.url,
                        probe_target.request_referer,
                        probe_target.request_cookies,
                        &probe_target.request_method,
                        probe_target.request_form_fields,
                        false,
                    )
                    .await
                    .ok()
                    .map(|bootstrap| bootstrap.probe),
                    _ => None,
                }
            };

            if let Some(probe) = validation_probe
                && !resume_validators_compatible(&runtime_download.validators, &probe.validators)
            {
                let mut registry = self.registry_guard()?;
                if let Some(download) = registry
                    .downloads
                    .iter_mut()
                    .find(|download| download.id == runtime_download.id)
                {
                    reset_download_progress(download);
                    clear_runtime_checkpoint(download);
                    ensure_segment_plan(download, &settings);
                    download.error_message = None;
                    download.diagnostics.failure_kind = None;
                    download.diagnostics.terminal_reason =
                        Some("Resume validators changed; restarting from zero.".to_string());
                    runtime_download = download.clone();
                }
                self.persist_registry(&registry)?;
                drop(registry);
                reset_temp_file_path(&runtime_download.temp_path)?;
            }
        }

        if requires_unknown_size_restart(&runtime_download) {
            let temp_path = runtime_download.temp_path.clone();
            let mut registry = self.registry_guard()?;
            if let Some(download) = registry
                .downloads
                .iter_mut()
                .find(|download| download.id == runtime_download.id)
            {
                reset_download_progress(download);
                clear_runtime_checkpoint(download);
                download.size = -1;
                download.downloaded = 0;
                download.speed = 0;
                download.time_left = None;
                download.capabilities.segmented = false;
                download.target_connections = 1;
                push_unique_diagnostic(
                    &mut download.diagnostics.warnings,
                    "Unknown-size partial data was restarted from zero because VDM could not verify a safe resume boundary without a trusted content length.",
                );
                download.diagnostics.terminal_reason = Some(
                    "Unknown-size partial state was reset before restarting the transfer."
                        .to_string(),
                );
                runtime_download = download.clone();
            }
            self.persist_registry(&registry)?;
            drop(registry);
            reset_temp_file_path(&temp_path)?;
        }

        let exact_request_shape_allows_segmentation =
            compatibility_request_context_supports_segmented_transfer(&runtime_download);
        if !exact_request_shape_allows_segmentation {
            let has_segment_progress = runtime_download
                .segments
                .iter()
                .any(|segment| segment.downloaded > 0);
            if has_segment_progress {
                return Err(
                    "This download was checkpointed with segmented state, but its preserved request context now requires guarded single-stream restart. Use Restart to retry from byte 0."
                        .to_string(),
                );
            }
            runtime_download.segments.clear();
            runtime_download.capabilities.segmented = false;
            runtime_download.target_connections = 1;
        }

        if runtime_download.segments.is_empty() {
            if runtime_download.size <= 0 || !exact_request_shape_allows_segmentation {
                if let Some(error) = bootstrap_error.clone() {
                    let mut registry = self.registry_guard()?;
                    if let Some(download) = registry
                        .downloads
                        .iter_mut()
                        .find(|download| download.id == runtime_download.id)
                    {
                        push_unique_diagnostic(&mut download.diagnostics.warnings, error);
                        runtime_download = download.clone();
                    }
                    self.persist_registry(&registry)?;
                }
                run_unknown_size_stream_runtime(
                    self,
                    &mut runtime_download,
                    &http_pool,
                    runtime_host_profile.clone(),
                    bootstrap_initial_stream.take(),
                    min_emit_interval_ms,
                    max_checkpoint_interval_ms,
                )
                .await?;
                return Ok(());
            }
            runtime_download.segments = build_segment_plan(
                runtime_download.size as u64,
                if should_use_segmented_mode(&runtime_download) {
                    runtime_download.target_connections.max(1)
                } else {
                    1
                },
                &settings,
                runtime_host_profile
                    .as_ref()
                    .and_then(|profile| profile.average_throughput_bytes_per_second)
                    .or(runtime_download.host_average_throughput_bytes_per_second),
                runtime_host_profile
                    .as_ref()
                    .and_then(|profile| profile.average_ttfb_ms)
                    .or(runtime_download.host_average_ttfb_ms),
            );
        }

        if let Some(parent) = Path::new(&runtime_download.temp_path).parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Failed creating temp directory: {error}"))?;
        }
        ensure_known_size_reservation_capacity(&runtime_download)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&runtime_download.temp_path)
            .map_err(|error| format!("Failed opening temp file: {error}"))?;
        if runtime_download.size > 0 {
            reserve_known_size_temp_file(&file, runtime_download.size as u64)?;
        }
        let output_file = Arc::new(file);
        let mut temp_file_lock =
            acquire_temp_transfer_lock(Arc::clone(&output_file), &runtime_download.temp_path);
        if let Some(warning) = temp_file_lock.warning.clone() {
            record_runtime_warning(self, &mut runtime_download, "runtime.temp-lock", warning);
        }
        let _wake_lock_guard = RuntimeWakeLockGuard::acquire(self);
        let disk_pool = Arc::new(disk_pool::DiskPool::new(256));
        let checkpoint_clock = Arc::new(AtomicI64::new(unix_epoch_millis()));
        let scheduler = SegmentScheduler::new(
            settings.min_segment_size_bytes,
            settings.late_segment_ratio_percent,
            settings.target_chunk_time_seconds,
        );
        let runtime_samples: Arc<Mutex<BTreeMap<u32, SegmentRuntimeSample>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let segment_started_at: Arc<Mutex<BTreeMap<u32, i64>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let runtime_telemetry: Arc<Mutex<RuntimeTelemetrySnapshot>> =
            Arc::new(Mutex::new(RuntimeTelemetrySnapshot::default()));
        if let Ok(mut samples) = runtime_samples.lock() {
            for sample in &runtime_download.runtime_checkpoint.segment_samples {
                samples.insert(
                    sample.segment_id,
                    SegmentRuntimeSample {
                        segment_id: sample.segment_id,
                        remaining_bytes: sample.remaining_bytes,
                        eta_seconds: sample.eta_seconds,
                        throughput_bytes_per_second: sample.throughput_bytes_per_second,
                        active_for_ms: None,
                    },
                );
            }
        }
        let segment_controls: Arc<Mutex<BTreeMap<u32, SegmentRuntimeControl>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let effective_download_limit = effective_download_speed_limit(&runtime_download, &settings);
        let per_download_limiter =
            Some(self.download_rate_limiter(&runtime_download.id, effective_download_limit));
        let per_host_limiter = self.host_rate_limiter(
            &runtime_download.host,
            host_token_bucket_rate(runtime_host_profile.as_ref()),
        );
        let initial_stream_slot = Arc::new(Mutex::new(
            if should_use_bootstrap_stream(&runtime_download) {
                bootstrap_initial_stream.take()
            } else {
                None
            },
        ));
        let mut runtime_attempt = 0u32;
        let mut ramp_state = RuntimeRampState::new(
            unix_epoch_millis(),
            runtime_download.target_connections.max(1),
        );

        loop {
            let mut pending_segments: Vec<DownloadSegment> = runtime_download
                .segments
                .iter()
                .filter(|segment| !matches!(segment.status, DownloadSegmentStatus::Finished))
                .cloned()
                .collect();
            if pending_segments.is_empty() {
                break;
            }

            pending_segments.sort_by(|left, right| {
                left.downloaded
                    .cmp(&right.downloaded)
                    .then(left.start.cmp(&right.start))
                    .then(left.id.cmp(&right.id))
            });

            let connection_cap = runtime_download
                .target_connections
                .max(1)
                .min(runtime_download.max_connections.max(1));
            let mut desired_parallel = usize::try_from(connection_cap).unwrap_or(usize::MAX).max(1);
            let mut queue: VecDeque<DownloadSegment> = pending_segments.into_iter().collect();
            let mut join_set: JoinSet<(DownloadSegment, Result<(), String>)> = JoinSet::new();
            let mut completed_segments: Vec<DownloadSegment> = Vec::new();
            let pending_ids: BTreeMap<u32, ()> =
                queue.iter().map(|segment| (segment.id, ())).collect();
            let mut race_by_segment =
                restore_runtime_races_from_checkpoint(&runtime_download, &pending_ids);
            if runtime_attempt == 0 {
                let restored_pairs = race_by_segment.len() / 2;
                let checkpoint_pairs = runtime_download.runtime_checkpoint.active_races.len();
                if checkpoint_pairs > restored_pairs {
                    let dropped_pairs = checkpoint_pairs.saturating_sub(restored_pairs);
                    if let Ok(mut registry) = self.registry_guard() {
                        if let Some(download) = registry
                            .downloads
                            .iter_mut()
                            .find(|download| download.id == runtime_download.id)
                        {
                            persist_runtime_races(
                                &mut download.runtime_checkpoint,
                                &race_by_segment,
                            );
                            append_download_log(
                                download,
                                DownloadLogLevel::Warn,
                                "race.restore-pruned",
                                format!(
                                    "Dropped {} stale persisted race pairs during restart recovery.",
                                    dropped_pairs
                                ),
                            );
                            runtime_download.runtime_checkpoint =
                                download.runtime_checkpoint.clone();
                        }
                        let _ = self.persist_registry(&registry);
                    }
                } else if restored_pairs > 0
                    && let Ok(mut registry) = self.registry_guard()
                {
                    if let Some(download) = registry
                        .downloads
                        .iter_mut()
                        .find(|download| download.id == runtime_download.id)
                    {
                        append_download_log(
                            download,
                            DownloadLogLevel::Info,
                            "race.restore-active",
                            format!(
                                "Restored {} active race pair(s) from checkpoint recovery.",
                                restored_pairs
                            ),
                        );
                    }
                    let _ = self.persist_registry(&registry);
                }
            }
            let mut expected_canceled: BTreeMap<u32, ()> = BTreeMap::new();
            let mut runtime_error: Option<String> = None;

            let launch_next =
                |queue: &mut VecDeque<DownloadSegment>,
                 join_set: &mut JoinSet<(DownloadSegment, Result<(), String>)>| {
                    let Some(mut segment) = queue.pop_front() else {
                        return;
                    };
                    let control = SegmentRuntimeControl::new(segment.end);
                    if let Ok(mut controls) = segment_controls.lock() {
                        controls.insert(segment.id, control.clone());
                    }
                    let config = TransferWorkerConfig {
                        url: runtime_download.final_url.clone(),
                        request_referer: runtime_download.compatibility.request_referer.clone(),
                        request_cookies: runtime_download.compatibility.request_cookies.clone(),
                        request_method: runtime_download.compatibility.request_method.clone(),
                        request_form_fields: runtime_download
                            .compatibility
                            .request_form_fields
                            .clone(),
                        chunk_buffer_size: runtime_chunk_buffer_size_with_pressure(
                            &runtime_download.traffic_mode,
                            runtime_download.speed,
                            disk_pool.queue_utilization_percent(),
                        ),
                        request_timeout_secs: 30,
                        retry_budget: segment.retry_budget.max(1),
                        backoff_base_ms: 160,
                        backoff_max_ms: 2_500,
                        per_download_limiter: per_download_limiter.clone(),
                        per_host_limiter: per_host_limiter.clone(),
                    };
                    let engine = self.clone();
                    let download_id = runtime_download.id.clone();
                    let disk_pool = Arc::clone(&disk_pool);
                    let output_file = Arc::clone(&output_file);
                    let http_pool = http_pool.clone();
                    let checkpoint_clock = Arc::clone(&checkpoint_clock);
                    let runtime_samples = Arc::clone(&runtime_samples);
                    let segment_started_at = Arc::clone(&segment_started_at);
                    let runtime_telemetry = Arc::clone(&runtime_telemetry);
                    let initial_stream_slot = Arc::clone(&initial_stream_slot);
                    if let Ok(mut started_at) = segment_started_at.lock() {
                        started_at.insert(segment.id, unix_epoch_millis());
                    }
                    let initial_response = if segment.start <= 0 {
                        initial_stream_slot
                            .lock()
                            .ok()
                            .and_then(|mut slot| slot.take())
                    } else {
                        None
                    };
                    join_set.spawn(async move {
                        let result = run_segment_worker(
                            &config,
                            &http_pool,
                            &disk_pool,
                            &output_file,
                            &mut segment,
                            SegmentWorkerStart {
                                control,
                                initial_response,
                            },
                            |progress| {
                                take_disk_pool_error(disk_pool.as_ref())?;
                                let mut registry = engine.registry_guard()?;
                                let should_flush;
                                let response = {
                                    let Some(download) = registry
                                        .downloads
                                        .iter_mut()
                                        .find(|download| download.id == download_id)
                                    else {
                                        return Err(
                                            "Download was removed during transfer.".to_string()
                                        );
                                    };
                                    if matches!(
                                        download.status,
                                        DownloadStatus::Paused
                                            | DownloadStatus::Stopped
                                            | DownloadStatus::Error
                                    ) {
                                        return Err("Download is no longer active.".to_string());
                                    }
                                    let mut aggregate_speed = None;
                                    if let Some(reg_segment) = download
                                        .segments
                                        .iter_mut()
                                        .find(|segment| segment.id == progress.segment_id)
                                    {
                                        reg_segment.downloaded = progress.downloaded;
                                        reg_segment.status = progress.status.clone();
                                        reg_segment.retry_attempts = progress.retry_attempts;
                                        let current_offset = reg_segment
                                            .start
                                            .saturating_add(reg_segment.downloaded.max(0))
                                            .max(reg_segment.start)
                                            as u64;
                                        let end = reg_segment.end.max(reg_segment.start) as u64;
                                        let remaining_bytes =
                                            end.saturating_sub(current_offset).saturating_add(1);
                                        let segment_age_ms = segment_started_at
                                            .lock()
                                            .ok()
                                            .and_then(|started_at| {
                                                started_at.get(&reg_segment.id).copied()
                                            })
                                            .map(|started_at_ms| {
                                                u64::try_from(
                                                    unix_epoch_millis()
                                                        .saturating_sub(started_at_ms),
                                                )
                                                .unwrap_or(u64::MAX)
                                            });
                                        let mut runtime_sample = SegmentRuntimeSample {
                                            segment_id: reg_segment.id,
                                            remaining_bytes,
                                            eta_seconds: None,
                                            throughput_bytes_per_second: None,
                                            active_for_ms: segment_age_ms,
                                        };
                                        if let Ok(mut samples) = runtime_samples.lock() {
                                            let previous_throughput =
                                                samples.get(&reg_segment.id).and_then(|sample| {
                                                    sample.throughput_bytes_per_second
                                                });
                                            let stable_throughput = stabilized_segment_throughput(
                                                previous_throughput,
                                                progress.throughput_bytes_per_second,
                                                &progress.status,
                                            );
                                            runtime_sample = SegmentRuntimeSample {
                                                segment_id: reg_segment.id,
                                                remaining_bytes,
                                                eta_seconds: stable_throughput
                                                    .filter(|value| *value > 0)
                                                    .map(|value| remaining_bytes / value.max(1)),
                                                throughput_bytes_per_second: stable_throughput,
                                                active_for_ms: segment_age_ms,
                                            };
                                            samples.insert(reg_segment.id, runtime_sample.clone());
                                            aggregate_speed = Some(recompute_download_speed(
                                                download.speed,
                                                &samples,
                                            ));
                                        }
                                        upsert_runtime_segment_sample(
                                            &mut download.runtime_checkpoint,
                                            &runtime_sample,
                                        );
                                        upsert_runtime_segment_health(
                                            &mut download.runtime_checkpoint,
                                            reg_segment.id,
                                            progress.retry_attempts,
                                            progress.terminal_failure_reason.clone(),
                                        );
                                        if let Some(telemetry) = progress.telemetry.as_ref()
                                            && let Ok(mut snapshot) = runtime_telemetry.lock()
                                        {
                                            if let Some(ttfb_ms) =
                                                telemetry.ttfb_ms.filter(|value| *value > 0)
                                            {
                                                snapshot.ttfb_samples_ms.push(ttfb_ms);
                                            }
                                            if let Some(reused) = telemetry.connection_reused {
                                                if reused {
                                                    snapshot.reused_true =
                                                        snapshot.reused_true.saturating_add(1);
                                                } else {
                                                    snapshot.reused_false =
                                                        snapshot.reused_false.saturating_add(1);
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
                                    }
                                    download.downloaded = download
                                        .segments
                                        .iter()
                                        .map(|segment| segment.downloaded.max(0))
                                        .sum::<i64>()
                                        .max(0);
                                    if let Some(aggregate_speed) = aggregate_speed {
                                        download.speed = aggregate_speed;
                                    } else if progress.throughput_bytes_per_second > 0 {
                                        download.speed = progress.throughput_bytes_per_second;
                                    }
                                    download.time_left = estimate_time_left(
                                        download.size,
                                        download.downloaded,
                                        download.speed,
                                    );
                                    // Detect write saturation: if the disk pool queue is
                                    // >60% full, surface that as writer backpressure so the
                                    // UI and diagnostic logs can distinguish disk stalls
                                    // from network throttling.
                                    let under_pressure =
                                        disk_queue_under_pressure(disk_pool.as_ref());
                                    if under_pressure && !download.writer_backpressure {
                                        download.diagnostics.checkpoint_disk_pressure_events =
                                            download
                                                .diagnostics
                                                .checkpoint_disk_pressure_events
                                                .saturating_add(1);
                                    }
                                    download.writer_backpressure = under_pressure;
                                    let now = unix_epoch_millis();
                                    let elapsed = now
                                        .saturating_sub(checkpoint_clock.load(Ordering::Relaxed));
                                    should_flush = elapsed >= max_checkpoint_interval_ms
                                        || matches!(
                                            progress.status,
                                            DownloadSegmentStatus::Finished
                                        );
                                    if should_flush {
                                        download.diagnostics.checkpoint_flushes = download
                                            .diagnostics
                                            .checkpoint_flushes
                                            .saturating_add(1);
                                        download.diagnostics.checkpoint_last_flush_ms = 0;
                                        download.diagnostics.checkpoint_avg_flush_ms = 0;
                                        checkpoint_clock.store(now, Ordering::Relaxed);
                                    } else {
                                        download.diagnostics.checkpoint_skips =
                                            download.diagnostics.checkpoint_skips.saturating_add(1);
                                    }
                                    download.clone()
                                };
                                if should_flush {
                                    engine.persist_registry(&registry)?;
                                }
                                drop(registry);
                                engine.emit_download_progress_diff_if_due(
                                    &response,
                                    min_emit_interval_ms,
                                );
                                Ok(())
                            },
                        )
                        .await;
                        (segment, result)
                    });
                };

            while join_set.len() < desired_parallel && !queue.is_empty() {
                launch_next(&mut queue, &mut join_set);
            }

            while !join_set.is_empty() {
                let median_ttfb_ms = runtime_telemetry
                    .lock()
                    .ok()
                    .and_then(|snapshot| median_runtime_ttfb(&snapshot));
                let runtime_sample_snapshot = runtime_samples
                    .lock()
                    .ok()
                    .map(|value| value.values().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                let adjustment = adjust_runtime_segment_supply(
                    self,
                    &runtime_download.id,
                    RuntimeSupplyInputs {
                        settings: &settings,
                        scheduler: &scheduler,
                        runtime_samples: &runtime_sample_snapshot,
                        disk_pool: disk_pool.as_ref(),
                        ramp_state: &mut ramp_state,
                        median_ttfb_ms,
                    },
                )?;
                desired_parallel = adjustment.desired_parallel;
                if !adjustment.control_updates.is_empty()
                    && let Ok(controls) = segment_controls.lock()
                {
                    for (segment_id, end) in adjustment.control_updates {
                        if let Some(control) = controls.get(&segment_id) {
                            control.set_end(end);
                        }
                    }
                }
                for next_segment in adjustment.appended_segments {
                    queue.push_back(next_segment);
                }
                if let Some(response) = adjustment.response {
                    runtime_download.segments = response.segments.clone();
                    runtime_download.downloaded = response.downloaded;
                    runtime_download.runtime_checkpoint = response.runtime_checkpoint.clone();
                    runtime_download.target_connections = response.target_connections.max(1);
                    runtime_download.max_connections = response.max_connections.max(1);
                    runtime_download.host_max_connections = response.host_max_connections;
                    runtime_download.host_cooldown_until = response.host_cooldown_until;
                    self.emit_download_upsert(&response);
                    self.emit_download_progress_diff_if_due(&response, min_emit_interval_ms);
                }
                self.apply_runtime_dispatch_plan(adjustment.dispatch_plan, min_emit_interval_ms);
                while join_set.len() < desired_parallel && !queue.is_empty() {
                    launch_next(&mut queue, &mut join_set);
                }

                let joined = match tokio::time::timeout(
                    Duration::from_millis(RUNTIME_ADAPTIVE_POLL_MS),
                    join_set.join_next(),
                )
                .await
                {
                    Ok(joined) => joined,
                    Err(_) => continue,
                };
                let Some(joined) = joined else {
                    break;
                };

                match joined {
                    Ok((segment, result)) => {
                        let segment_id = segment.id;
                        let terminal_failure_reason = result.as_ref().err().cloned();
                        if let Ok(mut controls) = segment_controls.lock() {
                            controls.remove(&segment_id);
                        }
                        if let Ok(mut started_at) = segment_started_at.lock() {
                            started_at.remove(&segment_id);
                        }
                        if (segment.retry_attempts > 0 || terminal_failure_reason.is_some())
                            && let Ok(mut registry) = self.registry_guard()
                        {
                            if let Some(download) = registry
                                .downloads
                                .iter_mut()
                                .find(|download| download.id == runtime_download.id)
                            {
                                upsert_runtime_segment_health(
                                    &mut download.runtime_checkpoint,
                                    segment_id,
                                    segment.retry_attempts,
                                    terminal_failure_reason,
                                );
                            }
                            let _ = self.persist_registry(&registry);
                        }
                        completed_segments.push(segment);
                        if let Err(error) = result {
                            if error == "segment-canceled"
                                && expected_canceled.remove(&segment_id).is_some()
                            {
                                while join_set.len() < desired_parallel && !queue.is_empty() {
                                    launch_next(&mut queue, &mut join_set);
                                }
                            } else {
                                runtime_error = Some(error);
                                join_set.abort_all();
                            }
                        } else {
                            let winner_id = completed_segments.last().map_or(0, |value| value.id);
                            if let Ok(mut registry) = self.registry_guard() {
                                if let Some(download) = registry
                                    .downloads
                                    .iter_mut()
                                    .find(|download| download.id == runtime_download.id)
                                    && let Some(race_winner) = resolve_runtime_race_winner(
                                        download,
                                        winner_id,
                                        &mut race_by_segment,
                                    )
                                {
                                    expected_canceled.insert(race_winner.loser_id, ());
                                    if let Ok(controls) = segment_controls.lock()
                                        && let Some(control) = controls.get(&race_winner.loser_id)
                                    {
                                        control.cancel();
                                    }
                                }
                                let _ = self.persist_registry(&registry);
                            }
                            while join_set.len() < desired_parallel && !queue.is_empty() {
                                launch_next(&mut queue, &mut join_set);
                            }
                            if join_set.len() < desired_parallel && queue.is_empty() {
                                let mut appended: Option<DownloadSegment> = None;
                                let mut control_updates: Vec<(u32, i64)> = Vec::new();
                                if let Ok(mut registry) = self.registry_guard()
                                    && let Some(download) = registry
                                        .downloads
                                        .iter_mut()
                                        .find(|download| download.id == runtime_download.id)
                                {
                                    let samples = runtime_samples
                                        .lock()
                                        .ok()
                                        .map(|value| value.values().cloned().collect::<Vec<_>>())
                                        .unwrap_or_default();
                                    let expansion = attempt_runtime_queue_expansion(
                                        download,
                                        &scheduler,
                                        &samples,
                                        &mut race_by_segment,
                                    );
                                    appended = expansion.appended_segment;
                                    control_updates = expansion.control_updates;
                                    if appended.is_some() {
                                        let _ = self.persist_registry(&registry);
                                    }
                                }
                                if !control_updates.is_empty()
                                    && let Ok(controls) = segment_controls.lock()
                                {
                                    for (segment_id, end) in control_updates {
                                        if let Some(control) = controls.get(&segment_id) {
                                            control.set_end(end);
                                        }
                                    }
                                }
                                if let Some(next_segment) = appended {
                                    queue.push_back(next_segment);
                                    while join_set.len() < desired_parallel && !queue.is_empty() {
                                        launch_next(&mut queue, &mut join_set);
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        if error.is_cancelled() {
                            if runtime_error.is_some() {
                                continue;
                            }
                            continue;
                        }
                        runtime_error = Some(format!("Runtime worker failed: {error}"));
                        join_set.abort_all();
                    }
                }
            }

            for updated in completed_segments {
                if let Some(segment) = runtime_download
                    .segments
                    .iter_mut()
                    .find(|segment| segment.id == updated.id)
                {
                    *segment = updated;
                }
            }
            if let Ok(registry) = self.registry_guard()
                && let Some(download) = registry
                    .downloads
                    .iter()
                    .find(|download| download.id == runtime_download.id)
            {
                runtime_download.segments = download.segments.clone();
                runtime_download.downloaded = download.downloaded;
                runtime_download.runtime_checkpoint = download.runtime_checkpoint.clone();
                runtime_download.target_connections = download.target_connections.max(1);
                runtime_download.max_connections = download.max_connections.max(1);
                runtime_download.host_max_connections = download.host_max_connections;
                runtime_download.host_cooldown_until = download.host_cooldown_until;
            }

            if runtime_error.is_none() {
                continue;
            }
            let Some(error) = runtime_error else {
                continue;
            };
            if runtime_control_flow_error(&error) {
                return Ok(());
            }

            runtime_attempt = runtime_attempt.saturating_add(1);
            let validation_error = runtime_validation_error(&error);
            let (throttle_event, timeout_event, reset_event) = classify_runtime_error(&error);
            let (should_retry, retry_delay_ms, retry_response) = {
                let mut registry = self.registry_guard()?;
                let mut telemetry_payload: Option<HostTelemetryArgs> = None;
                let mut scope_range_validation: Option<(String, String, Option<u64>)> = None;
                let mut retry_allowed = false;
                let runtime_telemetry_snapshot = runtime_telemetry
                    .lock()
                    .ok()
                    .map(|value| value.clone())
                    .unwrap_or_default();
                if let Some(download) = registry
                    .downloads
                    .iter_mut()
                    .find(|download| download.id == runtime_download.id)
                {
                    telemetry_payload = Some(HostTelemetryArgs {
                        host: download.host.clone(),
                        scope_key: Some(download_scope_key(download)),
                        attempted_connections: Some(download.target_connections.max(1)),
                        sustained_gain_bytes_per_second: None,
                        throughput_bytes_per_second: Some(download.speed)
                            .filter(|value| *value > 0),
                        ttfb_ms: median_runtime_ttfb(&runtime_telemetry_snapshot),
                        negotiated_protocol: runtime_protocol_hint(&runtime_telemetry_snapshot)
                            .or_else(|| download.host_protocol.clone()),
                        connection_reused: runtime_connection_reuse_hint(
                            &runtime_telemetry_snapshot,
                        ),
                        throttle_event,
                        timeout_event,
                        reset_event,
                        range_validation_failed: false,
                    });
                    let scope_key = probe_scope_key(
                        &download.url,
                        &download.compatibility.request_method,
                        &download.compatibility.request_form_fields,
                    );
                    let range_validation_content_length =
                        u64::try_from(download.size).ok().filter(|value| *value > 0);
                    let recovery = reconcile_runtime_error(
                        download,
                        &error,
                        runtime_attempt,
                        validation_error,
                    );
                    retry_allowed = recovery.retry_allowed;
                    if recovery.range_validation_failed {
                        scope_range_validation = Some((
                            download.host.clone(),
                            scope_key,
                            range_validation_content_length,
                        ));
                    }
                }
                if let Some((host, scope_key, content_length_hint)) =
                    scope_range_validation.as_ref()
                {
                    let profile = registry.host_profiles.entry(host.clone()).or_default();
                    apply_scope_range_validation_failure(
                        profile,
                        scope_key,
                        *content_length_hint,
                        unix_epoch_millis(),
                    );
                }
                if let Some(payload) = telemetry_payload.as_ref() {
                    apply_host_feedback_to_registry(&mut registry, &payload.host, payload);
                }
                let dispatch_plan = plan_runtime_dispatch(&mut registry);
                let updated_download = registry
                    .downloads
                    .iter()
                    .find(|download| download.id == runtime_download.id)
                    .cloned();
                let retry_delay_ms = updated_download
                    .as_ref()
                    .filter(|_| retry_allowed)
                    .and_then(|download| cooldown_retry_delay_ms(download.host_cooldown_until));
                self.persist_registry(&registry)?;
                if let Some(updated_download) = updated_download.as_ref() {
                    self.emit_download_upsert(updated_download);
                    self.emit_download_progress_diff_if_due(updated_download, min_emit_interval_ms);
                }
                self.apply_runtime_dispatch_plan(dispatch_plan, min_emit_interval_ms);
                (retry_allowed, retry_delay_ms, updated_download)
            };
            if let Some(response) = retry_response {
                runtime_download = response;
            }
            if should_retry {
                let backoff_ms = 250_u64.saturating_mul(2_u64.saturating_pow(runtime_attempt));
                let retry_delay_ms = retry_delay_ms.unwrap_or_default();
                let sleep_ms = backoff_ms.min(3_000).max(retry_delay_ms);
                tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                continue;
            }
            return Ok(());
        }

        wait_for_disk_pool_drain(disk_pool.as_ref()).await?;
        output_file
            .sync_data()
            .map_err(|error| format!("Failed syncing temp file before finalize: {error}"))?;
        take_disk_pool_error(disk_pool.as_ref())?;
        let mut temp_lock_release_warning = temp_file_lock.release();
        drop(output_file);
        drop(disk_pool);

        let finalize_result =
            finalize_download_file(&runtime_download.temp_path, &runtime_download.target_path)?;
        let finalize_used_copy_fallback = finalize_result.used_copy_fallback;
        let mut finalize_warnings = finalize_result.warnings;
        if let Some(warning) = temp_lock_release_warning.take() {
            finalize_warnings.push(warning);
        }
        let mut registry = self.registry_guard()?;
        if let Some(download_index) = registry
            .downloads
            .iter()
            .position(|download| download.id == runtime_download.id)
        {
            let throughput_snapshot = runtime_samples
                .lock()
                .ok()
                .and_then(|samples| aggregate_runtime_throughput(&samples));
            let runtime_telemetry_snapshot = runtime_telemetry
                .lock()
                .ok()
                .map(|value| value.clone())
                .unwrap_or_default();
            let telemetry_payload = {
                let download = &mut registry.downloads[download_index];
                download.status = DownloadStatus::Finished;
                download.downloaded = download
                    .segments
                    .iter()
                    .map(|segment| segment.downloaded.max(0))
                    .sum::<i64>()
                    .max(0);
                if download.size > 0 {
                    download.downloaded = download.size;
                }
                download.speed = 0;
                download.time_left = Some(0);
                download.error_message = None;
                download.diagnostics.failure_kind = None;
                download.diagnostics.restart_required = false;
                download.diagnostics.terminal_reason = Some("Transfer completed.".to_string());
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
                    "Completed the segmented transfer and finalized the file.",
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
                    attempted_connections: Some(download.target_connections.max(1)),
                    sustained_gain_bytes_per_second: None,
                    throughput_bytes_per_second: throughput_snapshot,
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
            self.persist_registry(&registry)?;
            drop(registry);
            self.emit_download_progress_diff_if_due(&response, min_emit_interval_ms);
            self.emit_download_upsert(&response);
            self.trigger_download_completion_actions(&response);
            self.apply_runtime_dispatch_plan(dispatch_plan, min_emit_interval_ms);
            self.spawn_checksum_verification(response.id.clone());
        }
        Ok(())
    }
}
