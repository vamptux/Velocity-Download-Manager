use std::collections::BTreeMap;

use crate::model::{
    DownloadRecord, DownloadStatus, EngineSettings, HostProfile, RegistrySnapshot,
};

use super::{
    apply_download_host_profile, compatibility_request_context_supports_segmented_transfer,
    ensure_segment_plan,
};
use super::host_planner::effective_connection_target;

#[derive(Default)]
pub(super) struct RuntimeDispatchPlan {
    pub(super) changed_downloads: Vec<DownloadRecord>,
    pub(super) launch_ids: Vec<String>,
}

impl RuntimeDispatchPlan {
    pub(super) fn apply(
        self,
        mut on_download_change: impl FnMut(&DownloadRecord),
        mut on_launch: impl FnMut(String),
    ) {
        for download in &self.changed_downloads {
            on_download_change(download);
        }
        for id in self.launch_ids {
            on_launch(id);
        }
    }
}

fn queue_position_key(download: &DownloadRecord) -> u32 {
    if download.queue_position == 0 {
        u32::MAX
    } else {
        download.queue_position
    }
}

fn dispatch_sort_key(download: &DownloadRecord) -> (u8, u32, i64, String) {
    (
        u8::from(!download.manual_start_requested),
        queue_position_key(download),
        download.date_added,
        download.id.clone(),
    )
}

fn requested_connection_cap(download: &DownloadRecord, settings: &EngineSettings) -> u32 {
    download
        .custom_max_connections
        .unwrap_or(settings.default_max_connections)
        .max(1)
}

fn scheduler_host_connection_budget(
    requested_connections: u32,
    settings: &EngineSettings,
    host_profile: Option<&HostProfile>,
) -> u32 {
    effective_connection_target(requested_connections.max(1), settings, host_profile).max(1)
}

fn scheduler_can_launch(download: &DownloadRecord, queue_running: bool) -> bool {
    matches!(download.status, DownloadStatus::Queued)
        && (queue_running || download.manual_start_requested)
}

fn desired_runtime_connections(download: &DownloadRecord) -> u32 {
    if !download.capabilities.segmented
        || !compatibility_request_context_supports_segmented_transfer(download)
    {
        return 1;
    }

    download
        .target_connections
        .max(1)
        .min(download.max_connections.max(1))
}

fn rebalance_host_active_targets(
    registry: &mut RegistrySnapshot,
    settings: &EngineSettings,
    host: &str,
    active_indices: &[usize],
    changed_ids: &mut BTreeMap<String, ()>,
) {
    if active_indices.is_empty() {
        return;
    }

    let host_profile = registry.host_profiles.get(host);
    let requested_connections =
        active_indices
            .iter()
            .fold(settings.default_max_connections.max(1), |current, index| {
                current.max(requested_connection_cap(
                    &registry.downloads[*index],
                    settings,
                ))
            });
    let host_budget =
        scheduler_host_connection_budget(requested_connections, settings, host_profile);

    let mut ordered_indices = active_indices.to_vec();
    ordered_indices.sort_by_key(|index| dispatch_sort_key(&registry.downloads[*index]));

    let mut assigned: BTreeMap<usize, u32> = ordered_indices
        .iter()
        .map(|index| (*index, 1_u32))
        .collect();
    let active_count = u32::try_from(ordered_indices.len()).unwrap_or(u32::MAX);
    let mut remaining_budget = host_budget.saturating_sub(active_count);

    while remaining_budget > 0 {
        let mut granted = false;
        for index in &ordered_indices {
            let desired = desired_runtime_connections(&registry.downloads[*index]);
            let Some(current) = assigned.get_mut(index) else {
                continue;
            };
            if *current >= desired {
                continue;
            }
            *current = current.saturating_add(1);
            remaining_budget = remaining_budget.saturating_sub(1);
            granted = true;
            if remaining_budget == 0 {
                break;
            }
        }

        if !granted {
            break;
        }
    }

    for index in ordered_indices {
        let Some(download) = registry.downloads.get_mut(index) else {
            continue;
        };
        let next_target = assigned
            .get(&index)
            .copied()
            .unwrap_or(1)
            .min(download.max_connections.max(1));
        if download.target_connections != next_target {
            download.target_connections = next_target;
            changed_ids.insert(download.id.clone(), ());
        }
        ensure_segment_plan(download, settings);
    }
}

pub(super) fn plan_runtime_dispatch(registry: &mut RegistrySnapshot) -> RuntimeDispatchPlan {
    let settings = registry.settings.clone();
    let queue_running = registry.queue_running;
    let host_profiles = registry.host_profiles.clone();
    let mut changed_ids = BTreeMap::new();

    for download in &mut registry.downloads {
        let before_max_connections = download.max_connections;
        let before_target_connections = download.target_connections;
        let before_host_max_connections = download.host_max_connections;
        let before_host_cooldown_until = download.host_cooldown_until;
        let before_host_average_ttfb_ms = download.host_average_ttfb_ms;
        let before_host_average_throughput = download.host_average_throughput_bytes_per_second;
        let before_host_protocol = download.host_protocol.clone();
        let before_host_diagnostics = download.host_diagnostics.clone();
        let before_segment_count = download.segments.len();
        apply_download_host_profile(download, &settings, host_profiles.get(&download.host));
        if before_max_connections != download.max_connections
            || before_target_connections != download.target_connections
            || before_host_max_connections != download.host_max_connections
            || before_host_cooldown_until != download.host_cooldown_until
            || before_host_average_ttfb_ms != download.host_average_ttfb_ms
            || before_host_average_throughput != download.host_average_throughput_bytes_per_second
            || before_host_protocol != download.host_protocol
            || before_host_diagnostics != download.host_diagnostics
            || before_segment_count != download.segments.len()
        {
            changed_ids.insert(download.id.clone(), ());
        }
    }

    let active_limit = usize::try_from(settings.max_active_downloads.max(1)).unwrap_or(usize::MAX);
    let mut active_indices = Vec::new();
    let mut active_count = 0_usize;
    let mut active_by_host: BTreeMap<String, u32> = BTreeMap::new();
    let mut requested_by_host: BTreeMap<String, u32> = BTreeMap::new();

    for (index, download) in registry.downloads.iter().enumerate() {
        if !matches!(download.status, DownloadStatus::Downloading) {
            continue;
        }
        active_indices.push(index);
        active_count = active_count.saturating_add(1);
        *active_by_host.entry(download.host.clone()).or_default() = active_by_host
            .get(&download.host)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        requested_by_host
            .entry(download.host.clone())
            .and_modify(|value| {
                *value = (*value).max(requested_connection_cap(download, &settings));
            })
            .or_insert_with(|| requested_connection_cap(download, &settings));
    }

    let mut queued_indices: Vec<usize> = registry
        .downloads
        .iter()
        .enumerate()
        .filter_map(|(index, download)| scheduler_can_launch(download, queue_running).then_some(index))
        .collect();
    queued_indices.sort_by_key(|index| dispatch_sort_key(&registry.downloads[*index]));

    let mut launch_indices = Vec::new();
    for index in queued_indices {
        if active_count >= active_limit {
            break;
        }

        let Some(download) = registry.downloads.get(index) else {
            continue;
        };
        let next_requested = requested_by_host
            .get(&download.host)
            .copied()
            .unwrap_or(settings.default_max_connections.max(1))
            .max(requested_connection_cap(download, &settings));
        let host_cap = scheduler_host_connection_budget(
            next_requested,
            &settings,
            host_profiles.get(&download.host),
        );
        if active_by_host.get(&download.host).copied().unwrap_or(0) >= host_cap {
            continue;
        }

        let download = &mut registry.downloads[index];
        download.status = DownloadStatus::Downloading;
        changed_ids.insert(download.id.clone(), ());
        active_indices.push(index);
        launch_indices.push(index);
        active_count = active_count.saturating_add(1);
        *active_by_host.entry(download.host.clone()).or_default() = active_by_host
            .get(&download.host)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        requested_by_host
            .entry(download.host.clone())
            .and_modify(|value| {
                *value = (*value).max(requested_connection_cap(download, &settings));
            })
            .or_insert_with(|| requested_connection_cap(download, &settings));
    }

    let mut active_groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for index in active_indices {
        if let Some(download) = registry.downloads.get(index) {
            active_groups
                .entry(download.host.clone())
                .or_default()
                .push(index);
        }
    }

    for (host, indices) in active_groups {
        rebalance_host_active_targets(registry, &settings, &host, &indices, &mut changed_ids);
    }

    let changed_downloads = changed_ids
        .into_keys()
        .filter_map(|id| {
            registry
                .downloads
                .iter()
                .find(|download| download.id == id)
                .cloned()
        })
        .collect();
    let launch_ids = launch_indices
        .into_iter()
        .filter_map(|index| {
            registry
                .downloads
                .get(index)
                .map(|download| download.id.clone())
        })
        .collect();

    RuntimeDispatchPlan {
        changed_downloads,
        launch_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::plan_runtime_dispatch;
    use crate::model::{
        DownloadCapabilities, DownloadCategory, DownloadCompatibility, DownloadDiagnostics,
        DownloadIntegrity, DownloadRecord, DownloadRuntimeCheckpoint, DownloadSegment,
        DownloadSegmentStatus, DownloadStatus, EngineSettings, HostDiagnosticsSummary, HostProfile,
        RegistrySnapshot, ResumeValidators, TrafficMode,
    };

    fn fixture_download(id: &str, host: &str, max_connections: u32) -> DownloadRecord {
        DownloadRecord {
            id: id.to_string(),
            name: format!("{id}.bin"),
            url: format!("https://{host}/{id}.bin"),
            final_url: format!("https://{host}/{id}.bin"),
            host: host.to_string(),
            size: 2_048,
            downloaded: 0,
            status: DownloadStatus::Queued,
            manual_start_requested: false,
            category: DownloadCategory::Programs,
            speed: 0,
            time_left: None,
            date_added: 0,
            save_path: "C:\\Downloads".to_string(),
            target_path: format!("C:\\Downloads\\{id}.bin"),
            temp_path: format!("C:\\Downloads\\{id}.bin.part"),
            queue: "default".to_string(),
            queue_position: 1,
            max_connections,
            host_max_connections: None,
            custom_max_connections: None,
            host_cooldown_until: None,
            host_average_ttfb_ms: None,
            host_average_throughput_bytes_per_second: None,
            host_protocol: None,
            host_diagnostics: HostDiagnosticsSummary::default(),
            traffic_mode: TrafficMode::Max,
            speed_limit_bytes_per_second: None,
            open_folder_on_completion: false,
            error_message: None,
            content_type: None,
            capabilities: DownloadCapabilities {
                resumable: true,
                range_supported: true,
                segmented: true,
            },
            validators: ResumeValidators::default(),
            compatibility: DownloadCompatibility::default(),
            integrity: DownloadIntegrity::default(),
            diagnostics: DownloadDiagnostics::default(),
            segments: Vec::new(),
            target_connections: max_connections,
            writer_backpressure: false,
            engine_log: Vec::new(),
            runtime_checkpoint: DownloadRuntimeCheckpoint::default(),
        }
    }

    #[test]
    fn scheduler_dispatch_respects_global_active_limit() {
        let settings = EngineSettings {
            max_active_downloads: 2,
            ..EngineSettings::default()
        };
        let mut registry = RegistrySnapshot {
            next_id: 4,
            settings,
            queue_running: true,
            host_profiles: std::collections::BTreeMap::new(),
            recent_probes: std::collections::BTreeMap::new(),
            downloads: vec![
                fixture_download("download-1", "cdn-a.example.test", 8),
                fixture_download("download-2", "cdn-b.example.test", 8),
                fixture_download("download-3", "cdn-c.example.test", 8),
            ],
        };
        registry.downloads[0].queue_position = 1;
        registry.downloads[1].queue_position = 2;
        registry.downloads[2].queue_position = 3;

        let dispatch = plan_runtime_dispatch(&mut registry);

        assert_eq!(dispatch.launch_ids.len(), 2);
        assert_eq!(dispatch.launch_ids, vec!["download-1", "download-2"]);
        assert_eq!(
            registry
                .downloads
                .iter()
                .filter(|download| download.status == DownloadStatus::Downloading)
                .count(),
            2
        );
        assert_eq!(registry.downloads[2].status, DownloadStatus::Queued);
    }

    #[test]
    fn scheduler_dispatch_respects_per_host_launch_budget() {
        let settings = EngineSettings {
            max_active_downloads: 3,
            ..EngineSettings::default()
        };
        let mut registry = RegistrySnapshot {
            next_id: 4,
            settings,
            queue_running: true,
            host_profiles: std::collections::BTreeMap::from([(
                "cdn.example.test".to_string(),
                HostProfile {
                    max_connections: Some(1),
                    ..HostProfile::default()
                },
            )]),
            recent_probes: std::collections::BTreeMap::new(),
            downloads: vec![
                fixture_download("download-1", "cdn.example.test", 8),
                fixture_download("download-2", "cdn.example.test", 8),
                fixture_download("download-3", "mirror.example.test", 8),
            ],
        };
        registry.downloads[0].queue_position = 1;
        registry.downloads[1].queue_position = 2;
        registry.downloads[2].queue_position = 3;

        let dispatch = plan_runtime_dispatch(&mut registry);

        assert_eq!(dispatch.launch_ids, vec!["download-1", "download-3"]);
        assert_eq!(registry.downloads[0].status, DownloadStatus::Downloading);
        assert_eq!(registry.downloads[1].status, DownloadStatus::Queued);
        assert_eq!(registry.downloads[2].status, DownloadStatus::Downloading);
    }

    #[test]
    fn scheduler_rebalances_active_downloads_on_same_host() {
        let mut first = fixture_download("download-1", "cdn.example.test", 8);
        first.status = DownloadStatus::Downloading;
        first.target_connections = 6;
        first.segments = vec![DownloadSegment {
            id: 1,
            start: 0,
            end: 1_023,
            downloaded: 0,
            retry_attempts: 0,
            retry_budget: 4,
            status: DownloadSegmentStatus::Downloading,
        }];

        let mut second = fixture_download("download-2", "cdn.example.test", 8);
        second.status = DownloadStatus::Downloading;
        second.target_connections = 6;
        second.queue_position = 2;
        second.segments = vec![DownloadSegment {
            id: 2,
            start: 1_024,
            end: 2_047,
            downloaded: 0,
            retry_attempts: 0,
            retry_budget: 4,
            status: DownloadSegmentStatus::Downloading,
        }];

        let mut registry = RegistrySnapshot {
            next_id: 3,
            settings: EngineSettings::default(),
            queue_running: true,
            host_profiles: std::collections::BTreeMap::new(),
            recent_probes: std::collections::BTreeMap::new(),
            downloads: vec![first, second],
        };

        let dispatch = plan_runtime_dispatch(&mut registry);

        assert!(dispatch.launch_ids.is_empty());
        assert_eq!(registry.downloads[0].target_connections, 4);
        assert_eq!(registry.downloads[1].target_connections, 4);
        assert_eq!(registry.downloads[0].segments.len(), 1);
        assert_eq!(registry.downloads[1].segments.len(), 1);
        assert_eq!(
            registry.downloads[0].segments[0].status,
            DownloadSegmentStatus::Downloading
        );
        assert_eq!(
            registry.downloads[1].segments[0].status,
            DownloadSegmentStatus::Downloading
        );
    }

    #[test]
    fn scheduler_rebalances_same_host_budget_fairly_across_three_active_downloads() {
        let mut first = fixture_download("download-1", "cdn.example.test", 8);
        first.status = DownloadStatus::Downloading;
        first.target_connections = 6;
        first.segments = vec![DownloadSegment {
            id: 1,
            start: 0,
            end: 1_023,
            downloaded: 0,
            retry_attempts: 0,
            retry_budget: 4,
            status: DownloadSegmentStatus::Downloading,
        }];

        let mut second = fixture_download("download-2", "cdn.example.test", 8);
        second.status = DownloadStatus::Downloading;
        second.target_connections = 6;
        second.queue_position = 2;
        second.segments = vec![DownloadSegment {
            id: 2,
            start: 1_024,
            end: 2_047,
            downloaded: 0,
            retry_attempts: 0,
            retry_budget: 4,
            status: DownloadSegmentStatus::Downloading,
        }];

        let mut third = fixture_download("download-3", "cdn.example.test", 8);
        third.status = DownloadStatus::Downloading;
        third.target_connections = 6;
        third.queue_position = 3;
        third.segments = vec![DownloadSegment {
            id: 3,
            start: 2_048,
            end: 3_071,
            downloaded: 0,
            retry_attempts: 0,
            retry_budget: 4,
            status: DownloadSegmentStatus::Downloading,
        }];

        let mut registry = RegistrySnapshot {
            next_id: 4,
            settings: EngineSettings::default(),
            queue_running: true,
            host_profiles: std::collections::BTreeMap::from([(
                "cdn.example.test".to_string(),
                HostProfile {
                    max_connections: Some(5),
                    ..HostProfile::default()
                },
            )]),
            recent_probes: std::collections::BTreeMap::new(),
            downloads: vec![first, second, third],
        };

        let dispatch = plan_runtime_dispatch(&mut registry);

        assert!(dispatch.launch_ids.is_empty());
        assert_eq!(
            registry
                .downloads
                .iter()
                .map(|download| download.target_connections)
                .collect::<Vec<_>>(),
            vec![2, 2, 1]
        );
    }
}
