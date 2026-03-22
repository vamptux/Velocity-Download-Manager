import { invoke } from "@tauri-apps/api/core";
import type {
  AppUpdateInfo,
  AppUpdateStartupHealth,
  AddDownloadArgs as BackendAddDownloadArgs,
  CapturePayload,
  ChecksumSpec,
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

export interface EngineBootstrapState {
  ready: boolean;
  error: string | null;
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

export async function ipcCheckAppUpdate(): Promise<AppUpdateInfo | null> {
  return invoke<AppUpdateInfo | null>("check_app_update");
}

export async function ipcInstallAppUpdate(): Promise<AppUpdateInfo> {
  return invoke<AppUpdateInfo>("install_app_update");
}

export async function ipcRestartApp(): Promise<void> {
  await invoke("restart_app");
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
  checksum?: ChecksumSpec;
  sizeHintBytes?: number;
  rangeSupportedHint?: boolean;
  resumableHint?: boolean;
  startImmediately?: boolean;
}

export async function ipcTakePendingCapturePayload(): Promise<CapturePayload | null> {
  return invoke<CapturePayload | null>("take_pending_capture_payload");
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
    checksum: args.checksum ?? null,
    sizeHintBytes: args.sizeHintBytes ?? null,
    rangeSupportedHint: args.rangeSupportedHint ?? null,
    resumableHint: args.resumableHint ?? null,
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

export async function ipcSetDownloadChecksum(id: string, checksum: ChecksumSpec | null): Promise<Download> {
  const raw = await invoke<RawDownload>("set_download_checksum", { id, checksum });
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

export async function ipcOpenDownloadFolder(id: string): Promise<void> {
  await invoke("open_download_folder", { id });
}
