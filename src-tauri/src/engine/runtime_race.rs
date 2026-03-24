use std::collections::BTreeMap;

use super::engine_log::append_download_log;
use super::runtime_support::{
    RuntimeRaceState, persist_runtime_races, resolve_runtime_race, restore_runtime_races,
};
use super::scheduler::{SegmentRuntimeSample, SegmentScheduler};
use crate::model::{DownloadLogLevel, DownloadRecord, DownloadSegment};

pub(super) struct RuntimeQueueExpansion {
    pub(super) appended_segment: Option<DownloadSegment>,
    pub(super) control_updates: Vec<(u32, i64)>,
}

pub(super) struct RuntimeRaceWinner {
    pub(super) loser_id: u32,
}

pub(super) fn restore_runtime_races_from_checkpoint(
    download: &DownloadRecord,
    pending_ids: &BTreeMap<u32, ()>,
) -> BTreeMap<u32, RuntimeRaceState> {
    restore_runtime_races(&download.runtime_checkpoint, pending_ids)
}

pub(super) fn resolve_runtime_race_winner(
    download: &mut DownloadRecord,
    winner_id: u32,
    race_by_segment: &mut BTreeMap<u32, RuntimeRaceState>,
) -> Option<RuntimeRaceWinner> {
    let race = race_by_segment.remove(&winner_id)?;
    race_by_segment.remove(&race.companion_segment_id);
    let loser_id = race.companion_segment_id;
    resolve_runtime_race(download, winner_id, race, race_by_segment);
    append_download_log(
        download,
        DownloadLogLevel::Info,
        "race.resolved",
        format!(
            "Segment {} won the slow-peer race; segment {} was canceled.",
            winner_id, loser_id
        ),
    );
    Some(RuntimeRaceWinner { loser_id })
}

pub(super) fn attempt_runtime_queue_expansion(
    download: &mut DownloadRecord,
    scheduler: &SegmentScheduler,
    runtime_samples: &[SegmentRuntimeSample],
    race_by_segment: &mut BTreeMap<u32, RuntimeRaceState>,
    idle_worker_count: u32,
) -> RuntimeQueueExpansion {
    if download.writer_backpressure || idle_worker_count == 0 {
        return RuntimeQueueExpansion {
            appended_segment: None,
            control_updates: Vec::new(),
        };
    }

    let mut control_updates = Vec::new();
    if let Some(stolen) = {
        let before_ends: BTreeMap<u32, i64> = download
            .segments
            .iter()
            .map(|segment| (segment.id, segment.end))
            .collect();
        let stolen = scheduler.attempt_work_steal(
            &mut download.segments,
            runtime_samples,
            download.size.max(0) as u64,
            idle_worker_count,
        );
        if stolen.is_some() {
            for segment in &download.segments {
                if before_ends.get(&segment.id).copied() != Some(segment.end) {
                    control_updates.push((segment.id, segment.end));
                }
            }
        }
        stolen
    } {
        download.segments.push(stolen.clone());
        return RuntimeQueueExpansion {
            appended_segment: Some(stolen),
            control_updates,
        };
    }

    let Some(plan) = scheduler.attempt_slow_peer_race_steal(
        &download.segments,
        runtime_samples,
        download.size.max(0) as u64,
        idle_worker_count,
    )
    else {
        return RuntimeQueueExpansion {
            appended_segment: None,
            control_updates,
        };
    };

    let already_exists = download
        .segments
        .iter()
        .any(|segment| segment.id == plan.challenger_segment.id);
    if already_exists || race_by_segment.contains_key(&plan.slow_segment_id) {
        return RuntimeQueueExpansion {
            appended_segment: None,
            control_updates,
        };
    }

    let slow_baseline = download
        .segments
        .iter()
        .find(|segment| segment.id == plan.slow_segment_id)
        .map(|segment| segment.downloaded)
        .unwrap_or(0);
    let challenger_id = plan.challenger_segment.id;
    download.segments.push(plan.challenger_segment.clone());
    race_by_segment.insert(
        plan.slow_segment_id,
        RuntimeRaceState {
            companion_segment_id: challenger_id,
            slow_segment_id: plan.slow_segment_id,
            slow_baseline_downloaded: slow_baseline,
        },
    );
    race_by_segment.insert(
        challenger_id,
        RuntimeRaceState {
            companion_segment_id: plan.slow_segment_id,
            slow_segment_id: plan.slow_segment_id,
            slow_baseline_downloaded: slow_baseline,
        },
    );
    append_download_log(
        download,
        DownloadLogLevel::Info,
        "race.started",
        format!(
            "Started slow-peer race: challenger segment {} against slow segment {} at byte {}.",
            challenger_id, plan.slow_segment_id, slow_baseline
        ),
    );
    persist_runtime_races(&mut download.runtime_checkpoint, race_by_segment);
    RuntimeQueueExpansion {
        appended_segment: Some(plan.challenger_segment),
        control_updates,
    }
}
