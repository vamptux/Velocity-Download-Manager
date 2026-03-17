use crate::model::{
    DownloadRequestField, DownloadRequestMethod, HostProfile, ProbeScopeCache,
    RecentProbeCacheEntry, RegistrySnapshot,
};

use super::http_helpers::request_context_supports_segmented_transfer;
use super::probe::{DownloadProbeData, RangeObservation};

const PROBE_CAPABILITY_TTL_MS: i64 = 30 * 60 * 1_000;
const PROBE_CAPABILITY_STABLE_TTL_MS: i64 = 45 * 60 * 1_000;
const PROBE_CAPABILITY_UNSTABLE_TTL_MS: i64 = 6 * 60 * 1_000;
const PROBE_RESULT_REUSE_TTL_MS: i64 = 3 * 60 * 1_000;
const PROBE_SCOPE_FAILURE_TTL_MS: i64 = 10 * 60 * 1_000;
const PROBE_SCOPE_STABILITY_WINDOW_MS: i64 = 15 * 60 * 1_000;
const PROBE_FAILURE_LOCK_THRESHOLD: u32 = 3;
const PROBE_SCOPE_STABLE_TELEMETRY_SAMPLES: u32 = 4;
const MAX_SCOPE_CACHE_ENTRIES: usize = 24;
const MAX_RECENT_PROBE_ENTRIES: usize = 64;

#[derive(Clone, Copy)]
pub(super) struct CachedProbeCapabilities {
    pub(super) range_supported: Option<bool>,
    pub(super) hard_no_range: bool,
    pub(super) resumable: Option<bool>,
    pub(super) content_length_hint: Option<u64>,
}

pub(super) fn probe_scope_key(
    url: &str,
    request_method: &DownloadRequestMethod,
    request_form_fields: &[DownloadRequestField],
) -> String {
    let normalized_url = normalize_probe_url(url);
    let method = match request_method {
        DownloadRequestMethod::Get => "get",
        DownloadRequestMethod::Post => "post",
    };
    if request_form_fields.is_empty() {
        return format!("{method}|{normalized_url}");
    }

    let field_signature = request_form_fields
        .iter()
        .map(|field| format!("{}={}", field.name, field.value))
        .collect::<Vec<_>>()
        .join("&");
    format!("{method}|{normalized_url}|{field_signature}")
}

pub(super) fn update_profile_probe_cache(
    profile: &mut HostProfile,
    scope_key: &str,
    probe: &DownloadProbeData,
    now: i64,
) {
    prune_scope_cache(profile, now);

    let scope = profile
        .probe_scopes
        .entry(scope_key.to_string())
        .or_default();
    match probe.range_observation {
        RangeObservation::Supported => {
            scope.range_supported = Some(true);
            scope.resumable = Some(probe.resumable);
            scope.hard_no_range = false;
        }
        RangeObservation::Unsupported => {
            scope.range_supported = Some(false);
            scope.resumable = Some(false);
            scope.hard_no_range = true;
        }
        RangeObservation::Unknown => {}
    }
    scope.content_length_hint = probe.size.or(scope.content_length_hint);
    scope.last_probe_at = Some(now);
    scope.probe_failure_streak = 0;
    scope.last_probe_error_at = None;

    profile.content_length_hint = probe.size.or(profile.content_length_hint);
    profile.last_probe_at = Some(now);
    profile.probe_failure_streak = 0;
    profile.last_probe_error_at = None;
    if let Some(protocol) = probe
        .negotiated_protocol
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        profile.negotiated_protocol = Some(protocol.clone());
    }
    if profile.lock_reason.as_deref() == Some("probe-failures") {
        profile.concurrency_locked = false;
        profile.locked_connections = None;
        profile.lock_reason = None;
    }
}

pub(super) fn record_probe_failure(profile: &mut HostProfile, scope_key: &str, now: i64) {
    prune_scope_cache(profile, now);

    let scope = profile
        .probe_scopes
        .entry(scope_key.to_string())
        .or_default();
    scope.probe_failure_streak = scope.probe_failure_streak.saturating_add(1);
    scope.last_probe_error_at = Some(now);
    scope.last_instability_at = Some(now);

    profile.probe_failure_streak = profile.probe_failure_streak.saturating_add(1);
    profile.last_probe_error_at = Some(now);
}

pub(super) fn fresh_probe_capabilities(
    profile: &HostProfile,
    scope_key: &str,
    now: i64,
) -> Option<CachedProbeCapabilities> {
    profile
        .probe_scopes
        .get(scope_key)
        .and_then(|scope| fresh_scope_capabilities(scope, now))
}

pub(super) fn probe_cache_stale(profile: &HostProfile, scope_key: &str, now: i64) -> bool {
    if let Some(scope) = profile.probe_scopes.get(scope_key) {
        return scope
            .last_probe_at
            .is_some_and(|timestamp| now.saturating_sub(timestamp) > scope_capability_ttl_ms(scope, now));
    }

    false
}

pub(super) fn append_probe_cache_warning(
    warnings: &mut Vec<String>,
    profile: Option<&HostProfile>,
    scope_key: &str,
    cached_probe: Option<&CachedProbeCapabilities>,
    now: i64,
) {
    let Some(profile) = profile else {
        return;
    };

    if cached_probe.is_none() && probe_cache_stale(profile, scope_key, now) {
        let warning = profile
            .probe_scopes
            .get(scope_key)
            .filter(|scope| scope_recent_instability(scope, now))
            .map(|_| {
                "Saved capabilities for this request shape expired early after recent instability; waiting for a fresh probe."
                    .to_string()
            })
            .unwrap_or_else(|| {
                "Saved capabilities for this request shape expired; waiting for a fresh probe."
                    .to_string()
            });
        warnings.push(warning);
    }

    if let Some(scope) = profile.probe_scopes.get(scope_key)
        && scoped_probe_failure_active(scope, now) {
            warnings.push(
                "Repeated probe failures were seen on this exact request shape; VDM is planning it conservatively until a fresh probe succeeds."
                    .to_string(),
            );
        }

    if let Some(cached_probe) = cached_probe
        && cached_probe.hard_no_range && cached_probe.range_supported == Some(false) {
            warnings.push(
                "Saved capability learning for this exact request shape shows the host rejected byte-range requests; segmented mode will stay disabled until a fresh probe proves otherwise."
                    .to_string(),
            );
        }
}

pub(super) fn scoped_hard_no_range(
    profile: Option<&HostProfile>,
    scope_key: &str,
    now: i64,
) -> bool {
    profile
        .and_then(|value| fresh_probe_capabilities(value, scope_key, now))
        .is_some_and(|cached| cached.hard_no_range && cached.range_supported == Some(false))
}

pub(super) fn scoped_probe_failures(
    profile: Option<&HostProfile>,
    scope_key: &str,
    now: i64,
) -> u32 {
    let Some(profile) = profile else {
        return 0;
    };
    let Some(scope) = profile.probe_scopes.get(scope_key) else {
        return 0;
    };
    if scoped_probe_failure_active(scope, now) {
        scope.probe_failure_streak
    } else {
        0
    }
}

pub(super) fn apply_scope_range_validation_failure(
    profile: &mut HostProfile,
    scope_key: &str,
    content_length_hint: Option<u64>,
    now: i64,
) {
    prune_scope_cache(profile, now);

    let scope = profile
        .probe_scopes
        .entry(scope_key.to_string())
        .or_default();
    scope.range_supported = Some(false);
    scope.resumable = Some(false);
    scope.hard_no_range = true;
    scope.content_length_hint = content_length_hint.or(scope.content_length_hint);
    scope.last_probe_at = Some(now);
    scope.probe_failure_streak = 0;
    scope.last_probe_error_at = None;
    scope.last_instability_at = Some(now);
}

pub(super) fn store_recent_probe(
    registry: &mut RegistrySnapshot,
    scope_key: &str,
    host: &str,
    probe: &DownloadProbeData,
    now: i64,
) {
    prune_recent_probe_cache(registry, now);
    registry.recent_probes.insert(
        scope_key.to_string(),
        RecentProbeCacheEntry {
            captured_at: now,
            host: host.to_string(),
            final_url: probe.final_url.clone(),
            size: probe.size,
            mime_type: probe.mime_type.clone(),
            negotiated_protocol: probe.negotiated_protocol.clone(),
            range_supported: probe.range_supported,
            resumable: probe.resumable,
            validators: probe.validators.clone(),
            suggested_name: probe.suggested_name.clone(),
            compatibility: probe.compatibility.clone(),
            warnings: probe.warnings.clone(),
        },
    );
    trim_recent_probe_cache(registry);
}

pub(super) fn fresh_recent_probe(
    registry: &RegistrySnapshot,
    scope_key: &str,
    now: i64,
) -> Option<RecentProbeCacheEntry> {
    let cached = registry.recent_probes.get(scope_key)?;
    if now.saturating_sub(cached.captured_at) > PROBE_RESULT_REUSE_TTL_MS {
        return None;
    }
    Some(cached.clone())
}

pub(super) fn cached_probe_to_download_probe(cached: &RecentProbeCacheEntry) -> DownloadProbeData {
    let exact_request_shape_allows_segmentation = request_context_supports_segmented_transfer(
        &cached.compatibility.request_method,
        &cached.compatibility.request_form_fields,
    );
    let range_observation = if cached.range_supported {
        RangeObservation::Supported
    } else if exact_request_shape_allows_segmentation {
        RangeObservation::Unsupported
    } else {
        RangeObservation::Unknown
    };

    DownloadProbeData {
        final_url: cached.final_url.clone(),
        size: cached.size,
        mime_type: cached.mime_type.clone(),
        negotiated_protocol: cached.negotiated_protocol.clone(),
        range_supported: cached.range_supported,
        range_observation,
        resumable: cached.resumable,
        validators: cached.validators.clone(),
        suggested_name: cached.suggested_name.clone(),
        compatibility: cached.compatibility.clone(),
        warnings: cached.warnings.clone(),
    }
}

fn normalize_probe_url(url: &str) -> String {
    if let Ok(mut parsed) = reqwest::Url::parse(url) {
        parsed.set_fragment(None);
        return parsed.to_string();
    }

    url.split('#').next().unwrap_or(url).trim().to_string()
}

fn fresh_scope_capabilities(scope: &ProbeScopeCache, now: i64) -> Option<CachedProbeCapabilities> {
    let last_probe_at = scope.last_probe_at?;
    if now.saturating_sub(last_probe_at) > scope_capability_ttl_ms(scope, now) {
        return None;
    }

    Some(CachedProbeCapabilities {
        range_supported: scope.range_supported,
        hard_no_range: scope.hard_no_range,
        resumable: scope.resumable,
        content_length_hint: scope.content_length_hint,
    })
}

fn scoped_probe_failure_active(scope: &ProbeScopeCache, now: i64) -> bool {
    scope.probe_failure_streak >= PROBE_FAILURE_LOCK_THRESHOLD
        && scope
            .last_probe_error_at
            .is_some_and(|value| now.saturating_sub(value) <= PROBE_SCOPE_FAILURE_TTL_MS)
}

fn prune_scope_cache(profile: &mut HostProfile, now: i64) {
    profile.probe_scopes.retain(|_, scope| {
        let freshest = scope
            .last_probe_at
            .unwrap_or(i64::MIN)
            .max(scope.last_probe_error_at.unwrap_or(i64::MIN));
        let retention_ttl = scope_capability_ttl_ms(scope, now).max(PROBE_SCOPE_FAILURE_TTL_MS);
        freshest != i64::MIN && now.saturating_sub(freshest) <= retention_ttl
    });

    if profile.probe_scopes.len() <= MAX_SCOPE_CACHE_ENTRIES {
        return;
    }

    let mut ordered = profile
        .probe_scopes
        .iter()
        .map(|(key, scope)| {
            (
                key.clone(),
                scope
                    .last_probe_at
                    .unwrap_or(i64::MIN)
                    .max(scope.last_probe_error_at.unwrap_or(i64::MIN)),
            )
        })
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(_, timestamp)| *timestamp);

    let remove_count = profile
        .probe_scopes
        .len()
        .saturating_sub(MAX_SCOPE_CACHE_ENTRIES);
    for (key, _) in ordered.into_iter().take(remove_count) {
        profile.probe_scopes.remove(&key);
    }
}

fn scope_capability_ttl_ms(scope: &ProbeScopeCache, now: i64) -> i64 {
    if scope_recent_instability(scope, now) {
        PROBE_CAPABILITY_UNSTABLE_TTL_MS
    } else if scope_is_stable(scope, now) {
        PROBE_CAPABILITY_STABLE_TTL_MS
    } else {
        PROBE_CAPABILITY_TTL_MS
    }
}

fn scope_recent_instability(scope: &ProbeScopeCache, now: i64) -> bool {
    if scope.cooldown_until.is_some_and(|until| until > now) {
        return true;
    }

    if scope.probe_failure_streak > 0
        && scope
            .last_probe_error_at
            .is_some_and(|value| now.saturating_sub(value) <= PROBE_SCOPE_FAILURE_TTL_MS)
    {
        return true;
    }

    scope
        .last_instability_at
        .is_some_and(|value| now.saturating_sub(value) <= PROBE_SCOPE_STABILITY_WINDOW_MS)
}

fn scope_is_stable(scope: &ProbeScopeCache, now: i64) -> bool {
    !scope_recent_instability(scope, now)
        && scope.telemetry_samples >= PROBE_SCOPE_STABLE_TELEMETRY_SAMPLES
        && scope.last_probe_at.is_some()
        && scope
            .last_telemetry_at
            .is_some_and(|value| now.saturating_sub(value) <= PROBE_CAPABILITY_STABLE_TTL_MS)
        && (scope.range_supported.is_some() || scope.content_length_hint.is_some())
}

fn prune_recent_probe_cache(registry: &mut RegistrySnapshot, now: i64) {
    registry
        .recent_probes
        .retain(|_, probe| now.saturating_sub(probe.captured_at) <= PROBE_RESULT_REUSE_TTL_MS);
}

fn trim_recent_probe_cache(registry: &mut RegistrySnapshot) {
    if registry.recent_probes.len() <= MAX_RECENT_PROBE_ENTRIES {
        return;
    }

    let mut ordered = registry
        .recent_probes
        .iter()
        .map(|(key, probe)| (key.clone(), probe.captured_at))
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(_, captured_at)| *captured_at);

    let remove_count = registry
        .recent_probes
        .len()
        .saturating_sub(MAX_RECENT_PROBE_ENTRIES);
    for (key, _) in ordered.into_iter().take(remove_count) {
        registry.recent_probes.remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        fresh_probe_capabilities, probe_cache_stale, PROBE_CAPABILITY_TTL_MS,
        PROBE_SCOPE_STABLE_TELEMETRY_SAMPLES, PROBE_CAPABILITY_UNSTABLE_TTL_MS,
    };
    use crate::model::{HostProfile, ProbeScopeCache};

    #[test]
    fn stable_scope_capabilities_outlive_the_default_ttl() {
        let now = 1_000_000_000;
        let mut profile = HostProfile::default();
        profile.probe_scopes.insert(
            "get|https://example.com/file".to_string(),
            ProbeScopeCache {
                range_supported: Some(true),
                resumable: Some(true),
                content_length_hint: Some(1024),
                last_probe_at: Some(now - (PROBE_CAPABILITY_TTL_MS + 5 * 60 * 1_000)),
                telemetry_samples: PROBE_SCOPE_STABLE_TELEMETRY_SAMPLES,
                last_telemetry_at: Some(now - 60_000),
                ..ProbeScopeCache::default()
            },
        );

        assert!(fresh_probe_capabilities(&profile, "get|https://example.com/file", now).is_some());
        assert!(!probe_cache_stale(&profile, "get|https://example.com/file", now));
    }

    #[test]
    fn unstable_scope_capabilities_expire_early() {
        let now = 2_000_000_000;
        let mut profile = HostProfile::default();
        profile.probe_scopes.insert(
            "post|https://example.com/wrapper".to_string(),
            ProbeScopeCache {
                range_supported: Some(true),
                resumable: Some(true),
                content_length_hint: Some(4096),
                last_probe_at: Some(now - (PROBE_CAPABILITY_UNSTABLE_TTL_MS + 1_000)),
                last_instability_at: Some(now - 1_000),
                ..ProbeScopeCache::default()
            },
        );

        assert!(fresh_probe_capabilities(&profile, "post|https://example.com/wrapper", now).is_none());
        assert!(probe_cache_stale(&profile, "post|https://example.com/wrapper", now));
    }
}
