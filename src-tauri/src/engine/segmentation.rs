use crate::model::{DownloadSegment, DownloadSegmentStatus};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SegmentPlanningHints {
    pub throughput_bytes_per_second: Option<u64>,
    pub ttfb_ms: Option<u64>,
    pub target_chunk_time_seconds: u32,
}

pub(super) fn pending_segment(id: u32, start: i64, end: i64, retry_budget: u32) -> DownloadSegment {
    DownloadSegment {
        id,
        start,
        end,
        downloaded: 0,
        retry_attempts: 0,
        retry_budget,
        status: DownloadSegmentStatus::Pending,
    }
}

pub fn compute_segments_with_hints(
    total_size: u64,
    num_segments: u32,
    min_segment_size: u64,
    retry_budget: u32,
    hints: SegmentPlanningHints,
) -> Vec<DownloadSegment> {
    if total_size == 0 || num_segments == 0 {
        return Vec::new();
    }

    let min_segment_size = min_segment_size.max(1);
    let actual_segments = actual_segment_count(total_size, num_segments, min_segment_size);

    if actual_segments == 0 {
        return Vec::new();
    }

    let segment_sizes =
        throughput_shaped_segment_sizes(total_size, actual_segments, min_segment_size, hints)
            .unwrap_or_else(|| equal_segment_sizes(total_size, actual_segments));

    build_segments_from_sizes(&segment_sizes, retry_budget)
}

fn actual_segment_count(total_size: u64, num_segments: u32, min_segment_size: u64) -> u64 {
    let num_segments = u64::from(num_segments.max(1));
    std::cmp::min(
        num_segments,
        std::cmp::max(1, total_size / min_segment_size.max(1)),
    )
}

fn equal_segment_sizes(total_size: u64, actual_segments: u64) -> Vec<u64> {
    let segment_size = total_size / actual_segments.max(1);
    let mut sizes = Vec::with_capacity(usize::try_from(actual_segments).unwrap_or(usize::MAX));
    let mut assigned = 0_u64;

    for idx in 0..actual_segments {
        let size = if idx == actual_segments - 1 {
            total_size.saturating_sub(assigned)
        } else {
            segment_size
        };
        sizes.push(size);
        assigned = assigned.saturating_add(size);
    }

    sizes
}

fn throughput_shaped_segment_sizes(
    total_size: u64,
    actual_segments: u64,
    min_segment_size: u64,
    hints: SegmentPlanningHints,
) -> Option<Vec<u64>> {
    let throughput_hint = hints
        .throughput_bytes_per_second
        .filter(|value| *value > 0)?;
    if actual_segments <= 1 {
        return None;
    }

    let equal_segment_size = total_size / actual_segments.max(1);
    let target_chunk_time_ms =
        u64::from(hints.target_chunk_time_seconds.max(1)).saturating_mul(1_000);
    let effective_chunk_time_ms =
        target_chunk_time_ms.saturating_add(hints.ttfb_ms.unwrap_or(0).min(target_chunk_time_ms));
    let per_connection_throughput = throughput_hint.div_ceil(actual_segments);
    let target_window_bytes = per_connection_throughput
        .saturating_mul(effective_chunk_time_ms)
        .div_ceil(1_000)
        .max(min_segment_size);

    if target_window_bytes >= equal_segment_size {
        return None;
    }

    let mut sizes =
        vec![target_window_bytes; usize::try_from(actual_segments).unwrap_or(usize::MAX)];
    let assigned = target_window_bytes.saturating_mul(actual_segments);
    let remaining = total_size.saturating_sub(assigned);
    if remaining == 0 {
        return Some(sizes);
    }

    let weight_sum = actual_segments.saturating_mul(actual_segments.saturating_add(1)) / 2;
    let mut distributed = 0_u64;
    for (idx, size) in sizes.iter_mut().enumerate() {
        let weight = u64::try_from(idx).unwrap_or(u64::MAX).saturating_add(1);
        let extra = remaining.saturating_mul(weight) / weight_sum.max(1);
        *size = size.saturating_add(extra);
        distributed = distributed.saturating_add(extra);
    }
    if let Some(last) = sizes.last_mut() {
        *last = last.saturating_add(remaining.saturating_sub(distributed));
    }

    Some(sizes)
}

fn build_segments_from_sizes(segment_sizes: &[u64], retry_budget: u32) -> Vec<DownloadSegment> {
    let mut start = 0_u64;
    let mut segments = Vec::with_capacity(segment_sizes.len());

    for (idx, size) in segment_sizes.iter().copied().enumerate() {
        let end = start.saturating_add(size.saturating_sub(1));
        segments.push(pending_segment(
            idx as u32,
            start as i64,
            end as i64,
            retry_budget,
        ));
        start = end.saturating_add(1);
    }

    segments
}

/// Split an existing segment at the given byte offset for idle-worker stealing.
pub fn split_segment(
    segment: &DownloadSegment,
    split_offset: i64,
    min_segment_size: i64,
    new_segment_id: u32,
) -> Option<(DownloadSegment, DownloadSegment)> {
    if min_segment_size <= 0 {
        return None;
    }

    let consumed = segment.downloaded.max(0);
    let current_start = segment.start.saturating_add(consumed);
    if split_offset <= current_start || split_offset > segment.end {
        return None;
    }

    let left_remaining = split_offset.saturating_sub(current_start);
    let right_remaining = segment.end.saturating_sub(split_offset).saturating_add(1);
    if left_remaining < min_segment_size || right_remaining < min_segment_size {
        return None;
    }

    let new_segment = pending_segment(
        new_segment_id,
        split_offset,
        segment.end,
        segment.retry_budget,
    );

    let modified = DownloadSegment {
        end: split_offset - 1,
        ..segment.clone()
    };

    Some((new_segment, modified))
}
