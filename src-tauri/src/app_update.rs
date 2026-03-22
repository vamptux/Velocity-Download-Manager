use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use tauri_plugin_updater::Error as UpdaterError;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::model::{
    AppUpdateChannel, AppUpdateInfo, AppUpdateProgressEvent, AppUpdateStartupHealth,
    AppUpdateStartupHealthStatus, EngineSettings,
};

pub const APP_UPDATE_PROGRESS_EVENT: &str = "app://update-progress";
const STABLE_UPDATE_ENDPOINT: &str =
    "https://github.com/vamptux/Velocity-Download-Manager/releases/latest/download/latest.json";
const PREVIEW_UPDATE_ENDPOINT: &str =
    "https://github.com/vamptux/Velocity-Download-Manager/releases/latest/download/latest-preview.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingAppUpdateTransition {
    channel: AppUpdateChannel,
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
    pub settings_changed: bool,
}

fn to_update_info(update: &Update, channel: &AppUpdateChannel) -> AppUpdateInfo {
    AppUpdateInfo {
        version: update.version.to_string(),
        current_version: update.current_version.to_string(),
        channel: channel.clone(),
        notes: update.body.clone(),
    }
}

fn updater_endpoints(channel: &AppUpdateChannel) -> Result<Vec<Url>, String> {
    let raw = match channel {
        AppUpdateChannel::Stable => vec![STABLE_UPDATE_ENDPOINT],
        // Preview first, then stable as a safe fallback when no preview manifest is published.
        AppUpdateChannel::Preview => vec![PREVIEW_UPDATE_ENDPOINT, STABLE_UPDATE_ENDPOINT],
    };

    raw.into_iter()
        .map(|value| {
            Url::parse(value).map_err(|error| {
                format!("Failed parsing updater endpoint '{value}': {error}")
            })
        })
        .collect()
}

fn updater_for_channel(
    app: &AppHandle,
    channel: &AppUpdateChannel,
) -> Result<tauri_plugin_updater::Updater, String> {
    let builder = app
        .updater_builder()
        .endpoints(updater_endpoints(channel)?)
        .map_err(|error| format!("Failed preparing updater endpoints: {error}"))?;

    builder
        .build()
        .map_err(|error| format!("Failed preparing the updater: {error}"))
}

fn apply_failed_update_guardrails(
    settings: &mut EngineSettings,
    pending: &PendingAppUpdateTransition,
) -> String {
    settings.skipped_update_version = Some(pending.target_version.clone());

    if pending.channel == AppUpdateChannel::Preview
        && settings.update_channel == AppUpdateChannel::Preview
    {
        settings.update_channel = AppUpdateChannel::Stable;
        return format!(
            "The first restart after updating to {} on the preview channel failed. VDM will skip that build on future checks and has switched the updater back to the stable channel.",
            pending.target_version
        );
    }

    format!(
        "The first restart after updating to {} failed. VDM will skip that build on future checks until a newer release is available.",
        pending.target_version
    )
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
        channel: info.channel.clone(),
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
        channel: pending.channel,
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
    let mut settings_changed = false;

    let (status, message) = if observed_version != pending.target_version {
        if let Some(current_settings) = settings {
            settings_changed = true;
            (
                AppUpdateStartupHealthStatus::RollbackTriggered,
                Some(format!(
                    "Expected version {} after restart, but the app started as {}. {}",
                    pending.target_version,
                    observed_version,
                    apply_failed_update_guardrails(current_settings, &pending)
                )),
            )
        } else {
            (
                AppUpdateStartupHealthStatus::Failed,
                Some(format!(
                    "Expected version {} after restart, but the app started as {}.",
                    pending.target_version, observed_version
                )),
            )
        }
    } else if let Some(error) = bootstrap_error {
        if let Some(current_settings) = settings {
            settings_changed = true;
            (
                AppUpdateStartupHealthStatus::RollbackTriggered,
                Some(format!(
                    "The updated build started, but engine bootstrap failed: {error} {}",
                    apply_failed_update_guardrails(current_settings, &pending)
                )),
            )
        } else {
            (
                AppUpdateStartupHealthStatus::Failed,
                Some(format!(
                    "The updated build started, but engine bootstrap failed: {error}"
                )),
            )
        }
    } else if let Some(current_settings) = settings {
        let default_settings = EngineSettings::default();
        if *current_settings == default_settings && pending.settings != default_settings {
            *current_settings = pending.settings.clone();
            settings_restored = true;
            settings_changed = true;
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
            channel: pending.channel,
            from_version: pending.from_version,
            target_version: pending.target_version,
            observed_version,
            checked_at,
            message,
        }),
        settings_restored,
        settings_changed,
    })
}

pub async fn check_for_update(
    app: &AppHandle,
    channel: &AppUpdateChannel,
    skipped_version: Option<&str>,
) -> Result<Option<AppUpdateInfo>, String> {
    let update = updater_for_channel(app, channel)?
        .check()
        .await
        .or_else(|error| {
            if matches!(channel, AppUpdateChannel::Preview)
                && matches!(error, UpdaterError::ReleaseNotFound)
            {
                Ok(None)
            } else {
                Err(error)
            }
        })
        .map_err(|error| format!("Failed checking for updates: {error}"))?;

    Ok(update.as_ref().and_then(|candidate| {
        let info = to_update_info(candidate, channel);
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
    let channel = settings.update_channel.clone();
    let Some(update) = updater_for_channel(app, &channel)?
        .check()
        .await
        .map_err(|error| format!("Failed checking for updates: {error}"))?
    else {
        return Err("Velocity Download Manager is already up to date.".to_string());
    };

    let info = to_update_info(&update, &channel);
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
