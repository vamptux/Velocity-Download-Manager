use std::fs::File;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::engine::disk::write_at_offset;

const DISK_QUEUE_PRESSURE_WARM_PERCENT: usize = 55;
const DISK_QUEUE_PRESSURE_ELEVATED_PERCENT: usize = 70;
const DISK_QUEUE_PRESSURE_HIGH_PERCENT: usize = 85;
const DISK_QUEUE_PRESSURE_CRITICAL_PERCENT: usize = 92;
const DISK_WRITE_LATENCY_WARM_US: u64 = 12_000;
const DISK_WRITE_LATENCY_ELEVATED_US: u64 = 24_000;
const DISK_WRITE_LATENCY_HIGH_US: u64 = 60_000;
const DISK_WRITE_LATENCY_CRITICAL_US: u64 = 150_000;
const DISK_WRITE_LATENCY_SUSTAINED_STREAK: usize = 4;
const DISK_WRITE_LATENCY_STREAK_FAST_DECAY_STEP: usize = 2;
const DISK_WRITE_LATENCY_STREAK_MAX: usize = DISK_WRITE_LATENCY_SUSTAINED_STREAK * 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QueuePressureTier {
    Normal,
    Warm,
    Elevated,
    High,
    Critical,
}

pub struct WriteBlock {
    pub file: Arc<File>,
    pub buffer: Vec<u8>,
    pub offset: u64,
}

pub struct DiskPool {
    sender: flume::Sender<WriteBlock>,
    capacity: usize,
    worker_count: usize,
    queue_depth: Arc<AtomicUsize>,
    pending_writes: Arc<AtomicUsize>,
    average_write_micros: Arc<AtomicU64>,
    last_write_micros: Arc<AtomicU64>,
    slow_write_streak: Arc<AtomicUsize>,
    write_error: Arc<Mutex<Option<String>>>,
}

impl DiskPool {
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = flume::bounded::<WriteBlock>(capacity);
        let queue_depth = Arc::new(AtomicUsize::new(0));
        let pending_writes = Arc::new(AtomicUsize::new(0));
        let average_write_micros = Arc::new(AtomicU64::new(0));
        let last_write_micros = Arc::new(AtomicU64::new(0));
        let slow_write_streak = Arc::new(AtomicUsize::new(0));
        let write_error = Arc::new(Mutex::new(None));
        let worker_count = thread::available_parallelism()
            .map(|parallelism| disk_worker_count_from_parallelism(parallelism.get()))
            .unwrap_or(2);
        let mut spawned_worker_count = 0;

        for worker_index in 0..worker_count {
            let receiver = receiver.clone();
            let depth = Arc::clone(&queue_depth);
            let pending = Arc::clone(&pending_writes);
            let average_write_micros = Arc::clone(&average_write_micros);
            let last_write_micros = Arc::clone(&last_write_micros);
            let slow_write_streak = Arc::clone(&slow_write_streak);
            let worker_write_error = Arc::clone(&write_error);
            let spawn_result = thread::Builder::new()
                .name(format!("disk-io-worker-{worker_index}"))
                .spawn(move || {
                    loop {
                        let Ok(block) = receiver.recv() else {
                            break;
                        };
                        depth.fetch_sub(1, Ordering::Relaxed);
                        let started_at = Instant::now();
                        let write_result = write_at_offset(&block.file, &block.buffer, block.offset);
                        note_write_latency(
                            &average_write_micros,
                            &last_write_micros,
                            &slow_write_streak,
                            started_at.elapsed().as_micros().min(u64::MAX as u128) as u64,
                        );
                        if let Err(error) = write_result
                            && let Ok(mut slot) = worker_write_error.lock()
                            && slot.is_none()
                        {
                            *slot = Some(error.to_string());
                        }
                        pending.fetch_sub(1, Ordering::Relaxed);
                    }
                });
            match spawn_result {
                Ok(_) => {
                    spawned_worker_count += 1;
                }
                Err(error) => {
                    if let Ok(mut slot) = write_error.lock()
                        && slot.is_none()
                    {
                        *slot = Some(format!(
                            "Failed to spawn disk IO worker thread {worker_index}: {error}"
                        ));
                    }
                    break;
                }
            }
        }

        Self {
            sender,
            capacity,
            worker_count: spawned_worker_count,
            queue_depth,
            pending_writes,
            average_write_micros,
            last_write_micros,
            slow_write_streak,
            write_error,
        }
    }

    pub fn queue_depth(&self) -> usize {
        self.queue_depth.load(Ordering::Relaxed)
    }

    pub fn queue_utilization_percent(&self) -> usize {
        queue_utilization_percent_from_counts(self.queue_depth(), self.capacity)
    }

    pub fn pressure_tier(&self) -> QueuePressureTier {
        queue_pressure_tier_from_utilization(self.queue_utilization_percent()).max(
            write_latency_pressure_tier(
                self.pending_writes(),
                self.worker_count,
                self.average_write_micros.load(Ordering::Relaxed),
                self.last_write_micros.load(Ordering::Relaxed),
                self.slow_write_streak.load(Ordering::Relaxed),
            ),
        )
    }

    pub fn under_pressure(&self) -> bool {
        self.pressure_tier() >= QueuePressureTier::Warm
    }

    pub fn blocks_new_supply(&self) -> bool {
        self.pressure_tier() >= QueuePressureTier::Critical
    }

    pub fn recommended_parallelism(&self, requested_parallelism: u32) -> u32 {
        recommended_parallelism_for_pressure(requested_parallelism, self.pressure_tier())
    }

    pub fn pending_writes(&self) -> usize {
        self.pending_writes.load(Ordering::Relaxed)
    }

    pub fn take_error(&self) -> Option<String> {
        self.write_error
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    pub async fn enqueue_write(
        &self,
        block: WriteBlock,
    ) -> Result<(), flume::SendError<WriteBlock>> {
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
        self.pending_writes.fetch_add(1, Ordering::Relaxed);
        let result = self.sender.send_async(block).await;
        if result.is_err() {
            self.queue_depth.fetch_sub(1, Ordering::Relaxed);
            self.pending_writes.fetch_sub(1, Ordering::Relaxed);
        }
        result
    }
}

fn queue_utilization_percent_from_counts(queue_depth: usize, capacity: usize) -> usize {
    if capacity == 0 {
        0
    } else {
        (queue_depth.saturating_mul(100) / capacity).min(100)
    }
}

fn queue_pressure_tier_from_utilization(utilization_percent: usize) -> QueuePressureTier {
    match utilization_percent {
        DISK_QUEUE_PRESSURE_CRITICAL_PERCENT..=100 => QueuePressureTier::Critical,
        DISK_QUEUE_PRESSURE_HIGH_PERCENT.. => QueuePressureTier::High,
        DISK_QUEUE_PRESSURE_ELEVATED_PERCENT.. => QueuePressureTier::Elevated,
        DISK_QUEUE_PRESSURE_WARM_PERCENT.. => QueuePressureTier::Warm,
        _ => QueuePressureTier::Normal,
    }
}

fn write_latency_pressure_tier(
    pending_writes: usize,
    worker_count: usize,
    average_write_micros: u64,
    last_write_micros: u64,
    slow_write_streak: usize,
) -> QueuePressureTier {
    if pending_writes == 0 {
        return QueuePressureTier::Normal;
    }

    let has_backlog = pending_writes > worker_count.max(1);
    let sustained_pressure = slow_write_streak >= DISK_WRITE_LATENCY_SUSTAINED_STREAK;
    if !has_backlog && !sustained_pressure {
        return QueuePressureTier::Normal;
    }

    if average_write_micros >= DISK_WRITE_LATENCY_CRITICAL_US
        || (last_write_micros >= DISK_WRITE_LATENCY_CRITICAL_US && slow_write_streak >= 2)
    {
        return QueuePressureTier::Critical;
    }
    if average_write_micros >= DISK_WRITE_LATENCY_HIGH_US
        || (last_write_micros >= DISK_WRITE_LATENCY_HIGH_US && sustained_pressure)
    {
        return QueuePressureTier::High;
    }
    if average_write_micros >= DISK_WRITE_LATENCY_ELEVATED_US
        || slow_write_streak >= DISK_WRITE_LATENCY_SUSTAINED_STREAK.saturating_mul(2)
    {
        return QueuePressureTier::Elevated;
    }
    if average_write_micros >= DISK_WRITE_LATENCY_WARM_US || sustained_pressure {
        return QueuePressureTier::Warm;
    }

    QueuePressureTier::Normal
}

fn note_write_latency(
    average_write_micros: &AtomicU64,
    last_write_micros: &AtomicU64,
    slow_write_streak: &AtomicUsize,
    elapsed_micros: u64,
) {
    last_write_micros.store(elapsed_micros, Ordering::Relaxed);
    update_average_write_micros(average_write_micros, elapsed_micros);
    update_slow_write_streak(slow_write_streak, elapsed_micros);
}

fn update_slow_write_streak(slow_write_streak: &AtomicUsize, elapsed_micros: u64) {
    let _ = slow_write_streak.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |previous| {
        let next = match elapsed_micros {
            DISK_WRITE_LATENCY_CRITICAL_US.. => previous.saturating_add(3),
            DISK_WRITE_LATENCY_HIGH_US.. => previous.saturating_add(2),
            DISK_WRITE_LATENCY_ELEVATED_US.. => previous.saturating_add(1),
            DISK_WRITE_LATENCY_WARM_US.. => previous.saturating_sub(1),
            _ => previous.saturating_sub(DISK_WRITE_LATENCY_STREAK_FAST_DECAY_STEP),
        }
        .min(DISK_WRITE_LATENCY_STREAK_MAX);

        Some(next)
    });
}

fn update_average_write_micros(average_write_micros: &AtomicU64, elapsed_micros: u64) {
    let mut previous = average_write_micros.load(Ordering::Relaxed);
    loop {
        let next = if previous == 0 {
            elapsed_micros
        } else {
            previous
                .saturating_mul(7)
                .saturating_add(elapsed_micros)
                / 8
        };
        match average_write_micros.compare_exchange_weak(
            previous,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(observed) => previous = observed,
        }
    }
}

pub fn chunk_buffer_target_window_ms_for_signals(
    queue_utilization_percent: usize,
    pressure_tier: QueuePressureTier,
) -> u64 {
    let queue_window_ms = match queue_utilization_percent {
        85..=100 => 128_u64,
        70..=84 => 96_u64,
        55..=69 => 64_u64,
        0..=34 => 32_u64,
        _ => 48_u64,
    };
    let pressure_window_ms = match pressure_tier {
        QueuePressureTier::Normal => 32_u64,
        QueuePressureTier::Warm => 64_u64,
        QueuePressureTier::Elevated => 96_u64,
        QueuePressureTier::High | QueuePressureTier::Critical => 128_u64,
    };

    queue_window_ms.max(pressure_window_ms)
}

pub fn adaptive_chunk_buffer_size(
    base_chunk_buffer_size: usize,
    throughput_bytes_per_second: u64,
    floor: usize,
    ceiling: usize,
    queue_utilization_percent: usize,
    pressure_tier: QueuePressureTier,
) -> usize {
    if throughput_bytes_per_second == 0 {
        return base_chunk_buffer_size.clamp(floor, ceiling);
    }

    let target_window_ms =
        chunk_buffer_target_window_ms_for_signals(queue_utilization_percent, pressure_tier);
    let target = throughput_bytes_per_second
        .saturating_mul(target_window_ms)
        .div_ceil(1_000);
    usize::try_from(target)
        .unwrap_or(usize::MAX)
        .clamp(floor, ceiling)
}

fn recommended_parallelism_for_pressure(
    requested_parallelism: u32,
    pressure_tier: QueuePressureTier,
) -> u32 {
    let requested_parallelism = requested_parallelism.max(1);
    match pressure_tier {
        QueuePressureTier::Normal | QueuePressureTier::Warm => requested_parallelism,
        QueuePressureTier::Elevated => requested_parallelism.saturating_sub(1).max(1),
        QueuePressureTier::High => {
            let reduction = requested_parallelism.div_ceil(3);
            requested_parallelism.saturating_sub(reduction).max(1)
        }
        QueuePressureTier::Critical => 1,
    }
}

fn disk_worker_count_from_parallelism(parallelism: usize) -> usize {
    // Limit disk workers to 4 to prevent I/O thrashing and severe fragmentation on HDDs,
    // while still allowing sufficient parallelism for fast NVMe drives.
    (parallelism / 4).clamp(1, 4)
}
