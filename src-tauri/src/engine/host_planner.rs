use crate::model::{EngineSettings, HostDiagnosticsSummary, HostProfile, HostTelemetryArgs};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_STABLE_HOST_CAP: u32 = 20;
const EXPERIMENTAL_HOST_CAP: u32 = 64;
const TRAFFIC_MODE_LOW_CONNECTION_CAP: u32 = 2;
const TRAFFIC_MODE_MEDIUM_CONNECTION_CAP: u32 = 4;
const TRAFFIC_MODE_HIGH_CONNECTION_CAP: u32 = 8;
const DEFAULT_TARGET_CHUNK_TIME_SECONDS: u64 = 2;
const NO_GAIN_LOCK_ATTEMPTS: u32 = 5;
const NO_GAIN_THRESHOLD: i64 = 128 * 1024;
const MAX_GAIN_THRESHOLD: i64 = 32 * 1024 * 1024;
const RELATIVE_GAIN_THRESHOLD_DIVISOR: u64 = 24;
const MAX_COOLDOWN_SECONDS: i64 = 300;
const HOST_CAP_RECOVERY_STABLE_SAMPLES: u32 = 4;
const HOST_CAP_RECOVERY_MIN_THROUGHPUT_BYTES_PER_SECOND: u64 = 4 * 1024 * 1024;
const PROBE_FAILURE_LOCK_WINDOW_MS: i64 = 10 * 60 * 1_000;
const PROTOCOL_MULTIPLEX_MAX_CONNECTIONS: u32 = 8;
const PROTOCOL_DEGRADED_MAX_CONNECTIONS: u32 = 2;
const PROTOCOL_MULTIPLEX_CONFIDENCE_MIN_SAMPLES: u32 = 6;
const PROTOCOL_RECOVERY_SAMPLE_MULTIPLIER: u32 = 8;
const WARM_START_CONFIDENCE_SAMPLES: u32 = 4;
const WARM_START_DEFAULT_CONNECTIONS: u32 = 2;
const WARM_START_LARGE_FILE_BYTES: u64 = 64 * 1024 * 1024;
const WARM_START_LARGE_FILE_CONNECTIONS: u32 = 3;
const WARM_START_HUGE_FILE_BYTES: u64 = 512 * 1024 * 1024;
const WARM_START_HUGE_FILE_CONNECTIONS: u32 = 4;
const WARM_START_SLOW_HOST_BYTES_PER_SECOND: u64 = 4 * 1024 * 1024;
const WARM_START_VERY_FAST_HOST_BYTES_PER_SECOND: u64 = 128 * 1024 * 1024;
const WARM_START_VERY_FAST_CONNECTIONS: u32 = 3;

pub fn effective_connection_target(
    requested_connections: u32,
    settings: &EngineSettings,
    profile: Option<&HostProfile>,
) -> u32 {
    let host_hard_cap = if settings.experimental_uncapped_mode {
        EXPERIMENTAL_HOST_CAP
    } else {
        DEFAULT_STABLE_HOST_CAP
    };

    let traffic_mode_cap = traffic_mode_connection_cap(settings);
    let mut target = requested_connections
        .max(1)
        .min(host_hard_cap.min(traffic_mode_cap));

    if let Some(profile) = profile {
        if let Some(host_limit) = profile.max_connections {
            target = target.min(host_limit.max(1));
        }
        if cooldown_active(profile.cooldown_until) {
            if let Some(locked) = profile.locked_connections {
                target = target.min(locked.max(1));
            } else {
                target = target.min(1);
            }
        }
        if should_apply_concurrency_lock(profile) {
            if let Some(locked) = profile.locked_connections {
                target = target.min(locked.max(1).min(host_hard_cap));
            }
        }
        if protocol_is_multiplexed(profile.negotiated_protocol.as_deref()) {
            let reused = profile.protocol_reuse_events;
            let opened = profile.protocol_new_connection_events;
            let samples = reused.saturating_add(opened);
            if samples >= PROTOCOL_MULTIPLEX_CONFIDENCE_MIN_SAMPLES
                && reused >= opened.saturating_mul(2)
            {
                target = target.min(PROTOCOL_MULTIPLEX_MAX_CONNECTIONS);
            }
        }
        if profile.protocol_downgrade_events > 0 {
            target = target.min(PROTOCOL_DEGRADED_MAX_CONNECTIONS);
        }
    }

    target
}

pub fn initial_target_connections(
    max_connections: u32,
    settings: &EngineSettings,
    profile: Option<&HostProfile>,
    planned_size: Option<u64>,
) -> u32 {
    let max_connections = max_connections.max(1);
    if max_connections == 1 {
        return 1;
    }

    let mut target = warm_start_seed_connections(max_connections, settings, planned_size);
    let Some(profile) = profile else {
        return target;
    };

    if cooldown_active(profile.cooldown_until) {
        return 1;
    }

    if should_apply_concurrency_lock(profile) {
        if let Some(locked) = profile.locked_connections {
            return locked.max(1).min(max_connections);
        }
    }

    if profile.telemetry_samples < WARM_START_CONFIDENCE_SAMPLES {
        return target;
    }

    if let Some(throughput) = profile.average_throughput_bytes_per_second {
        if throughput <= WARM_START_SLOW_HOST_BYTES_PER_SECOND {
            target = 1;
        } else if protocol_is_multiplexed(profile.negotiated_protocol.as_deref())
            && throughput >= WARM_START_VERY_FAST_HOST_BYTES_PER_SECOND
            && max_connections >= WARM_START_VERY_FAST_CONNECTIONS
        {
            target = target.max(WARM_START_VERY_FAST_CONNECTIONS);
        }
    }

    target.min(max_connections).max(1)
}

fn warm_start_seed_connections(
    max_connections: u32,
    settings: &EngineSettings,
    planned_size: Option<u64>,
) -> u32 {
    let mut target = max_connections.min(WARM_START_DEFAULT_CONNECTIONS);
    let Some(planned_size) = planned_size.filter(|value| *value > 0) else {
        return scale_target_connections_for_chunk_time(target, max_connections, settings);
    };

    if planned_size >= WARM_START_LARGE_FILE_BYTES
        && max_connections >= WARM_START_LARGE_FILE_CONNECTIONS
    {
        target = target.max(WARM_START_LARGE_FILE_CONNECTIONS);
    }
    if planned_size >= WARM_START_HUGE_FILE_BYTES
        && max_connections >= WARM_START_HUGE_FILE_CONNECTIONS
    {
        target = target.max(WARM_START_HUGE_FILE_CONNECTIONS);
    }

    scale_target_connections_for_chunk_time(target, max_connections, settings)
}

fn scale_target_connections_for_chunk_time(
    target: u32,
    max_connections: u32,
    settings: &EngineSettings,
) -> u32 {
    let scaled = u64::from(target.max(1))
        .saturating_mul(DEFAULT_TARGET_CHUNK_TIME_SECONDS)
        .div_ceil(u64::from(settings.target_chunk_time_seconds.max(1)));
    u32::try_from(scaled)
        .unwrap_or(u32::MAX)
        .clamp(1, max_connections)
}

fn traffic_mode_connection_cap(settings: &EngineSettings) -> u32 {
    match settings.traffic_mode {
        crate::model::TrafficMode::Low => TRAFFIC_MODE_LOW_CONNECTION_CAP,
        crate::model::TrafficMode::Medium => TRAFFIC_MODE_MEDIUM_CONNECTION_CAP,
        crate::model::TrafficMode::High => TRAFFIC_MODE_HIGH_CONNECTION_CAP,
        crate::model::TrafficMode::Max => {
            if settings.experimental_uncapped_mode {
                EXPERIMENTAL_HOST_CAP
            } else {
                DEFAULT_STABLE_HOST_CAP
            }
        }
    }
}

pub fn ramp_gain_threshold_bytes_per_second(reference_throughput: u64) -> i64 {
    let relative_threshold = reference_throughput
        .checked_div(RELATIVE_GAIN_THRESHOLD_DIVISOR)
        .unwrap_or(reference_throughput);
    i64::try_from(relative_threshold)
        .unwrap_or(i64::MAX)
        .clamp(NO_GAIN_THRESHOLD, MAX_GAIN_THRESHOLD)
}

pub fn apply_host_telemetry(profile: &mut HostProfile, payload: &HostTelemetryArgs) {
    let now = unix_epoch_millis();
    maybe_release_stale_probe_failure_lock(profile, now);
    if payload.range_validation_failed {
        apply_range_validation_downgrade(profile, now);
    }
    let mut cooldown_base_seconds = 0_i64;
    let mut soft_reset_penalty = false;
    if payload.throttle_event {
        profile.throttle_events = profile.throttle_events.saturating_add(1);
        cooldown_base_seconds = cooldown_base_seconds.max(4);
    }
    if payload.timeout_event {
        profile.timeout_events = profile.timeout_events.saturating_add(1);
        cooldown_base_seconds = cooldown_base_seconds.max(2);
    }
    if payload.reset_event {
        profile.reset_events = profile.reset_events.saturating_add(1);
        if !payload.throttle_event && !payload.timeout_event {
            soft_reset_penalty = true;
        }
    }
    if payload.throttle_event || payload.timeout_event || payload.reset_event {
        profile.stable_recovery_samples = 0;
    }

    if let Some(ttfb_ms) = payload.ttfb_ms {
        profile.average_ttfb_ms = moving_average(
            profile.average_ttfb_ms,
            ttfb_ms,
            profile.telemetry_samples.saturating_add(1),
        );
    }
    if let Some(throughput) = payload.throughput_bytes_per_second {
        profile.average_throughput_bytes_per_second = moving_average(
            profile.average_throughput_bytes_per_second,
            throughput,
            profile.telemetry_samples.saturating_add(1),
        );
    }
    if let Some(protocol) = payload
        .negotiated_protocol
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        observe_negotiated_protocol(profile, protocol);
    }
    if let Some(reused) = payload.connection_reused {
        if reused {
            profile.protocol_reuse_events = profile.protocol_reuse_events.saturating_add(1);
        } else {
            profile.protocol_new_connection_events =
                profile.protocol_new_connection_events.saturating_add(1);
        }
    }

    profile.telemetry_samples = profile.telemetry_samples.saturating_add(1);

    if let Some(gain) = payload.sustained_gain_bytes_per_second {
        let gain_threshold = ramp_gain_threshold_bytes_per_second(
            payload.throughput_bytes_per_second.unwrap_or_default(),
        );
        if gain <= gain_threshold {
            profile.ramp_attempts_without_gain =
                profile.ramp_attempts_without_gain.saturating_add(1);
        } else {
            profile.ramp_attempts_without_gain = 0;
            profile.concurrency_locked = false;
            profile.lock_reason = None;
        }
    }

    if profile.ramp_attempts_without_gain >= NO_GAIN_LOCK_ATTEMPTS {
        profile.concurrency_locked = true;
        profile.locked_connections = payload
            .attempted_connections
            .map(reduced_no_gain_lock_connections);
        profile.lock_reason = Some("ramp-no-gain".to_string());
    }

    if cooldown_base_seconds > 0 {
        apply_penalty_cooldown(profile, payload, cooldown_base_seconds, now);
    } else if soft_reset_penalty {
        apply_soft_reset_penalty(profile, payload);
    } else if !cooldown_active(profile.cooldown_until)
        && profile.lock_reason.as_deref() == Some("cooldown-active")
    {
        profile.concurrency_locked = false;
        profile.locked_connections = None;
        profile.lock_reason = None;
    }

    maybe_recover_host_max_connections(profile, payload);
}

pub fn profile_warning(profile: Option<&HostProfile>) -> Option<String> {
    let profile = profile?;
    if let Some(seconds) = cooldown_remaining_seconds(profile.cooldown_until) {
        return Some(format!(
            "Host cooldown active for about {seconds}s due to throttling or network instability."
        ));
    }

    if profile.concurrency_locked {
        if profile.lock_reason.as_deref() == Some("probe-failures") {
            return None;
        }
        return Some("Host concurrency is temporarily locked due to low ramp-up gain.".to_string());
    }
    if profile.throttle_events > 0 {
        return Some(
            "Host previously throttled parallel requests; conservative concurrency applied."
                .to_string(),
        );
    }
    if profile.protocol_downgrade_events > 0 {
        return Some(
            "Host recently downgraded protocol behavior; planner is using conservative socket limits."
                .to_string(),
        );
    }

    None
}

pub fn host_diagnostics_summary(profile: Option<&HostProfile>) -> HostDiagnosticsSummary {
    let Some(profile) = profile else {
        return HostDiagnosticsSummary::default();
    };

    HostDiagnosticsSummary {
        hard_no_range: false,
        concurrency_locked: profile.concurrency_locked && should_apply_concurrency_lock(profile),
        lock_reason: if profile.lock_reason.as_deref() == Some("probe-failures") {
            None
        } else {
            profile.lock_reason.clone()
        },
        cooldown_until: profile.cooldown_until,
        negotiated_protocol: profile.negotiated_protocol.clone(),
        reuse_connections: host_reuse_preference(profile),
    }
}

fn host_reuse_preference(profile: &HostProfile) -> Option<bool> {
    let reused = profile.protocol_reuse_events;
    let fresh = profile.protocol_new_connection_events;
    let total = reused.saturating_add(fresh);
    if total < WARM_START_CONFIDENCE_SAMPLES {
        return None;
    }
    if reused >= fresh.saturating_mul(2).max(1) {
        return Some(true);
    }
    if fresh >= reused.saturating_mul(2).max(1) {
        return Some(false);
    }
    None
}

fn apply_soft_reset_penalty(profile: &mut HostProfile, payload: &HostTelemetryArgs) {
    let Some(attempted) = payload.attempted_connections.filter(|value| *value > 2) else {
        return;
    };
    let reduced = attempted.saturating_sub(1).max(2);
    profile.max_connections = Some(
        profile
            .max_connections
            .unwrap_or(reduced)
            .min(reduced)
            .max(2),
    );
}

fn apply_range_validation_downgrade(profile: &mut HostProfile, now_millis: i64) {
    profile.range_supported = Some(false);
    profile.resumable = Some(false);
    profile.hard_no_range = true;
    profile.last_probe_at = Some(now_millis);
    profile.stable_recovery_samples = 0;
}

fn observe_negotiated_protocol(profile: &mut HostProfile, protocol: &str) {
    let is_multiplexed = protocol_is_multiplexed(Some(protocol));
    let downgrade_observed = match profile.negotiated_protocol.as_deref() {
        Some(previous) => protocol_is_multiplexed(Some(previous)) && !is_multiplexed,
        None => !is_multiplexed,
    };
    if downgrade_observed {
        profile.protocol_downgrade_events = profile.protocol_downgrade_events.saturating_add(1);
    }

    profile.negotiated_protocol = Some(protocol.to_string());
    profile.protocol_samples = profile.protocol_samples.saturating_add(1);

    if is_multiplexed {
        maybe_recover_protocol_downgrade(profile);
    }
}

fn maybe_recover_protocol_downgrade(profile: &mut HostProfile) {
    if profile.protocol_downgrade_events == 0 {
        return;
    }

    let recovery_samples = profile
        .protocol_reuse_events
        .saturating_add(profile.protocol_samples);
    let required_samples = profile
        .protocol_downgrade_events
        .saturating_mul(PROTOCOL_RECOVERY_SAMPLE_MULTIPLIER);
    if recovery_samples >= required_samples.max(PROTOCOL_MULTIPLEX_CONFIDENCE_MIN_SAMPLES) {
        profile.protocol_downgrade_events = profile.protocol_downgrade_events.saturating_sub(1);
    }
}

fn moving_average(previous: Option<u64>, current: u64, sample_count: u32) -> Option<u64> {
    if sample_count <= 1 {
        return Some(current);
    }

    let old = previous.unwrap_or(current) as u128;
    let next = current as u128;
    let n = sample_count as u128;
    let weighted = old.saturating_mul(n.saturating_sub(1)).saturating_add(next) / n;
    Some(weighted.min(u64::MAX as u128) as u64)
}

fn apply_penalty_cooldown(
    profile: &mut HostProfile,
    payload: &HostTelemetryArgs,
    base_seconds: i64,
    now_millis: i64,
) {
    let event_count = profile
        .throttle_events
        .saturating_add(profile.timeout_events);
    let multiplier_shift = event_count.min(6).saturating_sub(1);
    let multiplier = 1_i64.checked_shl(multiplier_shift).unwrap_or(64);
    let jitter = i64::from(profile.telemetry_samples % 7);
    let cooldown_seconds = base_seconds
        .saturating_mul(multiplier)
        .saturating_add(jitter)
        .clamp(base_seconds, MAX_COOLDOWN_SECONDS);
    profile.cooldown_until =
        Some(now_millis.saturating_add(cooldown_seconds.saturating_mul(1_000)));
    profile.concurrency_locked = true;
    profile.locked_connections = Some(cooldown_lock_connections(payload));
    profile.lock_reason = Some("cooldown-active".to_string());

    if let Some(attempted) = payload.attempted_connections.filter(|value| *value > 1) {
        let reduced = attempted.saturating_sub(1).max(2);
        profile.max_connections = Some(
            profile
                .max_connections
                .unwrap_or(reduced)
                .min(reduced)
                .max(2),
        );
    } else if profile.max_connections.is_none() {
        profile.max_connections = Some(2);
    }
}

fn cooldown_lock_connections(payload: &HostTelemetryArgs) -> u32 {
    let attempted = payload.attempted_connections.unwrap_or(1).max(1);
    let throttle_lock = payload
        .throttle_event
        .then(|| cooldown_lock_connections_for_throttle(attempted));
    let timeout_lock = payload
        .timeout_event
        .then(|| cooldown_lock_connections_for_timeout(attempted));
    match (throttle_lock, timeout_lock) {
        (Some(throttle), Some(timeout)) => throttle.min(timeout).max(1),
        (Some(throttle), None) => throttle.max(1),
        (None, Some(timeout)) => timeout.max(1),
        (None, None) => 1,
    }
}

fn cooldown_lock_connections_for_throttle(attempted_connections: u32) -> u32 {
    if attempted_connections <= 2 {
        return 1;
    }
    if attempted_connections <= 4 {
        return 2;
    }
    attempted_connections
        .saturating_sub((attempted_connections / 2).max(1))
        .max(2)
}

fn cooldown_lock_connections_for_timeout(attempted_connections: u32) -> u32 {
    if attempted_connections <= 2 {
        return 1;
    }
    attempted_connections.saturating_sub(1).max(2)
}

fn maybe_recover_host_max_connections(profile: &mut HostProfile, payload: &HostTelemetryArgs) {
    if cooldown_active(profile.cooldown_until)
        || payload.throttle_event
        || payload.timeout_event
        || payload.reset_event
        || payload.range_validation_failed
    {
        profile.stable_recovery_samples = 0;
        return;
    }

    let Some(current_cap) = profile.max_connections else {
        profile.stable_recovery_samples = 0;
        return;
    };
    let Some(attempted_connections) = payload.attempted_connections.filter(|value| *value > 0)
    else {
        profile.stable_recovery_samples = 0;
        return;
    };
    let Some(_throughput) = payload
        .throughput_bytes_per_second
        .filter(|value| *value >= HOST_CAP_RECOVERY_MIN_THROUGHPUT_BYTES_PER_SECOND)
    else {
        profile.stable_recovery_samples = 0;
        return;
    };

    if attempted_connections < current_cap {
        profile.stable_recovery_samples = 0;
        return;
    }

    profile.stable_recovery_samples = profile.stable_recovery_samples.saturating_add(1);
    if profile.stable_recovery_samples < HOST_CAP_RECOVERY_STABLE_SAMPLES {
        return;
    }

    profile.max_connections = Some(current_cap.saturating_add(1).min(EXPERIMENTAL_HOST_CAP));
    profile.stable_recovery_samples = 0;
    if profile.lock_reason.as_deref() == Some("ramp-no-gain") {
        profile.concurrency_locked = false;
        profile.locked_connections = None;
        profile.lock_reason = None;
    }
}

fn maybe_release_stale_probe_failure_lock(profile: &mut HostProfile, now_millis: i64) {
    if !profile.concurrency_locked || profile.lock_reason.as_deref() != Some("probe-failures") {
        return;
    }
    let is_active = profile
        .last_probe_error_at
        .is_some_and(|value| now_millis.saturating_sub(value) <= PROBE_FAILURE_LOCK_WINDOW_MS);
    if !is_active {
        profile.concurrency_locked = false;
        profile.locked_connections = None;
        profile.lock_reason = None;
    }
}

fn reduced_no_gain_lock_connections(attempted_connections: u32) -> u32 {
    if attempted_connections <= 2 {
        return 1;
    }
    if attempted_connections <= 4 {
        return attempted_connections.saturating_sub(1).max(1);
    }
    attempted_connections
        .saturating_sub((attempted_connections / 3).max(1))
        .max(2)
}

fn cooldown_remaining_seconds(cooldown_until: Option<i64>) -> Option<i64> {
    let until = cooldown_until?;
    let remaining = until.saturating_sub(unix_epoch_millis());
    if remaining <= 0 {
        None
    } else {
        Some((remaining / 1_000).max(1))
    }
}

fn cooldown_active(cooldown_until: Option<i64>) -> bool {
    cooldown_remaining_seconds(cooldown_until).is_some()
}

fn should_apply_concurrency_lock(profile: &HostProfile) -> bool {
    profile.concurrency_locked && profile.lock_reason.as_deref() != Some("probe-failures")
}

fn protocol_is_multiplexed(protocol: Option<&str>) -> bool {
    let Some(protocol) = protocol.map(str::trim) else {
        return false;
    };
    protocol.eq_ignore_ascii_case("http2")
        || protocol.eq_ignore_ascii_case("http/2")
        || protocol.eq_ignore_ascii_case("h2")
        || protocol.eq_ignore_ascii_case("http3")
        || protocol.eq_ignore_ascii_case("http/3")
        || protocol.eq_ignore_ascii_case("h3")
}

fn unix_epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        apply_host_telemetry, effective_connection_target, initial_target_connections,
        ramp_gain_threshold_bytes_per_second, NO_GAIN_LOCK_ATTEMPTS,
    };
    use crate::model::{EngineSettings, HostProfile, HostTelemetryArgs, TrafficMode};

    #[test]
    fn recovers_host_cap_after_stable_samples() {
        let mut profile = HostProfile {
            max_connections: Some(4),
            ..HostProfile::default()
        };

        for _ in 0..4 {
            apply_host_telemetry(
                &mut profile,
                &HostTelemetryArgs {
                    host: "example.com".to_string(),
                    attempted_connections: Some(4),
                    sustained_gain_bytes_per_second: Some(0),
                    throughput_bytes_per_second: Some(8 * 1024 * 1024),
                    ttfb_ms: None,
                    negotiated_protocol: None,
                    connection_reused: None,
                    throttle_event: false,
                    timeout_event: false,
                    reset_event: false,
                    range_validation_failed: false,
                },
            );
        }

        assert_eq!(profile.max_connections, Some(5));
    }

    #[test]
    fn does_not_cap_multiplexed_hosts_without_explicit_reuse_signal() {
        let settings = EngineSettings::default();
        let profile = HostProfile {
            negotiated_protocol: Some("h2".to_string()),
            protocol_samples: 10,
            ..HostProfile::default()
        };

        assert_eq!(
            effective_connection_target(12, &settings, Some(&profile)),
            12
        );
    }

    #[test]
    fn warm_start_starts_low_for_unknown_or_slow_hosts() {
        let settings = EngineSettings::default();

        assert_eq!(initial_target_connections(8, &settings, None, None), 2);
        assert_eq!(
            initial_target_connections(8, &settings, None, Some(128 * 1024 * 1024)),
            3
        );

        let slow_profile = HostProfile {
            telemetry_samples: 6,
            average_throughput_bytes_per_second: Some(2 * 1024 * 1024),
            ..HostProfile::default()
        };

        assert_eq!(
            initial_target_connections(8, &settings, Some(&slow_profile), Some(128 * 1024 * 1024)),
            1
        );
    }

    #[test]
    fn warm_start_allows_three_connections_only_for_verified_fast_multiplexed_hosts() {
        let settings = EngineSettings::default();
        let profile = HostProfile {
            telemetry_samples: 8,
            negotiated_protocol: Some("h2".to_string()),
            average_throughput_bytes_per_second: Some(160 * 1024 * 1024),
            ..HostProfile::default()
        };

        assert_eq!(
            initial_target_connections(8, &settings, Some(&profile), Some(128 * 1024 * 1024)),
            3
        );
    }

    #[test]
    fn lower_traffic_mode_caps_effective_connections() {
        let settings = EngineSettings {
            traffic_mode: TrafficMode::Low,
            ..EngineSettings::default()
        };

        assert_eq!(effective_connection_target(8, &settings, None), 2);
    }

    #[test]
    fn multiplexed_samples_clear_downgrade_penalty_after_recovery_window() {
        let mut profile = HostProfile {
            negotiated_protocol: Some("http/1.1".to_string()),
            protocol_downgrade_events: 1,
            ..HostProfile::default()
        };

        for _ in 0..super::PROTOCOL_RECOVERY_SAMPLE_MULTIPLIER {
            apply_host_telemetry(
                &mut profile,
                &HostTelemetryArgs {
                    host: "example.com".to_string(),
                    attempted_connections: Some(4),
                    sustained_gain_bytes_per_second: None,
                    throughput_bytes_per_second: Some(8 * 1024 * 1024),
                    ttfb_ms: None,
                    negotiated_protocol: Some("h2".to_string()),
                    connection_reused: None,
                    throttle_event: false,
                    timeout_event: false,
                    reset_event: false,
                    range_validation_failed: false,
                },
            );
        }

        assert_eq!(profile.protocol_downgrade_events, 0);
    }

    #[test]
    fn shorter_target_chunk_time_increases_warm_start_supply() {
        let fast_settings = EngineSettings {
            target_chunk_time_seconds: 1,
            ..EngineSettings::default()
        };
        let conservative_settings = EngineSettings {
            target_chunk_time_seconds: 4,
            ..EngineSettings::default()
        };

        assert!(
            initial_target_connections(8, &fast_settings, None, Some(128 * 1024 * 1024))
                > initial_target_connections(
                    8,
                    &conservative_settings,
                    None,
                    Some(128 * 1024 * 1024)
                )
        );
    }

    #[test]
    fn gain_threshold_scales_with_throughput() {
        assert_eq!(ramp_gain_threshold_bytes_per_second(0), 128 * 1024);
        assert!(ramp_gain_threshold_bytes_per_second(600 * 1024 * 1024) > 20 * 1024 * 1024);
    }

    #[test]
    fn repeated_no_gain_steps_down_locked_connections() {
        let mut profile = HostProfile::default();

        for _ in 0..NO_GAIN_LOCK_ATTEMPTS {
            apply_host_telemetry(
                &mut profile,
                &HostTelemetryArgs {
                    host: "example.com".to_string(),
                    attempted_connections: Some(8),
                    sustained_gain_bytes_per_second: Some(2 * 1024 * 1024),
                    throughput_bytes_per_second: Some(600 * 1024 * 1024),
                    ttfb_ms: None,
                    negotiated_protocol: None,
                    connection_reused: None,
                    throttle_event: false,
                    timeout_event: false,
                    reset_event: false,
                    range_validation_failed: false,
                },
            );
        }

        assert!(profile.concurrency_locked);
        assert_eq!(profile.locked_connections, Some(6));
        assert_eq!(profile.lock_reason.as_deref(), Some("ramp-no-gain"));
    }

    #[test]
    fn reset_events_reduce_cap_without_forcing_cooldown_lock() {
        let mut profile = HostProfile::default();

        apply_host_telemetry(
            &mut profile,
            &HostTelemetryArgs {
                host: "example.com".to_string(),
                attempted_connections: Some(8),
                sustained_gain_bytes_per_second: None,
                throughput_bytes_per_second: Some(60 * 1024 * 1024),
                ttfb_ms: None,
                negotiated_protocol: None,
                connection_reused: None,
                throttle_event: false,
                timeout_event: false,
                reset_event: true,
                range_validation_failed: false,
            },
        );

        assert_eq!(profile.max_connections, Some(7));
        assert_eq!(profile.lock_reason, None);
        assert_eq!(profile.locked_connections, None);
    }

    #[test]
    fn cooldown_respects_locked_connections_in_effective_target() {
        let settings = EngineSettings::default();
        let profile = HostProfile {
            cooldown_until: Some(super::unix_epoch_millis().saturating_add(30_000)),
            concurrency_locked: true,
            locked_connections: Some(3),
            lock_reason: Some("cooldown-active".to_string()),
            ..HostProfile::default()
        };

        assert_eq!(effective_connection_target(8, &settings, Some(&profile)), 3);
    }

    #[test]
    fn timeout_cooldown_avoids_hard_drop_to_single_connection() {
        let mut profile = HostProfile::default();

        apply_host_telemetry(
            &mut profile,
            &HostTelemetryArgs {
                host: "example.com".to_string(),
                attempted_connections: Some(8),
                sustained_gain_bytes_per_second: None,
                throughput_bytes_per_second: Some(240 * 1024 * 1024),
                ttfb_ms: None,
                negotiated_protocol: None,
                connection_reused: None,
                throttle_event: false,
                timeout_event: true,
                reset_event: false,
                range_validation_failed: false,
            },
        );

        assert_eq!(profile.lock_reason.as_deref(), Some("cooldown-active"));
        assert_eq!(profile.locked_connections, Some(7));
        assert_eq!(profile.max_connections, Some(7));
    }

    #[test]
    fn effective_target_never_exceeds_requested_when_locked() {
        let settings = EngineSettings::default();
        let profile = HostProfile {
            concurrency_locked: true,
            locked_connections: Some(8),
            lock_reason: Some("ramp-no-gain".to_string()),
            ..HostProfile::default()
        };

        assert_eq!(effective_connection_target(4, &settings, Some(&profile)), 4);
    }

    #[test]
    fn stale_probe_failure_lock_does_not_limit_warm_start_or_warning() {
        let stale = super::unix_epoch_millis().saturating_sub(11 * 60 * 1_000);
        let profile = HostProfile {
            concurrency_locked: true,
            locked_connections: Some(1),
            lock_reason: Some("probe-failures".to_string()),
            last_probe_error_at: Some(stale),
            ..HostProfile::default()
        };

        assert_eq!(
            initial_target_connections(
                8,
                &EngineSettings::default(),
                Some(&profile),
                Some(128 * 1024 * 1024)
            ),
            3
        );
        assert_eq!(super::profile_warning(Some(&profile)), None);
    }

    #[test]
    fn range_validation_failure_marks_host_as_no_range_without_global_cap_drop() {
        let settings = EngineSettings::default();
        let mut profile = HostProfile::default();

        apply_host_telemetry(
            &mut profile,
            &HostTelemetryArgs {
                host: "example.com".to_string(),
                attempted_connections: Some(6),
                sustained_gain_bytes_per_second: None,
                throughput_bytes_per_second: Some(24 * 1024 * 1024),
                ttfb_ms: None,
                negotiated_protocol: Some("h2".to_string()),
                connection_reused: Some(true),
                throttle_event: false,
                timeout_event: false,
                reset_event: false,
                range_validation_failed: true,
            },
        );

        assert_eq!(profile.range_supported, Some(false));
        assert_eq!(profile.resumable, Some(false));
        assert!(profile.hard_no_range);
        assert_eq!(effective_connection_target(8, &settings, Some(&profile)), 8);
    }
}
