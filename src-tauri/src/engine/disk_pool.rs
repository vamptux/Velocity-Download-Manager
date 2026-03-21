use std::fs::File;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::engine::disk::write_at_offset;

const DISK_QUEUE_PRESSURE_WARM_PERCENT: usize = 55;
const DISK_QUEUE_PRESSURE_ELEVATED_PERCENT: usize = 70;
const DISK_QUEUE_PRESSURE_HIGH_PERCENT: usize = 85;
const DISK_QUEUE_PRESSURE_CRITICAL_PERCENT: usize = 92;

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
    queue_depth: Arc<AtomicUsize>,
    pending_writes: Arc<AtomicUsize>,
    write_error: Arc<Mutex<Option<String>>>,
}

impl DiskPool {
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = flume::bounded::<WriteBlock>(capacity);
        let queue_depth = Arc::new(AtomicUsize::new(0));
        let pending_writes = Arc::new(AtomicUsize::new(0));
        let write_error = Arc::new(Mutex::new(None));
        let worker_count = thread::available_parallelism()
            .map(|parallelism| disk_worker_count_from_parallelism(parallelism.get()))
            .unwrap_or(2);

        for worker_index in 0..worker_count {
            let receiver = receiver.clone();
            let depth = Arc::clone(&queue_depth);
            let pending = Arc::clone(&pending_writes);
            let write_error = Arc::clone(&write_error);
            thread::Builder::new()
                .name(format!("disk-io-worker-{worker_index}"))
                .spawn(move || loop {
                    let Ok(block) = receiver.recv() else {
                        break;
                    };
                    depth.fetch_sub(1, Ordering::Relaxed);
                    if let Err(error) = write_at_offset(&block.file, &block.buffer, block.offset)
                        && let Ok(mut slot) = write_error.lock()
                            && slot.is_none() {
                                *slot = Some(error.to_string());
                            }
                    pending.fetch_sub(1, Ordering::Relaxed);
                })
                .expect("Failed to spawn IO worker thread");
        }

        Self {
            sender,
            capacity,
            queue_depth,
            pending_writes,
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
        queue_pressure_tier_from_utilization(self.queue_utilization_percent())
    }

    pub fn under_pressure(&self) -> bool {
        self.pressure_tier() >= QueuePressureTier::Warm
    }

    pub fn blocks_new_supply(&self) -> bool {
        self.pressure_tier() >= QueuePressureTier::Critical
    }

    pub fn recommended_parallelism(&self, requested_parallelism: u32) -> u32 {
        recommended_parallelism_for_pressure(
            requested_parallelism,
            self.queue_utilization_percent(),
        )
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

fn recommended_parallelism_for_pressure(
    requested_parallelism: u32,
    utilization_percent: usize,
) -> u32 {
    let requested_parallelism = requested_parallelism.max(1);
    match queue_pressure_tier_from_utilization(utilization_percent) {
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
    (parallelism / 2).clamp(2, 16)
}
