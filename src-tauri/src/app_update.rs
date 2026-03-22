use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::model::{
    AppUpdateInfo, AppUpdateProgressEvent, AppUpdateStartupHealth,
    AppUpdateStartupHealthStatus, EngineSettings,
};

pub const APP_UPDATE_PROGRESS_EVENT: &str = "app://update-progress";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingAppUpdateTransition {
    from_version: String,
    target_version: String,
    recorded_at: i64,
    notes: Option<String>,
    settings: EngineSettings,
}

#[derive(Debug, Default)]
pub struct StartupHealthEvaluation {
    pub health: Option<AppUpdateStartupHealth>,
    pub settings_restored: bool,
}

fn to_update_info(update: &Update) -> AppUpdateInfo {
    AppUpdateInfo {
        version: update.version.to_string(),
        current_version: update.current_version.to_string(),
        notes: update.body.clone(),
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}

fn pending_update_transition_path(app: &AppHandle) -> Result<PathBuf, String> {
    let base_dir = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("vdm"));
    fs::create_dir_all(&base_dir).map_err(|error| {
        format!(
            "Failed creating updater state directory '{}': {error}",
            base_dir.display()
        )
    })?;
    Ok(base_dir.join("pending-update-transition.json"))
}

fn load_pending_update_transition(
    app: &AppHandle,
) -> Result<Option<PendingAppUpdateTransition>, String> {
    let path = pending_update_transition_path(app)?;
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("Failed reading updater state '{}': {error}", path.display()))?;
    let pending = serde_json::from_str::<PendingAppUpdateTransition>(&raw).map_err(|error| {
        format!(
            "Failed parsing updater state '{}': {error}",
            path.display()
        )
    })?;
    Ok(Some(pending))
}

fn persist_pending_update_transition(
    app: &AppHandle,
    info: &AppUpdateInfo,
    settings: &EngineSettings,
) -> Result<(), String> {
    let path = pending_update_transition_path(app)?;
    let payload = PendingAppUpdateTransition {
        from_version: info.current_version.clone(),
        target_version: info.version.clone(),
        recorded_at: now_millis(),
        notes: info.notes.clone(),
        settings: settings.clone(),
    };
    let raw = serde_json::to_string_pretty(&payload)
        .map_err(|error| format!("Failed serializing updater transition metadata: {error}"))?;
    fs::write(&path, raw)
        .map_err(|error| format!("Failed writing updater state '{}': {error}", path.display()))
}

fn clear_pending_update_transition(app: &AppHandle) -> Result<(), String> {
    let path = pending_update_transition_path(app)?;
    if !path.exists() {
        return Ok(());
    }

    fs::remove_file(&path)
        .map_err(|error| format!("Failed removing updater state '{}': {error}", path.display()))
}

fn app_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

pub fn pending_startup_health(app: &AppHandle) -> Option<AppUpdateStartupHealth> {
    let pending = load_pending_update_transition(app).ok().flatten()?;
    Some(AppUpdateStartupHealth {
        status: AppUpdateStartupHealthStatus::Pending,
        from_version: pending.from_version,
        target_version: pending.target_version,
        observed_version: app_version(app),
        checked_at: pending.recorded_at,
        message: Some(
            "Validating the updated build and confirming persisted engine settings."
                .to_string(),
        ),
    })
}

pub fn finalize_startup_health(
    app: &AppHandle,
    settings: Option<&mut EngineSettings>,
    bootstrap_error: Option<&str>,
) -> Result<StartupHealthEvaluation, String> {
    let Some(pending) = load_pending_update_transition(app)? else {
        return Ok(StartupHealthEvaluation::default());
    };

    let observed_version = app_version(app);
    let checked_at = now_millis();
    let mut settings_restored = false;

    let (status, message) = if observed_version != pending.target_version {
        (
            AppUpdateStartupHealthStatus::Failed,
            Some(format!(
                "Expected version {} after restart, but the app started as {}.",
                pending.target_version, observed_version
            )),
        )
    } else if let Some(error) = bootstrap_error {
        (
            AppUpdateStartupHealthStatus::Failed,
            Some(format!(
                "The updated build started, but engine bootstrap failed: {error}"
            )),
        )
    } else if let Some(current_settings) = settings {
        let default_settings = EngineSettings::default();
        if *current_settings == default_settings && pending.settings != default_settings {
            *current_settings = pending.settings.clone();
            settings_restored = true;
            (
                AppUpdateStartupHealthStatus::RestoredSettings,
                Some(
                    "The updated build started with default engine settings, so VDM restored your previous transfer profile."
                        .to_string(),
                ),
            )
        } else {
            (
                AppUpdateStartupHealthStatus::Healthy,
                Some(
                    "The updated build passed its first-start health check and kept your engine settings intact."
                        .to_string(),
                ),
            )
        }
    } else {
        (
            AppUpdateStartupHealthStatus::Failed,
            Some(
                "VDM could not inspect persisted engine settings after the update finished."
                    .to_string(),
            ),
        )
    };

    clear_pending_update_transition(app)?;

    Ok(StartupHealthEvaluation {
        health: Some(AppUpdateStartupHealth {
            status,
            from_version: pending.from_version,
            target_version: pending.target_version,
            observed_version,
            checked_at,
            message,
        }),
        settings_restored,
    })
}

pub async fn check_for_update(
    app: &AppHandle,
    skipped_version: Option<&str>,
) -> Result<Option<AppUpdateInfo>, String> {
    let update = app
        .updater_builder()
        .build()
        .map_err(|error| format!("Failed preparing the updater: {error}"))?
        .check()
        .await
        .map_err(|error| format!("Failed checking for updates: {error}"))?;

    Ok(update.as_ref().and_then(|candidate| {
        let info = to_update_info(candidate);
        match skipped_version {
            Some(skipped) if skipped == info.version => None,
            _ => Some(info),
        }
    }))
}

pub async fn install_update(
    app: &AppHandle,
    settings: &EngineSettings,
) -> Result<AppUpdateInfo, String> {
    let Some(update) = app
        .updater_builder()
        .build()
        .map_err(|error| format!("Failed preparing the updater: {error}"))?
        .check()
        .await
        .map_err(|error| format!("Failed checking for updates: {error}"))?
    else {
        return Err("Velocity Download Manager is already up to date.".to_string());
    };

    let info = to_update_info(&update);
    persist_pending_update_transition(app, &info, settings)?;
    let mut started = false;
    let started_app = app.clone();
    let progress_app = app.clone();
    let finished_app = app.clone();

    update
        .download_and_install(
            move |chunk_length, content_length| {
                if !started {
                    started = true;
                    let _ = started_app.emit(
                        APP_UPDATE_PROGRESS_EVENT,
                        AppUpdateProgressEvent::Started { content_length },
                    );
                }

                let _ = progress_app.emit(
                    APP_UPDATE_PROGRESS_EVENT,
                    AppUpdateProgressEvent::Progress {
                        chunk_length: chunk_length as u64,
                    },
                );
            },
            move || {
                let _ =
                    finished_app.emit(APP_UPDATE_PROGRESS_EVENT, AppUpdateProgressEvent::Finished);
            },
        )
        .await
        .map_err(|error| format!("Failed installing the update: {error}"))?;

    Ok(info)
}
