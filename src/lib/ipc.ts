import { invoke } from "@tauri-apps/api/core";
import type {
  AppUpdateCheckResult,
  AppUpdateInfo,
  AppUpdateStartupHealth,
  CaptureBridgeStatus,
  AddDownloadArgs as BackendAddDownloadArgs,
  CapturePayload,
  Download,
  DownloadContentCategory,
  DownloadRecord,
  DownloadProbe,
  DownloadRequestField,
  DownloadRequestMethod,
  DownloadLogEntry,
  DownloadRuntimeCheckpoint,
  EngineSettings,
  ProbeDownloadArgs as BackendProbeDownloadArgs,
  QueueState,
  ReorderDirection,
} from "@/types/download";

export type RawDownload = DownloadRecord;

export type EngineBootstrapPhase = "starting" | "ready" | "retrying" | "failed";

export interface EngineBootstrapState {
  phase: EngineBootstrapPhase;
  error: string | null;
}

export function isEngineBootstrapReady(state: EngineBootstrapState): boolean {
  return state.phase === "ready";
}

export function isEngineBootstrapSettled(state: EngineBootstrapState): boolean {
  return state.phase === "ready" || state.phase === "failed";
}

interface RawAppState {
  downloads: RawDownload[];
  settings: EngineSettings;
  queueState: QueueState;
}

interface RawAppRowState {
  downloads: RawDownload[];
  settings: EngineSettings;
  queueState: QueueState;
}

interface RawDownloadDetailSnapshot {
  id: string;
  engineLog: DownloadLogEntry[];
  runtimeCheckpoint: DownloadRuntimeCheckpoint;
}

interface RawStartupSnapshot {
  bootstrap: EngineBootstrapState;
  settings: EngineSettings;
  queueState: QueueState;
  updateHealth: AppUpdateStartupHealth | null;
  activeDownloads: RawDownload[];
}

export interface AppState {
  downloads: Download[];
  settings: EngineSettings;
  queueState: QueueState;
}

export interface AppRowState {
  downloads: Download[];
  settings: EngineSettings;
  queueState: QueueState;
}

export interface StartupSnapshot {
  bootstrap: EngineBootstrapState;
  settings: EngineSettings;
  queueState: QueueState;
  updateHealth: AppUpdateStartupHealth | null;
  activeDownloads: Download[];
}

export interface DownloadDetailSnapshot {
  id: string;
  engineLog: DownloadLogEntry[];
  runtimeCheckpoint: DownloadRuntimeCheckpoint;
}

export function fromRawDownload(raw: RawDownload): Download {
  return { ...raw, dateAdded: new Date(raw.dateAdded) };
}

export async function ipcGetDownloads(): Promise<Download[]> {
  const raw = await invoke<RawDownload[]>("get_downloads");
  return raw.map(fromRawDownload);
}

export async function ipcGetDownloadRows(): Promise<Download[]> {
  const raw = await invoke<RawAppRowState>("get_app_state_rows");
  return raw.downloads.map(fromRawDownload);
}

export async function ipcGetEngineSettings(): Promise<EngineSettings> {
  return invoke<EngineSettings>("get_engine_settings");
}

export async function ipcGetQueueState(): Promise<QueueState> {
  return invoke<QueueState>("get_queue_state");
}

export async function ipcGetEngineBootstrapState(): Promise<EngineBootstrapState> {
  return invoke<EngineBootstrapState>("get_engine_bootstrap_state");
}

export async function ipcRetryEngineBootstrap(): Promise<EngineBootstrapState> {
  return invoke<EngineBootstrapState>("retry_engine_bootstrap");
}

export async function ipcCheckAppUpdate(): Promise<AppUpdateCheckResult> {
  return invoke<AppUpdateCheckResult>("check_app_update");
}

export async function ipcInstallAppUpdate(): Promise<AppUpdateInfo> {
  return invoke<AppUpdateInfo>("install_app_update");
}

export async function ipcRestartApp(): Promise<void> {
  await invoke("restart_app", { updateInfo: null });
}

export async function ipcRestartToApplyUpdate(updateInfo: AppUpdateInfo): Promise<void> {
  await invoke("restart_app", { updateInfo });
}

export async function ipcOpenExternalUrl(url: string): Promise<void> {
  await invoke("open_external_url", { url });
}

export async function ipcGetAppState(): Promise<AppState> {
  const raw = await invoke<RawAppState>("get_app_state");
  return {
    downloads: raw.downloads.map(fromRawDownload),
    settings: raw.settings,
    queueState: raw.queueState,
  };
}

export async function ipcGetAppStateRows(): Promise<AppRowState> {
  const raw = await invoke<RawAppRowState>("get_app_state_rows");
  return {
    downloads: raw.downloads.map(fromRawDownload),
    settings: raw.settings,
    queueState: raw.queueState,
  };
}

export async function ipcGetStartupSnapshot(): Promise<StartupSnapshot> {
  const raw = await invoke<RawStartupSnapshot>("get_startup_snapshot");
  return {
    bootstrap: raw.bootstrap,
    settings: raw.settings,
    queueState: raw.queueState,
    updateHealth: raw.updateHealth,
    activeDownloads: raw.activeDownloads.map(fromRawDownload),
  };
}

export async function ipcGetDownloadDetails(id: string): Promise<DownloadDetailSnapshot> {
  return invoke<RawDownloadDetailSnapshot>("get_download_details", { id });
}

export async function ipcUpdateEngineSettings(settings: EngineSettings): Promise<EngineSettings> {
  return invoke<EngineSettings>("update_engine_settings", { settings });
}

export async function ipcProbeDownload(
  url: string,
  savePath?: string,
  name?: string,
  requestReferer?: string | null,
  requestCookies?: string | null,
  requestMethod?: DownloadRequestMethod,
  requestFormFields?: DownloadRequestField[],
): Promise<DownloadProbe> {
  const args: BackendProbeDownloadArgs = {
    url,
    savePath: savePath ?? null,
    name: name ?? null,
    requestReferer: requestReferer ?? null,
    requestCookies: requestCookies ?? null,
    requestMethod: requestMethod ?? "get",
    requestFormFields: requestFormFields ?? [],
  };

  return invoke<DownloadProbe>("probe_download", {
    args,
  });
}

export interface IpcAddArgs {
  url: string;
  name?: string;
  category: DownloadContentCategory;
  savePath: string;
  requestReferer?: string | null;
  requestCookies?: string | null;
  requestMethod?: DownloadRequestMethod;
  requestFormFields?: DownloadRequestField[];
  sizeHintBytes?: number;
  rangeSupportedHint?: boolean;
  resumableHint?: boolean;
  scheduledFor?: number | null;
  startImmediately?: boolean;
}

export async function ipcGetCaptureBridgeStatus(): Promise<CaptureBridgeStatus> {
  return invoke<CaptureBridgeStatus>("get_capture_bridge_status");
}

export async function ipcCaptureWindowReady(windowLabel: string): Promise<CapturePayload | null> {
  return invoke<CapturePayload | null>("capture_window_ready", {
    windowLabel,
  });
}

export async function ipcAddDownload(args: IpcAddArgs): Promise<Download> {
  const payload: BackendAddDownloadArgs = {
    url: args.url,
    name: args.name ?? null,
    category: args.category,
    savePath: args.savePath,
    requestReferer: args.requestReferer ?? null,
    requestCookies: args.requestCookies ?? null,
    requestMethod: args.requestMethod ?? "get",
    requestFormFields: args.requestFormFields ?? [],
    sizeHintBytes: args.sizeHintBytes ?? null,
    rangeSupportedHint: args.rangeSupportedHint ?? null,
    resumableHint: args.resumableHint ?? null,
    scheduledFor: args.scheduledFor ?? null,
    startImmediately: args.startImmediately ?? true,
  };

  const raw = await invoke<RawDownload>("add_download", {
    args: payload,
  });
  return fromRawDownload(raw);
}

export async function ipcPauseDownload(id: string): Promise<void> {
  await invoke("pause_download", { id });
}

export async function ipcResumeDownload(id: string): Promise<void> {
  await invoke("resume_download", { id });
}

export async function ipcRestartDownload(id: string): Promise<void> {
  await invoke("restart_download", { id });
}

export async function ipcRemoveDownload(id: string, deleteFile: boolean = false): Promise<void> {
  await invoke("remove_download", { id, deleteFile });
}

export async function ipcRemoveDownloads(ids: string[], deleteFile: boolean = false): Promise<string[]> {
  return invoke<string[]>("remove_downloads", { ids, deleteFile });
}

export async function ipcReorderDownload(id: string, direction: ReorderDirection): Promise<Download> {
  const raw = await invoke<RawDownload>("reorder_download", { id, direction });
  return fromRawDownload(raw);
}

export async function ipcStartQueue(): Promise<QueueState> {
  return invoke<QueueState>("start_queue");
}

export async function ipcStopQueue(): Promise<QueueState> {
  return invoke<QueueState>("stop_queue");
}

export async function ipcSetDownloadSchedule(id: string, scheduledFor: number | null): Promise<Download> {
  const raw = await invoke<RawDownload>("set_download_schedule", { id, scheduledFor });
  return fromRawDownload(raw);
}

export async function ipcSetDownloadTransferOptions(
  id: string,
  speedLimitBytesPerSecond: number | null,
): Promise<Download> {
  const raw = await invoke<RawDownload>("set_download_transfer_options", {
    id,
    speedLimitBytesPerSecond,
  });
  return fromRawDownload(raw);
}

export async function ipcSetDownloadCompletionOptions(
  id: string,
  openFolderOnCompletion: boolean,
): Promise<Download> {
  const raw = await invoke<RawDownload>("set_download_completion_options", {
    id,
    openFolderOnCompletion,
  });
  return fromRawDownload(raw);
}

export async function ipcSetDownloadIntegrityExpectedHash(
  id: string,
  expectedHash: string | null,
): Promise<Download> {
  const raw = await invoke<RawDownload>("set_download_integrity_expected_hash", {
    id,
    expectedHash,
  });
  return fromRawDownload(raw);
}

export async function ipcVerifyDownloadChecksum(id: string): Promise<Download> {
  const raw = await invoke<RawDownload>("verify_download_checksum", { id });
  return fromRawDownload(raw);
}

export async function ipcRecalculateDownloadChecksum(id: string): Promise<Download> {
  const raw = await invoke<RawDownload>("recalculate_download_checksum", { id });
  return fromRawDownload(raw);
}

export async function ipcOpenDownloadFolder(id: string): Promise<void> {
  await invoke("open_download_folder", { id });
}

export async function ipcOpenDownloadFile(id: string): Promise<void> {
  await invoke("open_download_file", { id });
}

export async function ipcFocusMainWindow(): Promise<void> {
  await invoke("focus_main_window");
}
