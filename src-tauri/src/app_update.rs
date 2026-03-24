use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::{header, Client, StatusCode, Url};
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
const GITHUB_RELEASES_PAGE: &str =
    "https://github.com/vamptux/Velocity-Download-Manager/releases";
const GITHUB_RELEASES_API_LATEST: &str =
    "https://api.github.com/repos/vamptux/Velocity-Download-Manager/releases/latest";
const GITHUB_TAGS_API_LIST: &str =
    "https://api.github.com/repos/vamptux/Velocity-Download-Manager/tags?per_page=32";
const STABLE_UPDATE_MANIFEST_NAME: &str = "latest.json";
const UPDATER_TEMP_ARTIFACT_RETENTION: Duration = Duration::from_secs(60 * 60 * 24 * 3);
const LEGACY_UPDATES_DIR_RETENTION: Duration = Duration::from_secs(60 * 60 * 24 * 7);
const GITHUB_FALLBACK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const GITHUB_FALLBACK_REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const APP_PRODUCT_NAME: &str = "Velocity Download Manager";

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    #[serde(alias = "tagName")]
    name: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    #[serde(alias = "tagName")]
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default, alias = "htmlUrl")]
    html_url: Option<String>,
    #[serde(default)]
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug)]
struct GithubReleaseCandidate {
    release: GithubRelease,
    manifest_name: &'static str,
}

#[derive(Debug, Deserialize)]
struct GithubTag {
    name: String,
}

enum ResolvedAppUpdate {
    Available {
        update: Box<Update>,
        info: AppUpdateInfo,
    },
    UpToDate {
        message: String,
    },
    Unavailable {
        info: Option<AppUpdateInfo>,
        message: String,
    },
}

enum UpdaterCheckFailure {
    Prepare(String),
    Check(UpdaterError),
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

fn highest_version_tag_name(tags: Vec<GithubTag>) -> Option<String> {
    tags.into_iter()
        .filter_map(|tag| parse_version(&tag.name).map(|version| (version, tag.name)))
        .filter(|(version, _)| version.pre.is_empty())
        .max_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, name)| name)
}

fn should_try_release_fallback(error: &UpdaterError) -> bool {
    if matches!(error, UpdaterError::ReleaseNotFound) {
        return true;
    }

    let message = error.to_string().to_ascii_lowercase();
    message.contains("valid release json")
        || message.contains("latest.json")
        || message.contains("404")
        || message.contains("not found")
}

fn github_client(current_version: &str) -> Result<Client, String> {
    Client::builder()
        .connect_timeout(GITHUB_FALLBACK_CONNECT_TIMEOUT)
        .timeout(GITHUB_FALLBACK_REQUEST_TIMEOUT)
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
        .map_err(|error| {
            if error.is_timeout() {
                format!(
                    "GitHub release fallback timed out while requesting {url}. Try again shortly or install manually from {GITHUB_RELEASES_PAGE}."
                )
            } else {
                format!("Failed requesting GitHub release metadata: {error}")
            }
        })?;

    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }

    if github_rate_limited(&response) {
        return Err(github_rate_limit_message(&response));
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
        .map_err(|error| {
            if error.is_timeout() {
                format!(
                    "GitHub release fallback timed out while reading {url}. Try again shortly or install manually from {GITHUB_RELEASES_PAGE}."
                )
            } else {
                format!("Failed reading GitHub release metadata: {error}")
            }
        })?;

    serde_json::from_str::<T>(&raw)
        .map(Some)
        .map_err(|error| format!("Failed parsing GitHub release metadata: {error}"))
}

fn github_rate_limited(response: &reqwest::Response) -> bool {
    response.status() == StatusCode::TOO_MANY_REQUESTS
        || (response.status() == StatusCode::FORBIDDEN
            && response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                == Some("0"))
}

fn github_rate_limit_message(response: &reqwest::Response) -> String {
    let retry_hint = github_retry_hint(response)
        .map(|hint| format!(" {hint}"))
        .unwrap_or_default();
    format!(
        "GitHub temporarily rate-limited updater fallback requests. Try again shortly or install manually from {GITHUB_RELEASES_PAGE}.{retry_hint}"
    )
}

fn github_retry_hint(response: &reqwest::Response) -> Option<String> {
    let retry_after_seconds = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .or_else(|| {
            response
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .and_then(|reset_at| {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
                    Some(reset_at.saturating_sub(now))
                })
        })?;

    Some(format!(
        "Estimated wait: {}.",
        format_wait_duration(retry_after_seconds)
    ))
}

fn format_wait_duration(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{}s", seconds.max(1));
    }

    if seconds < 60 * 60 {
        return format!("{}m", seconds.div_ceil(60));
    }

    format!("{}h", seconds.div_ceil(60 * 60))
}

async fn fetch_latest_stable_release(client: &Client) -> Result<Option<GithubRelease>, String> {
    fetch_json_or_404(client, GITHUB_RELEASES_API_LATEST).await
}

async fn fetch_latest_stable_tag(client: &Client) -> Result<Option<String>, String> {
    let Some(tags) = fetch_json_or_404::<Vec<GithubTag>>(client, GITHUB_TAGS_API_LIST).await? else {
        return Ok(None);
    };

    Ok(highest_version_tag_name(tags))
}

fn release_manifest_endpoint(tag_name: &str, manifest_name: &str) -> Result<Url, String> {
    Url::parse(&format!(
        "https://github.com/vamptux/Velocity-Download-Manager/releases/download/{tag_name}/{manifest_name}"
    ))
    .map_err(|error| {
        format!(
            "Failed parsing updater manifest URL for tag '{tag_name}' and asset '{manifest_name}': {error}"
        )
    })
}

fn updater_for_endpoints(
    app: &AppHandle,
    endpoints: Vec<Url>,
) -> Result<tauri_plugin_updater::Updater, String> {
    let builder = app
        .updater_builder()
        .endpoints(endpoints)
        .map_err(|error| format!("Failed preparing updater endpoints: {error}"))?;

    builder
        .build()
        .map_err(|error| format!("Failed preparing the updater: {error}"))
}

fn updater_for_release_candidate(
    app: &AppHandle,
    candidate: &GithubReleaseCandidate,
) -> Result<tauri_plugin_updater::Updater, String> {
    updater_for_endpoints(
        app,
        vec![release_manifest_endpoint(
            &candidate.release.tag_name,
            candidate.manifest_name,
        )?],
    )
}

async fn try_updater_check(
    app: &AppHandle,
    channel: &AppUpdateChannel,
) -> Result<Option<Update>, UpdaterCheckFailure> {
    updater_for_channel(app, channel)
        .map_err(UpdaterCheckFailure::Prepare)?
        .check()
        .await
        .map_err(UpdaterCheckFailure::Check)
}

async fn fetch_release_candidate(
    client: &Client,
    channel: &AppUpdateChannel,
) -> Result<Option<GithubReleaseCandidate>, String> {
    let _ = channel;
    Ok(fetch_latest_stable_release(client)
        .await?
        .map(|release| GithubReleaseCandidate {
            release,
            manifest_name: STABLE_UPDATE_MANIFEST_NAME,
        }))
}

async fn fallback_update_resolution(
    app: &AppHandle,
    channel: &AppUpdateChannel,
    skipped_version: Option<&str>,
    updater_error: Option<&UpdaterError>,
) -> Result<Option<ResolvedAppUpdate>, String> {
    if updater_error.is_some_and(|error| !should_try_release_fallback(error)) {
        return Ok(None);
    }

    let current_version = app_version(app);
    let client = github_client(&current_version)?;
    let Some(candidate) = fetch_release_candidate(&client, channel).await? else {
        if let Some(tag_name) = fetch_latest_stable_tag(&client).await? {
            let version = normalize_release_version(&tag_name);
            if is_newer_version(&version, &current_version) {
                if skipped_version.is_some_and(|skipped| skipped == version) {
                    return Ok(Some(ResolvedAppUpdate::UpToDate {
                        message: format!(
                            "Version {} is currently skipped on this device until a newer release appears.",
                            version
                        ),
                    }));
                }

                return Ok(Some(ResolvedAppUpdate::Unavailable {
                    info: Some(AppUpdateInfo {
                        version: version.clone(),
                        current_version: current_version.clone(),
                        channel: channel.clone(),
                        notes: None,
                    }),
                    message: format!(
                        "Version {} is tagged on GitHub, but no GitHub Release with updater assets is published yet. Run the release workflow or attach the updater artifacts before expecting in-app updates to install.",
                        version
                    ),
                }));
            }
        }

        if updater_error.is_some() {
            return Ok(Some(ResolvedAppUpdate::UpToDate {
                message: "No published release is available for this channel yet.".to_string(),
            }));
        }

        return Ok(None);
    };

    let info = update_info_from_release(&candidate.release, channel, &current_version);
    if !is_newer_version(&info.version, &current_version) {
        let message = if release_has_manifest(&candidate.release, candidate.manifest_name) {
            "You already have the latest published build.".to_string()
        } else {
            "This build already matches the latest GitHub release. No in-app update is needed yet."
                .to_string()
        };

        return Ok(Some(ResolvedAppUpdate::UpToDate { message }));
    }

    if skipped_version.is_some_and(|skipped| skipped == info.version) {
        return Ok(Some(ResolvedAppUpdate::UpToDate {
            message: format!(
                "Version {} is currently skipped on this device until a newer release appears.",
                info.version
            ),
        }));
    }

    let release_url = candidate
        .release
        .html_url
        .clone()
        .unwrap_or_else(|| GITHUB_RELEASES_PAGE.to_string());
    let manifest_available = release_has_manifest(&candidate.release, candidate.manifest_name);
    if !manifest_available {
        return Ok(Some(ResolvedAppUpdate::Unavailable {
            info: Some(info),
            message: format!(
                "Version {} is published on GitHub, but the updater manifest '{}' is not attached yet. Try again shortly or install it from {}.",
                candidate.release.tag_name,
                candidate.manifest_name,
                release_url
            ),
        }));
    }

    match updater_for_release_candidate(app, &candidate)?
        .check()
        .await
        .map_err(|error| format!("Failed validating the tagged updater manifest: {error}"))?
    {
        Some(update) => Ok(Some(ResolvedAppUpdate::Available {
            info: to_update_info(&update, channel),
            update: Box::new(update),
        })),
        None => Ok(Some(ResolvedAppUpdate::Unavailable {
            info: Some(info.clone()),
            message: format!(
                "Version {} is published and its updater manifest is attached, but GitHub is still serving updater metadata that resolves to the current build. Try again shortly or install it from {}.",
                info.version,
                release_url
            ),
        })),
    }
}

fn updater_endpoints(channel: &AppUpdateChannel) -> Result<Vec<Url>, String> {
    let _ = channel;
    let raw = vec![STABLE_UPDATE_ENDPOINT];

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
    updater_for_endpoints(app, updater_endpoints(channel)?)
}

pub fn skipped_version_for_channel<'a>(
    settings: &'a EngineSettings,
    channel: &AppUpdateChannel,
) -> Option<&'a str> {
    let _ = channel;
    settings.skipped_update_version.as_deref()
}

fn set_skipped_version_for_channel(
    settings: &mut EngineSettings,
    channel: &AppUpdateChannel,
    version: Option<String>,
) {
    let _ = channel;
    settings.skipped_update_version = version;
}

fn apply_failed_update_guardrails(
    settings: &mut EngineSettings,
    pending: &PendingAppUpdateTransition,
) -> String {
    set_skipped_version_for_channel(
        settings,
        &pending.channel,
        Some(pending.target_version.clone()),
    );

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
    let update = match try_updater_check(app, channel).await {
        Ok(update) => update,
        Err(UpdaterCheckFailure::Prepare(error)) => return Err(error),
        Err(UpdaterCheckFailure::Check(error)) => {
            if let Some(result) = fallback_update_resolution(app, channel, skipped_version, Some(&error)).await? {
                return Ok(match result {
                    ResolvedAppUpdate::Available { info, .. } => {
                        update_check_result(AppUpdateCheckStatus::Available, Some(info), None)
                    }
                    ResolvedAppUpdate::UpToDate { message } => {
                        update_check_result(AppUpdateCheckStatus::UpToDate, None, Some(message))
                    }
                    ResolvedAppUpdate::Unavailable { info, message } => {
                        update_check_result(AppUpdateCheckStatus::Unavailable, info, Some(message))
                    }
                });
            }

            return Err(format!("Failed checking for updates: {error}"));
        }
    };

    let Some(candidate) = update else {
        if let Some(result) = fallback_update_resolution(app, channel, skipped_version, None).await? {
            return Ok(match result {
                ResolvedAppUpdate::Available { info, .. } => {
                    update_check_result(AppUpdateCheckStatus::Available, Some(info), None)
                }
                ResolvedAppUpdate::UpToDate { message } => {
                    update_check_result(AppUpdateCheckStatus::UpToDate, None, Some(message))
                }
                ResolvedAppUpdate::Unavailable { info, message } => {
                    update_check_result(AppUpdateCheckStatus::Unavailable, info, Some(message))
                }
            });
        }

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
    let resolved = match try_updater_check(app, &channel).await {
        Ok(Some(update)) => {
            let info = to_update_info(&update, &channel);
            ResolvedAppUpdate::Available {
                update: Box::new(update),
                info,
            }
        }
        Ok(None) => fallback_update_resolution(app, &channel, None, None)
            .await?
            .unwrap_or(ResolvedAppUpdate::UpToDate {
                message: "Velocity Download Manager is already up to date.".to_string(),
            }),
        Err(UpdaterCheckFailure::Prepare(error)) => return Err(error),
        Err(UpdaterCheckFailure::Check(error)) => fallback_update_resolution(app, &channel, None, Some(&error))
            .await?
            .unwrap_or(ResolvedAppUpdate::Unavailable {
                info: None,
                message: format!("Failed checking for updates: {error}"),
            }),
    };

    let (update, info) = match resolved {
        ResolvedAppUpdate::Available { update, info } => (*update, info),
        ResolvedAppUpdate::UpToDate { message } => return Err(message),
        ResolvedAppUpdate::Unavailable { message, .. } => return Err(message),
    };
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
