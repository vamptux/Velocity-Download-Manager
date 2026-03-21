use std::collections::BTreeMap;

use crate::model::{
    DownloadRecord, DownloadRuntimeCheckpoint, DownloadRuntimeRaceState,
    DownloadRuntimeSegmentSample, DownloadSegmentStatus,
};

use super::scheduler::SegmentRuntimeSample;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RuntimeRaceState {
    pub companion_segment_id: u32,
    pub slow_segment_id: u32,
    pub slow_baseline_downloaded: i64,
}

pub(super) fn clear_runtime_checkpoint(download: &mut DownloadRecord) {
    download.runtime_checkpoint = DownloadRuntimeCheckpoint::default();
}

pub(super) fn upsert_runtime_segment_sample(
    checkpoint: &mut DownloadRuntimeCheckpoint,
    sample: &SegmentRuntimeSample,
) {
    if let Some(existing) = checkpoint
        .segment_samples
        .iter_mut()
        .find(|value| value.segment_id == sample.segment_id)
    {
        existing.remaining_bytes = sample.remaining_bytes;
        existing.eta_seconds = sample.eta_seconds;
        existing.throughput_bytes_per_second = sample.throughput_bytes_per_second;
        return;
    }
    checkpoint
        .segment_samples
        .push(DownloadRuntimeSegmentSample {
            segment_id: sample.segment_id,
            remaining_bytes: sample.remaining_bytes,
            eta_seconds: sample.eta_seconds,
            throughput_bytes_per_second: sample.throughput_bytes_per_second,
            retry_attempts: 0,
            terminal_failure_reason: None,
        });
}

pub(super) fn upsert_runtime_segment_health(
    checkpoint: &mut DownloadRuntimeCheckpoint,
    segment_id: u32,
    retry_attempts: u32,
    terminal_failure_reason: Option<String>,
) {
    if let Some(existing) = checkpoint
        .segment_samples
        .iter_mut()
        .find(|value| value.segment_id == segment_id)
    {
        existing.retry_attempts = retry_attempts;
        existing.terminal_failure_reason = terminal_failure_reason;
        return;
    }
    checkpoint.segment_samples.push(DownloadRuntimeSegmentSample {
        segment_id,
        remaining_bytes: 0,
        eta_seconds: None,
        throughput_bytes_per_second: None,
        retry_attempts,
        terminal_failure_reason,
    });
}

pub(super) fn persist_runtime_races(
    checkpoint: &mut DownloadRuntimeCheckpoint,
    race_by_segment: &BTreeMap<u32, RuntimeRaceState>,
) {
    checkpoint.active_races.clear();
    for race in race_by_segment.values() {
        if race.slow_segment_id > race.companion_segment_id {
            continue;
        }
        checkpoint.active_races.push(DownloadRuntimeRaceState {
            slow_segment_id: race.slow_segment_id,
            companion_segment_id: race.companion_segment_id,
            slow_baseline_downloaded: race.slow_baseline_downloaded,
        });
    }
}

pub(super) fn restore_runtime_races(
    checkpoint: &DownloadRuntimeCheckpoint,
    pending_ids: &BTreeMap<u32, ()>,
) -> BTreeMap<u32, RuntimeRaceState> {
    let mut race_by_segment = BTreeMap::new();
    for race in &checkpoint.active_races {
        if pending_ids.contains_key(&race.slow_segment_id)
            && pending_ids.contains_key(&race.companion_segment_id)
        {
            race_by_segment.insert(
                race.slow_segment_id,
                RuntimeRaceState {
                    companion_segment_id: race.companion_segment_id,
                    slow_segment_id: race.slow_segment_id,
                    slow_baseline_downloaded: race.slow_baseline_downloaded,
                },
            );
            race_by_segment.insert(
                race.companion_segment_id,
                RuntimeRaceState {
                    companion_segment_id: race.slow_segment_id,
                    slow_segment_id: race.slow_segment_id,
                    slow_baseline_downloaded: race.slow_baseline_downloaded,
                },
            );
        }
    }
    race_by_segment
}

pub(super) fn resolve_runtime_race(
    download: &mut DownloadRecord,
    winner_id: u32,
    race: RuntimeRaceState,
    race_by_segment: &BTreeMap<u32, RuntimeRaceState>,
) {
    if winner_id == race.slow_segment_id {
        if let Some(challenger) = download
            .segments
            .iter_mut()
            .find(|value| value.id == race.companion_segment_id)
        {
            challenger.downloaded = 0;
            challenger.status = DownloadSegmentStatus::Finished;
        }
    } else if let Some(slow) = download
        .segments
        .iter_mut()
        .find(|value| value.id == race.slow_segment_id)
    {
        slow.downloaded = slow.downloaded.min(race.slow_baseline_downloaded);
        slow.status = DownloadSegmentStatus::Finished;
    }
    persist_runtime_races(&mut download.runtime_checkpoint, race_by_segment);
}
