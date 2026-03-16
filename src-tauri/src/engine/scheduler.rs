use crate::engine::segmentation::{pending_segment, split_segment};
use crate::model::{DownloadSegment, DownloadSegmentStatus};
use std::collections::BTreeMap;

const STEAL_TAIL_DIVISOR: u64 = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WorkStealScan {
    downloaded: u64,
    active_workers: u64,
    largest_active: Option<(usize, u64)>,
}

pub struct SegmentScheduler {
    min_segment_size_bytes: u64,
    late_segment_ratio_percent: u32,
}

fn segment_current_offset(segment: &DownloadSegment) -> u64 {
    (segment.start.max(0) as u64).saturating_add(segment.downloaded.max(0) as u64)
}

fn segment_remaining_bytes(segment: &DownloadSegment) -> u64 {
    let current_offset = segment_current_offset(segment);
    let end = segment.end.max(0) as u64;
    if current_offset > end {
        return 0;
    }

    end.saturating_sub(current_offset).saturating_add(1)
}

impl SegmentScheduler {
    pub fn new(min_segment_size_bytes: u64, late_segment_ratio_percent: u32) -> Self {
        Self {
            min_segment_size_bytes,
            late_segment_ratio_percent,
        }
    }

    fn scan_work_steal_window(
        &self,
        segments: &[DownloadSegment],
        total_file_size: u64,
    ) -> WorkStealScan {
        let mut downloaded = 0_u64;
        let mut active_workers = 0_u64;
        let mut largest_active = None;
        let mut largest_remaining = 0_u64;

        for (idx, seg) in segments.iter().enumerate() {
            downloaded = downloaded.saturating_add(seg.downloaded.max(0) as u64);
            if seg.status != DownloadSegmentStatus::Downloading {
                continue;
            }

            active_workers = active_workers.saturating_add(1);
            let remaining = segment_remaining_bytes(seg);
            if remaining > largest_remaining {
                largest_active = Some((idx, remaining));
                largest_remaining = remaining;
            }
        }

        WorkStealScan {
            downloaded: downloaded.min(total_file_size),
            active_workers,
            largest_active,
        }
    }

    fn is_in_late_segment_zone(
        &self,
        downloaded: u64,
        active_workers: u64,
        total_file_size: u64,
    ) -> bool {
        if total_file_size == 0 {
            return false;
        }

        let remaining = total_file_size.saturating_sub(downloaded);
        let remaining_work_window = self
            .min_segment_size_bytes
            .saturating_mul(active_workers.max(1).saturating_add(1));
        if remaining > remaining_work_window {
            return false;
        }
        let progress_percent = downloaded.saturating_mul(100) / total_file_size;
        progress_percent >= u64::from(100u32.saturating_sub(self.late_segment_ratio_percent))
    }

    pub fn attempt_work_steal(
        &self,
        segments: &mut [DownloadSegment],
        total_file_size: u64,
        idle_worker_count: u32,
    ) -> Option<DownloadSegment> {
        if idle_worker_count == 0 {
            return None;
        }

        let scan = self.scan_work_steal_window(segments, total_file_size);
        if self.is_in_late_segment_zone(scan.downloaded, scan.active_workers, total_file_size) {
            return None;
        }

        let (largest_idx, largest_remaining) = scan.largest_active?;

        if largest_remaining < self.min_segment_size_bytes.saturating_mul(2) {
            return None;
        }

        let largest_seg = &segments[largest_idx];
        let current_offset = segment_current_offset(largest_seg);

        let challenger_size = largest_remaining
            .checked_div(STEAL_TAIL_DIVISOR)
            .unwrap_or(0)
            .max(self.min_segment_size_bytes)
            .min(largest_remaining.saturating_sub(self.min_segment_size_bytes));
        if challenger_size < self.min_segment_size_bytes {
            return None;
        }
        let split_offset =
            current_offset.saturating_add(largest_remaining.saturating_sub(challenger_size)) as i64;
        let next_segment_id = segments
            .iter()
            .map(|segment| segment.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);

        let (stolen_segment, modified_segment) = split_segment(
            largest_seg,
            split_offset,
            self.min_segment_size_bytes.min(i64::MAX as u64) as i64,
            next_segment_id,
        )?;

        segments[largest_idx] = modified_segment;

        Some(stolen_segment)
    }

    pub fn fill_idle_slots(
        &self,
        segments: &mut Vec<DownloadSegment>,
        total_file_size: u64,
        desired_parallelism: usize,
    ) -> SegmentRefillPlan {
        let desired_parallelism = desired_parallelism.max(1);
        let mut control_updates = BTreeMap::new();
        let mut appended_segments = Vec::new();

        while segments
            .iter()
            .filter(|segment| segment.status != DownloadSegmentStatus::Finished)
            .count()
            < desired_parallelism
        {
            let before_ends: BTreeMap<u32, i64> = segments
                .iter()
                .map(|segment| (segment.id, segment.end))
                .collect();
            let Some(stolen) = self.attempt_work_steal(segments.as_mut_slice(), total_file_size, 1)
            else {
                break;
            };

            for segment in segments.iter() {
                if before_ends.get(&segment.id).copied() != Some(segment.end) {
                    control_updates.insert(segment.id, segment.end);
                }
            }

            segments.push(stolen.clone());
            appended_segments.push(stolen);
        }

        SegmentRefillPlan {
            appended_segments,
            control_updates: control_updates.into_iter().collect(),
        }
    }
    pub fn attempt_slow_peer_race_steal(
        &self,
        segments: &[DownloadSegment],
        samples: &[SegmentRuntimeSample],
        idle_worker_count: u32,
    ) -> Option<SlowPeerRacePlan> {
        if idle_worker_count == 0 || segments.is_empty() || samples.is_empty() {
            return None;
        }

        let sample_by_segment: BTreeMap<u32, &SegmentRuntimeSample> = samples
            .iter()
            .map(|sample| (sample.segment_id, sample))
            .collect();

        let mut active_eta: Vec<u64> = Vec::new();
        let mut active_throughput: Vec<u64> = Vec::new();
        for segment in segments {
            if segment.status != DownloadSegmentStatus::Downloading {
                continue;
            }
            let Some(sample) = sample_by_segment.get(&segment.id).copied() else {
                continue;
            };
            if sample.remaining_bytes < self.min_segment_size_bytes {
                continue;
            }
            if let Some(eta) = sample.eta_seconds.filter(|eta| *eta > 0) {
                active_eta.push(eta);
            }
            if let Some(throughput) = sample
                .throughput_bytes_per_second
                .filter(|value| *value > 0)
            {
                active_throughput.push(throughput);
            }
        }

        if active_eta.len() < 2 || active_throughput.len() < 2 {
            return None;
        }

        active_eta.sort_unstable();
        active_throughput.sort_unstable();
        let median_eta = active_eta[active_eta.len() / 2];
        let median_throughput = active_throughput[active_throughput.len() / 2];

        if median_eta == 0 || median_throughput == 0 {
            return None;
        }

        let mut candidate: Option<(&DownloadSegment, &SegmentRuntimeSample)> = None;
        for segment in segments {
            if segment.status != DownloadSegmentStatus::Downloading {
                continue;
            }
            let Some(sample) = sample_by_segment.get(&segment.id).copied() else {
                continue;
            };
            let eta = sample.eta_seconds.unwrap_or(0);
            let throughput = sample.throughput_bytes_per_second.unwrap_or(0);
            if sample.remaining_bytes < self.min_segment_size_bytes || eta == 0 || throughput == 0 {
                continue;
            }
            if eta < median_eta.saturating_mul(2) {
                continue;
            }
            if throughput.saturating_mul(100) > median_throughput.saturating_mul(55) {
                continue;
            }
            match candidate {
                Some((_, current)) if current.eta_seconds.unwrap_or(0) >= eta => {}
                _ => candidate = Some((segment, sample)),
            }
        }

        let (slow_segment, sample) = candidate?;
        let start_offset = segment_current_offset(slow_segment);
        let end_offset = slow_segment.end as u64;
        if end_offset <= start_offset {
            return None;
        }
        let remaining = end_offset.saturating_sub(start_offset).saturating_add(1);
        if remaining < self.min_segment_size_bytes.min(sample.remaining_bytes) {
            return None;
        }

        let next_segment_id = segments
            .iter()
            .map(|segment| segment.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);

        Some(SlowPeerRacePlan {
            slow_segment_id: slow_segment.id,
            challenger_segment: pending_segment(
                next_segment_id,
                start_offset as i64,
                slow_segment.end,
                slow_segment.retry_budget,
            ),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRuntimeSample {
    pub segment_id: u32,
    pub remaining_bytes: u64,
    pub eta_seconds: Option<u64>,
    pub throughput_bytes_per_second: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlowPeerRacePlan {
    pub slow_segment_id: u32,
    pub challenger_segment: DownloadSegment,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SegmentRefillPlan {
    pub appended_segments: Vec<DownloadSegment>,
    pub control_updates: Vec<(u32, i64)>,
}

#[cfg(test)]
mod tests {
    use super::{SegmentScheduler, STEAL_TAIL_DIVISOR};
    use crate::model::{DownloadSegment, DownloadSegmentStatus};

    fn downloading_segment(id: u32, start: i64, end: i64, downloaded: i64) -> DownloadSegment {
        DownloadSegment {
            id,
            start,
            end,
            downloaded,
            retry_attempts: 0,
            retry_budget: 4,
            status: DownloadSegmentStatus::Downloading,
        }
    }

    #[test]
    fn fill_idle_slots_proactively_splits_active_segments() {
        let scheduler = SegmentScheduler::new(100, 20);
        let mut segments = vec![downloading_segment(1, 0, 1_199, 200)];

        let refill = scheduler.fill_idle_slots(&mut segments, 1_200, 3);

        assert_eq!(refill.appended_segments.len(), 2);
        assert_eq!(segments.len(), 3);
        assert!(!refill.control_updates.is_empty());
    }

    #[test]
    fn work_steal_biases_new_segment_toward_tail() {
        let scheduler = SegmentScheduler::new(100, 20);
        let mut segments = vec![downloading_segment(1, 0, 899, 100)];

        let stolen = scheduler.attempt_work_steal(&mut segments, 900, 1);

        assert!(stolen.is_some());
        let stolen = stolen.unwrap_or_else(|| unreachable!());
        let remaining = 800_u64;
        let expected_min_start =
            100_i64 + i64::try_from(remaining - remaining / STEAL_TAIL_DIVISOR).unwrap_or(i64::MAX);
        assert!(stolen.start >= expected_min_start);
    }

    #[test]
    fn late_segment_guard_allows_large_remaining_window() {
        let scheduler = SegmentScheduler::new(100, 20);
        let mut segments = vec![downloading_segment(1, 0, 1_999, 1_600)];

        let stolen = scheduler.attempt_work_steal(&mut segments, 2_000, 1);

        assert!(stolen.is_some());
    }

    #[test]
    fn work_steal_skips_exhausted_active_segments() {
        let scheduler = SegmentScheduler::new(100, 20);
        let mut segments = vec![downloading_segment(1, 0, 99, 100)];

        let stolen = scheduler.attempt_work_steal(&mut segments, 100, 1);

        assert!(stolen.is_none());
    }

    #[test]
    fn late_segment_guard_blocks_small_tail_window() {
        let scheduler = SegmentScheduler::new(100, 20);
        let mut segments = vec![downloading_segment(1, 0, 999, 850)];

        let stolen = scheduler.attempt_work_steal(&mut segments, 1_000, 1);

        assert!(stolen.is_none());
    }
}
