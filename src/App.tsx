import { lazy, startTransition, Suspense, useState, useEffect, useCallback, useMemo, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { type UiPrefs, loadUiPrefs } from "@/lib/uiPrefs";
import { TitleBar } from "@/components/TitleBar";
import { Sidebar } from "@/components/Sidebar";
import { Toolbar } from "@/components/Toolbar";
import { DownloadList } from "@/components/DownloadList";
import { StatusBar } from "@/components/StatusBar";
import { TooltipProvider } from "@/components/ui/tooltip";
import { formatBytes } from "@/lib/format";
import {
  canPauseDownload,
  canRestartDownload,
  canResumeDownload,
  runDownloadActionBatch,
  selectDownloadIds,
} from "@/lib/downloadActions";
import { useEngineBridge } from "@/hooks/useEngineBridge";
import { getQueueMoveState } from "@/lib/downloadQueue";
import { extractUpdateHighlights, summarizeUpdateNotes } from "@/lib/updatePresentation";
import {
  ipcCheckAppUpdate,
  ipcGetAppStateRows,
  ipcGetDownloadDetails,
  ipcGetDownloadRows,
  type EngineBootstrapState,
  ipcGetQueueState,
  ipcInstallAppUpdate,
  ipcOpenDownloadFolder,
  ipcPauseDownload,
  ipcReorderDownload,
  ipcRestartApp,
  ipcRestartDownload,
  ipcResumeDownload,
  ipcRemoveDownload,
  ipcRetryEngineBootstrap,
  ipcStartQueue,
  ipcStopQueue,
  ipcUpdateEngineSettings,
  type DownloadDetailSnapshot,
} from "@/lib/ipc";
import { simplifyUserMessage } from "@/lib/userFacingMessages";
import type {
  AppUpdateChannel,
  AppUpdateInfo,
  AppUpdateProgressEvent,
  AppUpdateStartupHealth,
  DownloadCompletedEvent,
  DownloadProgressDiffEvent,
  QueueState,
  SidebarCategory,
  Download,
  EngineSettings,
} from "@/types/download";

const DEFAULT_ENGINE_SETTINGS: EngineSettings = {
  maxActiveDownloads: 3,
  targetChunkTimeSeconds: 2,
  minSegmentSizeBytes: 512 * 1024,
  lateSegmentRatioPercent: 20,
  segmentCheckpointMinIntervalMs: 900,
  segmentCheckpointMaxIntervalMs: 3500,
  experimentalUncappedMode: false,
  trafficMode: "max",
  speedLimitBytesPerSecond: null,
  updateChannel: "stable",
  skippedUpdateVersion: null,
};

const LIVE_PROGRESS_HEARTBEAT_MS = 1200;
const LIVE_PROGRESS_STALL_MS = 2500;
const UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
type AppUpdateStage = "idle" | "available" | "downloading" | "downloaded" | "failed" | "up-to-date";

type AppUpdateState = {
  stage: AppUpdateStage;
  info: AppUpdateInfo | null;
  downloadedBytes: number;
  totalBytes: number | null;
  error: string | null;
  dismissedVersion: string | null;
};

function getActionErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  if (typeof error === "string" && error.trim()) {
    return error;
  }

  return fallback;
}

function getErrorMessage(error: unknown): string {
  return getActionErrorMessage(error, "Velocity Download Manager could not save the updated settings.");
}

function readStoredString(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStoredString(key: string, value: string | null): void {
  try {
    if (value == null) {
      window.localStorage.removeItem(key);
    } else {
      window.localStorage.setItem(key, value);
    }
  } catch {
    // Ignore storage failures on locked-down systems.
  }
}

function readStoredNumber(key: string): number {
  const raw = Number(readStoredString(key));
  return Number.isFinite(raw) && raw > 0 ? raw : 0;
}

function appUpdateProgressMessage(downloadedBytes: number, totalBytes: number | null): string {
  if (totalBytes && totalBytes > 0) {
    return `${formatBytes(downloadedBytes)} of ${formatBytes(totalBytes)} downloaded.`;
  }

  if (downloadedBytes > 0) {
    return `${formatBytes(downloadedBytes)} downloaded.`;
  }

  return "Downloading the update package...";
}

function updateChannelLabel(channel: AppUpdateChannel): string {
  return channel === "preview" ? "Preview" : "Stable";
}

type FloatingAlert = {
  id: string;
  tone: "error" | "warning" | "info" | "success";
  eyebrow: string;
  title: string;
  message: string;
  meta?: string;
  highlights?: string[];
  progressPercent?: number | null;
  actionLabel?: string;
  onAction?: () => void;
  onDismiss?: () => void;
};

function alertToneClasses(tone: FloatingAlert["tone"]): {
  border: string;
  eyebrow: string;
  button: string;
} {
  switch (tone) {
    case "error":
      return {
        border: "border-[hsl(var(--status-error)/0.24)] bg-[linear-gradient(180deg,hsl(var(--status-error)/0.12),hsl(0,0%,9.4%))]",
        eyebrow: "text-[hsl(var(--status-error))]",
        button: "border-[hsl(var(--status-error)/0.28)] text-[hsl(var(--status-error))] hover:bg-[hsl(var(--status-error)/0.12)]",
      };
    case "warning":
      return {
        border: "border-[hsl(var(--status-paused)/0.24)] bg-[linear-gradient(180deg,hsl(var(--status-paused)/0.12),hsl(0,0%,9.4%))]",
        eyebrow: "text-[hsl(var(--status-paused))]",
        button: "border-[hsl(var(--status-paused)/0.28)] text-[hsl(var(--status-paused))] hover:bg-[hsl(var(--status-paused)/0.12)]",
      };
    case "success":
      return {
        border: "border-[hsl(var(--status-finished)/0.24)] bg-[linear-gradient(180deg,hsl(var(--status-finished)/0.12),hsl(0,0%,9.4%))]",
        eyebrow: "text-[hsl(var(--status-finished))]",
        button: "border-[hsl(var(--status-finished)/0.28)] text-[hsl(var(--status-finished))] hover:bg-[hsl(var(--status-finished)/0.12)]",
      };
    default:
      return {
        border: "border-border/70 bg-[linear-gradient(180deg,hsl(var(--card)),hsl(var(--background)))]",
        eyebrow: "text-muted-foreground/56",
        button: "border-border/60 text-foreground/78 hover:bg-accent",
      };
  }
}

function CompletionNoticeStack({
  alerts,
  notices,
  completionHistoryExpanded,
  onToggleCompletionHistory,
  onOpenFolder,
  onDismiss,
}: {
  alerts: FloatingAlert[];
  notices: DownloadCompletedEvent[];
  completionHistoryExpanded: boolean;
  onToggleCompletionHistory: () => void;
  onOpenFolder: (id: string) => Promise<void> | void;
  onDismiss: (id: string) => void;
}) {
  const visibleNotices = completionHistoryExpanded ? notices : notices.slice(0, 3);
  const hiddenNoticeCount = Math.max(0, notices.length - visibleNotices.length);

  if (alerts.length === 0 && visibleNotices.length === 0 && hiddenNoticeCount === 0) {
    return null;
  }

  return (
    <div className="pointer-events-none absolute bottom-8 right-4 z-30 flex w-[320px] flex-col gap-2">
      {alerts.map((alert) => {
        const tone = alertToneClasses(alert.tone);
        return (
          <section
            key={alert.id}
            className={`pointer-events-auto rounded-xl border p-3 shadow-[0_18px_40px_rgba(0,0,0,0.36)] ${tone.border}`}
          >
            <div className={`text-[10px] font-semibold uppercase tracking-[0.14em] ${tone.eyebrow}`}>
              {alert.eyebrow}
            </div>
            <div className="mt-1 text-[13px] font-semibold text-foreground/88">{alert.title}</div>
            {alert.meta ? (
              <div className="mt-1 text-[10px] uppercase tracking-[0.08em] text-muted-foreground/42">{alert.meta}</div>
            ) : null}
            <div className="mt-1 text-[11px] leading-relaxed text-muted-foreground/66">{alert.message}</div>
            {alert.highlights && alert.highlights.length > 0 ? (
              <div className="mt-2 flex flex-col gap-1">
                {alert.highlights.map((highlight) => (
                  <div key={highlight} className="text-[10.5px] leading-relaxed text-foreground/72">
                    - {highlight}
                  </div>
                ))}
              </div>
            ) : null}
            {typeof alert.progressPercent === "number" ? (
              <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-black/20">
                <div
                  className="h-full rounded-full bg-[linear-gradient(90deg,hsl(var(--primary)),hsl(var(--status-downloading)/0.78))] transition-[width] duration-300"
                  style={{ width: `${Math.min(100, Math.max(4, alert.progressPercent))}%` }}
                />
              </div>
            ) : null}
            <div className="mt-3 flex items-center gap-2">
              {alert.actionLabel && alert.onAction ? (
                <button
                  type="button"
                  onClick={alert.onAction}
                  className={`rounded-md border px-2.5 py-1.5 text-[11px] font-medium transition-colors ${tone.button}`}
                >
                  {alert.actionLabel}
                </button>
              ) : null}
              {alert.onDismiss ? (
                <button
                  type="button"
                  onClick={alert.onDismiss}
                  className="rounded-md border border-border/60 px-2.5 py-1.5 text-[11px] text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground"
                >
                  Dismiss
                </button>
              ) : null}
            </div>
          </section>
        );
      })}
      {visibleNotices.map((notice) => (
        <section
          key={notice.id}
          className="pointer-events-auto rounded-xl border border-border/70 bg-[linear-gradient(180deg,hsl(var(--card)),hsl(var(--background)))] p-3 shadow-[0_18px_40px_rgba(0,0,0,0.36)]"
        >
          <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-[hsl(var(--status-finished))]">
            Download Finished
          </div>
          <div className="mt-1 truncate text-[13px] font-semibold text-foreground/86">{notice.name}</div>
          <div className="mt-1 text-[11px] text-muted-foreground/56">The file is ready in your download folder.</div>
          <div className="mt-3 flex items-center gap-2">
            <button
              type="button"
              onClick={() => void onOpenFolder(notice.id)}
              className="rounded-md border border-border/70 bg-black/10 px-2.5 py-1.5 text-[11px] font-medium text-foreground/78 transition-colors hover:bg-accent hover:text-foreground"
            >
              Open Folder
            </button>
            <button
              type="button"
              onClick={() => onDismiss(notice.id)}
              className="rounded-md border border-border/60 px-2.5 py-1.5 text-[11px] text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground"
            >
              Dismiss
            </button>
          </div>
        </section>
      ))}
      {notices.length > 3 ? (
        <section className="pointer-events-auto rounded-xl border border-border/70 bg-[linear-gradient(180deg,hsl(var(--card)),hsl(var(--background)))] p-3 shadow-[0_18px_40px_rgba(0,0,0,0.36)]">
          <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground/46">
            Completion History
          </div>
          <div className="mt-1 text-[11px] text-muted-foreground/64">
            {hiddenNoticeCount > 0
              ? `${hiddenNoticeCount} more completed download${hiddenNoticeCount === 1 ? " is" : "s are"} waiting.`
              : "Showing recent completion activity."}
          </div>
          <div className="mt-3 flex items-center gap-2">
            <button
              type="button"
              onClick={onToggleCompletionHistory}
              className="rounded-md border border-border/70 bg-black/10 px-2.5 py-1.5 text-[11px] font-medium text-foreground/78 transition-colors hover:bg-accent hover:text-foreground"
            >
              {completionHistoryExpanded ? "Show Less" : "Review All"}
            </button>
          </div>
        </section>
      ) : null}
    </div>
  );
}

const DownloadDetailsPanel = lazy(() =>
  import("@/components/DownloadDetailsPanel").then((module) => ({ default: module.DownloadDetailsPanel })),
);
const SettingsDialog = lazy(() =>
  import("@/components/SettingsDialog").then((module) => ({ default: module.SettingsDialog })),
);
const NewDownloadDialog = lazy(() =>
  import("@/components/NewDownloadDialog").then((module) => ({ default: module.NewDownloadDialog })),
);
const DeleteConfirmationDialog = lazy(() =>
  import("@/components/DeleteConfirmationDialog").then((module) => ({ default: module.DeleteConfirmationDialog })),
);
const BatchDownloadDialog = lazy(() =>
  import("@/components/BatchDownloadDialog").then((module) => ({ default: module.BatchDownloadDialog })),
);

function warmDeferredUi() {
  void import("@/components/DownloadDetailsPanel");
  void import("@/components/SettingsDialog");
  void import("@/components/NewDownloadDialog");
  void import("@/components/DeleteConfirmationDialog");
}

export function App() {
  const [activeCategory, setActiveCategory] = useState<SidebarCategory>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [newDownloadOpen, setNewDownloadOpen] = useState(false);
  const [batchDownloadOpen, setBatchDownloadOpen] = useState(false);
  const [newDownloadPrefillUrl, setNewDownloadPrefillUrl] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [downloads, setDownloads] = useState<Download[]>([]);
  const [settings, setSettings] = useState<EngineSettings>(DEFAULT_ENGINE_SETTINGS);
  const [queueState, setQueueState] = useState<QueueState>({ running: true });
  const [settingsSaving, setSettingsSaving] = useState(false);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [settingsNotice, setSettingsNotice] = useState<string | null>(null);
  const [bootstrapState, setBootstrapState] = useState<EngineBootstrapState>({ ready: false, error: null });
  const [startupUpdateHealth, setStartupUpdateHealth] = useState<AppUpdateStartupHealth | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [deleteTargetIds, setDeleteTargetIds] = useState<Set<string>>(new Set());
  const [completionNotices, setCompletionNotices] = useState<DownloadCompletedEvent[]>([]);
  const [completionHistoryExpanded, setCompletionHistoryExpanded] = useState(false);
  const [dismissedBootstrapError, setDismissedBootstrapError] = useState<string | null>(null);
  const [dismissedStartupUpdateHealthSignature, setDismissedStartupUpdateHealthSignature] = useState<string | null>(null);
  const [dismissedDownloadIssueSignature, setDismissedDownloadIssueSignature] = useState<string | null>(null);
  const [downloadDetails, setDownloadDetails] = useState<Record<string, DownloadDetailSnapshot>>({});
  const [uiPrefs, setUiPrefs] = useState<UiPrefs>(() => ({ ...loadUiPrefs() }));
  const [appUpdate, setAppUpdate] = useState<AppUpdateState>(() => ({
    stage: "idle",
    info: null,
    downloadedBytes: 0,
    totalBytes: null,
    error: null,
    dismissedVersion: null,
  }));
  const completionTimers = useRef<Map<string, number>>(new Map());
  const lastRealtimeSyncAt = useRef(Date.now());
  const eventBridgeAttached = useRef(false);

  const refreshDownloads = useCallback(async () => {
    try {
      const nextDownloads = await ipcGetDownloadRows();
      startTransition(() => {
        setDownloads(nextDownloads);
      });
    } catch {
      // Tauri not available in plain browser dev mode — list stays empty.
    }
  }, []);

  const refreshQueueState = useCallback(async () => {
    try {
      setQueueState(await ipcGetQueueState());
    } catch {
      // Browser-only mode has no Tauri queue bridge.
    }
  }, []);

  const refreshAppState = useCallback(async () => {
    try {
      const appState = await ipcGetAppStateRows();
      startTransition(() => {
        setDownloads(appState.downloads);
        setSettings(appState.settings);
        setQueueState(appState.queueState);
      });
    } catch {
      // Browser-only mode has no combined Tauri bridge.
    }
  }, []);

  const refreshDownloadsAndQueue = useCallback(async () => {
    await Promise.all([refreshDownloads(), refreshQueueState()]);
  }, [refreshDownloads, refreshQueueState]);

  const upsertDownload = useCallback((download: Download) => {
    setDownloads((prev) => {
      const index = prev.findIndex((entry) => entry.id === download.id);
      if (index === -1) {
        return [download, ...prev];
      }

      const next = [...prev];
      next[index] = download;
      return next;
    });
  }, []);

  const downloadById = useMemo(
    () => new Map(downloads.map((download) => [download.id, download])),
    [downloads],
  );

  const downloadStats = useMemo(
    () => downloads.reduce(
      (stats, download) => {
        if (download.status === "queued") {
          stats.queuedCount += 1;
        } else if (download.status === "paused") {
          stats.pausedCount += 1;
        } else if (download.status === "finished") {
          stats.finishedCount += 1;
        }

        if (download.status !== "finished" && download.speedLimitBytesPerSecond != null) {
          stats.manualOverrideCount += 1;
        }

        if (download.status !== "downloading") {
          return stats;
        }

        stats.activeCount += 1;
        stats.totalSpeed += download.speed;
        if (download.capabilities.segmented && download.segments.length > 0) {
          const activeSegments = download.segments.filter((segment) => segment.status === "downloading").length;
          stats.activeConnections += Math.max(activeSegments, 1);
        } else {
          stats.activeConnections += 1;
        }
        return stats;
      },
      {
        activeCount: 0,
        activeConnections: 0,
        queuedCount: 0,
        pausedCount: 0,
        finishedCount: 0,
        totalSpeed: 0,
        manualOverrideCount: 0,
      },
    ),
    [downloads],
  );

  const hasLiveTransfers = downloadStats.activeCount > 0 || downloadStats.queuedCount > 0;

  const applyProgressDiff = useCallback((event: DownloadProgressDiffEvent) => {
    setDownloads((prev) => {
      const index = prev.findIndex((entry) => entry.id === event.id);
      if (index === -1) {
        return prev;
      }
      const current = prev[index];
      const baseChanged =
        current.downloaded !== event.downloaded ||
        current.speed !== event.speed ||
        current.timeLeft !== event.timeLeft ||
        current.status !== event.status ||
        current.writerBackpressure !== event.writerBackpressure ||
        current.targetConnections !== event.targetConnections;

      if (event.segments.length === 0) {
        if (!baseChanged) {
          return prev;
        }
        const next = [...prev];
        next[index] = {
          ...current,
          downloaded: event.downloaded,
          speed: event.speed,
          timeLeft: event.timeLeft,
          status: event.status,
          writerBackpressure: event.writerBackpressure,
          targetConnections: event.targetConnections,
        };
        return next;
      }

      const segmentMap = new Map(current.segments.map((segment) => [segment.id, segment]));
      let segmentsChanged = false;
      for (const diff of event.segments) {
        const existing = segmentMap.get(diff.id);
        if (!existing) {
          continue;
        }
        if (existing.downloaded === diff.downloaded && existing.status === diff.status) {
          continue;
        }
        segmentMap.set(diff.id, { ...existing, downloaded: diff.downloaded, status: diff.status });
        segmentsChanged = true;
      }

      if (!segmentsChanged && !baseChanged) {
        return prev;
      }

      const next = [...prev];
      next[index] = {
        ...current,
        downloaded: event.downloaded,
        speed: event.speed,
        timeLeft: event.timeLeft,
        status: event.status,
        writerBackpressure: event.writerBackpressure,
        targetConnections: event.targetConnections,
        segments: segmentsChanged ? current.segments.map((segment) => segmentMap.get(segment.id) ?? segment) : current.segments,
      };
      return next;
    });
  }, []);

  const removeDownloadLocally = useCallback((id: string) => {
    setDownloads((prev) => prev.filter((download) => download.id !== id));
    setDownloadDetails((prev) => {
      if (prev[id] == null) {
        return prev;
      }
      const next = { ...prev };
      delete next[id];
      return next;
    });
    setSelectedIds((prev) => {
      if (!prev.has(id)) {
        return prev;
      }

      const next = new Set(prev);
      next.delete(id);
      return next;
    });
  }, []);

  const openNewDownload = useCallback((prefillUrl?: string) => {
    setNewDownloadPrefillUrl(prefillUrl?.trim() ?? "");
    setNewDownloadOpen(true);
  }, []);

  const dismissCompletionNotice = useCallback((id: string) => {
    const timer = completionTimers.current.get(id);
    if (timer) {
      window.clearTimeout(timer);
      completionTimers.current.delete(id);
    }

    setCompletionNotices((prev) => prev.filter((notice) => notice.id !== id));
  }, []);

  const handleOpenFolder = useCallback(
    async (id: string) => {
      await ipcOpenDownloadFolder(id).catch(() => null);
    },
    [],
  );

  const enqueueCompletionNotice = useCallback((notice: DownloadCompletedEvent) => {
    setCompletionNotices((prev) => [notice, ...prev.filter((entry) => entry.id !== notice.id)].slice(0, 8));

    const existingTimer = completionTimers.current.get(notice.id);
    if (existingTimer) {
      window.clearTimeout(existingTimer);
    }

    const timeoutId = window.setTimeout(() => {
      completionTimers.current.delete(notice.id);
      setCompletionNotices((prev) => prev.filter((entry) => entry.id !== notice.id));
    }, 6000);
    completionTimers.current.set(notice.id, timeoutId);

    if (typeof Notification !== "undefined" && document.visibilityState === "hidden" && Notification.permission === "granted") {
      new Notification("Download finished", { body: notice.name });
    }
  }, []);

  useEffect(() => {
    if (!settingsNotice) {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      setSettingsNotice(null);
    }, 4200);

    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [settingsNotice]);

  useEffect(() => {
    if (!bootstrapState.error) {
      setDismissedBootstrapError(null);
    }
  }, [bootstrapState.error]);

  useEffect(() => {
    if (!startupUpdateHealth) {
      setDismissedStartupUpdateHealthSignature(null);
    }
  }, [startupUpdateHealth]);

  useEffect(() => {
    if (!downloads.some((download) => download.status === "error")) {
      setDismissedDownloadIssueSignature(null);
    }
  }, [downloads]);

  const updateLastCheckKey = useMemo(
    () => `velocity-update:last-check:${settings.updateChannel}`,
    [settings.updateChannel],
  );

  useEffect(() => {
    setAppUpdate((prev) => {
      const dismissedVersion = settings.skippedUpdateVersion ?? null;
      if (prev.dismissedVersion === dismissedVersion) {
        return prev;
      }

      return {
        ...prev,
        dismissedVersion,
      };
    });
  }, [settings.skippedUpdateVersion]);

  useEffect(() => {
    if (import.meta.env.DEV) {
      return;
    }

    let disposed = false;
    const unlistenPromise = listen<AppUpdateProgressEvent>("app://update-progress", (event) => {
      if (disposed) {
        return;
      }

      setAppUpdate((prev) => {
        switch (event.payload.event) {
          case "started":
            return {
              ...prev,
              stage: "downloading",
              downloadedBytes: 0,
              totalBytes: event.payload.data.contentLength ?? null,
              error: null,
            };
          case "progress":
            return {
              ...prev,
              stage: "downloading",
              downloadedBytes: prev.downloadedBytes + event.payload.data.chunkLength,
            };
          case "finished":
            return {
              ...prev,
              downloadedBytes: prev.totalBytes ?? prev.downloadedBytes,
            };
          default:
            return prev;
        }
      });
    });

    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten()).catch(() => null);
    };
  }, []);

  useEffect(() => {
    if (import.meta.env.DEV) {
      return;
    }

    if (Date.now() - readStoredNumber(updateLastCheckKey) < UPDATE_CHECK_INTERVAL_MS) {
      return;
    }

    let active = true;

    void ipcCheckAppUpdate()
      .then((update) => {
        if (!active) {
          return;
        }

        writeStoredString(updateLastCheckKey, String(Date.now()));

        if (!update) {
          setAppUpdate((prev) => ({
            ...prev,
            stage: "up-to-date",
            info: null,
            downloadedBytes: 0,
            totalBytes: null,
            error: null,
          }));
          return;
        }

        setAppUpdate((prev) => ({
          ...prev,
          stage: prev.dismissedVersion === update.version ? "idle" : "available",
          info: update,
          downloadedBytes: 0,
          totalBytes: null,
          error: null,
        }));
      })
      .catch(() => null);

    return () => {
      active = false;
    };
  }, [settings.updateChannel, updateLastCheckKey]);

  useEffect(() => {
    const timers = completionTimers.current;

    return () => {
      for (const timeoutId of timers.values()) {
        window.clearTimeout(timeoutId);
      }
      timers.clear();
    };
  }, []);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      warmDeferredUi();
    }, 1200);

    return () => {
      window.clearTimeout(timeoutId);
    };
  }, []);

  useEngineBridge({
    setBootstrapState,
    setUpdateHealth: setStartupUpdateHealth,
    setSettings,
    setQueueState,
    setDownloads,
    refreshAppState,
    upsertDownload,
    removeDownloadLocally,
    enqueueCompletionNotice,
    applyProgressDiff,
    eventBridgeAttached,
    lastRealtimeSyncAt,
  });

  useEffect(() => {
    if (!hasLiveTransfers) {
      return;
    }

    const intervalId = window.setInterval(() => {
      if (eventBridgeAttached.current && Date.now() - lastRealtimeSyncAt.current < LIVE_PROGRESS_STALL_MS) {
        return;
      }

      void refreshDownloadsAndQueue();
    }, LIVE_PROGRESS_HEARTBEAT_MS);

    return () => {
      window.clearInterval(intervalId);
    };
  }, [hasLiveTransfers, refreshDownloadsAndQueue]);

  const selectedDownloads = useMemo(
    () => Array.from(selectedIds, (id) => downloadById.get(id)).filter((download): download is Download => download != null),
    [downloadById, selectedIds],
  );
  const selectedDownloadsWithDetails = useMemo(() => {
    if (selectedDownloads.length !== 1) {
      return selectedDownloads;
    }
    const selected = selectedDownloads[0];
    const details = downloadDetails[selected.id];
    if (!details) {
      return selectedDownloads;
    }
    return [{
      ...selected,
      engineLog: details.engineLog,
      runtimeCheckpoint: details.runtimeCheckpoint,
    }];
  }, [downloadDetails, selectedDownloads]);
  const selectedDetailId = selectedDownloads.length === 1 ? selectedDownloads[0].id : null;

  useEffect(() => {
    if (!selectedDetailId) {
      return;
    }
    let active = true;
    void ipcGetDownloadDetails(selectedDetailId)
      .then((details) => {
        if (!active) {
          return;
        }
        setDownloadDetails((prev) => ({
          ...prev,
          [details.id]: details,
        }));
      })
      .catch(() => null);
    return () => {
      active = false;
    };
  }, [selectedDetailId]);

  const selectedTransferState = useMemo(
    () => selectedDownloads.reduce(
      (state, download) => {
        if (canPauseDownload(download)) {
          state.canPause = true;
        }
        if (canResumeDownload(download)) {
          state.canResume = true;
        }
        if (canRestartDownload(download)) {
          state.canRestart = true;
        }
        if (download.diagnostics.restartRequired) {
          state.restartRequiredCount += 1;
        }
        return state;
      },
      { canPause: false, canResume: false, canRestart: false, restartRequiredCount: 0 },
    ),
    [selectedDownloads],
  );

  const queueMoveState = useMemo(() => getQueueMoveState(downloads), [downloads]);

  const selectedQueueState = useMemo(
    () => (selectedDownloads.length === 1 ? queueMoveState.get(selectedDownloads[0].id) ?? null : null),
    [queueMoveState, selectedDownloads],
  );

  const handlePauseSelected = useCallback(async () => {
    const ids = selectDownloadIds(selectedDownloads, canPauseDownload);
    const changed = await runDownloadActionBatch(ids, ipcPauseDownload);
    if (!changed) {
      return;
    }
    await refreshDownloadsAndQueue();
  }, [refreshDownloadsAndQueue, selectedDownloads]);

  const handleResumeSelected = useCallback(async () => {
    const ids = selectDownloadIds(selectedDownloads, canResumeDownload);
    const changed = await runDownloadActionBatch(ids, ipcResumeDownload);
    if (!changed) {
      return;
    }
    await refreshDownloadsAndQueue();
  }, [refreshDownloadsAndQueue, selectedDownloads]);

  const handleRestartSelected = useCallback(async () => {
    const ids = selectDownloadIds(selectedDownloads, canRestartDownload);
    const changed = await runDownloadActionBatch(ids, ipcRestartDownload);
    if (!changed) {
      return;
    }
    await refreshDownloadsAndQueue();
  }, [refreshDownloadsAndQueue, selectedDownloads]);

  const handlePauseOne = useCallback(
    async (id: string) => {
      await ipcPauseDownload(id).catch(() => null);
      void refreshDownloadsAndQueue();
    },
    [refreshDownloadsAndQueue],
  );

  const handleResumeOne = useCallback(
    async (id: string) => {
      await ipcResumeDownload(id).catch(() => null);
      void refreshDownloadsAndQueue();
    },
    [refreshDownloadsAndQueue],
  );

  const handleRestartOne = useCallback(
    async (id: string) => {
      await ipcRestartDownload(id).catch(() => null);
      void refreshDownloadsAndQueue();
    },
    [refreshDownloadsAndQueue],
  );

  const handleDeleteSelected = useCallback(() => {
    if (selectedIds.size > 0) {
      setDeleteTargetIds(new Set(selectedIds));
      setDeleteDialogOpen(true);
    }
  }, [selectedIds]);

  const handleActivateDownload = useCallback((id: string) => {
    setSelectedIds(new Set([id]));
  }, []);

  const handleDeleteOne = useCallback(async (id: string) => {
    setDeleteTargetIds(new Set([id]));
    setDeleteDialogOpen(true);
  }, []);

  const handleConfirmDelete = useCallback(
    async (deleteFile: boolean) => {
      const ids = [...deleteTargetIds];
      await Promise.allSettled(ids.map((id) => ipcRemoveDownload(id, deleteFile)));
      setSelectedIds((prev) => {
        const next = new Set(prev);
        for (const id of ids) {
          next.delete(id);
        }
        return next;
      });
      setDeleteTargetIds(new Set());
      await refreshDownloadsAndQueue();
    },
    [deleteTargetIds, refreshDownloadsAndQueue],
  );

  const handleReorderOne = useCallback(
    async (id: string, direction: "up" | "down") => {
      await ipcReorderDownload(id, direction).catch(() => null);
      void refreshDownloads();
    },
    [refreshDownloads],
  );

  const handleStartQueue = useCallback(async () => {
    try {
      setQueueState(await ipcStartQueue());
      await refreshDownloads();
    } catch {
      void refreshQueueState();
    }
  }, [refreshDownloads, refreshQueueState]);

  const handleStopQueue = useCallback(async () => {
    try {
      setQueueState(await ipcStopQueue());
      await refreshDownloads();
    } catch {
      void refreshQueueState();
    }
  }, [refreshDownloads, refreshQueueState]);

  const handleSaveSettings = useCallback(
    async (nextSettings: EngineSettings) => {
      setSettingsSaving(true);
      setSettingsError(null);
      try {
        const updated = await ipcUpdateEngineSettings(nextSettings);
        setSettings(updated);
        setSettingsNotice("Engine settings saved. New transfers will use the updated profile.");
        await refreshDownloads();
        setSettingsOpen(false);
      } catch (error) {
        setSettingsError(getErrorMessage(error));
      } finally {
        setSettingsSaving(false);
      }
    },
    [refreshDownloads],
  );

  const handleUpdateGlobalSpeedLimit = useCallback(
    async (limitBytesPerSecond: number | null) => {
      const updated = await ipcUpdateEngineSettings({
        ...settings,
        speedLimitBytesPerSecond: limitBytesPerSecond,
      });
      setSettings(updated);
      await refreshDownloads();
    },
    [refreshDownloads, settings],
  );

  const handleRetryBootstrap = useCallback(async () => {
    setDismissedBootstrapError(null);
    try {
      setBootstrapState(await ipcRetryEngineBootstrap());
    } catch (error) {
      setBootstrapState({ ready: true, error: getErrorMessage(error) });
    }
  }, []);

  const handleDismissAppUpdate = useCallback(() => {
    const dismissedVersion = appUpdate.info?.version ?? appUpdate.dismissedVersion;
    if (!dismissedVersion) {
      setAppUpdate((prev) => ({
        ...prev,
        stage: prev.stage === "downloaded" ? prev.stage : "idle",
        error: null,
      }));
      return;
    }

    void ipcUpdateEngineSettings({
      ...settings,
      skippedUpdateVersion: dismissedVersion,
    })
      .then((updated) => {
        setSettings(updated);
        setAppUpdate((prev) => ({
          ...prev,
          stage: prev.stage === "downloaded" ? prev.stage : "idle",
          dismissedVersion,
          error: null,
        }));
      })
      .catch((error) => {
        setAppUpdate((prev) => ({
          ...prev,
          stage: "failed",
          error: simplifyUserMessage(
            getActionErrorMessage(error, "Velocity Download Manager could not store the skipped version."),
          ),
        }));
      });
  }, [appUpdate.dismissedVersion, appUpdate.info, settings]);

  const handleInstallAppUpdate = useCallback(async () => {
    setAppUpdate((prev) => ({
      ...prev,
      stage: "downloading",
      downloadedBytes: 0,
      totalBytes: null,
      error: null,
    }));

    try {
      const installedUpdate = await ipcInstallAppUpdate();
      writeStoredString(updateLastCheckKey, String(Date.now()));

      if (settings.skippedUpdateVersion != null) {
        const updatedSettings = await ipcUpdateEngineSettings({
          ...settings,
          skippedUpdateVersion: null,
        });
        setSettings(updatedSettings);
      }

      setAppUpdate((prev) => ({
        ...prev,
        stage: "downloaded",
        info: installedUpdate,
        downloadedBytes: prev.totalBytes ?? prev.downloadedBytes,
        dismissedVersion: null,
        error: null,
      }));
    } catch (error) {
      setAppUpdate((prev) => ({
        ...prev,
        stage: "failed",
        error: simplifyUserMessage(
          getActionErrorMessage(error, "Velocity Download Manager could not install the update."),
        ),
        downloadedBytes: 0,
        totalBytes: null,
      }));
    }
  }, [settings, updateLastCheckKey]);

  const handleRestartAppUpdate = useCallback(async () => {
    try {
      await ipcRestartApp();
    } catch (error) {
      setAppUpdate((prev) => ({
        ...prev,
        stage: "failed",
        error: simplifyUserMessage(
          getActionErrorMessage(error, "Velocity Download Manager could not restart to finish the update."),
        ),
      }));
    }
  }, []);

  // Keyboard shortcuts: Ctrl+N, Delete, Space
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;

      if (e.ctrlKey && e.key === "n") {
        e.preventDefault();
        openNewDownload();
        return;
      }
      if (e.key === "Delete" && selectedIds.size > 0) {
        e.preventDefault();
        void handleDeleteSelected();
        return;
      }
      if (e.key === " " && selectedIds.size > 0) {
        e.preventDefault();
        void (selectedTransferState.canPause ? handlePauseSelected() : handleResumeSelected());
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [handleDeleteSelected, handlePauseSelected, handleResumeSelected, openNewDownload, selectedIds.size, selectedTransferState.canPause]);

  const resumeTooltip = selectedTransferState.restartRequiredCount > 0 && !selectedTransferState.canResume
    ? "Resume is unavailable. Selected downloads require a clean restart."
    : "Resume selected (Space)";
  const restartTooltip = selectedTransferState.restartRequiredCount > 0
    ? "Clean-restart selected downloads from byte 0"
    : "Restart selected";
  const failedDownloads = useMemo(
    () => downloads.filter((download) => download.status === "error"),
    [downloads],
  );
  const failedDownloadIssueSignature = useMemo(
    () => failedDownloads.map((download) => `${download.id}:${download.errorMessage ?? download.diagnostics.terminalReason ?? ""}`).join("|"),
    [failedDownloads],
  );
  const appUpdateProgressPercent = useMemo(() => {
    if (!appUpdate.totalBytes || appUpdate.totalBytes <= 0) {
      return null;
    }

    return Math.round((appUpdate.downloadedBytes / appUpdate.totalBytes) * 100);
  }, [appUpdate.downloadedBytes, appUpdate.totalBytes]);
  const appUpdateNotesSummary = useMemo(
    () => summarizeUpdateNotes(appUpdate.info?.notes),
    [appUpdate.info?.notes],
  );
  const appUpdateHighlights = useMemo(
    () => extractUpdateHighlights(appUpdate.info?.notes).filter((line) => line !== appUpdateNotesSummary),
    [appUpdate.info?.notes, appUpdateNotesSummary],
  );
  const appUpdateVersionMeta = useMemo(
    () => appUpdate.info
      ? `${updateChannelLabel(appUpdate.info.channel)} channel · ${appUpdate.info.currentVersion} -> ${appUpdate.info.version}`
      : undefined,
    [appUpdate.info],
  );
  const startupUpdateHealthSignature = useMemo(
    () => startupUpdateHealth
      ? `${startupUpdateHealth.status}:${startupUpdateHealth.channel}:${startupUpdateHealth.fromVersion}:${startupUpdateHealth.targetVersion}:${startupUpdateHealth.observedVersion}:${startupUpdateHealth.checkedAt}`
      : null,
    [startupUpdateHealth],
  );
  const floatingAlerts = useMemo<FloatingAlert[]>(() => {
    const alerts: FloatingAlert[] = [];

    if (bootstrapState.error && bootstrapState.error !== dismissedBootstrapError) {
      alerts.push({
        id: `bootstrap:${bootstrapState.error}`,
        tone: "error",
        eyebrow: "Engine",
        title: "Engine startup needs attention",
        message: simplifyUserMessage(bootstrapState.error),
        actionLabel: "Retry",
        onAction: () => {
          void handleRetryBootstrap();
        },
        onDismiss: () => {
          setDismissedBootstrapError(bootstrapState.error);
        },
      });
    }

    if (!settingsOpen && settingsError) {
      alerts.push({
        id: `settings-error:${settingsError}`,
        tone: "error",
        eyebrow: "Settings",
        title: "Settings were not saved",
        message: simplifyUserMessage(settingsError),
        actionLabel: "Open",
        onAction: () => {
          setSettingsOpen(true);
        },
        onDismiss: () => {
          setSettingsError(null);
        },
      });
    }

    if (startupUpdateHealth && startupUpdateHealthSignature !== dismissedStartupUpdateHealthSignature) {
      const meta = `${updateChannelLabel(startupUpdateHealth.channel)} channel · ${startupUpdateHealth.fromVersion} -> ${startupUpdateHealth.targetVersion}`;
      const tone = startupUpdateHealth.status === "failed"
        ? "error"
        : startupUpdateHealth.status === "rollbackTriggered"
          ? "warning"
        : startupUpdateHealth.status === "restoredSettings"
          ? "warning"
          : startupUpdateHealth.status === "healthy"
            ? "success"
            : "info";
      const title = startupUpdateHealth.status === "failed"
        ? "Updated build needs review"
        : startupUpdateHealth.status === "rollbackTriggered"
          ? "Rollback guard triggered"
        : startupUpdateHealth.status === "restoredSettings"
          ? "Engine settings were restored"
          : startupUpdateHealth.status === "healthy"
            ? "Update health check passed"
            : "Validating updated build";

      alerts.push({
        id: `startup-update-health:${startupUpdateHealthSignature}`,
        tone,
        eyebrow: "Update",
        title,
        meta,
        message: startupUpdateHealth.message ?? "VDM is validating the first restart after the update.",
        actionLabel: startupUpdateHealth.status === "restoredSettings" ? "Settings" : undefined,
        onAction: startupUpdateHealth.status === "restoredSettings"
          ? () => {
            setSettingsOpen(true);
          }
          : undefined,
        onDismiss: () => {
          setDismissedStartupUpdateHealthSignature(startupUpdateHealthSignature);
        },
      });
    }

    if (appUpdate.stage === "available" && appUpdate.info && appUpdate.info.version !== appUpdate.dismissedVersion) {
      alerts.push({
        id: `app-update:available:${appUpdate.info.version}`,
        tone: "info",
        eyebrow: "Update",
        title: `Version ${appUpdate.info.version} is ready`,
        meta: appUpdateVersionMeta,
        message: appUpdateNotesSummary ?? "A new build of Velocity Download Manager is available.",
        highlights: appUpdateHighlights,
        actionLabel: "Install",
        onAction: () => {
          void handleInstallAppUpdate();
        },
        onDismiss: handleDismissAppUpdate,
      });
    }

    if (appUpdate.stage === "downloading" && appUpdate.info) {
      alerts.push({
        id: `app-update:downloading:${appUpdate.info.version}`,
        tone: "info",
        eyebrow: "Update",
        title: `Installing ${appUpdate.info.version}`,
        meta: appUpdateVersionMeta,
        message: appUpdateProgressMessage(appUpdate.downloadedBytes, appUpdate.totalBytes),
        progressPercent: appUpdateProgressPercent,
      });
    }

    if (appUpdate.stage === "downloaded" && appUpdate.info) {
      alerts.push({
        id: `app-update:downloaded:${appUpdate.info.version}`,
        tone: "success",
        eyebrow: "Update",
        title: "Restart to finish the update",
        meta: appUpdateVersionMeta,
        message: `Velocity Download Manager ${appUpdate.info.version} is installed and ready to apply.`,
        highlights: appUpdateHighlights,
        actionLabel: "Restart",
        onAction: () => {
          void handleRestartAppUpdate();
        },
      });
    }

    if (appUpdate.stage === "failed" && appUpdate.error) {
      alerts.push({
        id: `app-update:failed:${appUpdate.info?.version ?? "unknown"}`,
        tone: "error",
        eyebrow: "Update",
        title: "Update install failed",
        message: appUpdate.error,
        actionLabel: "Retry",
        onAction: () => {
          void handleInstallAppUpdate();
        },
        onDismiss: handleDismissAppUpdate,
      });
    }

    if (failedDownloads.length > 0 && failedDownloadIssueSignature !== dismissedDownloadIssueSignature) {
      const lead = failedDownloads[0];
      const overflow = Math.max(0, failedDownloads.length - 2);
      const leadNames = failedDownloads.slice(0, 2).map((download) => download.name).join(", ");
      alerts.push({
        id: `download-issues:${failedDownloadIssueSignature}`,
        tone: "warning",
        eyebrow: "Downloads",
        title: failedDownloads.length === 1 ? `${lead.name} needs attention` : `${failedDownloads.length} downloads need attention`,
        message: failedDownloads.length === 1
          ? simplifyUserMessage(lead.errorMessage ?? lead.diagnostics.terminalReason ?? "The transfer stopped and may need a retry.")
          : `${leadNames}${overflow > 0 ? ` and ${overflow} more` : ""} stopped and may need a retry.`,
        actionLabel: "Review",
        onAction: () => {
          setSelectedIds(new Set([lead.id]));
          setDismissedDownloadIssueSignature(failedDownloadIssueSignature);
        },
        onDismiss: () => {
          setDismissedDownloadIssueSignature(failedDownloadIssueSignature);
        },
      });
    }

    if (settingsNotice) {
      alerts.push({
        id: `settings-notice:${settingsNotice}`,
        tone: "success",
        eyebrow: "Settings",
        title: "Settings saved",
        message: settingsNotice,
        onDismiss: () => {
          setSettingsNotice(null);
        },
      });
    }

    return alerts;
  }, [
    bootstrapState.error,
    dismissedStartupUpdateHealthSignature,
    dismissedBootstrapError,
    dismissedDownloadIssueSignature,
    failedDownloadIssueSignature,
    failedDownloads,
    appUpdate.dismissedVersion,
    appUpdate.downloadedBytes,
    appUpdate.error,
    appUpdate.info,
    appUpdate.stage,
    appUpdate.totalBytes,
    appUpdateHighlights,
    appUpdateNotesSummary,
    appUpdateProgressPercent,
    appUpdateVersionMeta,
    handleDismissAppUpdate,
    handleInstallAppUpdate,
    handleRestartAppUpdate,
    handleRetryBootstrap,
    settingsError,
    settingsNotice,
    settingsOpen,
    startupUpdateHealth,
    startupUpdateHealthSignature,
  ]);

  return (
    <TooltipProvider delayDuration={400}>
      <div className="relative flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground">
        <TitleBar 
          onSearch={setSearchQuery} 
          onNewDownload={openNewDownload}
          onOpenSettings={() => {
            setSettingsError(null);
            setSettingsOpen(true);
          }}
          onBatchDownload={() => setBatchDownloadOpen(true)}
          onStartQueue={() => void handleStartQueue()}
          onStopQueue={() => void handleStopQueue()}
          queueRunning={queueState.running}
        />
        <div id="vdm-content" className="flex flex-1 overflow-hidden">
          <Sidebar activeCategory={activeCategory} onCategoryChange={setActiveCategory} downloads={downloads} />
          <main className="flex flex-1 flex-col overflow-hidden">
            <Toolbar
              onNewDownload={openNewDownload}
              onOpenSettings={() => {
                setSettingsError(null);
                setSettingsOpen(true);
              }}
              queueRunning={queueState.running}
              canPause={selectedTransferState.canPause}
              canResume={selectedTransferState.canResume}
              canRestart={selectedTransferState.canRestart}
              canDelete={selectedIds.size > 0}
              resumeTooltip={resumeTooltip}
              restartTooltip={restartTooltip}
              onStartQueue={() => void handleStartQueue()}
              onStopQueue={() => void handleStopQueue()}
              onPause={() => void handlePauseSelected()}
              onResume={() => void handleResumeSelected()}
              onRestart={() => void handleRestartSelected()}
              onDelete={() => void handleDeleteSelected()}
            />
            <DownloadList
              downloads={downloads}
              activeCategory={activeCategory}
              searchQuery={searchQuery}
              selectedIds={selectedIds}
              onSelectedChange={setSelectedIds}
              onRowActivate={handleActivateDownload}
              onDelete={handleDeleteOne}
              onReorder={handleReorderOne}
              onOpenFolder={handleOpenFolder}
              onRefresh={refreshDownloads}
            />
            {selectedDownloads.length > 0 ? (
              <Suspense fallback={null}>
                <DownloadDetailsPanel
                  selectedDownloads={selectedDownloadsWithDetails}
                  onOpenFolder={handleOpenFolder}
                  onPause={handlePauseOne}
                  onResume={handleResumeOne}
                  onRestart={handleRestartOne}
                  onDelete={handleDeleteOne}
                  onReorder={handleReorderOne}
                  canMoveUp={selectedQueueState?.canMoveUp ?? false}
                  canMoveDown={selectedQueueState?.canMoveDown ?? false}
                  onClearSelection={() => setSelectedIds(new Set())}
                />
              </Suspense>
            ) : null}
          </main>
        </div>
        {uiPrefs.showStatusBar && (
          <StatusBar
            queuedCount={downloadStats.queuedCount}
            pausedCount={downloadStats.pausedCount}
            finishedCount={downloadStats.finishedCount}
            queueRunning={queueState.running}
            activeCount={downloadStats.activeCount}
            activeLimit={settings.maxActiveDownloads}
            connectionCount={downloadStats.activeConnections}
            downloadSpeed={downloadStats.totalSpeed}
            speedLimitBytesPerSecond={settings.speedLimitBytesPerSecond ?? null}
            manualOverrideCount={downloadStats.manualOverrideCount}
            engineBootstrapReady={bootstrapState.ready}
            engineBootstrapError={bootstrapState.error}
            onRetryEngineBootstrap={() => {
              void handleRetryBootstrap();
            }}
            onUpdateGlobalSpeedLimit={(limitBytesPerSecond) => handleUpdateGlobalSpeedLimit(limitBytesPerSecond)}
          />
        )}
        <CompletionNoticeStack
          alerts={floatingAlerts}
          notices={completionNotices}
          completionHistoryExpanded={completionHistoryExpanded}
          onToggleCompletionHistory={() => setCompletionHistoryExpanded((prev) => !prev)}
          onOpenFolder={async (id) => {
            await handleOpenFolder(id);
            dismissCompletionNotice(id);
          }}
          onDismiss={dismissCompletionNotice}
        />
      </div>
      {newDownloadOpen ? (
        <Suspense fallback={null}>
          <NewDownloadDialog
            open={newDownloadOpen}
            initialUrl={newDownloadPrefillUrl}
            existingDownloads={downloads}
            onOpenChange={(nextOpen) => {
              setNewDownloadOpen(nextOpen);
              if (!nextOpen) {
                setNewDownloadPrefillUrl("");
              }
            }}
            onDownloadAdded={(download) => {
              upsertDownload(download);
              setNewDownloadPrefillUrl("");
              setSelectedIds(new Set([download.id]));
            }}
          />
        </Suspense>
      ) : null}
      {batchDownloadOpen ? (
        <Suspense fallback={null}>
          <BatchDownloadDialog
            open={batchDownloadOpen}
            onOpenChange={setBatchDownloadOpen}
            existingDownloads={downloads}
            onDownloadsAdded={(addedDownloads) => {
              for (const download of addedDownloads) {
                upsertDownload(download);
              }
              if (addedDownloads.length > 0) {
                setSelectedIds(new Set([addedDownloads[addedDownloads.length - 1].id]));
              }
            }}
          />
        </Suspense>
      ) : null}
      {settingsOpen ? (
        <Suspense fallback={null}>
          <SettingsDialog
            open={settingsOpen}
            settings={settings}
            saving={settingsSaving}
            error={settingsError}
            onOpenChange={setSettingsOpen}
            onSave={handleSaveSettings}
            onClearError={() => setSettingsError(null)}
            onUiPrefsChange={setUiPrefs}
          />
        </Suspense>
      ) : null}
      {deleteDialogOpen ? (
        <Suspense fallback={null}>
          <DeleteConfirmationDialog
            open={deleteDialogOpen}
            onOpenChange={setDeleteDialogOpen}
            onConfirm={handleConfirmDelete}
            count={deleteTargetIds.size}
          />
        </Suspense>
      ) : null}
    </TooltipProvider>
  );
}
