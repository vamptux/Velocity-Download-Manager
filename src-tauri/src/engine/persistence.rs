use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension};

use crate::model::RegistrySnapshot;

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
    path.with_extension(format!("snapshot.corrupt.{millis}.json"))
}
