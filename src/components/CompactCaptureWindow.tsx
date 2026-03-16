import { useCallback, useEffect, useRef, useState } from "react";
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
import { cn } from "@/lib/utils";
import { canPauseDownload, canRestartDownload, canResumeDownload } from "@/lib/downloadActions";
import { formatBytes, formatBytesPerSecond, formatTimeRemaining } from "@/lib/format";
import {
  fromRawDownload,
  ipcAddDownload,
  ipcOpenDownloadFolder,
  ipcPauseDownload,
  ipcProbeDownload,
  ipcSetDownloadCompletionOptions,
  ipcSetDownloadTransferOptions,
  ipcRestartDownload,
  ipcResumeDownload,
  ipcTakePendingCapturePayload,
  type RawDownload,
} from "@/lib/ipc";
import type {
  CapturePayload,
  Download,
  DownloadContentCategory,
  DownloadProbe,
  DownloadProgressDiffEvent,
  DownloadSegment,
} from "@/types/download";
import {
  DownloadCapturePane,
  getCaptureErrorMessage,
  guessCaptureCategory,
  useDefaultCaptureSavePath,
} from "@/components/DownloadCapturePane";
import { TransferSegmentStrip } from "@/components/TransferSegmentStrip";
import { Checkbox } from "@/components/ui/checkbox";

type MonitorTab = "info" | "speed" | "completion";
type SpeedLimitUnit = "kb" | "mb" | "gb";

const SPEED_LIMIT_UNIT_FACTORS: Record<SpeedLimitUnit, number> = {
  kb: 1024,
  mb: 1024 * 1024,
  gb: 1024 * 1024 * 1024,
};

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
      return "Link click";
    case "manual":
      return "Manual";
    default:
      return null;
  }
}

function formatSpeedLimitEditorValue(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return "25";
  }

  const rounded = value >= 100 ? value.toFixed(0) : value >= 10 ? value.toFixed(1) : value.toFixed(2);
  return rounded.replace(/\.0+$/, "").replace(/(\.\d*[1-9])0+$/, "$1");
}

function speedLimitDraftFromValue(limitBytesPerSecond: number | null): {
  enabled: boolean;
  value: string;
  unit: SpeedLimitUnit;
} {
  if (!limitBytesPerSecond || limitBytesPerSecond <= 0) {
    return { enabled: false, value: "25", unit: "mb" };
  }

  for (const unit of ["gb", "mb", "kb"] as const) {
    const scaled = limitBytesPerSecond / SPEED_LIMIT_UNIT_FACTORS[unit];
    if (scaled >= 1) {
      return {
        enabled: true,
        value: formatSpeedLimitEditorValue(scaled),
        unit,
      };
    }
  }

  return {
    enabled: true,
    value: formatSpeedLimitEditorValue(limitBytesPerSecond / SPEED_LIMIT_UNIT_FACTORS.kb),
    unit: "kb",
  };
}

function parseSpeedLimitDraft(
  enabled: boolean,
  value: string,
  unit: SpeedLimitUnit,
): { limitBytesPerSecond: number | null; error: string | null } {
  if (!enabled) {
    return { limitBytesPerSecond: null, error: null };
  }

  const numeric = Number.parseFloat(value.trim());
  if (!Number.isFinite(numeric) || numeric <= 0) {
    return { limitBytesPerSecond: null, error: "Enter a positive bandwidth limit." };
  }

  const limitBytesPerSecond = Math.round(numeric * SPEED_LIMIT_UNIT_FACTORS[unit]);
  if (!Number.isSafeInteger(limitBytesPerSecond) || limitBytesPerSecond <= 0) {
    return { limitBytesPerSecond: null, error: "The selected bandwidth limit is too large." };
  }

  return { limitBytesPerSecond, error: null };
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
    <div className="flex h-[28px] shrink-0 items-stretch justify-between border-b border-border bg-[hsl(var(--toolbar))] select-none">
      <div data-tauri-drag-region className="flex flex-1 items-center gap-1.5 min-w-0 pl-2.5">
        <div
          className="flex h-[13px] w-[13px] shrink-0 items-center justify-center rounded-[3px]"
          style={{ background: "linear-gradient(135deg, hsl(24,55%,52%), hsl(12,48%,34%))" }}
        >
          <ArrowDownToLine size={7} className="text-white" strokeWidth={2.5} />
        </div>
        <span className="truncate text-[10.5px] font-medium text-foreground/60 tracking-tight">{title}</span>
      </div>
      <div className="flex shrink-0 items-stretch">
        <button
          type="button"
          onClick={onMinimize}
          aria-label="Minimize"
          className="flex w-[32px] items-center justify-center text-foreground/25 hover:text-foreground/60 hover:bg-white/[0.06] transition-colors"
        >
          <Minus size={11} strokeWidth={1.5} />
        </button>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close"
          className="flex w-[32px] items-center justify-center text-foreground/25 hover:text-white hover:bg-[hsl(0,62%,44%)] transition-colors"
        >
          <X size={10} strokeWidth={1.75} />
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
  const [adding, setAdding] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  const [isDuplicate, setIsDuplicate] = useState(false);

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

  const abortRef = useRef<AbortController | null>(null);
  const savePathRef = useRef("");
  const lastPayloadKeyRef = useRef("");
  const monitorDownloadId = monitorDownload?.id ?? null;
  const monitorDownloadSpeedLimit = monitorDownload?.speedLimitBytesPerSecond ?? null;

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
      const firstWarn = result.warnings?.[0];
      if (firstWarn) setProbeError(firstWarn);
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
      setAddError(null);
      setIsDuplicate(false);
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
    const unlistenPromise = listen<CapturePayload>("extension://capture", (event) => {
      applyIncomingPayload(event.payload);
    });
    void ipcTakePendingCapturePayload()
      .then((pending) => { if (pending) applyIncomingPayload(pending); })
      .catch(() => null);
    return () => { void unlistenPromise.then((u) => u()); };
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

  const handleBrowse = async () => {
    const selected = await openDialog({ directory: true, defaultPath: savePath || undefined });
    if (selected && typeof selected === "string") setSavePath(selected);
  };

  const handleAdd = async () => {
    if (!payload) return;
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
      // Transition to monitor phase instead of closing.
      setMonitorDownload(dl);
      setLiveSegments(dl.segments);
      setLiveStats({ status: dl.status, downloaded: dl.downloaded, speed: dl.speed, timeLeft: dl.timeLeft });
      setMonitorTab("info");
      setSegmentsExpanded(false);
    } catch (err: unknown) {
      const msg = getCaptureErrorMessage(err);
      if (msg.toLowerCase().includes("already exists")) setIsDuplicate(true);
      setAddError(msg);
    } finally {
      setAdding(false);
    }
  };

  const handleRestart = async () => {
    if (!payload) return;
    setIsDuplicate(false);
    setAddError(null);
    await handleAdd();
  };

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
        monitorDownload.customMaxConnections ?? null,
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
        monitorDownload.customMaxConnections ?? null,
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
        monitorDownload.customMaxConnections ?? null,
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
              void ipcTakePendingCapturePayload()
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
    const draftSpeedLimit = parseSpeedLimitDraft(speedLimitEnabled, speedLimitValue, speedLimitUnit);
    const speedLimitDirty = draftSpeedLimit.error == null && draftSpeedLimit.limitBytesPerSecond !== currentSpeedLimit;
    const speedLimitUtilization = currentSpeedLimit && currentSpeedLimit > 0
      ? Math.min(100, Math.round((live.speed / currentSpeedLimit) * 100))
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

        <div className="flex shrink-0 border-b border-border bg-[hsl(var(--toolbar))]">
          {MONITOR_TABS.map(({ id, label, Icon }) => {
            return (
              <button
                key={id}
                type="button"
                onClick={() => setMonitorTab(id)}
                className={cn(
                  "flex items-center gap-1 px-3 h-[26px] text-[11px] border-r border-border transition-colors",
                  monitorTab === id
                    ? "bg-[hsl(var(--background))] text-foreground/85 font-medium"
                    : "text-muted-foreground/55 hover:text-foreground/70 hover:bg-accent/50",
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
                value={currentSpeedLimit ? formatBytesPerSecond(currentSpeedLimit, { idleLabel: "—" }) : "Unlimited"}
                valueClass={currentSpeedLimit ? "text-[hsl(var(--status-downloading))]" : "text-[hsl(var(--status-finished))]"}
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
            <div className="flex flex-col gap-2.5 pt-0.5">
              <div className="flex items-center justify-between gap-2">
                <div className="min-w-0">
                  {isStarting ? (
                    <div className="font-semibold leading-none text-foreground/55" style={{ fontSize: "20px" }}>
                      Preparing…
                    </div>
                  ) : (
                    <div
                      key={Math.floor(live.speed / (64 * 1024))}
                      className="animate-speed-pop tabular-nums font-semibold leading-none"
                      style={{
                        fontSize: "22px",
                        color: live.speed > 0
                          ? "hsl(var(--foreground) / 0.88)"
                          : "hsl(var(--muted-foreground) / 0.32)",
                      }}
                    >
                      {formatBytesPerSecond(live.speed, { idleLabel: "— B/s" })}
                    </div>
                  )}
                  <div className="mt-0.5 text-[9.5px] text-muted-foreground/38">
                    {isStarting ? "opening connection" : "live transfer rate"}
                  </div>
                </div>

                <div className="flex shrink-0 flex-col items-end gap-0.5">
                  <span className="text-[8.5px] uppercase tracking-[0.1em] text-muted-foreground/32">Cap</span>
                  <span className={cn(
                    "rounded-[4px] border px-1.5 py-[2px] text-[11px] font-semibold tabular-nums",
                    currentSpeedLimit
                      ? "border-[hsl(var(--status-downloading)/0.4)] bg-[hsl(var(--status-downloading)/0.12)] text-[hsl(var(--status-downloading))]"
                      : "border-[hsl(var(--status-finished)/0.3)] bg-[hsl(var(--status-finished)/0.1)] text-[hsl(var(--status-finished))]",
                  )}>
                    {currentSpeedLimit
                      ? formatBytesPerSecond(currentSpeedLimit, { idleLabel: "—" })
                      : "Unlimited"}
                  </span>
                </div>
              </div>

              {currentSpeedLimit != null && speedLimitUtilization != null && (
                <div>
                  <div className="mb-[3px] flex items-center justify-between text-[9px] text-muted-foreground/32">
                    <span>Cap utilization</span>
                    <span className="tabular-nums">{speedLimitUtilization}%</span>
                  </div>
                  <div className="h-[3px] overflow-hidden rounded-full bg-border/35">
                    <div
                      className="h-full rounded-full bg-[linear-gradient(90deg,hsl(var(--primary)),hsl(198,85%,58%))] transition-[width] duration-300"
                      style={{ width: `${speedLimitUtilization}%` }}
                    />
                  </div>
                </div>
              )}

              <div className="border-t border-border/20" />

              <div className="flex flex-col gap-1.5">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-[10px] font-medium text-foreground/65">Bandwidth cap</span>
                  <div className="flex items-center rounded-[5px] border border-border/60 bg-black/15 p-[2px]">
                    <button
                      type="button"
                      onClick={() => void handleSetUnlimited()}
                      disabled={transferOptionsSaving}
                      className={cn(
                        "rounded-[3px] px-2.5 py-[3px] text-[10px] font-medium leading-none transition-colors",
                        !speedLimitEnabled
                          ? "bg-[hsl(var(--status-finished)/0.2)] text-[hsl(var(--status-finished))]"
                          : "text-muted-foreground/50 hover:text-foreground/72",
                      )}
                    >
                      Unlimited
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setSpeedLimitEnabled(true);
                        setTransferOptionsError(null);
                        setTransferOptionsNotice(null);
                      }}
                      className={cn(
                        "rounded-[3px] px-2.5 py-[3px] text-[10px] font-medium leading-none transition-colors",
                        speedLimitEnabled
                          ? "bg-[hsl(var(--status-downloading)/0.18)] text-[hsl(var(--status-downloading))]"
                          : "text-muted-foreground/50 hover:text-foreground/72",
                      )}
                    >
                      Set cap
                    </button>
                  </div>
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
                      className="h-[27px] min-w-0 flex-1 rounded-[5px] border border-border/65 bg-black/20 px-2 text-[12px] tabular-nums text-foreground/85 outline-none focus:border-[hsl(var(--status-downloading)/0.55)] transition-colors"
                    />
                    <select
                      disabled={transferOptionsSaving}
                      value={speedLimitUnit}
                      onChange={(event) => {
                        setSpeedLimitUnit(event.target.value as SpeedLimitUnit);
                        setTransferOptionsError(null);
                        setTransferOptionsNotice(null);
                      }}
                      className="h-[27px] w-[64px] shrink-0 rounded-[5px] border border-border/65 bg-black/20 px-1.5 text-[11px] text-foreground/85 outline-none transition-colors"
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
                        "flex h-[27px] shrink-0 items-center justify-center rounded-[5px] px-3 text-[11px] font-medium transition-colors",
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
                          "rounded-[999px] border px-2 py-[2px] text-[10px] font-medium transition-colors",
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
                  <div className="flex items-start gap-1.5 rounded-[5px] border border-red-500/20 bg-red-500/[0.06] px-2 py-1.5 text-[10.5px] text-red-300/80">
                    <AlertTriangle size={11} className="mt-[1px] shrink-0" />
                    {transferOptionsError}
                  </div>
                )}
                {!transferOptionsError && transferOptionsNotice && (
                  <div className="rounded-[5px] border border-[hsl(var(--status-finished)/0.22)] bg-[hsl(var(--status-finished)/0.08)] px-2 py-1.5 text-[10.5px] text-[hsl(var(--status-finished)/0.82)]">
                    {transferOptionsNotice}
                  </div>
                )}
              </div>

              {totalSize != null && totalSize > 0 && (
                <>
                  <div className="border-t border-border/20" />
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
                    {currentSpeedLimit != null && (
                      <>
                        <span className="text-muted-foreground/42">Headroom</span>
                        <span className="tabular-nums text-right text-foreground/68">
                          {formatBytesPerSecond(Math.max(0, currentSpeedLimit - live.speed), { idleLabel: "0 B/s" })}
                        </span>
                      </>
                    )}
                  </div>
                </>
              )}

            </div>
          )}

          {monitorTab === "completion" && (
            <div className="flex flex-col gap-2.5 pt-0.5">
              <div className="flex items-start gap-2.5">
                <Checkbox
                  checked={dl.openFolderOnCompletion}
                  disabled={completionOptionsSaving}
                  onChange={(checked) => { void handleToggleOpenFolderOnCompletion(checked); }}
                  className="mt-[1px] shrink-0"
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="text-[11px] font-medium text-foreground/82">Open folder on finish</span>
                    {completionOptionsSaving && <Loader2 size={10} className="animate-spin text-muted-foreground/45" />}
                  </div>
                  <div className="mt-0.5 text-[10px] leading-[1.4] text-muted-foreground/45">
                    Reveals the file in Explorer as soon as the download finishes.
                  </div>
                </div>
              </div>

              {completionOptionsError && (
                <div className="flex items-start gap-1.5 rounded-[5px] border border-red-500/20 bg-red-500/[0.06] px-2 py-1.5 text-[10.5px] text-red-300/80">
                  <AlertTriangle size={11} className="mt-[1px] shrink-0" />
                  {completionOptionsError}
                </div>
              )}

              <div className="border-t border-border/20" />

              <div className="flex flex-wrap gap-1.5">
                <button
                  type="button"
                  onClick={focusMainWindow}
                  className="flex items-center gap-1.5 rounded-[5px] border border-border/60 bg-black/10 px-2.5 py-1.5 text-[11px] text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground/82"
                >
                  <ExternalLink size={11} />
                  View in VDM
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
                ? "border-border/70 bg-accent text-foreground/70"
                : "border-border bg-[hsl(var(--card))] text-muted-foreground/45 hover:text-foreground/65 hover:bg-accent",
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
                  className="flex h-[26px] items-center gap-1 rounded-[3px] border border-border bg-[hsl(var(--card))] px-2 text-[11px] text-muted-foreground/50 hover:text-foreground/80 hover:bg-accent transition-colors"
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

  const resolvedName = name.trim() || probe?.suggestedName || "";
  const sourceBadge = payload ? sourceBadgeLabel(payload.source) : null;

  return (
    <div className="flex h-full flex-col overflow-hidden text-foreground bg-[hsl(var(--background))]">
      <CompactTitleBar title="Add Download" onClose={closeWindow} onMinimize={minimizeWindow} />

      <div className="flex flex-1 flex-col gap-1.5 p-2 overflow-y-auto min-h-0">
        <div
          className={cn(
            "flex items-center gap-1.5 rounded-[3px] border px-2 h-[26px] min-w-0 transition-[border-color] duration-200",
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
              <Loader2 size={11} className="animate-spin text-primary/50" />
            ) : probe && !probeError ? (
              <ShieldCheck size={11} className="text-emerald-500/70" />
            ) : probeError ? (
              <AlertTriangle size={11} className="text-yellow-500/55" />
            ) : (
              <ShieldCheck size={11} className="text-muted-foreground/18" />
            )}
          </span>
          <span
            className="min-w-0 flex-1 truncate text-[11px] text-foreground/58 font-mono"
            title={payload?.url}
          >
            {truncateUrl(payload?.url ?? "")}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-1">
          <span className="rounded-[3px] border border-border/70 bg-[hsl(var(--card))] px-1.5 py-px text-[10px] text-muted-foreground/60 font-mono">
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
            <span className="rounded-[3px] border border-border/50 bg-[hsl(var(--card))] px-1.5 py-px text-[10px] text-muted-foreground/50 uppercase tracking-widest font-mono">
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
          warningMessage={probeError}
          errorMessage={addError}
          duplicateActions={{
            active: isDuplicate,
            onKeepBoth: () => {
              setName((prev) => {
                const dot = prev.lastIndexOf(".");
                return dot !== -1 ? `${prev.slice(0, dot)} (2)${prev.slice(dot)}` : `${prev} (2)`;
              });
              setIsDuplicate(false);
              setAddError(null);
            },
            onOverwrite: () => {
              void handleRestart();
            },
          }}
          hideWarningWhenDuplicate
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
          onClick={handleAdd}
          disabled={adding || probing}
          style={{
            background: adding || probing
              ? "hsl(0,0%,20%)"
              : "linear-gradient(90deg, hsl(20,60%,46%) 0%, hsl(12,42%,31%) 55%, hsl(0,0%,20%) 100%)",
          }}
          className={cn(
            "flex h-[26px] items-center gap-1.5 rounded-[3px] px-3 text-[11.5px] font-semibold transition-all",
            "text-[hsl(24,10%,95%)] hover:brightness-110 active:brightness-95",
            "disabled:opacity-40 disabled:pointer-events-none",
          )}
        >
          {adding ? (
            <><Loader2 size={10} className="animate-spin" /> Adding…</>
          ) : (
            <><ArrowDownToLine size={10} /> Download</>
          )}
        </button>
      </div>
    </div>
  );
}
