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
