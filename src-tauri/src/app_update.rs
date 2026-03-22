use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::model::{AppUpdateInfo, AppUpdateProgressEvent};

pub const APP_UPDATE_PROGRESS_EVENT: &str = "app://update-progress";

fn to_update_info(update: &Update) -> AppUpdateInfo {
    AppUpdateInfo {
        version: update.version.to_string(),
        current_version: update.current_version.to_string(),
        notes: update.body.clone(),
    }
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

pub async fn install_update(app: &AppHandle) -> Result<AppUpdateInfo, String> {
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
