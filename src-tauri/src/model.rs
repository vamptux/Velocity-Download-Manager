use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Finished,
    Downloading,
    Paused,
    Queued,
    Error,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum DownloadCategory {
    Compressed,
    Programs,
    Videos,
    Music,
    Pictures,
    Documents,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResumeValidators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_length: Option<u64>,
    pub content_type: Option<String>,
    pub content_disposition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ChecksumAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChecksumSpec {
    pub algorithm: ChecksumAlgorithm,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
pub enum IntegrityState {
    #[default]
    None,
    Pending,
    Verifying,
    Verified,
    Mismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadIntegrity {
    #[serde(default)]
    pub expected: Option<ChecksumSpec>,
    pub actual: Option<String>,
    #[serde(default)]
    pub state: IntegrityState,
    pub message: Option<String>,
    pub checked_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadCapabilities {
    pub resumable: bool,
    pub range_supported: bool,
    pub segmented: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
pub enum TrafficMode {
    Low,
    Medium,
    High,
    #[default]
    Max,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
pub enum DownloadRequestMethod {
    #[default]
    Get,
    Post,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum AppUpdateChannel {
    #[default]
    Stable,
    Preview,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequestField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct EngineSettings {
    #[serde(default = "default_active_downloads")]
    pub max_active_downloads: u32,
    #[serde(default = "default_target_chunk_time_seconds")]
    pub target_chunk_time_seconds: u32,
    #[serde(default = "default_min_segment_size_bytes")]
    pub min_segment_size_bytes: u64,
    #[serde(default = "default_late_segment_ratio_percent")]
    pub late_segment_ratio_percent: u32,
    #[serde(default = "default_segment_checkpoint_min_interval_ms")]
    pub segment_checkpoint_min_interval_ms: u32,
    #[serde(default = "default_segment_checkpoint_max_interval_ms")]
    pub segment_checkpoint_max_interval_ms: u32,
    #[serde(default)]
    pub experimental_uncapped_mode: bool,
    #[serde(default)]
    pub traffic_mode: TrafficMode,
    #[serde(default)]
    pub speed_limit_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub update_channel: AppUpdateChannel,
    #[serde(default)]
    pub skipped_update_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct QueueState {
    #[serde(default = "default_queue_running")]
    pub running: bool,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            max_active_downloads: default_active_downloads(),
            target_chunk_time_seconds: default_target_chunk_time_seconds(),
            min_segment_size_bytes: default_min_segment_size_bytes(),
            late_segment_ratio_percent: default_late_segment_ratio_percent(),
            segment_checkpoint_min_interval_ms: default_segment_checkpoint_min_interval_ms(),
            segment_checkpoint_max_interval_ms: default_segment_checkpoint_max_interval_ms(),
            experimental_uncapped_mode: false,
            traffic_mode: TrafficMode::default(),
            speed_limit_bytes_per_second: None,
            update_channel: AppUpdateChannel::default(),
            skipped_update_version: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostProfile {
    pub max_connections: Option<u32>,
    #[serde(default)]
    pub range_supported: Option<bool>,
    #[serde(default)]
    pub hard_no_range: bool,
    #[serde(default)]
    pub resumable: Option<bool>,
    #[serde(default)]
    pub content_length_hint: Option<u64>,
    #[serde(default)]
    pub probe_failure_streak: u32,
    #[serde(default)]
    pub last_probe_error_at: Option<i64>,
    #[serde(default)]
    pub successful_downloads: u32,
    #[serde(default)]
    pub throttle_events: u32,
    #[serde(default)]
    pub timeout_events: u32,
    #[serde(default)]
    pub reset_events: u32,
    #[serde(default)]
    pub average_ttfb_ms: Option<u64>,
    #[serde(default)]
    pub average_throughput_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub telemetry_samples: u32,
    #[serde(default)]
    pub last_telemetry_at: Option<i64>,
    pub cooldown_until: Option<i64>,
    #[serde(default)]
    pub ramp_attempts_without_gain: u32,
    #[serde(default)]
    pub concurrency_locked: bool,
    #[serde(default)]
    pub locked_connections: Option<u32>,
    #[serde(default)]
    pub lock_reason: Option<String>,
    #[serde(default)]
    pub last_probe_at: Option<i64>,
    #[serde(default)]
    pub negotiated_protocol: Option<String>,
    #[serde(default)]
    pub protocol_samples: u32,
    #[serde(default)]
    pub protocol_downgrade_events: u32,
    #[serde(default)]
    pub protocol_reuse_events: u32,
    #[serde(default)]
    pub protocol_new_connection_events: u32,
    #[serde(default)]
    pub stable_recovery_samples: u32,
    #[serde(default)]
    pub probe_scopes: BTreeMap<String, ProbeScopeCache>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HostDiagnosticsSummary {
    #[serde(default)]
    pub hard_no_range: bool,
    #[serde(default)]
    pub concurrency_locked: bool,
    #[serde(default)]
    pub lock_reason: Option<String>,
    #[serde(default)]
    pub cooldown_until: Option<i64>,
    #[serde(default)]
    pub negotiated_protocol: Option<String>,
    #[serde(default)]
    pub reuse_connections: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeScopeCache {
    #[serde(default)]
    pub range_supported: Option<bool>,
    #[serde(default)]
    pub hard_no_range: bool,
    #[serde(default)]
    pub resumable: Option<bool>,
    #[serde(default)]
    pub content_length_hint: Option<u64>,
    #[serde(default)]
    pub last_probe_at: Option<i64>,
    #[serde(default)]
    pub probe_failure_streak: u32,
    #[serde(default)]
    pub last_probe_error_at: Option<i64>,
    #[serde(default)]
    pub average_ttfb_ms: Option<u64>,
    #[serde(default)]
    pub average_throughput_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub telemetry_samples: u32,
    #[serde(default)]
    pub last_telemetry_at: Option<i64>,
    #[serde(default)]
    pub throttle_events: u32,
    #[serde(default)]
    pub timeout_events: u32,
    #[serde(default)]
    pub reset_events: u32,
    #[serde(default)]
    pub cooldown_until: Option<i64>,
    #[serde(default)]
    pub ramp_attempts_without_gain: u32,
    #[serde(default)]
    pub concurrency_locked: bool,
    #[serde(default)]
    pub locked_connections: Option<u32>,
    #[serde(default)]
    pub lock_reason: Option<String>,
    #[serde(default)]
    pub last_instability_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecentProbeCacheEntry {
    pub captured_at: i64,
    pub host: String,
    pub final_url: String,
    pub size: Option<u64>,
    pub mime_type: Option<String>,
    pub negotiated_protocol: Option<String>,
    pub range_supported: bool,
    pub resumable: bool,
    pub validators: ResumeValidators,
    pub suggested_name: String,
    #[serde(default)]
    pub compatibility: DownloadCompatibility,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadCompatibility {
    #[serde(default)]
    pub redirect_chain: Vec<String>,
    #[serde(default)]
    pub filename_source: Option<String>,
    #[serde(default)]
    pub classification: Option<String>,
    #[serde(default)]
    pub wrapper_detected: bool,
    #[serde(default)]
    pub direct_url_recovered: bool,
    #[serde(default)]
    pub browser_interstitial_only: bool,
    #[serde(default)]
    pub request_referer: Option<String>,
    #[serde(default)]
    pub request_cookies: Option<String>,
    #[serde(default)]
    pub request_method: DownloadRequestMethod,
    #[serde(default)]
    pub request_form_fields: Vec<DownloadRequestField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
pub enum DownloadSegmentStatus {
    #[default]
    Pending,
    Downloading,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSegment {
    pub id: u32,
    pub start: i64,
    pub end: i64,
    pub downloaded: i64,
    #[serde(default)]
    pub retry_attempts: u32,
    #[serde(default)]
    pub retry_budget: u32,
    pub status: DownloadSegmentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRuntimeSegmentSample {
    pub segment_id: u32,

    pub remaining_bytes: u64,
    #[serde(default)]
    pub eta_seconds: Option<u64>,
    #[serde(default)]
    pub throughput_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub retry_attempts: u32,
    #[serde(default)]
    pub terminal_failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRuntimeRaceState {
    pub slow_segment_id: u32,
    pub companion_segment_id: u32,
    pub slow_baseline_downloaded: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRuntimeCheckpoint {
    #[serde(default)]
    pub segment_samples: Vec<DownloadRuntimeSegmentSample>,
    #[serde(default)]
    pub active_races: Vec<DownloadRuntimeRaceState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum DownloadFailureKind {
    Http,
    Network,
    Validation,
    FileSystem,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum DownloadLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadLogEntry {
    pub timestamp: i64,
    pub level: DownloadLogLevel,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDiagnostics {
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
    pub failure_kind: Option<DownloadFailureKind>,
    pub restart_required: bool,
    #[serde(default)]
    pub terminal_reason: Option<String>,
    #[serde(default)]
    pub checkpoint_flushes: u32,
    #[serde(default)]
    pub checkpoint_skips: u32,
    #[serde(default)]
    pub checkpoint_avg_flush_ms: u64,
    #[serde(default)]
    pub checkpoint_last_flush_ms: u64,
    #[serde(default)]
    pub checkpoint_disk_pressure_events: u32,
    #[serde(default)]
    pub contiguous_fsync_flushes: u32,
    #[serde(default)]
    pub contiguous_fsync_window_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRecord {
    pub id: String,
    pub name: String,
    pub url: String,
    pub final_url: String,
    pub host: String,
    pub size: i64,
    pub downloaded: i64,
    pub status: DownloadStatus,
    pub manual_start_requested: bool,
    pub category: DownloadCategory,
    pub speed: u64,
    pub time_left: Option<u64>,
    pub date_added: i64,
    pub save_path: String,
    pub target_path: String,
    pub temp_path: String,
    pub queue: String,
    #[serde(default)]
    pub scheduled_for: Option<i64>,
    #[serde(default)]
    pub queue_position: u32,
    #[serde(default = "default_max_connections_fallback")]
    pub max_connections: u32,
    pub host_max_connections: Option<u32>,
    pub host_cooldown_until: Option<i64>,
    #[serde(default)]
    pub host_average_ttfb_ms: Option<u64>,
    #[serde(default)]
    pub host_average_throughput_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub host_protocol: Option<String>,
    #[serde(default)]
    pub host_diagnostics: HostDiagnosticsSummary,
    #[serde(default)]
    pub traffic_mode: TrafficMode,
    #[serde(default)]
    pub speed_limit_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub open_folder_on_completion: bool,
    pub error_message: Option<String>,
    pub content_type: Option<String>,
    #[serde(default)]
    pub capabilities: DownloadCapabilities,
    #[serde(default)]
    pub validators: ResumeValidators,
    #[serde(default)]
    pub compatibility: DownloadCompatibility,
    #[serde(default)]
    pub integrity: DownloadIntegrity,
    #[serde(default)]
    pub diagnostics: DownloadDiagnostics,
    #[serde(default)]
    pub segments: Vec<DownloadSegment>,
    #[serde(default)]
    pub target_connections: u32,
    #[serde(default)]
    pub writer_backpressure: bool,
    #[serde(default)]
    pub engine_log: Vec<DownloadLogEntry>,
    #[serde(default)]
    pub runtime_checkpoint: DownloadRuntimeCheckpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub original_url: String,
    pub final_url: String,
    pub host: String,
    pub host_max_connections: Option<u32>,
    #[serde(default)]
    pub host_average_ttfb_ms: Option<u64>,
    #[serde(default)]
    pub host_average_throughput_bytes_per_second: Option<u64>,
    #[serde(default)]
    pub host_diagnostics: HostDiagnosticsSummary,
    pub suggested_name: String,
    pub target_path: Option<String>,
    pub size: Option<u64>,
    pub mime_type: Option<String>,
    pub available_space: Option<u64>,
    pub resumable: bool,
    pub range_supported: bool,
    #[serde(default)]
    pub segmented: bool,
    #[serde(default)]
    pub planned_connections: u32,
    pub suggested_category: DownloadCategory,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub validators: ResumeValidators,
    #[serde(default)]
    pub compatibility: DownloadCompatibility,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ProbeDownloadArgs {
    pub url: String,
    #[serde(default)]
    pub save_path: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub request_referer: Option<String>,
    #[serde(default)]
    pub request_cookies: Option<String>,
    #[serde(default)]
    pub request_method: DownloadRequestMethod,
    #[serde(default)]
    pub request_form_fields: Vec<DownloadRequestField>,
}

#[derive(Debug, Clone, Copy, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum ReorderDirection {
    Up,
    Down,
    Top,
    Bottom,
}

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AddDownloadArgs {
    pub url: String,
    pub name: Option<String>,
    pub category: DownloadCategory,
    pub save_path: String,
    #[serde(default)]
    pub request_referer: Option<String>,
    #[serde(default)]
    pub request_cookies: Option<String>,
    #[serde(default)]
    pub request_method: DownloadRequestMethod,
    #[serde(default)]
    pub request_form_fields: Vec<DownloadRequestField>,
    #[serde(default)]
    pub checksum: Option<ChecksumSpec>,
    #[serde(default)]
    pub size_hint_bytes: Option<u64>,
    #[serde(default)]
    pub range_supported_hint: Option<bool>,
    #[serde(default)]
    pub resumable_hint: Option<bool>,
    #[serde(default)]
    pub scheduled_for: Option<i64>,
    #[serde(default = "default_start_immediately")]
    pub start_immediately: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInfo {
    pub version: String,
    pub current_version: String,
    #[serde(default)]
    pub channel: AppUpdateChannel,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum AppUpdateCheckStatus {
    Available,
    UpToDate,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateCheckResult {
    pub status: AppUpdateCheckStatus,
    #[serde(default)]
    pub info: Option<AppUpdateInfo>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum AppUpdateStartupHealthStatus {
    Pending,
    Healthy,
    RestoredSettings,
    RollbackTriggered,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateStartupHealth {
    pub status: AppUpdateStartupHealthStatus,
    pub channel: AppUpdateChannel,
    pub from_version: String,
    pub target_version: String,
    pub observed_version: String,
    pub checked_at: i64,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum AppUpdateProgressEvent {
    #[serde(rename_all = "camelCase")]
    Started {
        content_length: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        chunk_length: u64,
    },
    Finished,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostTelemetryArgs {
    pub host: String,
    #[serde(default)]
    pub scope_key: Option<String>,
    pub attempted_connections: Option<u32>,
    pub sustained_gain_bytes_per_second: Option<i64>,
    pub throughput_bytes_per_second: Option<u64>,
    pub ttfb_ms: Option<u64>,
    #[serde(default)]
    pub negotiated_protocol: Option<String>,
    #[serde(default)]
    pub connection_reused: Option<bool>,
    #[serde(default)]
    pub throttle_event: bool,
    #[serde(default)]
    pub timeout_event: bool,
    #[serde(default)]
    pub reset_event: bool,
    #[serde(default)]
    pub range_validation_failed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySnapshot {
    pub next_id: u64,
    #[serde(default)]
    pub settings: EngineSettings,
    #[serde(default = "default_queue_running")]
    pub queue_running: bool,
    #[serde(default)]
    pub host_profiles: BTreeMap<String, HostProfile>,
    #[serde(default, skip_serializing, skip_deserializing)]
    pub recent_probes: BTreeMap<String, RecentProbeCacheEntry>,
    pub downloads: Vec<DownloadRecord>,
}

impl Default for RegistrySnapshot {
    fn default() -> Self {
        Self {
            next_id: 1,
            settings: EngineSettings::default(),
            queue_running: default_queue_running(),
            host_profiles: BTreeMap::new(),
            recent_probes: BTreeMap::new(),
            downloads: Vec::new(),
        }
    }
}

fn default_max_connections_fallback() -> u32 {
    16
}

fn default_active_downloads() -> u32 {
    3
}

fn default_queue_running() -> bool {
    true
}

fn default_start_immediately() -> bool {
    true
}

fn default_target_chunk_time_seconds() -> u32 {
    2
}

fn default_min_segment_size_bytes() -> u64 {
    512 * 1024
}

fn default_late_segment_ratio_percent() -> u32 {
    20
}

fn default_segment_checkpoint_min_interval_ms() -> u32 {
    900
}

fn default_segment_checkpoint_max_interval_ms() -> u32 {
    3_500
}
