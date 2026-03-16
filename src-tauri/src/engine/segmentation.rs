use crate::model::{DownloadSegment, DownloadSegmentStatus};

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

/// Divide a file into sequential pending segments while respecting the minimum segment size.
pub fn compute_segments(
    total_size: u64,
    num_segments: u32,
    min_segment_size: u64,
    retry_budget: u32,
) -> Vec<DownloadSegment> {
    if total_size == 0 || num_segments == 0 {
        return Vec::new();
    }

    let num_segments = num_segments as u64;
    let actual_segments = std::cmp::min(
        num_segments,
        std::cmp::max(1, total_size / min_segment_size),
    );

    if actual_segments == 0 {
        return Vec::new();
    }

    let segment_size = total_size / actual_segments;
    let mut segments = Vec::with_capacity(usize::try_from(actual_segments).unwrap_or(usize::MAX));

    for (id_idx, i) in (0..actual_segments).enumerate() {
        let start = i * segment_size;
        let end = if i == actual_segments - 1 {
            total_size - 1
        } else {
            (i + 1) * segment_size - 1
        };

        segments.push(pending_segment(
            id_idx as u32,
            start as i64,
            end as i64,
            retry_budget,
        ));
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
