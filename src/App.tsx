import { lazy, startTransition, Suspense, useState, useEffect, useCallback, useMemo, useRef } from "react";
import { TitleBar } from "@/components/TitleBar";
import { Sidebar } from "@/components/Sidebar";
import { Toolbar } from "@/components/Toolbar";
import { DownloadList } from "@/components/DownloadList";
import { StatusBar } from "@/components/StatusBar";
import { TooltipProvider } from "@/components/ui/tooltip";
import {
  canPauseDownload,
  canRestartDownload,
  canResumeDownload,
  runDownloadActionBatch,
  selectDownloadIds,
} from "@/lib/downloadActions";
import { useEngineBridge } from "@/hooks/useEngineBridge";
import { getQueueMoveState } from "@/lib/downloadQueue";
import {
  ipcGetAppStateRows,
  ipcGetDownloadDetails,
  ipcGetDownloadRows,
  type EngineBootstrapState,
  ipcGetQueueState,
  ipcOpenDownloadFolder,
  ipcPauseDownload,
  ipcReorderDownload,
  ipcRestartDownload,
  ipcResumeDownload,
  ipcRemoveDownload,
  ipcStartQueue,
  ipcStopQueue,
  ipcUpdateEngineSettings,
  type DownloadDetailSnapshot,
} from "@/lib/ipc";
import type {
  DownloadCompletedEvent,
  DownloadProgressDiffEvent,
  QueueState,
  SidebarCategory,
  Download,
  EngineSettings,
} from "@/types/download";

const DEFAULT_ENGINE_SETTINGS: EngineSettings = {
  defaultMaxConnections: 8,
  maxActiveDownloads: 3,
  targetChunkTimeSeconds: 2,
  minSegmentSizeBytes: 512 * 1024,
  lateSegmentRatioPercent: 20,
  segmentCheckpointMinIntervalMs: 900,
  segmentCheckpointMaxIntervalMs: 3500,
  experimentalUncappedMode: false,
  trafficMode: "max",
};

const LIVE_PROGRESS_HEARTBEAT_MS = 1200;
const LIVE_PROGRESS_STALL_MS = 2500;

function getErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  if (typeof error === "string" && error.trim()) {
    return error;
  }

  return "VDM could not save the updated settings.";
}

function CompletionNoticeStack({
  notices,
  onOpenFolder,
  onDismiss,
}: {
  notices: DownloadCompletedEvent[];
  onOpenFolder: (id: string) => Promise<void> | void;
  onDismiss: (id: string) => void;
}) {
  if (notices.length === 0) {
    return null;
  }

  return (
    <div className="pointer-events-none absolute bottom-8 right-4 z-30 flex w-[320px] flex-col gap-2">
      {notices.map((notice) => (
        <section
          key={notice.id}
          className="pointer-events-auto rounded-xl border border-border/70 bg-[linear-gradient(180deg,hsl(0,0%,11.5%),hsl(0,0%,8.8%))] p-3 shadow-[0_18px_40px_rgba(0,0,0,0.36)]"
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
  const [newDownloadPrefillUrl, setNewDownloadPrefillUrl] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [downloads, setDownloads] = useState<Download[]>([]);
  const [settings, setSettings] = useState<EngineSettings>(DEFAULT_ENGINE_SETTINGS);
  const [queueState, setQueueState] = useState<QueueState>({ running: true });
  const [settingsSaving, setSettingsSaving] = useState(false);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [bootstrapState, setBootstrapState] = useState<EngineBootstrapState>({ ready: false, error: null });
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [deleteTargetIds, setDeleteTargetIds] = useState<Set<string>>(new Set());
  const [completionNotices, setCompletionNotices] = useState<DownloadCompletedEvent[]>([]);
  const [downloadDetails, setDownloadDetails] = useState<Record<string, DownloadDetailSnapshot>>({});
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
      { activeCount: 0, activeConnections: 0, queuedCount: 0, totalSpeed: 0 },
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
    setCompletionNotices((prev) => [notice, ...prev.filter((entry) => entry.id !== notice.id)].slice(0, 3));

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

  return (
    <TooltipProvider delayDuration={400}>
      <div className="relative flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground">
        <TitleBar onSearch={setSearchQuery} />
        <div className="flex flex-1 overflow-hidden">
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
            <StatusBar
              queuedCount={downloadStats.queuedCount}
              queueRunning={queueState.running}
              activeCount={downloadStats.activeCount}
              activeLimit={settings.maxActiveDownloads}
              connectionCount={downloadStats.activeConnections}
              downloadSpeed={downloadStats.totalSpeed}
              trafficMode={settings.trafficMode}
              engineBootstrapReady={bootstrapState.ready}
              engineBootstrapError={bootstrapState.error}
            />
          </main>
        </div>
        <CompletionNoticeStack
          notices={completionNotices}
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
      {settingsOpen ? (
        <Suspense fallback={null}>
          <SettingsDialog
            open={settingsOpen}
            settings={settings}
            saving={settingsSaving}
            error={settingsError}
            onOpenChange={setSettingsOpen}
            onSave={handleSaveSettings}
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
