use std::path::Path;

use tauri::AppHandle;

use super::probe::DownloadProbeData;
use super::*;
use crate::model::{RecentProbeCacheEntry, ResumeValidators};

const LOW_SPACE_UNKNOWN_SIZE_WARNING_BYTES: u64 = 512 * 1024 * 1024;
const PROBE_SOURCE_WARNING_LIVE: &str = "Probe metadata source: live network probe.";
const PROBE_SOURCE_WARNING_CACHED: &str = "Probe metadata source: recent probe cache reuse.";
const PROBE_SOURCE_WARNING_FALLBACK: &str =
    "Probe metadata source: planning fallback without fresh metadata.";

fn normalize_scheduled_for(scheduled_for: Option<i64>) -> Option<i64> {
    let now = unix_epoch_millis();
    scheduled_for.filter(|value| *value > now)
}

fn target_path_matches(left: &str, right: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        left.eq_ignore_ascii_case(right)
    }

    #[cfg(not(target_os = "windows"))]
    {
        left == right
    }
}

fn urls_overlap(existing: &DownloadRecord, requested_url: &str, final_url: &str) -> bool {
    let existing_urls = [existing.url.as_str(), existing.final_url.as_str()];
    existing_urls.iter().any(|candidate| {
        super::probe_html::urls_match_after_normalization(candidate, requested_url)
            || super::probe_html::urls_match_after_normalization(candidate, final_url)
    })
}

fn etag_matches(left: &str, right: &str) -> bool {
    left.trim() == right.trim()
}

fn last_modified_matches(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn validators_match(existing: &ResumeValidators, requested: &ResumeValidators) -> bool {
    let (Some(existing_length), Some(requested_length)) =
        (existing.content_length, requested.content_length)
    else {
        return false;
    };

    if existing_length != requested_length {
        return false;
    }

    matches!((&existing.etag, &requested.etag), (Some(left), Some(right)) if etag_matches(left, right))
        || matches!(
            (&existing.last_modified, &requested.last_modified),
            (Some(left), Some(right)) if last_modified_matches(left, right)
        )
}

fn duplicate_download_message(existing: &DownloadRecord, reason: &str) -> String {
    format!(
        "Download already exists: '{}' is already {}. Existing status: {:?}.",
        existing.name, reason, existing.status
    )
}

fn find_duplicate_download<'a>(
    downloads: &'a [DownloadRecord],
    requested_url: &str,
    final_url: &str,
    requested_validators: Option<&ResumeValidators>,
    target_path: &str,
) -> Option<(&'a DownloadRecord, &'static str)> {
    for existing in downloads {
        if urls_overlap(existing, requested_url, final_url) {
            return Some((existing, "tracking the same source URL"));
        }

        if requested_validators
            .is_some_and(|requested| validators_match(&existing.validators, requested))
        {
            return Some((existing, "matching the same remote file validators"));
        }

        if target_path_matches(&existing.target_path, target_path) {
            return Some((existing, "using the same target file"));
        }
    }

    None
}

struct ProbePlanningState {
    now: i64,
    scope_key: String,
    request_form_fields: Vec<DownloadRequestField>,
    cached_recent_probe: Option<RecentProbeCacheEntry>,
    live_probe: Option<DownloadProbeData>,
    probe: Option<DownloadProbeData>,
    final_url: String,
    host: String,
}

fn append_storage_warning(
    warnings: &mut Vec<String>,
    available_space: Option<u64>,
    required_size: Option<u64>,
) {
    let Some(available_space) = available_space else {
        return;
    };

    if let Some(required_size) = required_size {
        if required_size > available_space {
            warnings.push(format!(
                "Selected folder has {} free but the remote file reports {}.",
                format_bytes_compact(available_space),
                format_bytes_compact(required_size)
            ));
        }
        return;
    }

    if available_space <= LOW_SPACE_UNKNOWN_SIZE_WARNING_BYTES {
        warnings.push(format!(
            "Selected folder currently has {} free. Unknown-size downloads will stream with live disk-space checks and may stop early if the host keeps sending data.",
            format_bytes_compact(available_space)
        ));
    }
}

fn append_probe_source_warning(
    warnings: &mut Vec<String>,
    has_cached_recent_probe: bool,
    has_live_probe: bool,
) {
    if has_cached_recent_probe {
        warnings.push(PROBE_SOURCE_WARNING_CACHED.to_string());
    } else if has_live_probe {
        warnings.push(PROBE_SOURCE_WARNING_LIVE.to_string());
    } else {
        warnings.push(PROBE_SOURCE_WARNING_FALLBACK.to_string());
    }
}

fn merge_probe_compatibility(
    probe_compatibility: Option<DownloadCompatibility>,
    request_referer: Option<String>,
    request_cookies: Option<String>,
    request_method: DownloadRequestMethod,
    request_form_fields: &[DownloadRequestField],
) -> DownloadCompatibility {
    let mut compatibility = probe_compatibility.unwrap_or_default();
    if compatibility.request_referer.is_none() {
        compatibility.request_referer = request_referer;
    }
    if compatibility.request_cookies.is_none() {
        compatibility.request_cookies = request_cookies;
    }
    if compatibility.request_method == DownloadRequestMethod::Get
        && compatibility.request_form_fields.is_empty()
    {
        compatibility.request_method = request_method;
        compatibility.request_form_fields = request_form_fields.to_vec();
    }
    compatibility
}

fn apply_probe_learning(
    registry: &mut RegistrySnapshot,
    host: &str,
    scope_key: &str,
    live_probe: Option<&DownloadProbeData>,
    probe: Option<&DownloadProbeData>,
    now: i64,
) {
    if let Some(probe_data) = live_probe {
        let profile = registry.host_profiles.entry(host.to_string()).or_default();
        update_profile_probe_cache(profile, scope_key, probe_data, now);
        store_recent_probe(registry, scope_key, host, probe_data, now);
    } else if probe.is_none() {
        let profile = registry.host_profiles.entry(host.to_string()).or_default();
        record_probe_failure(profile, scope_key, now);
    }
}

impl EngineState {
    async fn resolve_probe_planning_state(
        &self,
        url: &str,
        request_referer: Option<&str>,
        request_cookies: Option<&str>,
        request_method: &DownloadRequestMethod,
        request_form_fields: Vec<DownloadRequestField>,
    ) -> Result<ProbePlanningState, String> {
        let now = unix_epoch_millis();
        let scope_key = probe_scope_key(url, request_method, &request_form_fields);
        let cached_recent_probe = {
            let registry = self.registry_guard()?;
            fresh_recent_probe(&registry, &scope_key, now)
        };
        let live_probe = if cached_recent_probe.is_some() {
            None
        } else {
            probe_download_headers_with_context(
                url,
                request_referer,
                request_cookies,
                request_method,
                &request_form_fields,
            )
            .await
            .ok()
        };
        let probe = live_probe.clone().or_else(|| {
            cached_recent_probe
                .as_ref()
                .map(cached_probe_to_download_probe)
        });
        let final_url = probe
            .as_ref()
            .map(|value| value.final_url.clone())
            .unwrap_or_else(|| url.to_string());
        let host = cached_recent_probe
            .as_ref()
            .map(|value| value.host.clone())
            .unwrap_or_else(|| extract_host(&final_url));

        Ok(ProbePlanningState {
            now,
            scope_key,
            request_form_fields,
            cached_recent_probe,
            live_probe,
            probe,
            final_url,
            host,
        })
    }

    pub async fn get_app_state(&self) -> Result<AppStateSnapshot, String> {
        self.await_bootstrap().await;
        let registry = self.registry_guard()?;
        Ok(AppStateSnapshot {
            downloads: registry.downloads.clone(),
            settings: registry.settings.clone(),
            queue_state: QueueState {
                running: registry.queue_running,
            },
        })
    }

    pub async fn get_app_state_rows(&self) -> Result<AppStateRowSnapshot, String> {
        self.await_bootstrap().await;
        let registry = self.registry_guard()?;
        Ok(AppStateRowSnapshot {
            downloads: registry
                .downloads
                .iter()
                .map(Self::compact_download_for_row)
                .collect(),
            settings: registry.settings.clone(),
            queue_state: QueueState {
                running: registry.queue_running,
            },
        })
    }

    pub fn get_startup_snapshot(&self) -> StartupSnapshot {
        let bootstrap = self.get_bootstrap_state();
        let mut active_downloads = Vec::new();
        let mut settings = EngineSettings::default();
        let mut queue_running = true;
        let update_health = self
            .inner
            .update_health
            .lock()
            .map(|state| state.clone())
            .unwrap_or(None);
        if let Ok(registry) = self.registry_guard() {
            settings = registry.settings.clone();
            queue_running = registry.queue_running;
            active_downloads = registry
                .downloads
                .iter()
                .filter(|download| !matches!(download.status, DownloadStatus::Finished))
                .map(Self::compact_download_for_row)
                .collect();
        }
        StartupSnapshot {
            bootstrap,
            settings,
            queue_state: QueueState {
                running: queue_running,
            },
            update_health,
            active_downloads,
        }
    }

    pub async fn get_download_details(&self, id: &str) -> Result<DownloadDetailSnapshot, String> {
        self.await_bootstrap().await;
        let registry = self.registry_guard()?;
        let download = registry
            .downloads
            .iter()
            .find(|download| download.id == id)
            .ok_or_else(|| "Download not found".to_string())?;
        Ok(DownloadDetailSnapshot {
            id: download.id.clone(),
            engine_log: download.engine_log.clone(),
            runtime_checkpoint: download.runtime_checkpoint.clone(),
        })
    }

    pub fn list_downloads(&self) -> Vec<DownloadRecord> {
        self.registry_guard()
            .map(|registry| registry.downloads.clone())
            .unwrap_or_default()
    }

    pub fn get_settings(&self) -> EngineSettings {
        self.registry_guard()
            .map(|registry| registry.settings.clone())
            .unwrap_or_default()
    }

    pub fn get_queue_state(&self) -> QueueState {
        QueueState {
            running: self
                .registry_guard()
                .map(|registry| registry.queue_running)
                .unwrap_or(true),
        }
    }

    pub async fn probe_download(&self, args: ProbeDownloadArgs) -> Result<ProbeResult, String> {
        self.await_bootstrap().await;
        let ProbeDownloadArgs {
            url,
            save_path,
            name,
            request_referer,
            request_cookies,
            request_method,
            request_form_fields,
        } = args;
        let normalized_url =
            non_empty(url).ok_or_else(|| "Download URL cannot be empty.".to_string())?;
        let requested_name = name.and_then(non_empty);
        let planning = self
            .resolve_probe_planning_state(
                &normalized_url,
                request_referer.as_deref(),
                request_cookies.as_deref(),
                &request_method,
                super::http_helpers::sanitize_request_fields(request_form_fields),
            )
            .await?;
        let ProbePlanningState {
            now,
            scope_key,
            request_form_fields,
            cached_recent_probe,
            live_probe,
            probe,
            final_url,
            host,
        } = planning;
        let detected_name = probe.as_ref().map(|value| value.suggested_name.as_str());
        let suggested_name = requested_name
            .as_deref()
            .map(|manual| apply_detected_extension(manual, detected_name))
            .unwrap_or_else(|| {
                probe
                    .as_ref()
                    .map(|value| value.suggested_name.clone())
                    .unwrap_or_else(|| suggested_name_from_url(&final_url))
            });
        let target_path = save_path.as_deref().and_then(|directory| {
            let trimmed = directory.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(join_target_path(trimmed, &suggested_name))
            }
        });
        let mut registry = self.registry_guard()?;
        let host_profile_was_stale = live_probe.is_some()
            && registry
                .host_profiles
                .get(&host)
                .is_some_and(|profile| probe_cache_stale(profile, &scope_key, now));
        apply_probe_learning(
            &mut registry,
            &host,
            &scope_key,
            live_probe.as_ref(),
            probe.as_ref(),
            now,
        );
        let host_profile = registry.host_profiles.get(&host);
        let cached_probe =
            host_profile.and_then(|profile| fresh_probe_capabilities(profile, &scope_key, now));
        let effective_connections = effective_connection_target_for_scope(
            16,
            &registry.settings,
            host_profile,
            Some(&scope_key),
        );
        let mut warnings = probe
            .as_ref()
            .map_or_else(Vec::new, |value| value.warnings.clone());
        append_probe_source_warning(
            &mut warnings,
            cached_recent_probe.is_some(),
            live_probe.is_some(),
        );
        if probe.is_none() {
            if cached_probe.is_some() {
                warnings.push(
                    "Remote probe failed; using fresh host capability cache for planning."
                        .to_string(),
                );
            } else {
                warnings.push(
                    "Remote probe failed; no fresh host capability cache is available.".to_string(),
                );
            }
        } else if host_profile_was_stale {
            warnings.push(
                "Host capability cache was stale before this probe and has now been refreshed."
                    .to_string(),
            );
        }
        append_probe_cache_warning(
            &mut warnings,
            host_profile,
            &scope_key,
            cached_probe.as_ref(),
            now,
        );
        if let Some(warning) = profile_warning_for_scope(host_profile, Some(&scope_key)) {
            warnings.push(warning);
        }
        let available_space = save_path
            .as_deref()
            .map(Path::new)
            .and_then(query_available_space);
        let size = probe.as_ref().and_then(|value| value.size).or_else(|| {
            cached_probe
                .as_ref()
                .and_then(|cached| cached.content_length_hint)
        });
        append_storage_warning(&mut warnings, available_space, size);
        let mime_type = probe.as_ref().and_then(|value| value.mime_type.clone());
        let range_supported = probe
            .as_ref()
            .map(|value| value.range_supported)
            .or_else(|| {
                cached_probe
                    .as_ref()
                    .and_then(|cached| cached.range_supported)
            })
            .unwrap_or(false);
        let resumable = probe
            .as_ref()
            .map(|value| value.resumable)
            .or_else(|| cached_probe.as_ref().and_then(|cached| cached.resumable))
            .unwrap_or(false);
        let compatibility = merge_probe_compatibility(
            probe.as_ref().map(|value| value.compatibility.clone()),
            request_referer.clone(),
            request_cookies.clone(),
            request_method.clone(),
            &request_form_fields,
        );
        let exact_request_shape_allows_segmentation =
            super::http_helpers::request_context_supports_segmented_transfer(
                &compatibility.request_method,
                &compatibility.request_form_fields,
            );
        let range_supported = range_supported && exact_request_shape_allows_segmentation;
        let resumable = resumable && exact_request_shape_allows_segmentation;
        let planned_connections = if range_supported && resumable && size.unwrap_or(0) > 0 {
            initial_target_connections_for_scope(
                effective_connections,
                &registry.settings,
                host_profile,
                Some(&scope_key),
                size,
            )
        } else {
            1
        };
        let segmented = range_supported && planned_connections > 1;
        if !exact_request_shape_allows_segmentation
            && !warnings
                .iter()
                .any(|warning| warning.contains("guarded single-stream"))
        {
            warnings.push(
                "This download requires an exact request replay; VDM will keep it on guarded single-stream mode until byte-range support is proven on that same request shape."
                    .to_string(),
            );
        }
        self.persist_registry(&registry)?;
        Ok(ProbeResult {
            original_url: normalized_url.clone(),
            final_url,
            host,
            host_max_connections: host_profile.and_then(|profile| profile.max_connections),
            host_average_ttfb_ms: effective_average_ttfb_ms(host_profile, Some(&scope_key)),
            host_average_throughput_bytes_per_second: effective_average_throughput_bytes_per_second(
                host_profile,
                Some(&scope_key),
            ),
            host_diagnostics: host_diagnostics_summary_for_scope(host_profile, Some(&scope_key)),
            suggested_name: suggested_name.clone(),
            target_path,
            size,
            mime_type,
            available_space,
            resumable,
            range_supported,
            segmented,
            planned_connections,
            suggested_category: classify_category(&suggested_name),
            warnings,
            validators: probe
                .as_ref()
                .map(|value| value.validators.clone())
                .or_else(|| {
                    cached_recent_probe
                        .as_ref()
                        .map(|cached| cached.validators.clone())
                })
                .unwrap_or_default(),
            compatibility,
        })
    }

    pub async fn add_download(
        &self,
        _app: &AppHandle,
        args: AddDownloadArgs,
    ) -> Result<DownloadRecord, String> {
        self.await_bootstrap().await;
        let url = non_empty(args.url).ok_or_else(|| "Download URL cannot be empty.".to_string())?;
        let save_path =
            non_empty(args.save_path).ok_or_else(|| "Save path cannot be empty.".to_string())?;
        let planning = self
            .resolve_probe_planning_state(
                &url,
                args.request_referer.as_deref(),
                args.request_cookies.as_deref(),
                &args.request_method,
                super::http_helpers::sanitize_request_fields(args.request_form_fields.clone()),
            )
            .await?;
        let ProbePlanningState {
            now,
            scope_key,
            request_form_fields,
            cached_recent_probe,
            live_probe,
            probe,
            final_url,
            host,
        } = planning;
        let detected_name = probe.as_ref().map(|value| value.suggested_name.as_str());
        let name = args
            .name
            .and_then(non_empty)
            .map(|manual| apply_detected_extension(&manual, detected_name))
            .unwrap_or_else(|| {
                probe
                    .as_ref()
                    .map(|value| value.suggested_name.clone())
                    .unwrap_or_else(|| suggested_name_from_url(&final_url))
            });

        let mut registry = self.registry_guard()?;
        let id = format!("download-{}", registry.next_id);
        registry.next_id = registry.next_id.saturating_add(1);
        apply_probe_learning(
            &mut registry,
            &host,
            &scope_key,
            live_probe.as_ref(),
            probe.as_ref(),
            now,
        );

        let scheduled_for = normalize_scheduled_for(args.scheduled_for);
        let settings = registry.settings.clone();
        let host_profile = registry.host_profiles.get(&host);
        let cached_probe =
            host_profile.and_then(|profile| fresh_probe_capabilities(profile, &scope_key, now));
        let max_connections =
            effective_connection_target_for_scope(16, &settings, host_profile, Some(&scope_key));
        let target_path = join_target_path(&save_path, &name);
        if let Some((existing, reason)) = find_duplicate_download(
            &registry.downloads,
            &url,
            &final_url,
            probe.as_ref().map(|value| &value.validators),
            &target_path,
        ) {
            return Err(duplicate_download_message(existing, reason));
        }
        let available_space = query_available_space(Path::new(&save_path));
        let cached_range = cached_probe
            .as_ref()
            .and_then(|cached| cached.range_supported);
        let cached_resumable = cached_probe.as_ref().and_then(|cached| cached.resumable);
        let probed_range = probe.as_ref().map(|value| value.range_supported);
        let probed_resumable = probe.as_ref().map(|value| value.resumable);
        let mut capabilities = DownloadCapabilities {
            resumable: probed_resumable
                .or(args.resumable_hint)
                .or(cached_resumable)
                .unwrap_or(false),
            range_supported: probed_range
                .or(args.range_supported_hint)
                .or(cached_range)
                .unwrap_or(false),
            segmented: false,
        };
        let planned_size = probe
            .as_ref()
            .and_then(|value| value.size)
            .or(args.size_hint_bytes)
            .or_else(|| {
                cached_probe
                    .as_ref()
                    .and_then(|cached| cached.content_length_hint)
            });
        let mut warnings = probe
            .as_ref()
            .map_or_else(Vec::new, |value| value.warnings.clone());
        append_probe_source_warning(
            &mut warnings,
            cached_recent_probe.is_some(),
            live_probe.is_some(),
        );
        if let Some(warning) = profile_warning_for_scope(host_profile, Some(&scope_key)) {
            warnings.push(warning);
        }
        append_probe_cache_warning(
            &mut warnings,
            host_profile,
            &scope_key,
            cached_probe.as_ref(),
            now,
        );
        if probe.is_none() {
            if cached_probe.is_some() {
                warnings.push(
                    "Network probe unavailable; using fresh host capability cache.".to_string(),
                );
            } else {
                warnings
                    .push("Network probe unavailable; planning with local hints only.".to_string());
            }
        }
        append_storage_warning(&mut warnings, available_space, planned_size);
        let compatibility = merge_probe_compatibility(
            probe.as_ref().map(|value| value.compatibility.clone()),
            args.request_referer.clone(),
            args.request_cookies.clone(),
            args.request_method.clone(),
            &request_form_fields,
        );
        let exact_request_shape_allows_segmentation =
            super::http_helpers::request_context_supports_segmented_transfer(
                &compatibility.request_method,
                &compatibility.request_form_fields,
            );
        if !exact_request_shape_allows_segmentation {
            capabilities.range_supported = false;
            capabilities.resumable = false;
            capabilities.segmented = false;
            warnings.push(
                "This download requires an exact request replay; VDM will keep it on guarded single-stream mode until byte-range support is proven on that same request shape."
                    .to_string(),
            );
        }
        let preferred_connections = if capabilities.range_supported
            && capabilities.resumable
            && planned_size.unwrap_or(0) > 0
        {
            initial_target_connections_for_scope(
                max_connections,
                &settings,
                host_profile,
                Some(&scope_key),
                planned_size,
            )
        } else {
            1
        };
        let segmented_mode = capabilities.range_supported
            && capabilities.resumable
            && planned_size.unwrap_or(0) > 0
            && preferred_connections > 1;
        let starting_connections = if segmented_mode {
            preferred_connections
        } else {
            1
        };
        capabilities.segmented = segmented_mode;
        let mut segments = if segmented_mode {
            build_segment_plan(
                planned_size.unwrap_or(0),
                starting_connections,
                &settings,
                effective_average_throughput_bytes_per_second(host_profile, Some(&scope_key)),
                effective_average_ttfb_ms(host_profile, Some(&scope_key)),
            )
        } else {
            Vec::new()
        };
        if segments.is_empty() && segmented_mode {
            capabilities.segmented = false;
        }
        let mut record = DownloadRecord {
            id,
            name,
            url: url.clone(),
            final_url,
            host,
            size: planned_size
                .map(|value| i64::try_from(value).unwrap_or(i64::MAX))
                .unwrap_or(-1),
            downloaded: 0,
            status: if args.start_immediately || scheduled_for.is_some() {
                DownloadStatus::Queued
            } else {
                DownloadStatus::Stopped
            },
            manual_start_requested: args.start_immediately && scheduled_for.is_none(),
            category: args.category,
            speed: 0,
            time_left: None,
            date_added: unix_epoch_millis(),
            save_path: save_path.clone(),
            temp_path: format!("{target_path}.part"),
            target_path,
            queue: DEFAULT_QUEUE.to_string(),
            scheduled_for,
            queue_position: next_queue_position(&registry.downloads),
            max_connections,
            host_max_connections: host_profile.and_then(|profile| profile.max_connections),
            host_cooldown_until: host_profile.and_then(|profile| profile.cooldown_until),
            host_average_ttfb_ms: effective_average_ttfb_ms(host_profile, Some(&scope_key)),
            host_average_throughput_bytes_per_second: effective_average_throughput_bytes_per_second(
                host_profile,
                Some(&scope_key),
            ),
            host_protocol: probe
                .as_ref()
                .and_then(|value| value.negotiated_protocol.clone())
                .or_else(|| host_profile.and_then(|profile| profile.negotiated_protocol.clone())),
            host_diagnostics: host_diagnostics_summary_for_scope(host_profile, Some(&scope_key)),
            traffic_mode: settings.traffic_mode.clone(),
            speed_limit_bytes_per_second: None,
            open_folder_on_completion: false,
            error_message: None,
            content_type: probe.as_ref().and_then(|value| value.mime_type.clone()),
            capabilities,
            validators: probe
                .as_ref()
                .map(|value| value.validators.clone())
                .unwrap_or_default(),
            compatibility,
            diagnostics: DownloadDiagnostics {
                warnings,
                notes: vec!["Runtime worker orchestration enabled.".to_string()],
                failure_kind: None,
                restart_required: false,
                terminal_reason: None,
                checkpoint_flushes: 0,
                checkpoint_skips: 0,
                checkpoint_avg_flush_ms: 0,
                checkpoint_last_flush_ms: 0,
                checkpoint_disk_pressure_events: 0,
                contiguous_fsync_flushes: 0,
                contiguous_fsync_window_bytes: 0,
            },
            segments: std::mem::take(&mut segments),
            target_connections: starting_connections,
            writer_backpressure: false,
            engine_log: Vec::new(),
            runtime_checkpoint: DownloadRuntimeCheckpoint::default(),
        };

        append_download_log(
            &mut record,
            DownloadLogLevel::Info,
            if cached_recent_probe.is_some() {
                "probe.cache-hit"
            } else if probe.is_some() {
                "probe.resolved"
            } else {
                "probe.unavailable"
            },
            if cached_recent_probe.is_some() {
                "Reused a fresh probe result while creating the download."
            } else if probe.is_some() {
                "Collected live metadata before creating the download."
            } else {
                "Created the download without live probe metadata; runtime will recover details later."
            },
        );
        if record.capabilities.segmented {
            let target_connections = record.target_connections;
            let segment_count = record.segments.len();
            let plan_message = format!(
                "Planned {} initial connections across {} segments.",
                target_connections, segment_count
            );
            append_download_log(
                &mut record,
                DownloadLogLevel::Info,
                "transfer.segment-plan",
                plan_message,
            );
        } else {
            append_download_log(
                &mut record,
                DownloadLogLevel::Info,
                "transfer.single-stream",
                "Planned guarded single-stream startup for this request shape.",
            );
        }
        if record.scheduled_for.is_some() {
            append_download_log(
                &mut record,
                DownloadLogLevel::Info,
                "queue.scheduled",
                "Scheduled this download to start automatically at the selected time.",
            );
        }

        apply_download_host_profile(&mut record, &settings, host_profile);

        registry.downloads.push(record.clone());
        normalize_queue_positions(&mut registry.downloads);
        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let response = registry
            .downloads
            .iter()
            .find(|download| download.id == record.id)
            .cloned()
            .unwrap_or(record.clone());
        let min_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        self.persist_registry_flush(&registry)?;
        drop(registry);
        self.emit_download_progress_diff_if_due(&response, min_interval_ms);
        self.emit_download_upsert(&response);
        self.apply_runtime_dispatch_plan(dispatch_plan, min_interval_ms);
        Ok(response)
    }

    pub async fn pause_download(&self, _app: &AppHandle, id: &str) -> Result<(), String> {
        self.await_bootstrap().await;
        self.abort_runtime_task(id);
        let mut registry = self.registry_guard()?;
        let Some(download) = registry
            .downloads
            .iter_mut()
            .find(|download| download.id == id)
        else {
            return Err(format!("No download found for id '{id}'."));
        };

        if matches!(
            download.status,
            DownloadStatus::Queued | DownloadStatus::Downloading
        ) {
            download.status = DownloadStatus::Paused;
        }
        download.manual_start_requested = false;
        reset_download_transient_state(download);
        clear_runtime_checkpoint(download);
        let response = download.clone();
        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let min_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        self.persist_registry_flush(&registry)?;
        drop(registry);
        self.emit_download_progress_diff_if_due(&response, min_interval_ms);
        self.emit_download_upsert(&response);
        self.apply_runtime_dispatch_plan(dispatch_plan, min_interval_ms);
        Ok(())
    }

    pub async fn resume_download(&self, _app: &AppHandle, id: &str) -> Result<(), String> {
        self.await_bootstrap().await;
        let mut registry = self.registry_guard()?;
        let queue_running = registry.queue_running;
        let settings = registry.settings.clone();
        let Some(download) = registry
            .downloads
            .iter_mut()
            .find(|download| download.id == id)
        else {
            return Err(format!("No download found for id '{id}'."));
        };
        if matches!(download.status, DownloadStatus::Finished) {
            return Err("Finished downloads cannot be resumed.".to_string());
        }
        download.status = DownloadStatus::Queued;
        download.scheduled_for = None;
        download.manual_start_requested = !queue_running;
        clear_download_terminal_state(download);
        reset_download_transient_state(download);
        ensure_segment_plan(download, &settings);
        let response = download.clone();
        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let min_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        self.persist_registry(&registry)?;
        self.emit_download_progress_diff_if_due(&response, min_interval_ms);
        self.emit_download_upsert(&response);
        drop(registry);
        self.apply_runtime_dispatch_plan(dispatch_plan, min_interval_ms);
        Ok(())
    }

    pub async fn restart_download(&self, _app: &AppHandle, id: &str) -> Result<(), String> {
        self.await_bootstrap().await;
        self.abort_runtime_task(id);
        let mut registry = self.registry_guard()?;
        let queue_running = registry.queue_running;
        let settings = registry.settings.clone();
        let Some(download) = registry
            .downloads
            .iter_mut()
            .find(|download| download.id == id)
        else {
            return Err(format!("No download found for id '{id}'."));
        };
        let temp_path = download.temp_path.clone();

        download.downloaded = 0;
        reset_download_transient_state(download);
        clear_download_terminal_state(download);
        download.status = DownloadStatus::Queued;
        download.scheduled_for = None;
        download.manual_start_requested = !queue_running;
        reset_download_progress(download);
        clear_runtime_checkpoint(download);
        append_download_log(
            download,
            DownloadLogLevel::Info,
            "transfer.restart",
            "Restarted the download from byte 0 and cleared previous completion state.",
        );
        ensure_segment_plan(download, &settings);
        let response = download.clone();
        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let min_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        self.persist_registry(&registry)?;
        self.emit_download_progress_diff_if_due(&response, min_interval_ms);
        self.emit_download_upsert(&response);
        drop(registry);
        reset_temp_file_path(&temp_path)?;
        self.apply_runtime_dispatch_plan(dispatch_plan, min_interval_ms);
        Ok(())
    }

    pub async fn remove_download(
        &self,
        _app: &AppHandle,
        id: &str,
        delete_file: bool,
    ) -> Result<(), String> {
        self.await_bootstrap().await;
        self.abort_runtime_task(id);
        let mut registry = self.registry_guard()?;

        if let Some(download) = registry.downloads.iter().find(|d| d.id == id) {
            if delete_file || download.status != DownloadStatus::Finished {
                let _ = std::fs::remove_file(&download.temp_path);
            }
            if delete_file {
                let _ = std::fs::remove_file(&download.target_path);
            }
        }

        let starting_len = registry.downloads.len();
        registry.downloads.retain(|download| download.id != id);
        if registry.downloads.len() == starting_len {
            return Err(format!("No download found for id '{id}'."));
        }

        normalize_queue_positions(&mut registry.downloads);
        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let min_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        self.persist_registry_flush(&registry)?;
        drop(registry);
        self.clear_download_rate_limiter(id);
        self.emit_download_removed(id);
        self.apply_runtime_dispatch_plan(dispatch_plan, min_interval_ms);
        Ok(())
    }

    pub async fn remove_downloads(
        &self,
        _app: &AppHandle,
        ids: &[String],
        delete_file: bool,
    ) -> Result<Vec<String>, String> {
        use std::collections::HashSet;

        self.await_bootstrap().await;

        if ids.is_empty() {
            return Ok(Vec::new());
        }

        for id in ids {
            self.abort_runtime_task(id);
        }

        let requested_ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let mut registry = self.registry_guard()?;

        for download in registry
            .downloads
            .iter()
            .filter(|download| requested_ids.contains(download.id.as_str()))
        {
            if delete_file || download.status != DownloadStatus::Finished {
                let _ = std::fs::remove_file(&download.temp_path);
            }
            if delete_file {
                let _ = std::fs::remove_file(&download.target_path);
            }
        }

        let mut removed_ids = Vec::new();
        registry.downloads.retain(|download| {
            let keep = !requested_ids.contains(download.id.as_str());
            if !keep {
                removed_ids.push(download.id.clone());
            }
            keep
        });

        if removed_ids.is_empty() {
            return Err("No downloads found for the requested ids.".to_string());
        }

        normalize_queue_positions(&mut registry.downloads);
        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let min_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        self.persist_registry_flush(&registry)?;
        drop(registry);

        for id in &removed_ids {
            self.clear_download_rate_limiter(id);
            self.emit_download_removed(id);
        }

        self.apply_runtime_dispatch_plan(dispatch_plan, min_interval_ms);
        Ok(removed_ids)
    }

    pub async fn reorder_download(
        &self,
        _app: &AppHandle,
        id: &str,
        direction: ReorderDirection,
    ) -> Result<DownloadRecord, String> {
        self.await_bootstrap().await;
        let mut registry = self.registry_guard()?;
        let Some(target_index) = registry
            .downloads
            .iter()
            .position(|download| download.id == id)
        else {
            return Err(format!("No download found for id '{id}'."));
        };

        let mut ordered_indices: Vec<usize> = (0..registry.downloads.len()).collect();
        ordered_indices.sort_by_key(|&index| registry.downloads[index].queue_position);
        let Some(position) = ordered_indices
            .iter()
            .position(|&index| index == target_index)
        else {
            return Err(format!("No download found for id '{id}'."));
        };

        let target_position = match direction {
            ReorderDirection::Up if position > 0 => position - 1,
            ReorderDirection::Down if position + 1 < ordered_indices.len() => position + 1,
            ReorderDirection::Top if position > 0 => 0,
            ReorderDirection::Bottom if position + 1 < ordered_indices.len() => {
                ordered_indices.len().saturating_sub(1)
            }
            _ => position,
        };

        if target_position != position {
            let moved_index = ordered_indices.remove(position);
            ordered_indices.insert(target_position, moved_index);
            for (normalized_index, download_index) in ordered_indices.iter().enumerate() {
                registry.downloads[*download_index].queue_position =
                    u32::try_from(normalized_index).unwrap_or(u32::MAX).saturating_add(1);
            }
            normalize_queue_positions(&mut registry.downloads);
        }

        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let response = registry.downloads[target_index].clone();
        let min_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        self.persist_registry(&registry)?;
        drop(registry);
        self.emit_download_upsert(&response);
        self.emit_download_progress_diff_if_due(&response, min_interval_ms);
        self.apply_runtime_dispatch_plan(dispatch_plan, min_interval_ms);
        Ok(response)
    }

    pub async fn start_queue(&self, _app: &AppHandle) -> Result<QueueState, String> {
        self.await_bootstrap().await;
        let mut registry = self.registry_guard()?;
        let settings = registry.settings.clone();
        registry.queue_running = true;
        for download in &mut registry.downloads {
            if matches!(download.status, DownloadStatus::Stopped) {
                download.status = DownloadStatus::Queued;
            }
            download.manual_start_requested = false;
            reset_download_transient_state(download);
            ensure_segment_plan(download, &settings);
        }

        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let min_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        self.persist_registry(&registry)?;
        drop(registry);
        self.apply_runtime_dispatch_plan(dispatch_plan, min_interval_ms);
        Ok(QueueState { running: true })
    }

    pub async fn stop_queue(&self, _app: &AppHandle) -> Result<QueueState, String> {
        self.await_bootstrap().await;
        self.abort_all_runtime_tasks();
        let mut registry = self.registry_guard()?;
        registry.queue_running = false;
        for download in &mut registry.downloads {
            if matches!(
                download.status,
                DownloadStatus::Queued | DownloadStatus::Downloading
            ) {
                download.status = DownloadStatus::Stopped;
            }
            download.manual_start_requested = false;
            reset_download_transient_state(download);
            clear_runtime_checkpoint(download);
        }

        self.persist_registry_flush(&registry)?;
        Ok(QueueState { running: false })
    }

    pub async fn set_download_schedule(
        &self,
        _app: &AppHandle,
        id: &str,
        scheduled_for: Option<i64>,
    ) -> Result<DownloadRecord, String> {
        self.await_bootstrap().await;
        let normalized_schedule = normalize_scheduled_for(scheduled_for);
        if normalized_schedule.is_some() {
            self.abort_runtime_task(id);
        }

        let mut registry = self.registry_guard()?;
        let settings = registry.settings.clone();
        let Some(download) = registry
            .downloads
            .iter_mut()
            .find(|download| download.id == id)
        else {
            return Err(format!("No download found for id '{id}'."));
        };

        if matches!(download.status, DownloadStatus::Finished) && normalized_schedule.is_some() {
            return Err("Finished downloads cannot be scheduled again.".to_string());
        }

        download.scheduled_for = normalized_schedule;
        if download.scheduled_for.is_some() {
            if !matches!(download.status, DownloadStatus::Finished) {
                download.status = DownloadStatus::Queued;
                download.manual_start_requested = false;
                clear_download_terminal_state(download);
                reset_download_transient_state(download);
                clear_runtime_checkpoint(download);
                ensure_segment_plan(download, &settings);
            }
            append_download_log(
                download,
                DownloadLogLevel::Info,
                "queue.scheduled",
                "Scheduled this download to start automatically at the selected time.",
            );
        } else {
            append_download_log(
                download,
                DownloadLogLevel::Info,
                "queue.schedule-cleared",
                "Cleared the scheduled start time for this download.",
            );
        }

        let response = download.clone();
        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let min_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        self.persist_registry(&registry)?;
        drop(registry);
        self.emit_download_upsert(&response);
        self.emit_download_progress_diff_if_due(&response, min_interval_ms);
        self.apply_runtime_dispatch_plan(dispatch_plan, min_interval_ms);
        Ok(response)
    }

    pub async fn set_download_transfer_options(
        &self,
        _app: &AppHandle,
        id: &str,
        speed_limit_bytes_per_second: Option<u64>,
    ) -> Result<DownloadRecord, String> {
        self.await_bootstrap().await;
        let mut registry = self.registry_guard()?;
        let settings = registry.settings.clone();
        let Some(download_index) = registry
            .downloads
            .iter()
            .position(|download| download.id == id)
        else {
            return Err(format!("No download found for id '{id}'."));
        };
        let host = registry.downloads[download_index].host.clone();
        let host_profile = registry.host_profiles.get(&host).cloned();
        let download = &mut registry.downloads[download_index];

        apply_download_host_profile(download, &settings, host_profile.as_ref());
        download.speed_limit_bytes_per_second =
            speed_limit_bytes_per_second.filter(|value| *value > 0);
        let response = download.clone();

        self.persist_registry(&registry)?;
        drop(registry);
        self.reconfigure_download_rate_limiter(id, response.speed_limit_bytes_per_second);
        self.emit_download_upsert(&response);
        Ok(response)
    }

    pub async fn set_download_completion_options(
        &self,
        _app: &AppHandle,
        id: &str,
        open_folder_on_completion: bool,
    ) -> Result<DownloadRecord, String> {
        self.await_bootstrap().await;
        let mut registry = self.registry_guard()?;
        let Some(download) = registry
            .downloads
            .iter_mut()
            .find(|download| download.id == id)
        else {
            return Err(format!("No download found for id '{id}'."));
        };

        download.open_folder_on_completion = open_folder_on_completion;
        let response = download.clone();

        self.persist_registry(&registry)?;
        drop(registry);
        self.emit_download_upsert(&response);
        Ok(response)
    }

    pub async fn record_host_telemetry(
        &self,
        _app: &AppHandle,
        payload: HostTelemetryArgs,
    ) -> Result<(), String> {
        self.await_bootstrap().await;
        let host = non_empty(payload.host.clone())
            .ok_or_else(|| "Host telemetry requires a non-empty host.".to_string())?;
        let mut registry = self.registry_guard()?;
        apply_host_feedback_to_registry(&mut registry, &host, &payload);

        self.persist_registry(&registry)?;
        Ok(())
    }

    pub async fn open_download_folder(&self, id: &str) -> Result<(), String> {
        self.await_bootstrap().await;
        let registry = self.registry_guard()?;
        let Some(download) = registry.downloads.iter().find(|download| download.id == id) else {
            return Err(format!("No download found for id '{id}'."));
        };

        // For finished downloads, highlight the completed file.  For everything
        // else (active, paused, queued) open the destination folder directly so
        // the user sees where the file will land instead of an internal .vdm temp.
        let target = Path::new(&download.target_path);
        if download.status == crate::model::DownloadStatus::Finished && target.exists() {
            return open_in_file_manager(target, true);
        }

        // Prefer the parent folder of the target path (destination dir).
        if let Some(parent) = target.parent().filter(|p| p.exists()) {
            return open_in_file_manager(parent, false);
        }

        // Fallback: save_path directory configured by the user.
        let save_path = Path::new(&download.save_path);
        if save_path.exists() {
            return open_in_file_manager(save_path, false);
        }

        Err(format!(
            "Download paths are unavailable for id '{id}'; expected target parent or save directory to exist."
        ))
    }

    pub async fn update_settings(
        &self,
        _app: &AppHandle,
        settings: EngineSettings,
    ) -> Result<EngineSettings, String> {
        self.await_bootstrap().await;
        let mut registry = self.registry_guard()?;
        let sanitized = sanitize_engine_settings(settings);

        registry.settings = sanitized.clone();
        let host_profiles = registry.host_profiles.clone();
        for download in &mut registry.downloads {
            download.traffic_mode = sanitized.traffic_mode.clone();
            let host_profile = host_profiles.get(&download.host);
            apply_download_host_profile(download, &sanitized, host_profile);
        }

        let dispatch_plan = plan_runtime_dispatch(&mut registry);
        let min_interval_ms = registry.settings.segment_checkpoint_min_interval_ms;
        let limiter_updates: Vec<(String, Option<u64>)> = registry
            .downloads
            .iter()
            .map(|download| {
                (
                    download.id.clone(),
                    effective_download_speed_limit(download, &sanitized),
                )
            })
            .collect();
        self.persist_registry(&registry)?;
        drop(registry);
        for (download_id, rate_bytes_per_second) in limiter_updates {
            self.reconfigure_download_rate_limiter(&download_id, rate_bytes_per_second);
        }
        self.emit_engine_settings(&sanitized);
        self.apply_runtime_dispatch_plan(dispatch_plan, min_interval_ms);
        Ok(sanitized)
    }
}
