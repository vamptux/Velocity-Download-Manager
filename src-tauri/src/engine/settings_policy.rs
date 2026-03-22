use crate::model::EngineSettings;

const MAX_ACTIVE_DOWNLOADS_LIMIT: u32 = 10;
const RUNTIME_CHECKPOINT_MIN_INTERVAL_MS: i64 = 400;
const RUNTIME_CHECKPOINT_MAX_INTERVAL_MS: i64 = 12_000;

pub(super) fn sanitize_engine_settings(mut settings: EngineSettings) -> EngineSettings {
    settings.max_active_downloads = settings
        .max_active_downloads
        .clamp(1, MAX_ACTIVE_DOWNLOADS_LIMIT);
    settings.target_chunk_time_seconds = settings.target_chunk_time_seconds.clamp(1, 10);
    settings.min_segment_size_bytes = settings
        .min_segment_size_bytes
        .clamp(64 * 1024, 64 * 1024 * 1024);
    settings.late_segment_ratio_percent = settings.late_segment_ratio_percent.clamp(5, 40);
    settings.segment_checkpoint_min_interval_ms =
        i64::from(settings.segment_checkpoint_min_interval_ms).clamp(
            RUNTIME_CHECKPOINT_MIN_INTERVAL_MS,
            RUNTIME_CHECKPOINT_MAX_INTERVAL_MS,
        ) as u32;
    settings.segment_checkpoint_max_interval_ms =
        i64::from(settings.segment_checkpoint_max_interval_ms).clamp(
            i64::from(settings.segment_checkpoint_min_interval_ms),
            RUNTIME_CHECKPOINT_MAX_INTERVAL_MS,
        ) as u32;
    settings.speed_limit_bytes_per_second = settings
        .speed_limit_bytes_per_second
        .filter(|value| *value > 0);
    settings.skipped_update_version = settings
        .skipped_update_version
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    settings
}
