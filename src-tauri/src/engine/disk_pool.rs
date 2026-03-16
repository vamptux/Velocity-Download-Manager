use std::fs::File;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc;

use crate::engine::disk::write_at_offset;

pub struct WriteBlock {
    pub file: Arc<File>,
    pub buffer: Vec<u8>,
    pub offset: u64,
}

pub struct DiskPool {
    sender: mpsc::Sender<WriteBlock>,
    capacity: usize,
    queue_depth: Arc<AtomicUsize>,
    pending_writes: Arc<AtomicUsize>,
    write_error: Arc<Mutex<Option<String>>>,
}

impl DiskPool {
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = mpsc::channel::<WriteBlock>(capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        let queue_depth = Arc::new(AtomicUsize::new(0));
        let pending_writes = Arc::new(AtomicUsize::new(0));
        let write_error = Arc::new(Mutex::new(None));
        let worker_count = thread::available_parallelism()
            .map(|parallelism| disk_worker_count_from_parallelism(parallelism.get()))
            .unwrap_or(2);

        for worker_index in 0..worker_count {
            let receiver = Arc::clone(&receiver);
            let depth = Arc::clone(&queue_depth);
            let pending = Arc::clone(&pending_writes);
            let write_error = Arc::clone(&write_error);
            thread::Builder::new()
                .name(format!("disk-io-worker-{worker_index}"))
                .spawn(move || loop {
                    let next_block = {
                        let mut receiver = receiver.lock().expect("disk receiver poisoned");
                        receiver.blocking_recv()
                    };
                    let Some(block) = next_block else {
                        break;
                    };
                    depth.fetch_sub(1, Ordering::Relaxed);
                    if let Err(error) = write_at_offset(&block.file, &block.buffer, block.offset) {
                        if let Ok(mut slot) = write_error.lock() {
                            if slot.is_none() {
                                *slot = Some(error.to_string());
                            }
                        }
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

    pub fn is_queue_utilization_at_least(&self, threshold_percent: usize) -> bool {
        self.capacity > 0 && self.queue_utilization_percent() >= threshold_percent.min(100)
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
    ) -> Result<(), mpsc::error::SendError<WriteBlock>> {
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
        self.pending_writes.fetch_add(1, Ordering::Relaxed);
        let result = self.sender.send(block).await;
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
        queue_depth.saturating_mul(100) / capacity
    }
}

fn disk_worker_count_from_parallelism(parallelism: usize) -> usize {
    (parallelism / 2).clamp(2, 16)
}

#[cfg(test)]
mod tests {
    use super::{disk_worker_count_from_parallelism, queue_utilization_percent_from_counts};

    #[test]
    fn queue_utilization_handles_zero_capacity() {
        assert_eq!(queue_utilization_percent_from_counts(4, 0), 0);
    }

    #[test]
    fn queue_utilization_uses_saturating_percentage() {
        assert_eq!(queue_utilization_percent_from_counts(0, 256), 0);
        assert_eq!(queue_utilization_percent_from_counts(64, 256), 25);
        assert_eq!(queue_utilization_percent_from_counts(192, 256), 75);
        assert_eq!(queue_utilization_percent_from_counts(256, 256), 100);
    }

    #[test]
    fn disk_worker_count_scales_with_parallelism() {
        assert_eq!(disk_worker_count_from_parallelism(1), 2);
        assert_eq!(disk_worker_count_from_parallelism(4), 2);
        assert_eq!(disk_worker_count_from_parallelism(8), 4);
        assert_eq!(disk_worker_count_from_parallelism(32), 16);
        assert_eq!(disk_worker_count_from_parallelism(64), 16);
    }
}
