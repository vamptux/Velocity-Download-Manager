use crate::model::{
    EngineSettings, HostDiagnosticsSummary, HostProfile, HostTelemetryArgs, ProbeScopeCache,
};

use super::probe::normalize_protocol_label;
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
const RAMP_NO_GAIN_LOCK_WINDOW_MS: i64 = 15 * 60 * 1_000;
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
    effective_connection_target_for_scope(requested_connections, settings, profile, None)
}

pub fn effective_connection_target_for_scope(
    requested_connections: u32,
    settings: &EngineSettings,
    profile: Option<&HostProfile>,
    scope_key: Option<&str>,
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
        if cooldown_active(effective_cooldown_until(profile, scope_key)) {
            if let Some(locked) = effective_locked_connections(profile, scope_key) {
                target = target.min(locked.max(1));
            } else {
                target = target.min(1);
            }
        }
        if should_apply_concurrency_lock_for_scope(profile, scope_key)
            && let Some(locked) = effective_locked_connections(profile, scope_key)
        {
            target = target.min(locked.max(1).min(host_hard_cap));
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

pub fn initial_target_connections_for_scope(
    max_connections: u32,
    settings: &EngineSettings,
    profile: Option<&HostProfile>,
    scope_key: Option<&str>,
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

    if cooldown_active(effective_cooldown_until(profile, scope_key)) {
        return 1;
    }

    if should_apply_concurrency_lock_for_scope(profile, scope_key)
        && let Some(locked) = effective_locked_connections(profile, scope_key)
    {
        return locked.max(1).min(max_connections);
    }

    if effective_telemetry_samples(profile, scope_key) < WARM_START_CONFIDENCE_SAMPLES {
        return target;
    }

    if let Some(throughput) =
        effective_average_throughput_bytes_per_second(Some(profile), scope_key)
    {
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

pub fn effective_average_ttfb_ms(
    profile: Option<&HostProfile>,
    scope_key: Option<&str>,
) -> Option<u64> {
    let profile = profile?;
    let Some(scope) = scope_entry(profile, scope_key) else {
        return profile.average_ttfb_ms;
    };

    if scope_has_telemetry(scope) {
        scope.average_ttfb_ms.or(profile.average_ttfb_ms)
    } else {
        profile.average_ttfb_ms
    }
}

pub fn effective_average_throughput_bytes_per_second(
    profile: Option<&HostProfile>,
    scope_key: Option<&str>,
) -> Option<u64> {
    let profile = profile?;
    let Some(scope) = scope_entry(profile, scope_key) else {
        return profile.average_throughput_bytes_per_second;
    };

    if scope_has_telemetry(scope) {
        scope
            .average_throughput_bytes_per_second
            .or(profile.average_throughput_bytes_per_second)
    } else {
        profile.average_throughput_bytes_per_second
    }
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
    maybe_release_stale_ramp_lock(profile, now);
    let scope_key = payload.scope_key.as_deref();

    if payload.throttle_event {
        profile.throttle_events = profile.throttle_events.saturating_add(1);
    }
    if payload.timeout_event {
        profile.timeout_events = profile.timeout_events.saturating_add(1);
    }
    if payload.reset_event {
        profile.reset_events = profile.reset_events.saturating_add(1);
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

    profile.last_telemetry_at = Some(now);
    profile.telemetry_samples = profile.telemetry_samples.saturating_add(1);

    if scope_key.is_none() {
        if payload.range_validation_failed {
            apply_range_validation_downgrade(profile, now);
        }
        let mut cooldown_base_seconds = 0_i64;
        let mut soft_reset_penalty = false;
        if payload.throttle_event {
            cooldown_base_seconds = cooldown_base_seconds.max(4);
        }
        if payload.timeout_event {
            cooldown_base_seconds = cooldown_base_seconds.max(2);
        }
        if payload.reset_event && !payload.throttle_event && !payload.timeout_event {
            soft_reset_penalty = true;
        }

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
                profile.locked_connections = None;
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
    }

    maybe_recover_host_max_connections(profile, payload);

    if let Some(scope_key) = scope_key {
        let scope = profile
            .probe_scopes
            .entry(scope_key.to_string())
            .or_default();
        apply_scope_telemetry(scope, payload, now);
    }
}

pub fn profile_warning_for_scope(
    profile: Option<&HostProfile>,
    scope_key: Option<&str>,
) -> Option<String> {
    let profile = profile?;
    if let Some(seconds) = cooldown_remaining_seconds(effective_cooldown_until(profile, scope_key))
    {
        if scope_entry(profile, scope_key).is_some() {
            return Some(format!(
                "This request shape is cooling down for about {seconds}s after throttling or unstable responses on the same replay context."
            ));
        }
        return Some(format!(
            "Host cooldown active for about {seconds}s due to throttling or network instability."
        ));
    }

    if should_apply_concurrency_lock_for_scope(profile, scope_key) {
        if scope_entry(profile, scope_key).is_some() {
            return Some(
                "This request shape is temporarily ramp-locked after repeated low-gain connection increases."
                    .to_string(),
            );
        }
        return Some("Host concurrency is temporarily locked due to low ramp-up gain.".to_string());
    }
    if effective_throttle_events(profile, scope_key) > 0 {
        if scope_entry(profile, scope_key).is_some() {
            return Some(
                "This request shape previously throttled parallel requests; conservative concurrency is applied only to this replay context."
                    .to_string(),
            );
        }
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

pub fn host_diagnostics_summary_for_scope(
    profile: Option<&HostProfile>,
    scope_key: Option<&str>,
) -> HostDiagnosticsSummary {
    let Some(profile) = profile else {
        return HostDiagnosticsSummary::default();
    };
    let concurrency_locked = should_apply_concurrency_lock_for_scope(profile, scope_key);

    HostDiagnosticsSummary {
        hard_no_range: false,
        concurrency_locked,
        lock_reason: if !concurrency_locked
            || effective_lock_reason(profile, scope_key) == Some("probe-failures")
        {
            None
        } else {
            effective_lock_reason(profile, scope_key).map(str::to_string)
        },
        cooldown_until: effective_cooldown_until(profile, scope_key),
        negotiated_protocol: profile
            .negotiated_protocol
            .as_deref()
            .map(normalize_protocol_label)
            .map(str::to_string),
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

fn scope_entry<'a>(
    profile: &'a HostProfile,
    scope_key: Option<&str>,
) -> Option<&'a ProbeScopeCache> {
    scope_key.and_then(|key| profile.probe_scopes.get(key))
}

fn scope_has_telemetry(scope: &ProbeScopeCache) -> bool {
    scope.telemetry_samples > 0
        || scope.average_ttfb_ms.is_some()
        || scope.average_throughput_bytes_per_second.is_some()
}

fn effective_telemetry_samples(profile: &HostProfile, scope_key: Option<&str>) -> u32 {
    scope_entry(profile, scope_key)
        .filter(|scope| scope_has_telemetry(scope))
        .map(|scope| scope.telemetry_samples)
        .unwrap_or(profile.telemetry_samples)
}

fn effective_cooldown_until(profile: &HostProfile, scope_key: Option<&str>) -> Option<i64> {
    scope_entry(profile, scope_key)
        .map(|scope| scope.cooldown_until)
        .unwrap_or(profile.cooldown_until)
}

fn effective_locked_connections(profile: &HostProfile, scope_key: Option<&str>) -> Option<u32> {
    scope_entry(profile, scope_key)
        .and_then(|scope| scope.locked_connections)
        .or(profile.locked_connections)
}

fn effective_lock_reason<'a>(profile: &'a HostProfile, scope_key: Option<&str>) -> Option<&'a str> {
    scope_entry(profile, scope_key)
        .and_then(|scope| scope.lock_reason.as_deref())
        .or(profile.lock_reason.as_deref())
}

fn effective_throttle_events(profile: &HostProfile, scope_key: Option<&str>) -> u32 {
    scope_entry(profile, scope_key)
        .map(|scope| scope.throttle_events)
        .unwrap_or(profile.throttle_events)
}

fn should_apply_concurrency_lock_for_scope(profile: &HostProfile, scope_key: Option<&str>) -> bool {
    let Some(scope) = scope_entry(profile, scope_key) else {
        return should_apply_concurrency_lock(profile);
    };
    if !scope.concurrency_locked {
        return false;
    }

    match scope.lock_reason.as_deref() {
        Some("cooldown-active") => cooldown_active(scope.cooldown_until),
        Some("ramp-no-gain") => scope_ramp_no_gain_lock_active(scope, unix_epoch_millis()),
        _ => true,
    }
}

fn scope_ramp_no_gain_lock_active(scope: &ProbeScopeCache, now_millis: i64) -> bool {
    scope
        .last_telemetry_at
        .is_some_and(|value| now_millis.saturating_sub(value) <= RAMP_NO_GAIN_LOCK_WINDOW_MS)
}

fn clear_scope_lock(scope: &mut ProbeScopeCache) {
    scope.concurrency_locked = false;
    scope.locked_connections = None;
    scope.lock_reason = None;
}

fn maybe_release_stale_scope_ramp_lock(scope: &mut ProbeScopeCache, now_millis: i64) {
    if scope.lock_reason.as_deref() != Some("ramp-no-gain") {
        return;
    }
    if scope_ramp_no_gain_lock_active(scope, now_millis) {
        return;
    }

    scope.ramp_attempts_without_gain = 0;
    clear_scope_lock(scope);
}

fn apply_range_validation_downgrade_to_scope(scope: &mut ProbeScopeCache, now_millis: i64) {
    scope.range_supported = Some(false);
    scope.resumable = Some(false);
    scope.hard_no_range = true;
    scope.last_probe_at = Some(now_millis);
    scope.last_instability_at = Some(now_millis);
}

fn apply_scope_telemetry(scope: &mut ProbeScopeCache, payload: &HostTelemetryArgs, now: i64) {
    maybe_release_stale_scope_ramp_lock(scope, now);
    if !cooldown_active(scope.cooldown_until)
        && scope.lock_reason.as_deref() == Some("cooldown-active")
    {
        clear_scope_lock(scope);
    }
    if payload.range_validation_failed {
        apply_range_validation_downgrade_to_scope(scope, now);
    }

    let mut cooldown_base_seconds = 0_i64;
    if payload.throttle_event {
        scope.throttle_events = scope.throttle_events.saturating_add(1);
        scope.last_instability_at = Some(now);
        cooldown_base_seconds = cooldown_base_seconds.max(4);
    }
    if payload.timeout_event {
        scope.timeout_events = scope.timeout_events.saturating_add(1);
        scope.last_instability_at = Some(now);
        cooldown_base_seconds = cooldown_base_seconds.max(2);
    }
    if payload.reset_event {
        scope.reset_events = scope.reset_events.saturating_add(1);
        scope.last_instability_at = Some(now);
    }

    if let Some(ttfb_ms) = payload.ttfb_ms {
        scope.average_ttfb_ms = moving_average(
            scope.average_ttfb_ms,
            ttfb_ms,
            scope.telemetry_samples.saturating_add(1),
        );
    }
    if let Some(throughput) = payload.throughput_bytes_per_second {
        scope.average_throughput_bytes_per_second = moving_average(
            scope.average_throughput_bytes_per_second,
            throughput,
            scope.telemetry_samples.saturating_add(1),
        );
    }

    scope.last_telemetry_at = Some(now);
    scope.telemetry_samples = scope.telemetry_samples.saturating_add(1);

    if let Some(gain) = payload.sustained_gain_bytes_per_second {
        let gain_threshold = ramp_gain_threshold_bytes_per_second(
            payload.throughput_bytes_per_second.unwrap_or_default(),
        );
        if gain <= gain_threshold {
            scope.ramp_attempts_without_gain = scope.ramp_attempts_without_gain.saturating_add(1);
            scope.last_instability_at = Some(now);
        } else {
            scope.ramp_attempts_without_gain = 0;
            if scope.lock_reason.as_deref() == Some("ramp-no-gain") {
                clear_scope_lock(scope);
            }
        }
    }

    if scope.ramp_attempts_without_gain >= NO_GAIN_LOCK_ATTEMPTS {
        scope.concurrency_locked = true;
        scope.locked_connections = payload
            .attempted_connections
            .map(reduced_no_gain_lock_connections);
        scope.lock_reason = Some("ramp-no-gain".to_string());
        scope.last_instability_at = Some(now);
    }

    if cooldown_base_seconds > 0 {
        apply_penalty_cooldown_to_scope(scope, payload, cooldown_base_seconds, now);
    } else if payload.reset_event && !payload.throttle_event && !payload.timeout_event {
        apply_penalty_cooldown_to_scope(scope, payload, 1, now);
    }
}

fn apply_penalty_cooldown_to_scope(
    scope: &mut ProbeScopeCache,
    payload: &HostTelemetryArgs,
    base_seconds: i64,
    now_millis: i64,
) {
    let event_count = scope.throttle_events.saturating_add(scope.timeout_events);
    let multiplier_shift = event_count.min(6).saturating_sub(1);
    let multiplier = 1_i64.checked_shl(multiplier_shift).unwrap_or(64);
    let jitter = i64::from(scope.telemetry_samples % 7);
    let cooldown_seconds = base_seconds
        .saturating_mul(multiplier)
        .saturating_add(jitter)
        .clamp(base_seconds, MAX_COOLDOWN_SECONDS);
    scope.cooldown_until = Some(now_millis.saturating_add(cooldown_seconds.saturating_mul(1_000)));
    scope.concurrency_locked = true;
    scope.locked_connections = Some(cooldown_lock_connections(payload));
    scope.lock_reason = Some("cooldown-active".to_string());
    scope.last_instability_at = Some(now_millis);
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
    let normalized = normalize_protocol_label(protocol);
    let is_multiplexed = protocol_is_multiplexed(Some(normalized));
    let downgrade_observed = match profile.negotiated_protocol.as_deref() {
        Some(previous) => protocol_is_multiplexed(Some(previous)) && !is_multiplexed,
        None => !is_multiplexed,
    };
    if downgrade_observed {
        profile.protocol_downgrade_events = profile.protocol_downgrade_events.saturating_add(1);
    }

    profile.negotiated_protocol = Some(normalized.to_string());
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
        profile.ramp_attempts_without_gain = 0;
        profile.concurrency_locked = false;
        profile.locked_connections = None;
        profile.lock_reason = None;
    }
}

fn maybe_release_stale_ramp_lock(profile: &mut HostProfile, now_millis: i64) {
    if profile.lock_reason.as_deref() != Some("ramp-no-gain") {
        return;
    }
    if ramp_no_gain_lock_active(profile, now_millis) {
        return;
    }

    profile.ramp_attempts_without_gain = 0;
    profile.concurrency_locked = false;
    profile.locked_connections = None;
    profile.lock_reason = None;
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
    if !profile.concurrency_locked {
        return false;
    }

    match profile.lock_reason.as_deref() {
        Some("probe-failures") => false,
        Some("cooldown-active") => cooldown_active(profile.cooldown_until),
        Some("ramp-no-gain") => ramp_no_gain_lock_active(profile, unix_epoch_millis()),
        _ => true,
    }
}

fn ramp_no_gain_lock_active(profile: &HostProfile, now_millis: i64) -> bool {
    profile
        .last_telemetry_at
        .is_some_and(|value| now_millis.saturating_sub(value) <= RAMP_NO_GAIN_LOCK_WINDOW_MS)
}

fn protocol_is_multiplexed(protocol: Option<&str>) -> bool {
    matches!(
        protocol.map(normalize_protocol_label),
        Some("http2") | Some("http3")
    )
}

fn unix_epoch_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}
