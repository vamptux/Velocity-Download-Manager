mod app_update;
mod capture_bridge;
mod engine;
mod model;

use std::process::Command;

use engine::{
    AppStateRowSnapshot, AppStateSnapshot, DownloadDetailSnapshot, EngineBootstrapState,
    EngineState, StartupSnapshot,
};
use model::{
    AddDownloadArgs, AppUpdateCheckResult, AppUpdateInfo, ChecksumSpec, DownloadRecord,
    EngineSettings,
    HostTelemetryArgs, ProbeDownloadArgs, ProbeResult, QueueState, ReorderDirection,
};
use tauri::{AppHandle, Manager, State};

type CommandResult<T> = Result<T, String>;

#[tauri::command]
fn get_downloads(state: State<'_, EngineState>) -> Vec<DownloadRecord> {
    state.inner().list_downloads()
}

#[tauri::command]
fn get_engine_settings(state: State<'_, EngineState>) -> EngineSettings {
    state.inner().get_settings()
}

#[tauri::command]
fn get_queue_state(state: State<'_, EngineState>) -> QueueState {
    state.inner().get_queue_state()
}

#[tauri::command]
fn get_engine_bootstrap_state(state: State<'_, EngineState>) -> EngineBootstrapState {
    state.inner().get_bootstrap_state()
}

#[tauri::command]
fn retry_engine_bootstrap(state: State<'_, EngineState>) -> EngineBootstrapState {
    state.inner().spawn_bootstrap();
    state.inner().get_bootstrap_state()
}

#[tauri::command]
async fn check_app_update(
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<AppUpdateCheckResult> {
    let settings = state.inner().get_settings();
    app_update::check_for_update(
        &app,
        &settings.update_channel,
        settings.skipped_update_version.as_deref(),
    )
    .await
}

#[tauri::command]
async fn install_app_update(
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<AppUpdateInfo> {
    let settings = state.inner().get_settings();
    app_update::install_update(&app, &settings).await
}

#[tauri::command]
fn restart_app(
    app: AppHandle,
    state: State<'_, EngineState>,
    update_info: Option<AppUpdateInfo>,
) -> CommandResult<()> {
    if let Some(info) = update_info {
        let settings = state.inner().get_settings();
        app_update::persist_pending_restart(&app, &info, &settings)?;
    }

    app.restart();
}

#[tauri::command]
fn open_external_url(url: String) -> CommandResult<()> {
    let parsed = reqwest::Url::parse(&url)
        .map_err(|error| format!("Failed parsing external URL '{url}': {error}"))?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "Refusing to open unsupported external URL scheme '{other}'."
            ));
        }
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer.exe")
            .arg(parsed.as_str())
            .spawn()
            .map_err(|error| format!("Failed opening external URL: {error}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(parsed.as_str())
            .spawn()
            .map_err(|error| format!("Failed opening external URL: {error}"))?;
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(parsed.as_str())
            .spawn()
            .map_err(|error| format!("Failed opening external URL: {error}"))?;
    }

    Ok(())
}

#[tauri::command]
async fn get_app_state(state: State<'_, EngineState>) -> CommandResult<AppStateSnapshot> {
    state.inner().get_app_state().await
}

#[tauri::command]
async fn get_app_state_rows(state: State<'_, EngineState>) -> CommandResult<AppStateRowSnapshot> {
    state.inner().get_app_state_rows().await
}

#[tauri::command]
fn get_startup_snapshot(state: State<'_, EngineState>) -> StartupSnapshot {
    state.inner().get_startup_snapshot()
}

#[tauri::command]
async fn get_download_details(
    id: String,
    state: State<'_, EngineState>,
) -> CommandResult<DownloadDetailSnapshot> {
    state.inner().get_download_details(&id).await
}

#[tauri::command]
async fn probe_download(
    args: ProbeDownloadArgs,
    state: State<'_, EngineState>,
) -> CommandResult<ProbeResult> {
    state.inner().probe_download(args).await
}

#[tauri::command]
async fn add_download(
    args: AddDownloadArgs,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<DownloadRecord> {
    state.inner().add_download(&app, args).await
}

#[tauri::command]
async fn pause_download(
    id: String,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<()> {
    state.inner().pause_download(&app, &id).await
}

#[tauri::command]
async fn resume_download(
    id: String,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<()> {
    state.inner().resume_download(&app, &id).await
}

#[tauri::command]
async fn restart_download(
    id: String,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<()> {
    state.inner().restart_download(&app, &id).await
}

#[tauri::command]
async fn remove_download(
    id: String,
    delete_file: bool,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<()> {
    state.inner().remove_download(&app, &id, delete_file).await
}

#[tauri::command]
async fn remove_downloads(
    ids: Vec<String>,
    delete_file: bool,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<Vec<String>> {
    state.inner().remove_downloads(&app, &ids, delete_file).await
}

#[tauri::command]
async fn reorder_download(
    id: String,
    direction: ReorderDirection,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<DownloadRecord> {
    state.inner().reorder_download(&app, &id, direction).await
}

#[tauri::command]
async fn start_queue(app: AppHandle, state: State<'_, EngineState>) -> CommandResult<QueueState> {
    state.inner().start_queue(&app).await
}

#[tauri::command]
async fn stop_queue(app: AppHandle, state: State<'_, EngineState>) -> CommandResult<QueueState> {
    state.inner().stop_queue(&app).await
}

#[tauri::command]
async fn set_download_checksum(
    id: String,
    checksum: Option<ChecksumSpec>,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<DownloadRecord> {
    state
        .inner()
        .set_download_checksum(&app, &id, checksum)
        .await
}

#[tauri::command]
async fn verify_download_checksum(
    id: String,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<DownloadRecord> {
    state.inner().verify_download_checksum(&app, &id).await
}

#[tauri::command]
async fn set_download_schedule(
    id: String,
    scheduled_for: Option<i64>,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<DownloadRecord> {
    state
        .inner()
        .set_download_schedule(&app, &id, scheduled_for)
        .await
}

#[tauri::command]
async fn set_download_transfer_options(
    id: String,
    speed_limit_bytes_per_second: Option<u64>,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<DownloadRecord> {
    state
        .inner()
        .set_download_transfer_options(&app, &id, speed_limit_bytes_per_second)
        .await
}

#[tauri::command]
async fn set_download_completion_options(
    id: String,
    open_folder_on_completion: bool,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<DownloadRecord> {
    state
        .inner()
        .set_download_completion_options(&app, &id, open_folder_on_completion)
        .await
}

#[tauri::command]
async fn record_host_telemetry(
    payload: HostTelemetryArgs,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<()> {
    state.inner().record_host_telemetry(&app, payload).await
}

#[tauri::command]
async fn open_download_folder(id: String, state: State<'_, EngineState>) -> CommandResult<()> {
    state.inner().open_download_folder(&id).await
}

#[tauri::command]
async fn update_engine_settings(
    settings: EngineSettings,
    app: AppHandle,
    state: State<'_, EngineState>,
) -> CommandResult<EngineSettings> {
    state.inner().update_settings(&app, settings).await
}

#[tauri::command]
fn take_pending_capture_payload(window_label: Option<String>) -> Option<capture_bridge::CapturePayload> {
    capture_bridge::take_pending_capture(window_label.as_deref())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let _ = app_update::run_startup_maintenance(app.handle());

            let app_handle = app.handle().clone();
            let engine = EngineState::new(app_handle.clone());
            let capture_bridge_state = capture_bridge::initialize_capture_bridge_state(&app_handle)
                .map_err(|error| {
                    std::io::Error::other(format!("capture bridge init failed: {error}"))
                })?;
            app.manage(engine.clone());
            app.manage(capture_bridge_state.clone());

            engine.spawn_bootstrap();
            // Start the browser-extension capture bridge.
            capture_bridge::spawn_capture_server(app_handle, capture_bridge_state);

            #[cfg(target_os = "windows")]
            {
                use window_vibrancy::{apply_acrylic, apply_mica};
                if let Some(window) = app.get_webview_window("main") {
                    // Try mica first, fallback to acrylic if unsupported
                    if apply_mica(&window, Some(true)).is_err() {
                        let _ = apply_acrylic(&window, Some((18, 18, 18, 125)));
                    }
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_downloads,
            get_engine_settings,
            get_queue_state,
            get_engine_bootstrap_state,
            retry_engine_bootstrap,
            check_app_update,
            install_app_update,
            restart_app,
            open_external_url,
            get_app_state,
            get_app_state_rows,
            get_startup_snapshot,
            get_download_details,
            probe_download,
            add_download,
            pause_download,
            resume_download,
            restart_download,
            remove_download,
            remove_downloads,
            reorder_download,
            start_queue,
            stop_queue,
            set_download_checksum,
            verify_download_checksum,
            set_download_schedule,
            set_download_transfer_options,
            set_download_completion_options,
            record_host_telemetry,
            open_download_folder,
            update_engine_settings,
            take_pending_capture_payload,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
