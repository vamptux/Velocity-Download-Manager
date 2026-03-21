use std::collections::BTreeMap;

use crate::model::{DownloadRecord, DownloadStatus, EngineSettings, HostProfile, RegistrySnapshot};

use super::host_planner::effective_connection_target;
use super::{
    apply_download_host_profile, compatibility_request_context_supports_segmented_transfer,
    ensure_segment_plan,
};

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

fn requested_connection_cap(download: &DownloadRecord, _settings: &EngineSettings) -> u32 {
    download.max_connections.max(1)
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
    let requested_connections = active_indices.iter().fold(16u32, |current, index| {
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
        .filter_map(|(index, download)| {
            scheduler_can_launch(download, queue_running).then_some(index)
        })
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
            .unwrap_or(16u32)
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
