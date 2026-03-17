use std::collections::BTreeMap;

use crate::model::DownloadSegmentStatus;

use super::scheduler::SegmentRuntimeSample;

const SPEED_RISE_NEW_WEIGHT: u64 = 70;
const SPEED_DROP_NEW_WEIGHT: u64 = 35;

pub(super) fn aggregate_runtime_throughput(
    samples: &BTreeMap<u32, SegmentRuntimeSample>,
) -> Option<u64> {
    let total = samples
        .values()
        .filter_map(|sample| sample.throughput_bytes_per_second)
        .filter(|value| *value > 0)
        .sum::<u64>();
    (total > 0).then_some(total)
}

pub(super) fn stabilized_segment_throughput(
    previous_throughput: Option<u64>,
    latest_throughput: u64,
    status: &DownloadSegmentStatus,
) -> Option<u64> {
    if *status == DownloadSegmentStatus::Finished {
        return Some(0);
    }
    if latest_throughput > 0 {
        return Some(latest_throughput);
    }
    previous_throughput.filter(|value| *value > 0)
}

pub(super) fn recompute_download_speed(
    previous_speed: u64,
    samples: &BTreeMap<u32, SegmentRuntimeSample>,
) -> u64 {
    let instantaneous = aggregate_runtime_throughput(samples).unwrap_or(0);
    smooth_download_speed(previous_speed, instantaneous)
}

pub(super) fn estimate_time_left(size: i64, downloaded: i64, speed: u64) -> Option<u64> {
    if size <= 0 || speed == 0 {
        return None;
    }
    let remaining = size.saturating_sub(downloaded).max(0) as u64;
    Some(remaining / speed.max(1))
}

fn smooth_download_speed(previous_speed: u64, instantaneous_speed: u64) -> u64 {
    if previous_speed == 0 || instantaneous_speed == 0 {
        return instantaneous_speed;
    }

    let new_weight = if instantaneous_speed >= previous_speed {
        SPEED_RISE_NEW_WEIGHT
    } else {
        SPEED_DROP_NEW_WEIGHT
    };
    let old_weight = 100_u64.saturating_sub(new_weight);
    previous_speed
        .saturating_mul(old_weight)
        .saturating_add(instantaneous_speed.saturating_mul(new_weight))
        / 100
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::model::DownloadSegmentStatus;

    use super::{
        aggregate_runtime_throughput, estimate_time_left, recompute_download_speed,
        stabilized_segment_throughput,
    };
    use crate::engine::scheduler::SegmentRuntimeSample;

    #[test]
    fn aggregates_all_active_segment_throughput() {
        let samples = BTreeMap::from([
            (
                1,
                SegmentRuntimeSample {
                    segment_id: 1,
                    remaining_bytes: 10,
                    eta_seconds: Some(1),
                    throughput_bytes_per_second: Some(4 * 1024 * 1024),
                    active_for_ms: None,
                },
            ),
            (
                2,
                SegmentRuntimeSample {
                    segment_id: 2,
                    remaining_bytes: 10,
                    eta_seconds: Some(1),
                    throughput_bytes_per_second: Some(6 * 1024 * 1024),
                    active_for_ms: None,
                },
            ),
        ]);

        assert_eq!(
            aggregate_runtime_throughput(&samples),
            Some(10 * 1024 * 1024)
        );
    }

    #[test]
    fn preserves_previous_segment_speed_when_latest_sample_is_zero() {
        assert_eq!(
            stabilized_segment_throughput(
                Some(8 * 1024 * 1024),
                0,
                &DownloadSegmentStatus::Downloading,
            ),
            Some(8 * 1024 * 1024)
        );
        assert_eq!(
            stabilized_segment_throughput(
                Some(8 * 1024 * 1024),
                0,
                &DownloadSegmentStatus::Finished
            ),
            Some(0)
        );
    }

    #[test]
    fn smooths_single_speed_drop_without_hiding_persistent_change() {
        let samples = BTreeMap::from([(
            1,
            SegmentRuntimeSample {
                segment_id: 1,
                remaining_bytes: 10,
                eta_seconds: Some(1),
                throughput_bytes_per_second: Some(5 * 1024 * 1024),
                active_for_ms: None,
            },
        )]);

        let smoothed = recompute_download_speed(15 * 1024 * 1024, &samples);
        assert!(smoothed > 5 * 1024 * 1024);
        assert!(smoothed < 15 * 1024 * 1024);
    }

    #[test]
    fn computes_time_left_from_aggregate_speed() {
        assert_eq!(estimate_time_left(100, 40, 20), Some(3));
        assert_eq!(estimate_time_left(100, 100, 20), Some(0));
        assert_eq!(estimate_time_left(100, 20, 0), None);
    }
}
