import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowDownToLine,
  FolderOpen,
  ChevronDown,
  ChevronUp,
  Loader2,
  X,
  Minus,
  AlertTriangle,
  ShieldCheck,
  Info,
  Zap,
  ListChecks,
  Pause,
  Play,
  ExternalLink,
} from "lucide-react";
import { InlineNotice } from "@/components/ui/inline-notice";
import { cn } from "@/lib/utils";
import { canPauseDownload, canRestartDownload, canResumeDownload } from "@/lib/downloadActions";
import { formatBytes, formatBytesPerSecond, formatTimeRemaining } from "@/lib/format";
import {
  fromRawDownload,
  ipcAddDownload,
  ipcCaptureWindowReady,
  ipcFocusMainWindow,
  ipcGetDownloadRows,
  ipcGetEngineSettings,
  ipcOpenDownloadFile,
  ipcOpenDownloadFolder,
  ipcPauseDownload,
  ipcProbeDownload,
  ipcSetDownloadCompletionOptions,
  ipcSetDownloadTransferOptions,
  ipcRestartDownload,
  ipcResumeDownload,
  type RawDownload,
} from "@/lib/ipc";
import {
  effectiveSpeedLimitBytesPerSecond,
  parseSpeedLimitDraft,
  SPEED_LIMIT_UNIT_FACTORS,
  speedLimitDraftFromValue,
  type SpeedLimitUnit,
} from "@/lib/speedLimits";
import type {
  CapturePayload,
  Download,
  DownloadContentCategory,
  EngineSettings,
  DownloadProbe,
  DownloadProgressDiffEvent,
  DownloadSegment,
} from "@/types/download";
import { DownloadCapturePane } from "@/components/DownloadCapturePane";
import {
  getCaptureErrorMessage,
  guessCaptureCategory,
  useDefaultCaptureSavePath,
} from "@/lib/captureUtils";
import {
  buildDuplicateLookupInput,
  describeDuplicateMatch,
  duplicateResolutionLabel,
  resolveDuplicateState,
  suggestAlternativeFilename,
} from "@/lib/downloadDuplicates";
import { firstVisibleProbeWarning, simplifyUserMessage } from "@/lib/userFacingMessages";
import { TransferSegmentStrip } from "@/components/TransferSegmentStrip";
import { Checkbox } from "@/components/ui/checkbox";

type MonitorTab = "info" | "speed" | "completion";
type RemovedDownloadEvent = { id: string };

const EXECUTABLE_EXTENSIONS = new Set([".exe", ".msi", ".bat", ".cmd", ".dmg", ".pkg", ".deb", ".rpm", ".appimage"]);
function isExecutable(filename: string): boolean {
  const dot = filename.lastIndexOf(".");
  return dot > -1 && EXECUTABLE_EXTENSIONS.has(filename.slice(dot).toLowerCase());
}

const SPEED_LIMIT_PRESETS_MB = [2, 5, 10, 25, 50, 100] as const;
const MONITOR_TABS: Array<{ id: MonitorTab; label: string; Icon: typeof Info }> = [
  { id: "info", label: "Info", Icon: Info },
  { id: "speed", label: "Speed", Icon: Zap },
  { id: "completion", label: "On Completion", Icon: ListChecks },
];

function truncateUrl(url: string, maxLen = 58): string {
  if (url.length <= maxLen) return url;
  try {
    const parsed = new URL(url);
    const tail = parsed.pathname.length > 1 ? parsed.pathname : "";
    const short = parsed.hostname + tail;
    if (short.length <= maxLen) return short;
    return short.slice(0, maxLen - 1) + "…";
  } catch {
    return url.slice(0, maxLen - 1) + "…";
  }
}

function hostFromUrl(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return "unknown-host";
  }
}

function sourceBadgeLabel(source: CapturePayload["source"]): string | null {
  switch (source) {
    case "download-api":
      return null;
    case "context-menu":
      return "Context menu";
    case "link-click":
      return null;
    case "manual":
      return "Manual";
    default:
      return null;
  }
}

function CompactTitleBar({
  title,
  onClose,
  onMinimize,
}: {
  title: string;
  onClose: () => void;
  onMinimize: () => void;
}) {
  return (
    <div className="flex h-[36px] shrink-0 items-stretch justify-between border-b border-border bg-[hsl(var(--toolbar))] select-none">
      <div data-tauri-drag-region className="flex flex-1 items-center gap-1.5 min-w-0 pl-2.5">
        <img
          src="/veloicon.ico"
          alt="Velocity DM"
          className="h-[20px] w-[20px] shrink-0 object-contain"
        />
        <span className="truncate text-[10.5px] font-medium text-foreground/68 tracking-tight">{title}</span>
      </div>
      <div className="flex shrink-0 items-stretch border-l border-white/[0.06]">
        <button
          type="button"
          onClick={onMinimize}
          aria-label="Minimize"
          className="flex w-[34px] items-center justify-center text-muted-foreground/55 transition-colors hover:bg-white/[0.1] hover:text-foreground/85"
        >
          <Minus size={11} strokeWidth={1.7} />
        </button>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close"
          className="flex w-[34px] items-center justify-center text-muted-foreground/55 transition-colors hover:bg-[hsl(0,66%,46%)] hover:text-white"
        >
          <X size={11} strokeWidth={1.9} />
        </button>
      </div>
    </div>
  );
}

function InfoRow({ label, value, valueClass }: { label: string; value: React.ReactNode; valueClass?: string }) {
  return (
    <div className="flex items-baseline gap-1 py-[2.5px]">
      <span className="w-[98px] shrink-0 text-[11px] text-muted-foreground/55">{label}:</span>
      <span className={cn("flex-1 truncate text-[11px] text-foreground/80", valueClass)}>{value}</span>
    </div>
  );
}

export function CompactCaptureWindow() {
  const [payload, setPayload] = useState<CapturePayload | null>(null);
  const [name, setName] = useState("");
  const [savePath, setSavePath] = useState("");
  const [category, setCategory] = useState<DownloadContentCategory>("documents");
  const [probe, setProbe] = useState<DownloadProbe | null>(null);
  const [probing, setProbing] = useState(false);
  const [probeError, setProbeError] = useState<string | null>(null);
  const [probeWarningDismissed, setProbeWarningDismissed] = useState(false);
  const [adding, setAdding] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  const [duplicateActionPending, setDuplicateActionPending] = useState(false);
  const [existingDownloads, setExistingDownloads] = useState<Download[]>([]);

  const [monitorDownload, setMonitorDownload] = useState<Download | null>(null);
  const [monitorTab, setMonitorTab] = useState<MonitorTab>("info");
  const [segmentsExpanded, setSegmentsExpanded] = useState(false);
  const [liveSegments, setLiveSegments] = useState<DownloadSegment[]>([]);
  const [liveStats, setLiveStats] = useState<{
    status: Download["status"];
    downloaded: number;
    speed: number;
    timeLeft: number | null;
  } | null>(null);
  const [speedLimitEnabled, setSpeedLimitEnabled] = useState(false);
  const [speedLimitValue, setSpeedLimitValue] = useState("25");
  const [speedLimitUnit, setSpeedLimitUnit] = useState<SpeedLimitUnit>("mb");
  const [transferOptionsSaving, setTransferOptionsSaving] = useState(false);
  const [transferOptionsError, setTransferOptionsError] = useState<string | null>(null);
  const [transferOptionsNotice, setTransferOptionsNotice] = useState<string | null>(null);
  const [completionOptionsSaving, setCompletionOptionsSaving] = useState(false);
  const [completionOptionsError, setCompletionOptionsError] = useState<string | null>(null);
  const [globalSpeedLimit, setGlobalSpeedLimit] = useState<number | null>(null);

  const abortRef = useRef<AbortController | null>(null);
  const savePathRef = useRef("");
  const lastPayloadKeyRef = useRef("");
  const windowLabelRef = useRef<string | null>(null);
  const monitorDownloadId = monitorDownload?.id ?? null;
  const monitorDownloadSpeedLimit = monitorDownload?.speedLimitBytesPerSecond ?? null;

  const refreshExistingDownloads = useCallback(async () => {
    try {
      setExistingDownloads(await ipcGetDownloadRows());
    } catch {
      // Duplicate checks degrade gracefully if the row snapshot is temporarily unavailable.
    }
  }, []);

  useEffect(() => {
    savePathRef.current = savePath;
  }, [savePath]);

  useEffect(() => {
    if (monitorDownloadId == null) {
      return;
    }

    const draft = speedLimitDraftFromValue(monitorDownloadSpeedLimit);
    setSpeedLimitEnabled(draft.enabled);
    setSpeedLimitValue(draft.value);
    setSpeedLimitUnit(draft.unit);
  }, [monitorDownloadId, monitorDownloadSpeedLimit]);

  useEffect(() => {
    setTransferOptionsError(null);
    setTransferOptionsNotice(null);
    setCompletionOptionsError(null);
  }, [monitorDownloadId]);

  useEffect(() => {
    if (!transferOptionsNotice) {
      return;
    }

    const timeoutId = window.setTimeout(() => {
      setTransferOptionsNotice(null);
    }, 3200);

    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [transferOptionsNotice]);

  useEffect(() => {
    let disposed = false;

    void ipcGetEngineSettings()
      .then((settings) => {
        if (!disposed) {
          setGlobalSpeedLimit(settings.speedLimitBytesPerSecond ?? null);
        }
      })
      .catch(() => null);

    const unlistenPromise = listen<EngineSettings>("engine://settings", (event) => {
      if (!disposed) {
        setGlobalSpeedLimit(event.payload.speedLimitBytesPerSecond ?? null);
      }
    });

    return () => {
      disposed = true;
      void unlistenPromise.then((unlisten) => unlisten()).catch(() => null);
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    void refreshExistingDownloads();

    const unlistenUpsertPromise = listen<RawDownload>("downloads://upsert", (event) => {
      if (disposed) {
        return;
      }

      const updated = fromRawDownload(event.payload);
      setExistingDownloads((prev) => {
        const index = prev.findIndex((download) => download.id === updated.id);
        if (index === -1) {
          return [updated, ...prev];
        }

        const next = [...prev];
        next[index] = updated;
        return next;
      });
    });

    const unlistenRemovePromise = listen<RemovedDownloadEvent>("downloads://remove", (event) => {
      if (disposed) {
        return;
      }

      setExistingDownloads((prev) => prev.filter((download) => download.id !== event.payload.id));
    });

    return () => {
      disposed = true;
      void unlistenUpsertPromise.then((unlisten) => unlisten()).catch(() => null);
      void unlistenRemovePromise.then((unlisten) => unlisten()).catch(() => null);
    };
  }, [refreshExistingDownloads]);

  useDefaultCaptureSavePath(true, savePath, setSavePath);

  const closeWindow = useCallback(async () => {
    const win = getCurrentWindow();
    try { await win.close(); } catch { await win.hide().catch(() => null); }
  }, []);

  const minimizeWindow = useCallback(() => {
    void getCurrentWindow().minimize().catch(() => null);
  }, []);

  const runProbe = useCallback(async (
    url: string,
    hintName: string,
    requestReferer?: string | null,
    requestCookies?: string | null,
    requestMethod?: CapturePayload["requestMethod"],
    requestFormFields?: CapturePayload["requestFormFields"],
  ) => {
    abortRef.current?.abort();
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    setProbing(true);
    setProbeError(null);
    setProbeWarningDismissed(false);
    try {
      const result = await ipcProbeDownload(
        url,
        savePathRef.current || undefined,
        hintName || undefined,
        requestReferer,
        requestCookies,
        requestMethod,
        requestFormFields,
      );
      if (ctrl.signal.aborted) return;
      setProbe(result);
      if (!hintName && result.suggestedName) setName(result.suggestedName);
      if (!hintName) setCategory(guessCaptureCategory(result.mimeType ?? null, result.suggestedName));
    } catch (err: unknown) {
      if (ctrl.signal.aborted) return;
      setProbeError(getCaptureErrorMessage(err));
    } finally {
      if (!ctrl.signal.aborted) setProbing(false);
    }
  }, []);

  const applyIncomingPayload = useCallback(
    (incoming: CapturePayload) => {
      const key = `${incoming.url}|${incoming.filename ?? ""}|${incoming.source}`;
      if (lastPayloadKeyRef.current === key) return;
      lastPayloadKeyRef.current = key;
      setPayload(incoming);
      const initialName = incoming.filename ?? "";
      setName(initialName);
      setCategory(guessCaptureCategory(incoming.mime, initialName));
      setProbe(null);
      setProbeError(null);
      setProbeWarningDismissed(false);
      setAddError(null);
      void runProbe(
        incoming.url,
        initialName,
        incoming.referrer,
        incoming.requestCookies,
        incoming.requestMethod,
        incoming.requestFormFields,
      );
    },
    [runProbe],
  );

  useEffect(() => {
    let disposed = false;
    const unlistenPromise = listen<CapturePayload>("extension://capture", (event) => {
      applyIncomingPayload(event.payload);
    });

    const currentWindow = getCurrentWindow();
    windowLabelRef.current = currentWindow.label;
    void ipcCaptureWindowReady(currentWindow.label)
      .then((pending) => {
        if (!disposed && pending) {
          applyIncomingPayload(pending);
        }
      })
      .catch(() => null);

    return () => {
      disposed = true;
      void unlistenPromise.then((u) => u());
    };
  }, [applyIncomingPayload]);

  const monitorId = monitorDownloadId;

  useEffect(() => {
    if (!monitorId) return;

    const unlistenUpsertPromise = listen<RawDownload>("downloads://upsert", (event) => {
      if (event.payload.id !== monitorId) return;
      const next = fromRawDownload(event.payload);
      setMonitorDownload(next);
      setLiveSegments(next.segments);
      setLiveStats({
        status: next.status,
        downloaded: next.downloaded,
        speed: next.speed,
        timeLeft: next.timeLeft,
      });
    });

    const unlistenProgressPromise = listen<DownloadProgressDiffEvent>("downloads://progress-diff", (event) => {
      const diff = event.payload;
      if (diff.id !== monitorId) return;
      setLiveStats({ status: diff.status, downloaded: diff.downloaded, speed: diff.speed, timeLeft: diff.timeLeft });
      if (diff.segments.length === 0) {
        return;
      }

      const diffById = new Map(diff.segments.map((segment) => [segment.id, segment]));
      setLiveSegments((prev) =>
        prev.map((segment) => {
          const next = diffById.get(segment.id);
          if (!next) {
            return segment;
          }

          return { ...segment, downloaded: next.downloaded, status: next.status };
        }),
      );
    });

    return () => {
      void unlistenUpsertPromise.then((u) => u());
      void unlistenProgressPromise.then((u) => u());
    };
  }, [monitorId]);

  const activateMonitorDownload = useCallback((download: Download) => {
    setMonitorDownload(download);
    setLiveSegments(download.segments);
    setLiveStats({
      status: download.status,
      downloaded: download.downloaded,
      speed: download.speed,
      timeLeft: download.timeLeft,
    });
    setMonitorTab("info");
    setSegmentsExpanded(false);
    setAddError(null);
  }, []);

  const resolvedName = name.trim() || probe?.suggestedName || "";
  const autoSuggestedName = probe?.suggestedName?.trim() ?? "";
  const filenameResetVisible = name.trim().length > 0
    && autoSuggestedName.length > 0
    && name.trim() !== autoSuggestedName;
  const {
    match: duplicateMatch,
    resolution: duplicateResolution,
    secondaryResolution: duplicateSecondaryResolution,
  } = useMemo(
    () => payload
      ? resolveDuplicateState(existingDownloads, buildDuplicateLookupInput({
        url: payload.url,
        finalUrl: probe?.finalUrl,
        savePath,
        name: resolvedName,
        validators: probe?.validators,
      }))
      : { match: null, resolution: null, secondaryResolution: null },
    [existingDownloads, payload, probe?.finalUrl, probe?.validators, resolvedName, savePath],
  );

  const handleBrowse = async () => {
    const selected = await openDialog({ directory: true, defaultPath: savePath || undefined });
    if (selected && typeof selected === "string") setSavePath(selected);
  };

  const handleAdd = async () => {
    if (!payload || duplicateMatch) return;
    setAdding(true);
    setAddError(null);
    try {
      const dl = await ipcAddDownload({
        url: payload.url,
        name: name.trim() || undefined,
        category,
        savePath,
        requestReferer: payload.referrer,
        requestCookies: payload.requestCookies,
        requestMethod: payload.requestMethod,
        requestFormFields: payload.requestFormFields,
        sizeHintBytes: probe?.size ?? payload.sizeHint ?? undefined,
        rangeSupportedHint: probe?.rangeSupported,
        resumableHint: probe?.resumable,
        startImmediately: true,
      });
      activateMonitorDownload(dl);
    } catch (err: unknown) {
      const msg = getCaptureErrorMessage(err);
      if (msg.toLowerCase().includes("already exists")) {
        void refreshExistingDownloads();
      }
      setAddError(msg);
    } finally {
      setAdding(false);
    }
  };

  const handleDuplicatePrimaryAction = useCallback(async () => {
    if (!duplicateMatch || !duplicateResolution || duplicateActionPending) {
      return;
    }

    if (duplicateResolution === "keepBoth") {
      setName(suggestAlternativeFilename(resolvedName || duplicateMatch.download.name));
      setAddError(null);
      return;
    }

    setDuplicateActionPending(true);
    setAddError(null);
    try {
      switch (duplicateResolution) {
        case "resume":
          await ipcResumeDownload(duplicateMatch.download.id);
          break;
        case "restart":
          await ipcRestartDownload(duplicateMatch.download.id);
          break;
        case "reveal":
          await ipcOpenDownloadFolder(duplicateMatch.download.id);
          break;
        case "inspect":
          break;
      }

      activateMonitorDownload(duplicateMatch.download);
      void refreshExistingDownloads();
    } catch (error) {
      setAddError(getCaptureErrorMessage(error));
    } finally {
      setDuplicateActionPending(false);
    }
  }, [activateMonitorDownload, duplicateActionPending, duplicateMatch, duplicateResolution, refreshExistingDownloads, resolvedName]);

  const handleDuplicateSecondaryAction = useCallback(async () => {
    if (!duplicateMatch || !duplicateSecondaryResolution || duplicateActionPending) {
      return;
    }

    setDuplicateActionPending(true);
    setAddError(null);
    try {
      switch (duplicateSecondaryResolution) {
        case "resume":
          await ipcResumeDownload(duplicateMatch.download.id);
          break;
        case "restart":
          await ipcRestartDownload(duplicateMatch.download.id);
          break;
        case "reveal":
          await ipcOpenDownloadFolder(duplicateMatch.download.id);
          break;
        case "inspect":
          break;
        case "keepBoth":
          break;
      }

      activateMonitorDownload(duplicateMatch.download);
      void refreshExistingDownloads();
    } catch (error) {
      setAddError(getCaptureErrorMessage(error));
    } finally {
      setDuplicateActionPending(false);
    }
  }, [activateMonitorDownload, duplicateActionPending, duplicateMatch, duplicateSecondaryResolution, refreshExistingDownloads]);

  const handleMonitorPause = () => {
    if (!monitorDownload) return;
    void ipcPauseDownload(monitorDownload.id).catch(() => null);
  };

  const handleMonitorResume = () => {
    if (!monitorDownload) return;
    void ipcResumeDownload(monitorDownload.id).catch(() => null);
  };

  const handleMonitorRestart = () => {
    if (!monitorDownload) return;
    void ipcRestartDownload(monitorDownload.id).catch(() => null);
  };

  const handleOpenFolder = () => {
    if (!monitorDownload) return;
    void ipcOpenDownloadFolder(monitorDownload.id).catch(() => null);
  };

  const mergeMonitorDownload = useCallback((next: Download) => {
    setMonitorDownload((current) => {
      if (!current || current.id !== next.id) {
        return next;
      }

      return {
        ...current,
        ...next,
        segments: current.segments,
      };
    });
  }, []);

  const handleApplySpeedLimit = useCallback(async () => {
    if (!monitorDownload) {
      return;
    }

    const parsed = parseSpeedLimitDraft(speedLimitEnabled, speedLimitValue, speedLimitUnit);
    if (parsed.error) {
      setTransferOptionsError(parsed.error);
      setTransferOptionsNotice(null);
      return;
    }

    setTransferOptionsSaving(true);
    setTransferOptionsError(null);
    setTransferOptionsNotice(null);
    try {
      const updated = await ipcSetDownloadTransferOptions(
        monitorDownload.id,
        parsed.limitBytesPerSecond,
      );
      mergeMonitorDownload(updated);
      setTransferOptionsNotice("Cap applied live across all segments.");
    } catch (error) {
      setTransferOptionsError(getCaptureErrorMessage(error));
    } finally {
      setTransferOptionsSaving(false);
    }
  }, [mergeMonitorDownload, monitorDownload, speedLimitEnabled, speedLimitUnit, speedLimitValue]);

  const handleSetSpeedLimitPreset = useCallback(async (limitBytesPerSecond: number) => {
    if (!monitorDownload) return;
    const draft = speedLimitDraftFromValue(limitBytesPerSecond);
    setSpeedLimitEnabled(draft.enabled);
    setSpeedLimitValue(draft.value);
    setSpeedLimitUnit(draft.unit);
    setTransferOptionsError(null);
    setTransferOptionsNotice(null);
    setTransferOptionsSaving(true);
    try {
      const updated = await ipcSetDownloadTransferOptions(
        monitorDownload.id,
        limitBytesPerSecond,
      );
      mergeMonitorDownload(updated);
    } catch (error) {
      setTransferOptionsError(getCaptureErrorMessage(error));
    } finally {
      setTransferOptionsSaving(false);
    }
  }, [mergeMonitorDownload, monitorDownload]);

  const handleSetUnlimited = useCallback(async () => {
    if (!monitorDownload) return;
    const wasLimited = monitorDownload.speedLimitBytesPerSecond != null;
    setSpeedLimitEnabled(false);
    setTransferOptionsError(null);
    setTransferOptionsNotice(null);
    if (!wasLimited) return;
    setTransferOptionsSaving(true);
    try {
      const updated = await ipcSetDownloadTransferOptions(
        monitorDownload.id,
        null,
      );
      mergeMonitorDownload(updated);
    } catch (error) {
      setSpeedLimitEnabled(true);
      setTransferOptionsError(getCaptureErrorMessage(error));
    } finally {
      setTransferOptionsSaving(false);
    }
  }, [mergeMonitorDownload, monitorDownload]);

  const handleToggleOpenFolderOnCompletion = useCallback(async (checked: boolean) => {
    if (!monitorDownload || completionOptionsSaving) {
      return;
    }

    const previous = monitorDownload.openFolderOnCompletion;
    mergeMonitorDownload({ ...monitorDownload, openFolderOnCompletion: checked });
    setCompletionOptionsSaving(true);
    setCompletionOptionsError(null);
    try {
      const updated = await ipcSetDownloadCompletionOptions(monitorDownload.id, checked);
      mergeMonitorDownload(updated);
    } catch (error) {
      mergeMonitorDownload({ ...monitorDownload, openFolderOnCompletion: previous });
      setCompletionOptionsError(getCaptureErrorMessage(error));
    } finally {
      setCompletionOptionsSaving(false);
    }
  }, [completionOptionsSaving, mergeMonitorDownload, monitorDownload]);

  const focusMainWindow = () => {
    void ipcFocusMainWindow().catch(() => null);
    void closeWindow();
  };

  if (!payload && !monitorDownload) {
    return (
      <div className="flex h-full flex-col bg-[hsl(var(--background))] text-foreground">
        <CompactTitleBar title="VDM Capture" onClose={closeWindow} onMinimize={minimizeWindow} />
        <div className="flex flex-1 flex-col items-center justify-center gap-2 text-[12px] text-muted-foreground/50">
          <Loader2 size={14} className="animate-spin" />
          <span>Waiting for browser extension…</span>
          <button
            type="button"
            onClick={() => {
              const windowLabel = windowLabelRef.current;
              if (!windowLabel) {
                return;
              }

              void ipcCaptureWindowReady(windowLabel)
                .then((pending) => { if (pending) applyIncomingPayload(pending); })
                .catch(() => null);
            }}
            className="mt-1 rounded-[3px] border border-border px-2 py-1 text-[11px] text-muted-foreground/60 hover:bg-accent hover:text-foreground transition-colors"
          >
            Retry Capture Sync
          </button>
        </div>
      </div>
    );
  }

  if (monitorDownload) {
    const dl = monitorDownload;
    const live = liveStats ?? { status: dl.status, downloaded: dl.downloaded, speed: dl.speed, timeLeft: dl.timeLeft };
    const monitorSegments = liveSegments.length > 0 ? liveSegments : dl.segments;
    const liveDownload: Download = {
      ...dl,
      status: live.status,
      downloaded: live.downloaded,
      speed: live.speed,
      timeLeft: live.timeLeft,
      segments: monitorSegments,
    };
    const totalSize = dl.size > 0 ? dl.size : probe?.size ?? payload?.sizeHint ?? null;
    const pct = totalSize && totalSize > 0
      ? Math.min(100, Math.max(0, (live.downloaded / totalSize) * 100))
      : null;
    const isActive = live.status === "downloading";
    const isStarting = live.status === "queued";
    const isPaused = live.status === "paused";
    const isFinished = live.status === "finished";
    const canPause = canPauseDownload(liveDownload);
    const canResume = canResumeDownload(liveDownload);
    const canRestart = canRestartDownload(liveDownload);
    const currentSpeedLimit = dl.speedLimitBytesPerSecond ?? null;
    const effectiveSpeedLimit = effectiveSpeedLimitBytesPerSecond(currentSpeedLimit, globalSpeedLimit);
    const usingGlobalSpeedLimit = currentSpeedLimit == null && effectiveSpeedLimit != null;
    const speedLimitModeLabel = currentSpeedLimit != null
      ? "Manual override"
      : usingGlobalSpeedLimit
        ? "Global default"
        : "Unlimited";
    const draftSpeedLimit = parseSpeedLimitDraft(speedLimitEnabled, speedLimitValue, speedLimitUnit);
    const speedLimitDirty = draftSpeedLimit.error == null && draftSpeedLimit.limitBytesPerSecond !== currentSpeedLimit;
    const speedLimitUtilization = effectiveSpeedLimit && effectiveSpeedLimit > 0
      ? Math.min(100, Math.round((live.speed / effectiveSpeedLimit) * 100))
      : null;
    const monitorSegmentStats = monitorSegments.reduce(
      (stats, segment) => {
        if (segment.status === "downloading") {
          stats.active += 1;
        } else if (segment.status === "finished") {
          stats.finished += 1;
        }
        return stats;
      },
      { active: 0, finished: 0 },
    );
    const monitorStatusText = isStarting
      ? live.downloaded > 0
        ? "Resuming"
        : "Starting"
      : live.status.charAt(0).toUpperCase() + live.status.slice(1);
    const resumeLabel = live.status === "error"
      ? "Retry"
      : live.downloaded > 0
      ? "Resume"
      : "Start";

    return (
      <div className="flex h-full flex-col overflow-hidden text-foreground bg-[hsl(var(--background))]">
        <CompactTitleBar
          title={dl.name || "Downloading…"}
          onClose={closeWindow}
          onMinimize={minimizeWindow}
        />

        <div className="h-[2px] w-full bg-border/30 shrink-0">
          <div
            className={cn(
              "h-full transition-[width] duration-300",
              isFinished
                ? "bg-[hsl(var(--status-finished))]"
                : "bg-[linear-gradient(90deg,hsl(var(--primary)),hsl(198,85%,58%))]",
            )}
            style={{ width: `${pct ?? 0}%` }}
          />
        </div>

      <div className="flex shrink-0 items-stretch border-b border-border bg-[hsl(var(--toolbar))]">
          {MONITOR_TABS.map(({ id, label, Icon }) => {
            return (
              <button
                key={id}
                type="button"
                onClick={() => setMonitorTab(id)}
                className={cn(
                  "relative flex items-center gap-1 px-3 h-[27px] text-[11px] border-r border-border transition-colors",
                  monitorTab === id
                    ? "bg-[hsl(var(--background))] text-foreground/90 font-semibold after:absolute after:bottom-0 after:left-0 after:right-0 after:h-[2px] after:bg-primary"
                    : "text-muted-foreground/68 hover:text-foreground/80 hover:bg-accent/50",
                )}
              >
                <Icon size={10} className="shrink-0" />
                {label}
              </button>
            );
          })}
        </div>

        <div className="flex-1 overflow-y-auto px-3 py-2">
          {monitorTab === "info" && (
            <div className="divide-y divide-border/30">
              <InfoRow label="Name" value={dl.name} />
              <InfoRow
                label="Status"
                value={monitorStatusText}
                valueClass={
                  isFinished ? "text-[hsl(var(--status-finished))]" :
                  isActive || isStarting ? "text-primary" :
                  isPaused ? "text-yellow-400" : undefined
                }
              />
              <InfoRow label="Size" value={totalSize != null ? formatBytes(totalSize) : "Unknown"} />
              <InfoRow
                label="Downloaded"
                value={
                  pct != null
                    ? `${formatBytes(live.downloaded)} (${pct.toFixed(1)}%)`
                    : formatBytes(live.downloaded)
                }
              />
              <InfoRow
                label="Speed"
                value={isStarting ? "Preparing connection…" : formatBytesPerSecond(live.speed, { idleLabel: "0 B/s" })}
              />
              <InfoRow
                label="Cap"
                value={
                  <span className="flex items-center gap-1.5 truncate">
                    <span>
                      {effectiveSpeedLimit
                        ? formatBytesPerSecond(effectiveSpeedLimit, { idleLabel: "—" })
                        : "Unlimited"}
                    </span>
                    <span className="shrink-0 text-[10px] text-muted-foreground/45">{speedLimitModeLabel}</span>
                  </span>
                }
                valueClass={
                  currentSpeedLimit
                    ? "text-[hsl(var(--status-downloading))]"
                    : usingGlobalSpeedLimit
                      ? "text-foreground/76"
                      : "text-[hsl(var(--status-finished))]"
                }
              />
              <InfoRow label="Time Left" value={formatTimeRemaining(live.timeLeft, { emptyLabel: "—" })} />
              <InfoRow
                label="Resumable"
                value={dl.capabilities.resumable ? "Yes" : "No"}
                valueClass={dl.capabilities.resumable ? "text-[hsl(var(--status-finished))]" : "text-muted-foreground/50"}
              />
              {dl.capabilities.segmented && monitorSegments.length > 0 && (
                <InfoRow
                  label="Segments"
                  value={`${monitorSegments.length} total · ${monitorSegmentStats.active} active`}
                />
              )}
            </div>
          )}

          {monitorTab === "speed" && (
            <div className="flex flex-col gap-3 pt-0.5">
              {/* Speed hero */}
              <div className="flex items-end justify-between gap-2">
                <div className="min-w-0">
                  <div
                    key={Math.floor(live.speed / (64 * 1024))}
                    className={cn(
                      "animate-speed-pop tabular-nums font-semibold leading-none tracking-[-0.02em]",
                      isStarting && "text-foreground/38",
                    )}
                    style={{
                      fontSize: "28px",
                      color: !isStarting && live.speed > 0
                        ? "hsl(var(--foreground) / 0.9)"
                        : undefined,
                    }}
                  >
                    {isStarting ? "—" : formatBytesPerSecond(live.speed, { idleLabel: "0 B/s" })}
                  </div>
                  <div className="mt-[3px] text-[9px] uppercase tracking-[0.1em] text-muted-foreground/32">
                    {isStarting ? "connecting" : "per second"}
                  </div>
                </div>
                <div className="flex shrink-0 flex-col items-end gap-1 pb-[2px]">
                  <span className={cn(
                    "rounded-[4px] border px-1.5 py-[3px] text-[11px] font-semibold tabular-nums",
                    effectiveSpeedLimit
                      ? currentSpeedLimit
                        ? "border-[hsl(var(--status-downloading)/0.4)] bg-[hsl(var(--status-downloading)/0.12)] text-[hsl(var(--status-downloading))]"
                        : "border-border/55 bg-white/[0.04] text-foreground/68"
                      : "border-[hsl(var(--status-finished)/0.28)] bg-[hsl(var(--status-finished)/0.08)] text-[hsl(var(--status-finished)/0.9)]",
                  )}>
                    {effectiveSpeedLimit
                      ? formatBytesPerSecond(effectiveSpeedLimit, { idleLabel: "—" })
                      : "Unlimited"}
                  </span>
                  <span className="text-[9px] text-muted-foreground/28">{speedLimitModeLabel}</span>
                </div>
              </div>

              {effectiveSpeedLimit != null && speedLimitUtilization != null && (
                <div>
                  <div className="h-[2px] overflow-hidden rounded-full bg-border/30">
                    <div
                      className="h-full rounded-full bg-[linear-gradient(90deg,hsl(var(--primary)),hsl(198,85%,58%))] transition-[width] duration-300"
                      style={{ width: `${speedLimitUtilization}%` }}
                    />
                  </div>
                  <div className="mt-[3px] flex justify-between text-[9px] text-muted-foreground/28">
                    <span>Utilization</span>
                    <span className="tabular-nums">{speedLimitUtilization}%</span>
                  </div>
                </div>
              )}

              <div className="h-px bg-border/20" />

              <div className="flex flex-col gap-2">
                {/* Override toggle */}
                <div className="flex items-center gap-2">
                  <span className="flex-1 text-[10px] font-medium text-foreground/58">Per-download cap</span>
                  <div className="flex items-center gap-px rounded-[4px] border border-border/50 bg-black/20 p-px">
                    <button
                      type="button"
                      onClick={() => void handleSetUnlimited()}
                      disabled={transferOptionsSaving}
                      className={cn(
                        "rounded-[3px] px-2.5 py-[4px] text-[10px] font-medium transition-colors",
                        !speedLimitEnabled
                          ? "bg-white/[0.08] text-foreground/80"
                          : "text-muted-foreground/40 hover:text-foreground/62",
                      )}
                    >
                      Default
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setSpeedLimitEnabled(true);
                        setTransferOptionsError(null);
                        setTransferOptionsNotice(null);
                      }}
                      className={cn(
                        "rounded-[3px] px-2.5 py-[4px] text-[10px] font-medium transition-colors",
                        speedLimitEnabled
                          ? "bg-[hsl(var(--status-downloading)/0.18)] text-[hsl(var(--status-downloading))]"
                          : "text-muted-foreground/40 hover:text-foreground/62",
                      )}
                    >
                      Override
                    </button>
                  </div>
                </div>

                <div className="text-[9.5px] leading-[1.5] text-muted-foreground/40">
                  {currentSpeedLimit != null
                    ? "This download uses its own cap, overriding the global default."
                    : usingGlobalSpeedLimit
                      ? `Global cap: ${formatBytesPerSecond(effectiveSpeedLimit!, { idleLabel: "—" })}.`
                      : "No cap set — download is unlimited."}
                </div>

                {speedLimitEnabled && (
                  <div className="flex items-center gap-1.5">
                    <input
                      type="number"
                      min="0"
                      step="0.1"
                      disabled={transferOptionsSaving}
                      value={speedLimitValue}
                      onChange={(event) => {
                        setSpeedLimitValue(event.target.value);
                        setTransferOptionsError(null);
                        setTransferOptionsNotice(null);
                      }}
                      className="h-[27px] min-w-0 flex-1 rounded-[4px] border border-border/65 bg-black/20 px-2 text-[12px] tabular-nums text-foreground/85 outline-none focus:border-[hsl(var(--status-downloading)/0.55)] transition-colors"
                    />
                    <select
                      disabled={transferOptionsSaving}
                      value={speedLimitUnit}
                      onChange={(event) => {
                        setSpeedLimitUnit(event.target.value as SpeedLimitUnit);
                        setTransferOptionsError(null);
                        setTransferOptionsNotice(null);
                      }}
                      className="h-[27px] w-[64px] shrink-0 rounded-[4px] border border-border/65 bg-black/20 px-1.5 text-[11px] text-foreground/85 outline-none transition-colors"
                    >
                      <option value="kb">KB/s</option>
                      <option value="mb">MB/s</option>
                      <option value="gb">GB/s</option>
                    </select>
                    <button
                      type="button"
                      onClick={() => void handleApplySpeedLimit()}
                      disabled={transferOptionsSaving || (!speedLimitDirty && draftSpeedLimit.error == null)}
                      className={cn(
                        "flex h-[27px] shrink-0 items-center justify-center rounded-[4px] px-3 text-[11px] font-medium transition-colors",
                        transferOptionsSaving || (!speedLimitDirty && draftSpeedLimit.error == null)
                          ? "cursor-not-allowed border border-border/50 bg-black/10 text-muted-foreground/30"
                          : "border border-[hsl(var(--status-downloading)/0.4)] bg-[hsl(var(--status-downloading)/0.14)] text-[hsl(var(--status-downloading))] hover:bg-[hsl(var(--status-downloading)/0.22)]",
                      )}
                    >
                      {transferOptionsSaving ? <Loader2 size={11} className="animate-spin" /> : "Apply"}
                    </button>
                  </div>
                )}

                <div className="flex flex-wrap gap-1">
                  {SPEED_LIMIT_PRESETS_MB.map((preset) => {
                    const presetBytes = preset * SPEED_LIMIT_UNIT_FACTORS.mb;
                    return (
                      <button
                        key={preset}
                        type="button"
                        onClick={() => void handleSetSpeedLimitPreset(presetBytes)}
                        className={cn(
                          "rounded-[4px] border px-2 py-[3px] text-[10px] font-medium transition-colors",
                          currentSpeedLimit === presetBytes
                            ? "border-[hsl(var(--status-downloading)/0.4)] bg-[hsl(var(--status-downloading)/0.14)] text-[hsl(var(--status-downloading))]"
                            : "border-border/55 bg-black/10 text-muted-foreground/55 hover:border-border/80 hover:text-foreground/78",
                        )}
                      >
                        {preset} MB/s
                      </button>
                    );
                  })}
                </div>

                {transferOptionsError && (
                  <InlineNotice
                    tone="error"
                    message={simplifyUserMessage(transferOptionsError)}
                    onDismiss={() => setTransferOptionsError(null)}
                    className="rounded-[5px] px-2 py-1.5 text-[10.5px]"
                  />
                )}
                {!transferOptionsError && transferOptionsNotice && (
                  <InlineNotice
                    tone="success"
                    message={transferOptionsNotice}
                    onDismiss={() => setTransferOptionsNotice(null)}
                    className="rounded-[5px] px-2 py-1.5 text-[10.5px]"
                  />
                )}
              </div>

              {totalSize != null && totalSize > 0 && (
                <>
                  <div className="h-px bg-border/20" />
                  <div className="grid grid-cols-2 gap-x-3 gap-y-[3px] text-[11px]">
                    <span className="text-muted-foreground/42">Downloaded</span>
                    <span className="tabular-nums text-right text-foreground/68">{formatBytes(live.downloaded)}</span>
                    <span className="text-muted-foreground/42">Remaining</span>
                    <span className="tabular-nums text-right text-foreground/68">
                      {formatBytes(Math.max(0, totalSize - live.downloaded))}
                    </span>
                    <span className="text-muted-foreground/42">Progress</span>
                    <span className="tabular-nums text-right text-foreground/68">{pct?.toFixed(1) ?? "0"}%</span>
                    {live.timeLeft != null && live.timeLeft > 0 && (
                      <>
                        <span className="text-muted-foreground/42">ETA</span>
                        <span className="tabular-nums text-right text-foreground/68">
                          {formatTimeRemaining(live.timeLeft, { emptyLabel: "—" })}
                        </span>
                      </>
                    )}
                    {effectiveSpeedLimit != null && (
                      <>
                        <span className="text-muted-foreground/42">Headroom</span>
                        <span className="tabular-nums text-right text-foreground/68">
                          {formatBytesPerSecond(Math.max(0, effectiveSpeedLimit - live.speed), { idleLabel: "0 B/s" })}
                        </span>
                      </>
                    )}
                  </div>
                </>
              )}
            </div>
          )}

          {monitorTab === "completion" && (
            <div className="flex flex-col gap-3 pt-0.5">
              <div className="flex items-center justify-between gap-2 rounded-[6px] border border-border/40 bg-black/10 px-2.5 py-2">
                <div className="min-w-0 flex-1">
                  <div className="text-[11px] font-medium text-foreground/80">Open folder on finish</div>
                  <div className="mt-[2px] text-[9.5px] text-muted-foreground/42">
                    Reveals the file in Explorer when done.
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  {completionOptionsSaving && <Loader2 size={10} className="animate-spin text-muted-foreground/45" />}
                  <Checkbox
                    checked={dl.openFolderOnCompletion}
                    disabled={completionOptionsSaving}
                    onChange={(checked) => { void handleToggleOpenFolderOnCompletion(checked); }}
                  />
                </div>
              </div>

              {completionOptionsError && (
                <InlineNotice
                  tone="error"
                  message={simplifyUserMessage(completionOptionsError)}
                  onDismiss={() => setCompletionOptionsError(null)}
                  className="rounded-[5px] px-2 py-1.5 text-[10.5px]"
                />
              )}

              <div className="h-px bg-border/20" />

              <div className="flex flex-wrap gap-1.5">
                <button
                  type="button"
                  onClick={focusMainWindow}
                  className="flex items-center gap-1.5 rounded-[5px] border border-[hsl(var(--primary)/0.35)] bg-[hsl(var(--primary)/0.1)] px-2.5 py-1.5 text-[11px] text-[hsl(var(--primary))] transition-colors hover:bg-[hsl(var(--primary)/0.18)]"
                >
                  <ExternalLink size={11} />
                  Open in VDM
                </button>
                {isFinished && (
                  <button
                    type="button"
                    onClick={handleOpenFolder}
                    className="flex items-center gap-1.5 rounded-[5px] border border-[hsl(var(--status-finished)/0.35)] bg-[hsl(var(--status-finished)/0.1)] px-2.5 py-1.5 text-[11px] text-[hsl(var(--status-finished))] transition-colors hover:bg-[hsl(var(--status-finished)/0.18)]"
                  >
                    <FolderOpen size={11} />
                    Open Folder
                  </button>
                )}
              </div>
            </div>
          )}
        </div>

        {segmentsExpanded && monitorSegments.length > 0 && (
          <div className="shrink-0 border-t border-border/40 px-3 pt-1.5 pb-2">
            <div className="flex items-center justify-between mb-1.5">
              <span className="text-[9.5px] uppercase tracking-[0.08em] text-muted-foreground/35">
                Segments
              </span>
              <span className="text-[9.5px] tabular-nums text-muted-foreground/35">
                {monitorSegmentStats.active} active
                {" · "}{monitorSegmentStats.finished} done
                {" · "}{monitorSegments.length} total
              </span>
            </div>
            <TransferSegmentStrip
              segments={monitorSegments}
              compact={false}
              barClassName="h-2.5"
              className="gap-0.5"
            />
          </div>
        )}

      <div className="shrink-0 flex items-center gap-1.5 border-t border-border/90 bg-[hsl(var(--card)/0.32)] px-2.5 py-2">
          <button
            type="button"
            onClick={() => setSegmentsExpanded((v) => !v)}
            title={segmentsExpanded ? "Hide segments" : "Show segments"}
            className={cn(
              "flex h-[26px] items-center gap-1 rounded-[3px] border px-2 text-[10px] tabular-nums transition-colors shrink-0",
              segmentsExpanded
                ? "border-primary/40 bg-[hsl(var(--status-downloading)/0.12)] text-[hsl(var(--status-downloading)/0.9)]"
                : "border-border/80 bg-[hsl(var(--card))] text-muted-foreground/68 hover:text-foreground/80 hover:bg-accent hover:border-border",
            )}
          >
            {segmentsExpanded ? <ChevronUp size={9} /> : <ChevronDown size={9} />}
            {monitorSegments.length > 0 && (
              <span>{monitorSegments.length} seg</span>
            )}
          </button>
          <div className="flex-1" />
          {isFinished ? (
            <>
              {isExecutable(dl.name) && (
                <button
                  type="button"
                  onClick={() => void ipcOpenDownloadFile(dl.id).catch(() => null)}
                  className={cn(
                    "flex h-[26px] items-center gap-1.5 rounded-[3px] px-3 text-[11.5px] font-medium transition-colors",
                    "bg-[linear-gradient(180deg,hsl(var(--primary)/0.28),hsl(var(--primary)/0.16))]",
                    "border border-[hsl(var(--primary)/0.38)] text-[hsl(var(--primary))] hover:brightness-125",
                  )}
                >
                  <Play size={10} />
                  Open
                </button>
              )}
              <button
                type="button"
                onClick={handleOpenFolder}
                className={cn(
                  "flex h-[26px] items-center gap-1.5 rounded-[3px] px-3 text-[11.5px] font-medium transition-colors",
                  "bg-[linear-gradient(180deg,hsl(var(--status-finished)/0.22),hsl(var(--status-finished)/0.12))]",
                  "border border-[hsl(var(--status-finished)/0.35)] text-[hsl(var(--status-finished))]",
                  "hover:brightness-125",
                )}
              >
                <FolderOpen size={11} />
                Open Folder
              </button>
              <button
                type="button"
                onClick={closeWindow}
                className="flex h-[26px] items-center gap-1 rounded-[3px] border border-border bg-[hsl(var(--card))] px-3 text-[11.5px] text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
              >
                Close
              </button>
            </>
          ) : (
            <>
              {(isActive || isPaused) && (
                <button
                  type="button"
                  onClick={handleOpenFolder}
                  title="Open download folder"
                  className="flex h-[26px] items-center gap-1 rounded-[3px] border border-border/80 bg-[hsl(var(--card))] px-2 text-[11px] text-muted-foreground/72 hover:text-foreground/88 hover:bg-accent transition-colors"
                >
                  <FolderOpen size={10} />
                </button>
              )}
              {canPause ? (
                <button
                  type="button"
                  onClick={handleMonitorPause}
                  className="flex h-[26px] items-center gap-1 rounded-[3px] border border-border bg-[hsl(var(--card))] px-3 text-[11.5px] text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
                >
                  <Pause size={10} />
                  Pause
                </button>
              ) : isStarting ? (
                <button
                  type="button"
                  disabled
                  className="flex h-[26px] items-center gap-1 rounded-[3px] border border-border bg-[hsl(var(--card))] px-3 text-[11.5px] text-muted-foreground/55"
                >
                  <Loader2 size={10} className="animate-spin" />
                  Starting…
                </button>
              ) : canRestart && live.status === "error" ? (
                <button
                  type="button"
                  onClick={handleMonitorRestart}
                  className={cn(
                    "flex h-[26px] items-center gap-1 rounded-[3px] px-3 text-[11.5px] font-medium transition-colors",
                    "bg-[linear-gradient(180deg,hsl(var(--primary)),hsl(var(--primary)/0.82))] text-white hover:brightness-110",
                  )}
                >
                  <Play size={10} />
                  Restart
                </button>
              ) : canResume ? (
                <button
                  type="button"
                  onClick={handleMonitorResume}
                  className={cn(
                    "flex h-[26px] items-center gap-1 rounded-[3px] px-3 text-[11.5px] font-medium transition-colors",
                    "bg-[linear-gradient(180deg,hsl(var(--primary)),hsl(var(--primary)/0.82))] text-white hover:brightness-110",
                  )}
                >
                  <Play size={10} />
                  {resumeLabel}
                </button>
              ) : (
                <button
                  type="button"
                  onClick={focusMainWindow}
                  className="flex h-[26px] items-center gap-1 rounded-[3px] border border-border bg-[hsl(var(--card))] px-3 text-[11.5px] text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
                >
                  Open in VDM
                </button>
              )}
              <button
                type="button"
                onClick={closeWindow}
                className="flex h-[26px] items-center gap-1 rounded-[3px] border border-border bg-[hsl(var(--card))] px-3 text-[11.5px] text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
              >
                Hide
              </button>
            </>
          )}
        </div>
      </div>
    );
  }

  const displaySize = probe?.size != null && probe.size > 0
    ? formatBytes(probe.size)
    : payload?.sizeHint != null
    ? formatBytes(payload.sizeHint)
    : null;
  const probeWarningMessage = probeWarningDismissed || !probe
    ? null
    : firstVisibleProbeWarning(probe.warnings);
  const captureErrorMessage = addError ?? probeError;

  const sourceBadge = payload ? sourceBadgeLabel(payload.source) : null;

  return (
    <div className="flex h-full flex-col overflow-hidden text-foreground bg-[hsl(var(--background))]">
      <CompactTitleBar title="Add Download" onClose={closeWindow} onMinimize={minimizeWindow} />

      <div className="flex flex-1 flex-col gap-1.5 p-2 overflow-y-auto min-h-0">
        <div
          className={cn(
            "flex items-center gap-2 rounded-[4px] border px-2.5 h-[30px] min-w-0 transition-[border-color] duration-200",
            "bg-[hsl(var(--card))]",
            probing
              ? "border-primary/30"
              : probe && !probeError
              ? "border-emerald-700/30"
              : probeError
              ? "border-yellow-600/25"
              : "border-border",
          )}
        >
          <span className="shrink-0">
            {probing ? (
              <Loader2 size={13} className="animate-spin text-primary/50" />
            ) : probe && !probeError ? (
              <ShieldCheck size={13} className="text-emerald-500/76" />
            ) : probeError ? (
              <AlertTriangle size={13} className="text-yellow-500/58" />
            ) : (
              <ShieldCheck size={13} className="text-muted-foreground/22" />
            )}
          </span>
          <span
            className="min-w-0 flex-1 truncate text-[11.5px] text-foreground/62 font-mono"
            title={payload?.url}
          >
            {truncateUrl(payload?.url ?? "")}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-1">
          <span className="rounded-[3px] border border-white/[0.14] bg-white/[0.07] px-1.5 py-px text-[10px] text-foreground/70 font-mono">
            {hostFromUrl(payload?.url ?? "")}
          </span>
          {sourceBadge && (
            <span className="rounded-[3px] border border-primary/20 bg-primary/[0.07] px-1.5 py-px text-[10px] text-primary/75">
              {sourceBadge}
            </span>
          )}
          {probe?.rangeSupported && (
            <span className="rounded-[3px] border border-[hsl(var(--status-finished)/0.28)] bg-[hsl(var(--status-finished)/0.07)] px-1.5 py-px text-[10px] text-[hsl(var(--status-finished)/0.85)]">
              Resumable
            </span>
          )}
          {probe?.segmented && probe.plannedConnections > 1 && (
            <span className="rounded-[3px] border border-[hsl(var(--status-downloading)/0.28)] bg-[hsl(var(--status-downloading)/0.07)] px-1.5 py-px text-[10px] text-[hsl(var(--status-downloading)/0.85)]">
              {probe.plannedConnections} streams
            </span>
          )}
          {probe?.hostDiagnostics?.negotiatedProtocol && (
            <span className="rounded-[3px] border border-[hsl(var(--status-downloading)/0.32)] bg-[hsl(var(--status-downloading)/0.09)] px-1.5 py-px text-[10px] text-[hsl(var(--status-downloading)/0.82)] uppercase tracking-widest font-mono">
              {probe.hostDiagnostics.negotiatedProtocol}
            </span>
          )}
          {probe?.hostDiagnostics?.hardNoRange && (
            <span className="rounded-[3px] border border-yellow-500/25 bg-yellow-500/[0.07] px-1.5 py-px text-[10px] text-yellow-500/75">
              No resume
            </span>
          )}
        </div>

        <DownloadCapturePane
          variant="compact"
          category={category}
          onCategoryChange={setCategory}
          savePath={savePath}
          onSavePathChange={setSavePath}
          onBrowseSavePath={handleBrowse}
          filename={resolvedName}
          onFilenameChange={setName}
          filenamePlaceholder={probing ? "Probing…" : "filename.ext"}
          sizeLabel={displaySize}
          warningMessage={probeWarningMessage}
          errorMessage={captureErrorMessage}
          onWarningDismiss={() => setProbeWarningDismissed(true)}
          onErrorDismiss={() => {
            setAddError(null);
            setProbeError(null);
          }}
          duplicateActions={duplicateMatch && duplicateResolution ? {
            active: true,
            title: describeDuplicateMatch(duplicateMatch),
            detail: duplicateMatch.reason === "targetPath"
              ? "Rename it to save another copy."
              : undefined,
            primaryLabel: duplicateActionPending && duplicateResolution !== "inspect" && duplicateResolution !== "keepBoth"
              ? "Working..."
              : duplicateResolutionLabel(duplicateResolution, "compact"),
            onPrimary: () => {
              void handleDuplicatePrimaryAction();
            },
            secondaryLabel: duplicateSecondaryResolution
              ? duplicateActionPending && duplicateSecondaryResolution !== "inspect"
                ? "Working..."
                : duplicateResolutionLabel(duplicateSecondaryResolution, "compact")
              : undefined,
            onSecondary: duplicateSecondaryResolution
              ? () => {
                void handleDuplicateSecondaryAction();
              }
              : undefined,
          } : undefined}
          hideWarningWhenDuplicate
          filenameResetVisible={filenameResetVisible}
          onFilenameReset={() => {
            setName(probe?.suggestedName ?? "");
          }}
          fieldIds={{
            category: "capture-category",
            savePath: "capture-savepath",
            filename: "capture-filename",
          }}
        />
      </div>

      {/* Footer */}
      <div className="flex items-center justify-end gap-1.5 border-t border-border/80 bg-[hsl(var(--card)/0.28)] px-2.5 py-1.5 shrink-0">
        <button
          type="button"
          onClick={closeWindow}
          className="h-[26px] rounded-[3px] border border-border bg-[hsl(var(--card))] px-3 text-[11.5px] text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={duplicateMatch ? () => void handleDuplicatePrimaryAction() : handleAdd}
          disabled={adding || probing || duplicateActionPending}
          style={{
            background: adding || probing || duplicateActionPending
              ? "hsl(0,0%,20%)"
              : "linear-gradient(90deg, hsl(var(--accent-h) 22% 32%) 0%, hsl(var(--accent-h) 15% 25%) 55%, hsl(0,0%,18%) 100%)",
          }}
          className={cn(
            "flex h-[26px] items-center gap-1.5 rounded-[3px] px-3 text-[11.5px] font-semibold transition-all",
            "text-[hsl(0,0%,92%)] hover:brightness-110 active:brightness-95",
            "disabled:opacity-40 disabled:pointer-events-none",
          )}
        >
          {duplicateMatch && duplicateResolution ? (
            <>
              {duplicateActionPending && duplicateResolution !== "inspect" && duplicateResolution !== "keepBoth" ? <Loader2 size={10} className="animate-spin" /> : <ArrowDownToLine size={10} />}
              {duplicateActionPending && duplicateResolution !== "inspect" && duplicateResolution !== "keepBoth"
                ? "Working..."
                : duplicateResolutionLabel(duplicateResolution, "compact")}
            </>
          ) : adding ? (
            <><Loader2 size={10} className="animate-spin" /> Adding…</>
          ) : (
            <><ArrowDownToLine size={10} /> Download</>
          )}
        </button>
      </div>
    </div>
  );
}
