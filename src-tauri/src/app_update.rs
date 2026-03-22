use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::{Client, StatusCode, Url};
use semver::Version;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri_plugin_updater::Error as UpdaterError;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::model::{
    AppUpdateChannel, AppUpdateCheckResult, AppUpdateCheckStatus, AppUpdateInfo,
    AppUpdateProgressEvent, AppUpdateStartupHealth, AppUpdateStartupHealthStatus,
    EngineSettings,
};

pub const APP_UPDATE_PROGRESS_EVENT: &str = "app://update-progress";
const STABLE_UPDATE_ENDPOINT: &str =
    "https://github.com/vamptux/Velocity-Download-Manager/releases/latest/download/latest.json";
const PREVIEW_UPDATE_ENDPOINT: &str =
    "https://github.com/vamptux/Velocity-Download-Manager/releases/latest/download/latest-preview.json";
const GITHUB_RELEASES_PAGE: &str =
    "https://github.com/vamptux/Velocity-Download-Manager/releases";
const GITHUB_RELEASES_API_LATEST: &str =
    "https://api.github.com/repos/vamptux/Velocity-Download-Manager/releases/latest";
const GITHUB_RELEASES_API_LIST: &str =
    "https://api.github.com/repos/vamptux/Velocity-Download-Manager/releases?per_page=12";
const STABLE_UPDATE_MANIFEST_NAME: &str = "latest.json";
const PREVIEW_UPDATE_MANIFEST_NAME: &str = "latest-preview.json";
const UPDATER_TEMP_ARTIFACT_RETENTION: Duration = Duration::from_secs(60 * 60 * 24 * 3);
const LEGACY_UPDATES_DIR_RETENTION: Duration = Duration::from_secs(60 * 60 * 24 * 7);
const APP_PRODUCT_NAME: &str = "Velocity Download Manager";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GithubReleaseAsset {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug)]
struct GithubReleaseCandidate {
    release: GithubRelease,
    manifest_name: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
enum PendingAppUpdatePhase {
    #[default]
    Downloaded,
    AwaitingValidation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingAppUpdateTransition {
    #[serde(default)]
    phase: PendingAppUpdatePhase,
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

fn update_info_from_release(
    release: &GithubRelease,
    channel: &AppUpdateChannel,
    current_version: &str,
) -> AppUpdateInfo {
    AppUpdateInfo {
        version: normalize_release_version(&release.tag_name),
        current_version: current_version.to_string(),
        channel: channel.clone(),
        notes: release.body.clone(),
    }
}

fn update_check_result(
    status: AppUpdateCheckStatus,
    info: Option<AppUpdateInfo>,
    message: Option<String>,
) -> AppUpdateCheckResult {
    AppUpdateCheckResult {
        status,
        info,
        message,
    }
}

fn normalize_release_version(raw: &str) -> String {
    raw.trim().trim_start_matches(['v', 'V']).to_string()
}

fn parse_version(raw: &str) -> Option<Version> {
    Version::parse(&normalize_release_version(raw)).ok()
}

fn is_newer_version(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => normalize_release_version(candidate) != normalize_release_version(current),
    }
}

fn release_has_manifest(release: &GithubRelease, manifest_name: &str) -> bool {
    release
        .assets
        .iter()
        .any(|asset| asset.name.eq_ignore_ascii_case(manifest_name))
}

fn should_try_release_fallback(error: &UpdaterError) -> bool {
    if matches!(error, UpdaterError::ReleaseNotFound) {
        return true;
    }

    let message = error.to_string().to_ascii_lowercase();
    message.contains("valid release json")
        || message.contains("latest.json")
        || message.contains("latest-preview.json")
        || message.contains("404")
        || message.contains("not found")
}

fn github_client(current_version: &str) -> Result<Client, String> {
    Client::builder()
        .user_agent(format!(
            "Velocity Download Manager/{current_version} (+{GITHUB_RELEASES_PAGE})"
        ))
        .build()
        .map_err(|error| format!("Failed preparing GitHub release fallback client: {error}"))
}

async fn fetch_json_or_404<T: DeserializeOwned>(client: &Client, url: &str) -> Result<Option<T>, String> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("Failed requesting GitHub release metadata: {error}"))?;

    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }

    if !response.status().is_success() {
        return Err(format!(
            "GitHub release metadata returned {} for {url}.",
            response.status()
        ));
    }

    let raw = response
        .text()
        .await
        .map_err(|error| format!("Failed reading GitHub release metadata: {error}"))?;

    serde_json::from_str::<T>(&raw)
        .map(Some)
        .map_err(|error| format!("Failed parsing GitHub release metadata: {error}"))
}

async fn fetch_latest_stable_release(client: &Client) -> Result<Option<GithubRelease>, String> {
    fetch_json_or_404(client, GITHUB_RELEASES_API_LATEST).await
}

async fn fetch_latest_preview_release(client: &Client) -> Result<Option<GithubRelease>, String> {
    let Some(releases) = fetch_json_or_404::<Vec<GithubRelease>>(client, GITHUB_RELEASES_API_LIST).await? else {
        return Ok(None);
    };

    Ok(releases
        .into_iter()
        .find(|release| !release.draft && release.prerelease))
}

async fn fetch_release_candidate(
    client: &Client,
    channel: &AppUpdateChannel,
) -> Result<Option<GithubReleaseCandidate>, String> {
    match channel {
        AppUpdateChannel::Stable => Ok(fetch_latest_stable_release(client)
            .await?
            .map(|release| GithubReleaseCandidate {
                release,
                manifest_name: STABLE_UPDATE_MANIFEST_NAME,
            })),
        AppUpdateChannel::Preview => {
            if let Some(release) = fetch_latest_preview_release(client).await? {
                return Ok(Some(GithubReleaseCandidate {
                    release,
                    manifest_name: PREVIEW_UPDATE_MANIFEST_NAME,
                }));
            }

            Ok(fetch_latest_stable_release(client)
                .await?
                .map(|release| GithubReleaseCandidate {
                    release,
                    manifest_name: STABLE_UPDATE_MANIFEST_NAME,
                }))
        }
    }
}

async fn fallback_update_result(
    app: &AppHandle,
    channel: &AppUpdateChannel,
    skipped_version: Option<&str>,
    updater_error: &UpdaterError,
) -> Result<Option<AppUpdateCheckResult>, String> {
    if !should_try_release_fallback(updater_error) {
        return Ok(None);
    }

    let current_version = app_version(app);
    let client = github_client(&current_version)?;
    let Some(candidate) = fetch_release_candidate(&client, channel).await? else {
        return Ok(Some(update_check_result(
            AppUpdateCheckStatus::UpToDate,
            None,
            Some("No published release is available for this channel yet.".to_string()),
        )));
    };

    let info = update_info_from_release(&candidate.release, channel, &current_version);
    if !is_newer_version(&info.version, &current_version) {
        let message = if release_has_manifest(&candidate.release, candidate.manifest_name) {
            "You already have the latest published build.".to_string()
        } else {
            "This build already matches the latest GitHub release. No in-app update is needed yet."
                .to_string()
        };

        return Ok(Some(update_check_result(
            AppUpdateCheckStatus::UpToDate,
            None,
            Some(message),
        )));
    }

    if skipped_version.is_some_and(|skipped| skipped == info.version) {
        return Ok(Some(update_check_result(
            AppUpdateCheckStatus::UpToDate,
            None,
            Some(format!(
                "Version {} is currently skipped on this device until a newer release appears.",
                info.version
            )),
        )));
    }

    let release_url = candidate
        .release
        .html_url
        .clone()
        .unwrap_or_else(|| GITHUB_RELEASES_PAGE.to_string());
    let manifest_available = release_has_manifest(&candidate.release, candidate.manifest_name);
    let message = if manifest_available {
        format!(
            "Version {} is published, but its in-app updater metadata could not be read cleanly yet. Try again shortly or install it from {}.",
            info.version, release_url
        )
    } else {
        format!(
            "Version {} is published on GitHub, but the updater manifest '{}' is not attached yet. Try again shortly or install it from {}.",
            info.version, candidate.manifest_name, release_url
        )
    };

    Ok(Some(update_check_result(
        AppUpdateCheckStatus::Unavailable,
        Some(info),
        Some(message),
    )))
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
        phase: PendingAppUpdatePhase::AwaitingValidation,
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

fn prune_matching_entries<F>(path: &PathBuf, cutoff: SystemTime, predicate: F) -> Result<(), String>
where
    F: Fn(&str) -> bool,
{
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "Failed listing updater maintenance directory '{}': {error}",
                path.display()
            ));
        }
    };

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "Failed reading updater maintenance directory '{}': {error}",
                path.display()
            )
        })?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if !predicate(&file_name) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let modified_at = match metadata.modified() {
            Ok(modified_at) => modified_at,
            Err(_) => continue,
        };
        if modified_at > cutoff {
            continue;
        }

        let entry_path = entry.path();
        if metadata.is_dir() {
            let _ = fs::remove_dir_all(&entry_path);
        } else {
            let _ = fs::remove_file(&entry_path);
        }
    }

    Ok(())
}

fn is_stale_updater_temp_artifact(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let product_prefix = format!("{}-", APP_PRODUCT_NAME.to_ascii_lowercase());
    lower.starts_with(&product_prefix)
        && (lower.contains("-updater-") || lower.contains("-installer"))
}

pub fn run_startup_maintenance(app: &AppHandle) -> Result<(), String> {
    let temp_cutoff = SystemTime::now()
        .checked_sub(UPDATER_TEMP_ARTIFACT_RETENTION)
        .unwrap_or(UNIX_EPOCH);
    prune_matching_entries(&std::env::temp_dir(), temp_cutoff, is_stale_updater_temp_artifact)?;

    if let Ok(base_dir) = app.path().app_local_data_dir() {
        let updates_dir = base_dir.join("updates");
        let updates_cutoff = SystemTime::now()
            .checked_sub(LEGACY_UPDATES_DIR_RETENTION)
            .unwrap_or(UNIX_EPOCH);
        prune_matching_entries(&updates_dir, updates_cutoff, |_| true)?;

        let is_empty = fs::read_dir(&updates_dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if is_empty {
            let _ = fs::remove_dir(&updates_dir);
        }
    }

    Ok(())
}

pub fn pending_startup_health(app: &AppHandle) -> Option<AppUpdateStartupHealth> {
    let pending = load_pending_update_transition(app).ok().flatten()?;
    if pending.phase != PendingAppUpdatePhase::AwaitingValidation {
        let _ = clear_pending_update_transition(app);
        return None;
    }

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

    if pending.phase != PendingAppUpdatePhase::AwaitingValidation {
        clear_pending_update_transition(app)?;
        return Ok(StartupHealthEvaluation::default());
    }

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
) -> Result<AppUpdateCheckResult, String> {
    let update = match updater_for_channel(app, channel)?
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
        }) {
        Ok(update) => update,
        Err(error) => {
            if let Some(result) = fallback_update_result(app, channel, skipped_version, &error).await? {
                return Ok(result);
            }

            return Err(format!("Failed checking for updates: {error}"));
        }
    };

    let Some(candidate) = update else {
        return Ok(update_check_result(
            AppUpdateCheckStatus::UpToDate,
            None,
            Some("You already have the latest available build.".to_string()),
        ));
    };

    let info = to_update_info(&candidate, channel);
    if skipped_version.is_some_and(|skipped| skipped == info.version) {
        return Ok(update_check_result(
            AppUpdateCheckStatus::UpToDate,
            None,
            Some(format!(
                "Version {} is currently skipped on this device until a newer release appears.",
                info.version
            )),
        ));
    }

    Ok(update_check_result(
        AppUpdateCheckStatus::Available,
        Some(info),
        None,
    ))
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

    clear_pending_update_transition(app)?;

    Ok(info)
}

pub fn persist_pending_restart(app: &AppHandle, info: &AppUpdateInfo, settings: &EngineSettings) -> Result<(), String> {
    persist_pending_update_transition(app, info, settings)
}
