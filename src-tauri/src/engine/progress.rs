use std::collections::BTreeMap;

use crate::model::DownloadSegmentStatus;

use super::scheduler::SegmentRuntimeSample;

const SPEED_RISE_NEW_WEIGHT: u64 = 70;
const SPEED_DROP_NEW_WEIGHT: u64 = 35;
const SEGMENT_SPEED_RISE_NEW_WEIGHT: u64 = 55;
const SEGMENT_SPEED_DROP_NEW_WEIGHT: u64 = 30;

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
        return Some(smooth_segment_throughput(
            previous_throughput,
            latest_throughput,
        ));
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

fn smooth_segment_throughput(previous_throughput: Option<u64>, latest_throughput: u64) -> u64 {
    let Some(previous_throughput) = previous_throughput.filter(|value| *value > 0) else {
        return latest_throughput;
    };

    let new_weight = if latest_throughput >= previous_throughput {
        SEGMENT_SPEED_RISE_NEW_WEIGHT
    } else {
        SEGMENT_SPEED_DROP_NEW_WEIGHT
    };
    let old_weight = 100_u64.saturating_sub(new_weight);
    previous_throughput
        .saturating_mul(old_weight)
        .saturating_add(latest_throughput.saturating_mul(new_weight))
        / 100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_previous_segment_speed_when_latest_sample_is_zero() {
        let stabilized = stabilized_segment_throughput(
            Some(120),
            0,
            &DownloadSegmentStatus::Downloading,
        );

        assert_eq!(stabilized, Some(120));
    }

    #[test]
    fn smooths_segment_speed_spikes_before_aggregation() {
        let stabilized = stabilized_segment_throughput(
            Some(100),
            1_000,
            &DownloadSegmentStatus::Downloading,
        );

        assert_eq!(stabilized, Some(595));
    }

    #[test]
    fn finished_segments_report_zero_throughput() {
        let stabilized = stabilized_segment_throughput(Some(500), 250, &DownloadSegmentStatus::Finished);

        assert_eq!(stabilized, Some(0));
    }
}
