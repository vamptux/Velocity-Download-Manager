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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::model::{
        DownloadCapabilities, DownloadCategory, DownloadCompatibility, DownloadDiagnostics,
        DownloadIntegrity, DownloadRecord, DownloadRuntimeCheckpoint, DownloadRuntimeRaceState,
        DownloadSegment, DownloadSegmentStatus, DownloadStatus, ResumeValidators, TrafficMode,
    };

    use super::{resolve_runtime_race, restore_runtime_races, RuntimeRaceState};

    fn build_download_with_race_checkpoint() -> DownloadRecord {
        DownloadRecord {
            id: "download-1".to_string(),
            name: "fixture.bin".to_string(),
            url: "https://example.test/file".to_string(),
            final_url: "https://example.test/file".to_string(),
            host: "example.test".to_string(),
            size: 1_000,
            downloaded: 700,
            status: DownloadStatus::Downloading,
            manual_start_requested: false,
            category: DownloadCategory::Programs,
            speed: 0,
            time_left: None,
            date_added: 0,
            save_path: "C:\\Downloads".to_string(),
            target_path: "C:\\Downloads\\fixture.bin".to_string(),
            temp_path: "C:\\Downloads\\fixture.bin.part".to_string(),
            queue: "default".to_string(),
            queue_position: 1,
            max_connections: 8,
            host_max_connections: None,
            custom_max_connections: None,
            host_cooldown_until: None,
            host_average_ttfb_ms: None,
            host_average_throughput_bytes_per_second: None,
            host_protocol: None,
            host_diagnostics: crate::model::HostDiagnosticsSummary::default(),
            traffic_mode: TrafficMode::Max,
            speed_limit_bytes_per_second: None,
            open_folder_on_completion: false,
            error_message: None,
            content_type: None,
            capabilities: DownloadCapabilities::default(),
            validators: ResumeValidators::default(),
            compatibility: DownloadCompatibility::default(),
            integrity: DownloadIntegrity::default(),
            diagnostics: DownloadDiagnostics::default(),
            segments: vec![
                DownloadSegment {
                    id: 4,
                    start: 0,
                    end: 499,
                    downloaded: 420,
                    retry_attempts: 0,
                    retry_budget: 4,
                    status: DownloadSegmentStatus::Downloading,
                },
                DownloadSegment {
                    id: 5,
                    start: 500,
                    end: 999,
                    downloaded: 120,
                    retry_attempts: 0,
                    retry_budget: 4,
                    status: DownloadSegmentStatus::Downloading,
                },
            ],
            target_connections: 4,
            writer_backpressure: false,
            engine_log: Vec::new(),
            runtime_checkpoint: DownloadRuntimeCheckpoint {
                segment_samples: Vec::new(),
                active_races: vec![
                    DownloadRuntimeRaceState {
                        slow_segment_id: 4,
                        companion_segment_id: 5,
                        slow_baseline_downloaded: 350,
                    },
                    DownloadRuntimeRaceState {
                        slow_segment_id: 7,
                        companion_segment_id: 8,
                        slow_baseline_downloaded: 10,
                    },
                ],
            },
        }
    }

    #[test]
    fn restores_runtime_races_only_for_pending_segments() {
        let checkpoint = build_download_with_race_checkpoint().runtime_checkpoint;
        let pending_ids = BTreeMap::from([(4_u32, ()), (5_u32, ())]);
        let races = restore_runtime_races(&checkpoint, &pending_ids);
        assert_eq!(races.len(), 2);
        assert_eq!(
            races.get(&4),
            Some(&RuntimeRaceState {
                companion_segment_id: 5,
                slow_segment_id: 4,
                slow_baseline_downloaded: 350
            })
        );
        assert_eq!(
            races.get(&5),
            Some(&RuntimeRaceState {
                companion_segment_id: 4,
                slow_segment_id: 4,
                slow_baseline_downloaded: 350
            })
        );
    }

    #[test]
    fn resolves_runtime_race_and_marks_loser_finished() {
        let mut download = build_download_with_race_checkpoint();
        let mut race_map = BTreeMap::new();
        race_map.insert(
            4,
            RuntimeRaceState {
                companion_segment_id: 5,
                slow_segment_id: 4,
                slow_baseline_downloaded: 350,
            },
        );
        let race = RuntimeRaceState {
            companion_segment_id: 5,
            slow_segment_id: 4,
            slow_baseline_downloaded: 350,
        };
        resolve_runtime_race(&mut download, 4, race, &race_map);
        let loser = download.segments.iter().find(|segment| segment.id == 5);
        assert!(loser.is_some());
        let loser = loser.unwrap_or_else(|| unreachable!());
        assert_eq!(loser.downloaded, 0);
        assert_eq!(loser.status, DownloadSegmentStatus::Finished);
        assert_eq!(download.runtime_checkpoint.active_races.len(), 1);
    }
}
