use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::model::RegistrySnapshot;

use super::persistence::persist_registry_snapshot;

const COALESCE_WINDOW_MS: u64 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PersistPriority {
    Deferred,
    Flush,
}

#[derive(Debug)]
struct PersistRequest {
    snapshot: RegistrySnapshot,
    ack: Option<Sender<Result<(), String>>>,
}

pub(super) struct SnapshotPersistQueue {
    sender: Sender<PersistRequest>,
}

impl SnapshotPersistQueue {
    pub(super) fn new(snapshot_path: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel::<PersistRequest>();
        let _ = thread::Builder::new()
            .name("vdm-snapshot-writer".to_string())
            .spawn(move || run_snapshot_writer(snapshot_path, receiver));
        Self { sender }
    }

    pub(super) fn persist(
        &self,
        snapshot: &RegistrySnapshot,
        priority: PersistPriority,
    ) -> Result<(), String> {
        match priority {
            PersistPriority::Deferred => self
                .sender
                .send(PersistRequest {
                    snapshot: snapshot.clone(),
                    ack: None,
                })
                .map_err(|_| "Snapshot writer is unavailable.".to_string()),
            PersistPriority::Flush => {
                let (ack_sender, ack_receiver) = mpsc::channel();
                self.sender
                    .send(PersistRequest {
                        snapshot: snapshot.clone(),
                        ack: Some(ack_sender),
                    })
                    .map_err(|_| "Snapshot writer is unavailable.".to_string())?;
                ack_receiver
                    .recv()
                    .map_err(|_| "Snapshot writer acknowledgement failed.".to_string())?
            }
        }
    }
}

fn run_snapshot_writer(snapshot_path: PathBuf, receiver: Receiver<PersistRequest>) {
    let mut disconnected = false;
    loop {
        let mut request = match receiver.recv() {
            Ok(request) => request,
            Err(_) => break,
        };

        let mut latest_snapshot = request.snapshot;
        let mut acknowledgements: Vec<Sender<Result<(), String>>> = Vec::new();
        if let Some(ack) = request.ack.take() {
            acknowledgements.push(ack);
        }

        if acknowledgements.is_empty() {
            loop {
                let next_request = receiver.recv_timeout(Duration::from_millis(COALESCE_WINDOW_MS));
                match next_request {
                    Ok(mut next) => {
                        latest_snapshot = next.snapshot;
                        if let Some(ack) = next.ack.take() {
                            acknowledgements.push(ack);
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        loop {
            let mut next = match receiver.try_recv() {
                Ok(next) => next,
                Err(_) => break,
            };
            latest_snapshot = next.snapshot;
            if let Some(ack) = next.ack.take() {
                acknowledgements.push(ack);
            }
        }

        let result = persist_registry_snapshot(&snapshot_path, &latest_snapshot);
        for acknowledgement in acknowledgements {
            let _ = acknowledgement.send(clone_persist_result(&result));
        }

        if disconnected {
            break;
        }
    }
}

fn clone_persist_result(result: &Result<(), String>) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        Err(error) => Err(error.clone()),
    }
}
