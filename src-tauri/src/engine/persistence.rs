use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension};

use crate::model::{QueuePersistPressure, RegistrySnapshot};

const COALESCE_WINDOW_MS: u64 = 40;
const MAX_DEFERRED_QUEUE_DEPTH: usize = 32;
const MAX_CORRUPT_SNAPSHOT_BACKUPS: usize = 5;
const OVERFLOW_REPLACEMENT_LOG_INTERVAL: u64 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PersistPriority {
    Deferred,
    Flush,
}

#[derive(Debug)]
struct PendingPersist {
    snapshot: RegistrySnapshot,
    generation: u64,
    priority: PersistPriority,
    ack: Option<Sender<Result<PersistFlushAck, String>>>,
}

#[derive(Debug)]
enum PersistRequest {
    Snapshot(PendingPersist),
    DrainOverflow,
}

#[derive(Clone, Copy, Debug)]
struct PersistFlushAck {
    persisted_generation: u64,
}

#[derive(Debug)]
struct OverflowPersist {
    snapshot: RegistrySnapshot,
    generation: u64,
}

#[derive(Debug, Default)]
struct PersistQueueState {
    next_generation: AtomicU64,
    queued_deferred: AtomicUsize,
    max_deferred_depth: AtomicUsize,
    deferred_backpressure_events: AtomicU64,
    deferred_overflow_replacements: AtomicU64,
    last_enqueued_generation: AtomicU64,
    last_persisted_generation: AtomicU64,
    last_flush_ack_generation: AtomicU64,
    flush_ack_count: AtomicU64,
    overflow_snapshot: Mutex<Option<OverflowPersist>>,
    last_persist_error: Mutex<Option<String>>,
}

pub(super) struct SnapshotPersistQueue {
    sender: Sender<PersistRequest>,
    state: Arc<PersistQueueState>,
}

pub fn load_registry_snapshot(path: &Path) -> Result<Option<RegistrySnapshot>, String> {
    if !path.exists() {
        return Ok(None);
    }

    let connection = open_snapshot_connection(path)?;
    let payload = connection
        .query_row(
            "SELECT payload FROM registry_snapshot WHERE id = 1 LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| {
            format!(
                "Failed querying engine snapshot row at '{}': {error}",
                path.display()
            )
        })?;

    let Some(raw) = payload else {
        return Ok(None);
    };

    match serde_json::from_str::<RegistrySnapshot>(&raw) {
        Ok(snapshot) => Ok(Some(snapshot)),
        Err(error) => {
            let backup = corrupt_backup_path(path);
            fs::rename(path, &backup).map_err(|rename_error| {
                format!(
                    "Failed to move corrupted engine snapshot to '{}': {rename_error}",
                    backup.display()
                )
            })?;
            cleanup_corrupt_backups(path)?;
            Err(format!(
                "Engine snapshot was corrupted and moved to '{}': {error}",
                backup.display()
            ))
        }
    }
}

pub fn persist_registry_snapshot(path: &Path, snapshot: &RegistrySnapshot) -> Result<(), String> {
    let mut connection = open_snapshot_connection(path)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default();
    let payload = serde_json::to_string(snapshot)
        .map_err(|error| format!("Failed serializing engine snapshot: {error}"))?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Failed opening snapshot transaction: {error}"))?;
    transaction
        .execute(
            "INSERT INTO registry_snapshot (id, payload, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at",
            (&payload, now),
        )
        .map_err(|error| format!("Failed writing snapshot payload: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Failed committing snapshot transaction: {error}"))?;

    Ok(())
}

pub fn snapshot_path(base_dir: &Path) -> PathBuf {
    base_dir.join("engine-registry.snapshot.db")
}

impl PersistQueueState {
    fn next_generation(&self) -> u64 {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.last_enqueued_generation
            .store(generation, Ordering::Relaxed);
        generation
    }

    fn reserve_deferred_slot(&self) -> usize {
        let depth = self.queued_deferred.fetch_add(1, Ordering::Relaxed) + 1;
        self.max_deferred_depth.fetch_max(depth, Ordering::Relaxed);
        depth
    }

    fn release_deferred_slot(&self) {
        self.queued_deferred.fetch_sub(1, Ordering::Relaxed);
    }

    fn note_backpressure(&self, replaced_existing: bool) {
        self.deferred_backpressure_events
            .fetch_add(1, Ordering::Relaxed);
        if replaced_existing {
            self.deferred_overflow_replacements
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn stash_overflow_snapshot(
        &self,
        snapshot: RegistrySnapshot,
        generation: u64,
    ) -> Result<bool, String> {
        let mut overflow = self
            .overflow_snapshot
            .lock()
            .map_err(|_| "Snapshot persist overflow lock was poisoned.".to_string())?;
        let replaced_existing = overflow.is_some();
        *overflow = Some(OverflowPersist {
            snapshot,
            generation,
        });
        self.note_backpressure(replaced_existing);
        Ok(replaced_existing)
    }

    fn take_overflow_snapshot(&self) -> Result<Option<PendingPersist>, String> {
        let mut overflow = self
            .overflow_snapshot
            .lock()
            .map_err(|_| "Snapshot persist overflow lock was poisoned.".to_string())?;
        Ok(overflow.take().map(|overflow| PendingPersist {
            snapshot: overflow.snapshot,
            generation: overflow.generation,
            priority: PersistPriority::Deferred,
            ack: None,
        }))
    }

    fn note_persist_result(&self, generation: u64, result: &Result<(), String>) {
        if result.is_ok() {
            self.last_persisted_generation
                .store(generation, Ordering::Relaxed);
        }
        if let Ok(mut last_error) = self.last_persist_error.lock() {
            *last_error = result.as_ref().err().cloned();
        }
    }

    fn note_flush_ack(&self, generation: u64) {
        self.flush_ack_count.fetch_add(1, Ordering::Relaxed);
        self.last_flush_ack_generation
            .store(generation, Ordering::Relaxed);
    }
}

impl SnapshotPersistQueue {
    pub(super) fn new(snapshot_path: PathBuf) -> Self {
        let state = Arc::new(PersistQueueState::default());
        let (sender, receiver) = mpsc::channel::<PersistRequest>();
        let writer_state = Arc::clone(&state);
        let _ = thread::Builder::new()
            .name("vdm-snapshot-writer".to_string())
            .spawn(move || run_snapshot_writer(snapshot_path, receiver, writer_state));
        Self { sender, state }
    }

    pub(super) fn persist(
        &self,
        snapshot: &RegistrySnapshot,
        priority: PersistPriority,
    ) -> Result<(), String> {
        let generation = self.state.next_generation();
        match priority {
            PersistPriority::Deferred => {
                let depth = self.state.reserve_deferred_slot();
                if depth > MAX_DEFERRED_QUEUE_DEPTH {
                    self.state.release_deferred_slot();
                    let replaced_existing = self
                        .state
                        .stash_overflow_snapshot(snapshot.clone(), generation)?;
                    if replaced_existing {
                        let replacement_count = self
                            .state
                            .deferred_overflow_replacements
                            .load(Ordering::Relaxed);
                        if replacement_count == 1
                            || replacement_count
                                .is_multiple_of(OVERFLOW_REPLACEMENT_LOG_INTERVAL)
                        {
                            eprintln!(
                                "[VDM] snapshot persist queue saturated; replaced the overflow snapshot {} time(s). Latest queued generation: {}.",
                                replacement_count, generation
                            );
                        }
                    } else {
                        let backpressure_events = self
                            .state
                            .deferred_backpressure_events
                            .load(Ordering::Relaxed);
                        eprintln!(
                            "[VDM] snapshot persist queue saturated; spilling generation {} into the overflow slot (backpressure event {}).",
                            generation, backpressure_events
                        );
                    }
                    if !replaced_existing {
                        self.sender
                            .send(PersistRequest::DrainOverflow)
                            .map_err(|_| "Snapshot writer is unavailable.".to_string())?;
                    }
                    return Ok(());
                }

                let send_result = self.sender.send(PersistRequest::Snapshot(PendingPersist {
                    snapshot: snapshot.clone(),
                    generation,
                    priority,
                    ack: None,
                }));
                if send_result.is_err() {
                    self.state.release_deferred_slot();
                }
                send_result.map_err(|_| "Snapshot writer is unavailable.".to_string())
            }
            PersistPriority::Flush => {
                let (ack_sender, ack_receiver) = mpsc::channel();
                self.sender
                    .send(PersistRequest::Snapshot(PendingPersist {
                        snapshot: snapshot.clone(),
                        generation,
                        priority,
                        ack: Some(ack_sender),
                    }))
                    .map_err(|_| "Snapshot writer is unavailable.".to_string())?;
                let acknowledgement = ack_receiver
                    .recv()
                    .map_err(|_| "Snapshot writer acknowledgement failed.".to_string())??;
                if acknowledgement.persisted_generation < generation {
                    return Err(format!(
                        "Snapshot flush acknowledged generation {} before requested generation {}.",
                        acknowledgement.persisted_generation, generation
                    ));
                }
                self.state
                    .note_flush_ack(acknowledgement.persisted_generation);
                Ok(())
            }
        }
    }

    pub(super) fn telemetry_snapshot(&self) -> QueuePersistPressure {
        let queued_deferred = self.state.queued_deferred.load(Ordering::Relaxed);
        let max_deferred_depth = self.state.max_deferred_depth.load(Ordering::Relaxed);
        let backpressure_events = self.state.deferred_backpressure_events.load(Ordering::Relaxed);
        let overflow_replacements = self
            .state
            .deferred_overflow_replacements
            .load(Ordering::Relaxed);
        let overflow_pending = self
            .state
            .overflow_snapshot
            .lock()
            .map(|overflow| overflow.is_some())
            .unwrap_or(false);
        let last_persist_error = self
            .state
            .last_persist_error
            .lock()
            .map(|error| error.clone())
            .unwrap_or_else(|_| Some("Snapshot persist state lock was poisoned.".to_string()));

        QueuePersistPressure {
            pressure_active: queued_deferred > 0 || overflow_pending,
            queued_deferred: u32::try_from(queued_deferred).unwrap_or(u32::MAX),
            max_deferred_depth: u32::try_from(max_deferred_depth).unwrap_or(u32::MAX),
            backpressure_events,
            overflow_replacements,
            overflow_pending,
            last_persist_error,
        }
    }
}

fn open_snapshot_connection(path: &Path) -> Result<Connection, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Invalid snapshot path '{}'.", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Failed creating snapshot directory '{}': {error}",
            parent.display()
        )
    })?;
    let connection = Connection::open(path).map_err(|error| {
        format!(
            "Failed opening SQLite snapshot '{}': {error}",
            path.display()
        )
    })?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             CREATE TABLE IF NOT EXISTS registry_snapshot (
               id INTEGER PRIMARY KEY CHECK(id = 1),
               payload TEXT NOT NULL,
               updated_at INTEGER NOT NULL
             );",
        )
        .map_err(|error| format!("Failed initializing SQLite snapshot schema: {error}"))?;
    Ok(connection)
}

fn corrupt_backup_path(path: &Path) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("engine-registry.snapshot.db");
    path.with_file_name(format!("{file_name}.snapshot.corrupt.{millis}.db"))
}

fn cleanup_corrupt_backups(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Invalid snapshot path '{}'.", path.display()))?;
    let prefix = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| format!("{value}.snapshot.corrupt."))
        .ok_or_else(|| format!("Invalid snapshot file name '{}'.", path.display()))?;

    let mut backups = fs::read_dir(parent)
        .map_err(|error| {
            format!(
                "Failed reading snapshot backup directory '{}': {error}",
                parent.display()
            )
        })?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
        })
        .map(|entry| {
            let modified_at = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis())
                .unwrap_or_default();
            (modified_at, entry.path())
        })
        .collect::<Vec<_>>();

    let backup_count = backups.len();
    if backup_count <= MAX_CORRUPT_SNAPSHOT_BACKUPS {
        return Ok(());
    }

    backups.sort_by_key(|(modified_at, backup_path)| (*modified_at, backup_path.clone()));
    for (_, backup_path) in backups
        .into_iter()
        .take(backup_count.saturating_sub(MAX_CORRUPT_SNAPSHOT_BACKUPS))
    {
        fs::remove_file(&backup_path).map_err(|error| {
            format!(
                "Failed pruning corrupted snapshot backup '{}': {error}",
                backup_path.display()
            )
        })?;
    }

    Ok(())
}

fn run_snapshot_writer(
    snapshot_path: PathBuf,
    receiver: Receiver<PersistRequest>,
    state: Arc<PersistQueueState>,
) {
    let mut disconnected = false;
    loop {
        let mut request = match recv_next_persist(&receiver, &state, None, &mut disconnected) {
            Ok(Some(request)) => request,
            Ok(None) => break,
            Err(error) => {
                state.note_persist_result(
                    state.last_persisted_generation.load(Ordering::Relaxed),
                    &Err(error),
                );
                break;
            }
        };

        let mut latest_snapshot = request.snapshot;
        let mut latest_generation = request.generation;
        let mut acknowledgements: Vec<Sender<Result<PersistFlushAck, String>>> = Vec::new();
        if let Some(ack) = request.ack.take() {
            acknowledgements.push(ack);
        }

        if acknowledgements.is_empty() {
            loop {
                match recv_next_persist(
                    &receiver,
                    &state,
                    Some(Duration::from_millis(COALESCE_WINDOW_MS)),
                    &mut disconnected,
                ) {
                    Ok(Some(mut next)) => {
                        latest_snapshot = next.snapshot;
                        latest_generation = next.generation;
                        if let Some(ack) = next.ack.take() {
                            acknowledgements.push(ack);
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let result = Err(error);
                        state.note_persist_result(latest_generation, &result);
                        for acknowledgement in acknowledgements {
                            let _ = acknowledgement.send(clone_persist_result(
                                latest_generation,
                                &result,
                            ));
                        }
                        return;
                    }
                }
            }
        }

        loop {
            let mut next = match try_next_persist(&receiver, &state) {
                Ok(Some(next)) => next,
                Ok(None) => break,
                Err(error) => {
                    let result = Err(error);
                    state.note_persist_result(latest_generation, &result);
                    for acknowledgement in acknowledgements {
                        let _ = acknowledgement.send(clone_persist_result(
                            latest_generation,
                            &result,
                        ));
                    }
                    return;
                }
            };
            latest_snapshot = next.snapshot;
            latest_generation = next.generation;
            if let Some(ack) = next.ack.take() {
                acknowledgements.push(ack);
            }
        }

        let result = persist_registry_snapshot(&snapshot_path, &latest_snapshot);
        state.note_persist_result(latest_generation, &result);
        for acknowledgement in acknowledgements {
            let _ = acknowledgement.send(clone_persist_result(latest_generation, &result));
        }

        if disconnected {
            match state.take_overflow_snapshot() {
                Ok(Some(overflow)) => {
                    let result = persist_registry_snapshot(&snapshot_path, &overflow.snapshot);
                    state.note_persist_result(overflow.generation, &result);
                }
                Ok(None) | Err(_) => break,
            }
        }
    }
}

fn recv_next_persist(
    receiver: &Receiver<PersistRequest>,
    state: &PersistQueueState,
    timeout: Option<Duration>,
    disconnected: &mut bool,
) -> Result<Option<PendingPersist>, String> {
    loop {
        let request = match timeout {
            Some(duration) => match receiver.recv_timeout(duration) {
                Ok(request) => request,
                Err(RecvTimeoutError::Timeout) => return Ok(None),
                Err(RecvTimeoutError::Disconnected) => {
                    *disconnected = true;
                    return state.take_overflow_snapshot();
                }
            },
            None => match receiver.recv() {
                Ok(request) => request,
                Err(_) => {
                    *disconnected = true;
                    return state.take_overflow_snapshot();
                }
            },
        };

        match request {
            PersistRequest::Snapshot(pending) => {
                if pending.priority == PersistPriority::Deferred {
                    state.release_deferred_slot();
                }
                return Ok(Some(pending));
            }
            PersistRequest::DrainOverflow => {
                if let Some(overflow) = state.take_overflow_snapshot()? {
                    return Ok(Some(overflow));
                }
            }
        }
    }
}

fn try_next_persist(
    receiver: &Receiver<PersistRequest>,
    state: &PersistQueueState,
) -> Result<Option<PendingPersist>, String> {
    loop {
        match receiver.try_recv() {
            Ok(PersistRequest::Snapshot(pending)) => {
                if pending.priority == PersistPriority::Deferred {
                    state.release_deferred_slot();
                }
                return Ok(Some(pending));
            }
            Ok(PersistRequest::DrainOverflow) => {
                if let Some(overflow) = state.take_overflow_snapshot()? {
                    return Ok(Some(overflow));
                }
            }
            Err(_) => return state.take_overflow_snapshot(),
        }
    }
}

fn clone_persist_result(
    persisted_generation: u64,
    result: &Result<(), String>,
) -> Result<PersistFlushAck, String> {
    match result {
        Ok(()) => Ok(PersistFlushAck {
            persisted_generation,
        }),
        Err(error) => Err(error.clone()),
    }
}
