import { useMemo, useRef, useState } from "react";
import {
  Activity,
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  FolderOpen,
  Info,
  Layers,
  Link,
  Pause,
  Play,
  RotateCcw,
  Trash2,
  X,
} from "lucide-react";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import {
  canPauseDownload,
  canRestartDownload,
  canResumeDownload,
  restartRequirementLabel,
  restartRequirementReason,
} from "@/lib/downloadActions";
import {
  activeConnectionCount,
  CATEGORY_ICONS,
  CATEGORY_ICON_COLORS,
  failureKindLabel,
  formatCooldownLabel,
  hostLockLabel,
  integrityBadgeLabel,
  integrityStateLabel,
  statusLabel,
  targetConnectionCount,
} from "@/lib/downloadPresentation";
import { checksumAlgorithmLabel } from "../lib/checksum";
import {
  formatBytes,
  formatBytesPerSecond,
  formatTimeRemaining,
} from "@/lib/format";
import { calculateDisplayProgress } from "@/lib/downloadProgress";
import { simplifyUserMessage } from "@/lib/userFacingMessages";
import type {
  Download as DownloadItem,
  DownloadIntegrity,
  DownloadLogEntry,
  DownloadLogLevel,
  DownloadSegment,
  DownloadStatus,
} from "@/types/download";

interface DownloadDetailsPanelProps {
  selectedDownloads: DownloadItem[];
  onOpenFolder: (id: string) => Promise<void> | void;
  onPause: (id: string) => Promise<void> | void;
  onResume: (id: string) => Promise<void> | void;
  onRestart: (id: string) => Promise<void> | void;
  onDelete: (id: string) => Promise<void> | void;
  onReorder: (id: string, direction: "up" | "down" | "top" | "bottom") => Promise<void> | void;
  canMoveUp: boolean;
  canMoveDown: boolean;
  onClearSelection: () => void;
}

const DETAIL_BYTE_FORMAT = {
  unknownLabel: "Unknown",
  integerAbove: 100,
} as const;
const DETAIL_SPEED_FORMAT = { idleLabel: "Idle", integerAbove: 100 } as const;
const DETAIL_TIME_FORMAT = { emptyLabel: "Unknown" } as const;

function logLevelLabel(level: DownloadLogLevel): string {
  switch (level) {
    case "info":
      return "Info";
    case "warn":
      return "Warn";
    case "error":
      return "Error";
  }
}

function formatLogTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function integrityBadgeTone(
  integrity: DownloadIntegrity,
): "neutral" | "good" | "warn" | "error" {
  if (!integrity.expected && integrity.actual) {
    return "neutral";
  }

  switch (integrity.state) {
    case "verified":
      return "good";
    case "verifying":
    case "pending":
      return "warn";
    case "mismatch":
      return "error";
    case "none":
      return "neutral";
  }
}

function integrityActualLabel(integrity: DownloadIntegrity): string {
  if (integrity.actual) {
    return integrity.actual;
  }

  switch (integrity.state) {
    case "verifying":
      return "Computing";
    case "pending":
      return "Pending";
    case "mismatch":
      return "Unavailable";
    case "verified":
      return "Unavailable";
    case "none":
      return "Not requested";
  }
}

function integrityStatusValue(integrity: DownloadIntegrity): string {
  if (!integrity.expected) {
    if (integrity.state === "verifying") {
      return "Computing";
    }
    if (integrity.actual) {
      return "Captured";
    }
    return "Automatic";
  }

  return integrityStateLabel(integrity.state);
}

function integrityAlgorithmValue(integrity: DownloadIntegrity): string {
  return checksumAlgorithmLabel(integrity.expected?.algorithm ?? "sha256");
}

function isSensitiveQueryKey(key: string): boolean {
  const normalized = key.toLowerCase();
  return (
    normalized.includes("token") ||
    normalized.includes("signature") ||
    normalized === "sig" ||
    normalized.includes("credential") ||
    normalized.includes("secret") ||
    normalized.includes("auth") ||
    normalized.includes("session") ||
    normalized.includes("expires") ||
    normalized.includes("key") ||
    normalized.startsWith("x-amz-") ||
    normalized.startsWith("x-goog-") ||
    normalized.startsWith("x-ms-")
  );
}

function redactUrlDisplay(value: string): string {
  try {
    const url = new URL(value);
    if (url.username) {
      url.username = "redacted";
    }
    if (url.password) {
      url.password = "redacted";
    }

    const queryKeys = Array.from(url.searchParams.keys());
    for (const key of queryKeys) {
      if (isSensitiveQueryKey(key)) {
        url.searchParams.set(key, "REDACTED");
      }
    }

    return url.toString();
  } catch {
    return value;
  }
}

function isUserFacingDiagnosticNote(message: string): boolean {
  const lower = message.toLowerCase();
  return !lower.includes("runtime worker orchestration enabled");
}

function StatusBadge({ status }: { status: DownloadStatus }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
        status === "downloading" &&
          "bg-[hsl(var(--status-downloading)/0.14)] text-[hsl(var(--status-downloading))]",
        status === "paused" &&
          "bg-[hsl(var(--status-paused)/0.14)] text-[hsl(var(--status-paused))]",
        status === "error" &&
          "bg-[hsl(var(--status-error)/0.14)] text-[hsl(var(--status-error))]",
        (status === "queued" || status === "stopped") &&
          "bg-white/6 text-muted-foreground/78",
        status === "finished" &&
          "bg-[hsl(var(--status-finished)/0.14)] text-[hsl(var(--status-finished))]",
      )}
    >
      {statusLabel(status)}
    </span>
  );
}

function CapabilityBadge({
  label,
  tone = "neutral",
}: {
  label: string;
  tone?: "neutral" | "good" | "warn" | "error";
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
        tone === "good" &&
          "bg-[hsl(var(--status-downloading)/0.12)] text-[hsl(var(--status-downloading))]",
        tone === "warn" &&
          "bg-[hsl(var(--status-paused)/0.14)] text-[hsl(var(--status-paused))]",
        tone === "error" &&
          "bg-[hsl(var(--status-error)/0.12)] text-[hsl(var(--status-error))]",
        tone === "neutral" && "bg-white/[0.065] text-foreground/62",
      )}
    >
      {label}
    </span>
  );
}

function ActionIconButton({
  icon: Icon,
  label,
  onClick,
  disabled = false,
  active = false,
  danger = false,
  variant = "blue",
}: {
  icon: React.ElementType;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
  danger?: boolean;
  variant?: "blue" | "green" | "amber";
}) {
  const activeClass =
    variant === "green"
      ? "bg-[hsl(var(--status-finished)/0.14)] text-[hsl(var(--status-finished))] hover:bg-[hsl(var(--status-finished)/0.22)]"
      : variant === "amber"
        ? "bg-[hsl(var(--status-paused)/0.14)] text-[hsl(var(--status-paused))] hover:bg-[hsl(var(--status-paused)/0.24)]"
        : "bg-[hsl(var(--status-downloading)/0.15)] text-[hsl(var(--status-downloading))] hover:bg-[hsl(var(--status-downloading)/0.25)]";
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          onClick={onClick}
          className={cn(
            "flex h-[26px] w-[26px] items-center justify-center rounded transition-colors",
            disabled
              ? "text-muted-foreground/20 pointer-events-none"
              : active
                ? activeClass
                : danger
                  ? "text-[hsl(var(--status-error)/0.65)] hover:text-[hsl(var(--status-error))] hover:bg-[hsl(var(--status-error)/0.12)]"
                  : "text-muted-foreground/50 hover:bg-accent hover:text-foreground",
          )}
        >
          <Icon size={14} strokeWidth={1.7} />
        </button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

function CloseButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex h-6 w-6 items-center justify-center rounded text-muted-foreground/50 transition-colors hover:bg-accent hover:text-foreground"
    >
      <X size={12} strokeWidth={2} />
    </button>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2 rounded-lg border border-border/60 bg-black/10 p-2.5">
      <div className="text-[9.5px] font-semibold uppercase tracking-[0.14em] text-muted-foreground/42">
        {title}
      </div>
      {children}
    </section>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border border-border/55 bg-black/8 px-2 py-1.5">
      <div className="text-[9.5px] uppercase tracking-[0.1em] text-muted-foreground/44">
        {label}
      </div>
      <div className="mt-0.5 break-all text-[11px] text-foreground/76">
        {value}
      </div>
    </div>
  );
}

function SegmentRow({
  segment,
  sample,
}: {
  segment: DownloadSegment;
  sample?: DownloadItem["runtimeCheckpoint"]["segmentSamples"][number];
}) {
  const total = Math.max(segment.end - segment.start + 1, 1);
  const pct = Math.min(100, (Math.max(0, segment.downloaded) / total) * 100);
  const isDone = segment.status === "finished";
  const isActive = segment.status === "downloading";
  const retryAttempts = sample?.retryAttempts ?? segment.retryAttempts ?? 0;
  const failureReason = sample?.terminalFailureReason ?? null;

  return (
    <div
      className="grid items-center gap-x-2.5 py-[4px]"
      style={{ gridTemplateColumns: "16px 1fr 14px" }}
    >
      <span className="text-right text-[9px] font-mono text-muted-foreground/25 tabular-nums select-none">
        {segment.id + 1}
      </span>

      <div className="relative h-[5px] overflow-hidden rounded-full bg-white/[0.06]">
        <div
          className={cn(
            "h-full rounded-full transition-[width] duration-300",
            isDone
              ? "bg-[hsl(var(--status-finished))]"
              : isActive
                ? "bg-[hsl(var(--status-downloading))]"
                : "bg-white/[0.12]",
          )}
          style={{ width: `${pct}%` }}
        />
        {isActive && (
          <div
            className="absolute right-0 top-0 h-full w-[14px] rounded-full"
            style={{
              background:
                "linear-gradient(90deg, transparent, hsl(var(--status-downloading)/0.5))",
            }}
          />
        )}
      </div>

      <span
        className={cn(
          "text-center text-[10px] font-bold leading-none",
          isDone
            ? "text-[hsl(var(--status-finished)/0.65)]"
            : isActive
              ? "text-[hsl(var(--status-downloading)/0.6)]"
              : failureReason
                ? "text-[hsl(var(--status-error)/0.5)]"
                : retryAttempts > 0
                  ? "text-[hsl(var(--status-paused)/0.55)]"
                  : "text-muted-foreground/16",
        )}
      >
        {isDone ? "done" : isActive ? "live" : failureReason ? "err" : retryAttempts > 0 ? "r" : "."}
      </span>
    </div>
  );
}

type BlockState = "complete" | "active" | "pending";

function computeBlockStates(
  size: number,
  segments: DownloadSegment[],
  totalBlocks: number,
): BlockState[] {
  const states: BlockState[] = new Array(totalBlocks).fill(
    "pending",
  ) as BlockState[];
  const bytesPerBlock = size / totalBlocks;

  for (const seg of segments) {
    const completedUpTo =
      seg.status === "finished" ? seg.end + 1 : seg.start + seg.downloaded;

    if (completedUpTo > seg.start) {
      const firstBlock = Math.floor(seg.start / bytesPerBlock);
      const lastBlock = Math.min(
        totalBlocks - 1,
        Math.floor((completedUpTo - 1) / bytesPerBlock),
      );
      for (let b = firstBlock; b <= lastBlock; b++) {
        states[b] = "complete";
      }
    }

    if (seg.status === "downloading") {
      const edgeBlock = Math.min(
        totalBlocks - 1,
        Math.floor(completedUpTo / bytesPerBlock),
      );
      if (states[edgeBlock] !== "complete") {
        states[edgeBlock] = "active";
      }
    }
  }

  return states;
}

function BlockProgressMap({ download }: { download: DownloadItem }) {
  const TOTAL_BLOCKS = 768;
  const { size, segments, status, downloaded } = download;
  const hasSegments = segments.length > 0 && size > 0;

  const blockStates = useMemo(
    () =>
      hasSegments ? computeBlockStates(size, segments, TOTAL_BLOCKS) : null,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [size, segments, hasSegments],
  );

  const progress = calculateDisplayProgress(downloaded, size, status);
  const finishedSegments = segments.filter(
    (s) => s.status === "finished",
  ).length;
  const activeConnections = activeConnectionCount(download);
  const targetConnections = targetConnectionCount(download);

  return (
    <div className="flex flex-col gap-2">
      {blockStates ? (
        <div
          className="w-full overflow-hidden rounded-sm"
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, 7px)",
            gap: "2px",
          }}
        >
          {blockStates.map((state, i) => (
            <div
              key={i}
              className={cn(
                "h-[7px] w-[7px] rounded-[2px]",
                state === "complete" && "bg-[hsl(var(--status-downloading))]",
                state === "active" && "bg-[hsl(var(--status-downloading)/0.4)]",
                state === "pending" && "bg-white/[0.06]",
              )}
            />
          ))}
        </div>
      ) : (
        <div className="h-[7px] overflow-hidden rounded-sm bg-white/[0.06]">
          <div
            className={cn(
              "h-full transition-[width] duration-300",
              status === "paused"
                ? "bg-[hsl(var(--status-paused))]"
                : status === "error"
                  ? "bg-[hsl(var(--status-error))]"
                  : status === "finished"
                    ? "bg-[hsl(var(--status-finished))]"
                    : "bg-[hsl(var(--status-downloading))]",
            )}
            style={{ width: `${progress}%` }}
          />
        </div>
      )}

      <div className="flex items-center gap-3 text-[10px] text-muted-foreground/40">
        {status === "finished" ? (
          <span className="text-[hsl(var(--status-finished)/0.8)] font-medium">Complete</span>
        ) : hasSegments ? (
          <>
            <span>
              <span className="tabular-nums text-foreground/55 font-medium">{finishedSegments}</span>
              <span className="ml-0.5 text-muted-foreground/32">/ {segments.length} parts</span>
            </span>
            <span className="h-2.5 w-px bg-border/30" />
            <span>
              <span className="tabular-nums text-foreground/55 font-medium">{activeConnections}</span>
              <span className="ml-0.5 text-muted-foreground/32">active</span>
              <span className="mx-1 text-muted-foreground/20">/</span>
              <span className="tabular-nums text-foreground/55 font-medium">{targetConnections}</span>
              <span className="ml-0.5 text-muted-foreground/32">target</span>
            </span>
            {download.writerBackpressure && (
              <>
                <span className="h-2.5 w-px bg-border/30" />
                <span className="text-[hsl(var(--status-paused)/0.75)] text-[9.5px]">Disk pressure</span>
              </>
            )}
          </>
        ) : (
          <span className="text-[9.5px]">
            {download.capabilities.rangeSupported ? "Range-resumable" : "Single connection"}
          </span>
        )}
      </div>
    </div>
  );
}

function SignalRow({
  icon: Icon,
  message,
  title,
  tone,
}: {
  icon: React.ElementType;
  message: string;
  title?: string;
  tone: "warn" | "error" | "note";
}) {
  return (
    <div
      title={title}
      className={cn(
        "flex items-start gap-2 rounded-lg border px-3 py-2 text-[11px]",
        tone === "warn" &&
          "border-[hsl(var(--status-paused)/0.22)] bg-[hsl(var(--status-paused)/0.08)] text-foreground/78",
        tone === "error" &&
          "border-[hsl(var(--status-error)/0.24)] bg-[hsl(var(--status-error)/0.08)] text-[hsl(var(--status-error))]",
        tone === "note" && "border-border/65 bg-black/10 text-foreground/74",
      )}
    >
      <Icon
        size={12}
        strokeWidth={1.9}
        className={cn(
          "mt-0.5 shrink-0",
          tone === "warn" && "text-[hsl(var(--status-paused))]",
          tone === "error" && "text-[hsl(var(--status-error))]",
          tone === "note" && "text-muted-foreground/56",
        )}
      />
      <span>{message}</span>
    </div>
  );
}

function EngineLogRow({ entry }: { entry: DownloadLogEntry }) {
  return (
    <div className="flex items-baseline gap-2 py-[3px] border-b border-border/20 last:border-0">
      <span
        className={cn(
          "shrink-0 text-[9px] font-bold uppercase tracking-wide w-[32px]",
          entry.level === "info" && "text-muted-foreground/40",
          entry.level === "warn" && "text-[hsl(var(--status-paused))]",
          entry.level === "error" && "text-[hsl(var(--status-error))]",
        )}
      >
        {logLevelLabel(entry.level)}
      </span>
      <span className="flex-1 text-[10.5px] text-foreground/72 leading-snug">
        {entry.message}
      </span>
      <span className="shrink-0 text-[9px] text-muted-foreground/30 tabular-nums">
        {formatLogTime(entry.timestamp)}
      </span>
    </div>
  );
}

function SelectionSummary({
  selectedDownloads,
  onClearSelection,
}: {
  selectedDownloads: DownloadItem[];
  onClearSelection: () => void;
}) {
  const errorCount = selectedDownloads.filter(
    (download) => download.status === "error",
  ).length;
  const restartCount = selectedDownloads.filter(
    (download) => download.diagnostics.restartRequired,
  ).length;

  return (
    <section className="shrink-0 border-t border-border/80 bg-[linear-gradient(180deg,hsl(var(--card)),hsl(var(--background)))] px-4 py-3 shadow-[0_-10px_30px_rgba(0,0,0,0.28)]">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground/44">
            Selection
          </div>
          <div className="mt-1 text-[13px] font-semibold text-foreground/86">
            {selectedDownloads.length} downloads selected
          </div>
          <div className="mt-1 flex flex-wrap gap-2 text-[10.5px] text-muted-foreground/58">
            <span>{errorCount} errors</span>
            <span>{restartCount} restart-only</span>
          </div>
        </div>
        <CloseButton onClick={onClearSelection} />
      </div>
    </section>
  );
}

type DetailTab = "general" | "segments" | "log";

function SingleSelection({
  download,
  onOpenFolder,
  onPause,
  onResume,
  onRestart,
  onDelete,
  onReorder,
  canMoveUp,
  canMoveDown,
  onClearSelection,
}: {
  download: DownloadItem;
  onOpenFolder: (id: string) => Promise<void> | void;
  onPause: (id: string) => Promise<void> | void;
  onResume: (id: string) => Promise<void> | void;
  onRestart: (id: string) => Promise<void> | void;
  onDelete: (id: string) => Promise<void> | void;
  onReorder: (id: string, direction: "up" | "down") => Promise<void> | void;
  canMoveUp: boolean;
  canMoveDown: boolean;
  onClearSelection: () => void;
}) {
  const [tab, setTab] = useState<DetailTab>("general");
  const [panelHeight, setPanelHeight] = useState(210);
  const isDragging = useRef(false);

  function startPanelResize(e: React.MouseEvent) {
    e.preventDefault();
    const startY = e.clientY;
    const startH = panelHeight;
    isDragging.current = true;

    function onMove(ev: MouseEvent) {
      const delta = startY - ev.clientY; // drag up = grow panel
      setPanelHeight(Math.max(168, Math.min(520, startH + delta)));
    }

    function onUp() {
      isDragging.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      // Snap to default if within range
      setPanelHeight((h) => (Math.abs(h - 210) <= 22 ? 210 : h));
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    }

    document.body.style.cursor = "ns-resize";
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }

  const segmentSamplesById = useMemo(
    () =>
      new Map(
        download.runtimeCheckpoint.segmentSamples.map((sample) => [
          sample.segmentId,
          sample,
        ]),
      ),
    [download.runtimeCheckpoint.segmentSamples],
  );
  const progress = calculateDisplayProgress(
    download.downloaded,
    download.size,
    download.status,
  );
  const failureLabel = failureKindLabel(download.diagnostics.failureKind);
  const sourceUrl =
    download.finalUrl !== download.url ? download.finalUrl : download.url;
  const displaySourceUrl = redactUrlDisplay(sourceUrl);
  const integrityBadge = integrityBadgeLabel(download.integrity);
  const CategoryIcon = CATEGORY_ICONS[download.category];
  const categoryIconColor = CATEGORY_ICON_COLORS[download.category];
  const recentLogEntries = download.engineLog.slice(-20).reverse();
  const restartLabel = restartRequirementLabel(download);
  const restartReason = restartRequirementReason(download);
  const hostBadges = (() => {
    const rows: Array<{
      label: string;
      tone: "neutral" | "good" | "warn" | "error";
    }> = [];
    if (restartLabel) {
      rows.push({ label: restartLabel, tone: "warn" });
    }
    if (download.compatibility.directUrlRecovered) {
      rows.push({ label: "Wrapper recovered", tone: "good" });
    } else if (download.compatibility.browserInterstitialOnly) {
      rows.push({ label: "Browser interstitial", tone: "warn" });
    }
    if (download.compatibility.requestReferer) {
      rows.push({ label: "Wrapper referer", tone: "neutral" });
    }
    if (
      download.hostDiagnostics.hardNoRange ||
      !download.capabilities.rangeSupported
    ) {
      rows.push({ label: "No-range host", tone: "warn" });
    }
    const cooldown = formatCooldownLabel(
      download.hostDiagnostics.cooldownUntil ?? download.hostCooldownUntil,
    );
    if (cooldown) {
      rows.push({ label: cooldown, tone: "warn" });
    }
    if (download.hostDiagnostics.concurrencyLocked) {
      rows.push({
        label: hostLockLabel(download.hostDiagnostics.lockReason),
        tone: "warn",
      });
    }
    const protocol =
      download.hostDiagnostics.negotiatedProtocol ?? download.hostProtocol;
    if (protocol) {
      rows.push({ label: protocol.toUpperCase(), tone: "neutral" });
    }
    if (download.hostDiagnostics.reuseConnections !== null) {
      rows.push({
        label: download.hostDiagnostics.reuseConnections
          ? "Keep-alive reuse"
          : "Fresh sockets",
        tone: download.hostDiagnostics.reuseConnections ? "good" : "neutral",
      });
    }
    if (download.hostMaxConnections !== null) {
      rows.push({
        label: `Cap ${download.hostMaxConnections}`,
        tone: "neutral",
      });
    }
    return rows.slice(0, 6);
  })();
  const hostFields = (() => {
    const rows: Array<{ label: string; value: string }> = [
      { label: "Host", value: download.host },
    ];
    if (download.hostAverageTtfbMs !== null) {
      rows.push({
        label: "Avg TTFB",
        value: `${download.hostAverageTtfbMs} ms`,
      });
    }
    if (download.hostAverageThroughputBytesPerSecond !== null) {
      rows.push({
        label: "Host Avg",
        value: formatBytesPerSecond(
          download.hostAverageThroughputBytesPerSecond,
          DETAIL_SPEED_FORMAT,
        ),
      });
    }
    if (download.hostDiagnostics.lockReason) {
      rows.push({
        label: "Planner",
        value: hostLockLabel(download.hostDiagnostics.lockReason),
      });
    }
    if (download.compatibility.directUrlRecovered) {
      rows.push({ label: "Access", value: "Recovered from wrapper page" });
    } else if (download.compatibility.browserInterstitialOnly) {
      rows.push({ label: "Access", value: "Browser interstitial only" });
    }
    if (download.compatibility.requestReferer) {
      rows.push({
        label: "Referer",
        value: redactUrlDisplay(download.compatibility.requestReferer),
      });
    }
    if (
      download.compatibility.requestMethod !== "get" ||
      download.compatibility.requestFormFields.length > 0
    ) {
      const fieldCount = download.compatibility.requestFormFields.length;
      rows.push({
        label: "Request",
        value:
          fieldCount > 0
            ? `${download.compatibility.requestMethod.toUpperCase()} + ${fieldCount} form field${fieldCount === 1 ? "" : "s"}`
            : download.compatibility.requestMethod.toUpperCase(),
      });
    }
    return rows.slice(0, 6);
  })();
  const signalRows = (() => {
    const seen = new Set<string>();
    const rows: Array<{
      icon: React.ElementType;
      message: string;
      title?: string;
      tone: "warn" | "error" | "note";
    }> = [];

    const push = (
      icon: React.ElementType,
      message: string | null | undefined,
      tone: "warn" | "error" | "note",
    ) => {
      if (!message) {
        return;
      }
      const normalized = simplifyUserMessage(message);
      if (seen.has(normalized)) {
        return;
      }
      seen.add(normalized);
      rows.push({
        icon,
        message: normalized,
        title: normalized === message ? undefined : message,
        tone,
      });
    };

    if (restartReason) {
      push(RotateCcw, restartReason, "warn");
    }

    if (download.errorMessage) {
      push(AlertTriangle, download.errorMessage, "error");
    } else if (failureLabel) {
      push(Info, failureLabel, "note");
    }

    if (
      download.diagnostics.terminalReason &&
      download.diagnostics.terminalReason !== download.errorMessage
    ) {
      push(
        download.diagnostics.failureKind ? AlertTriangle : Info,
        download.diagnostics.terminalReason,
        download.diagnostics.failureKind ? "warn" : "note",
      );
    }

    if (download.writerBackpressure) {
      push(
        AlertTriangle,
        "Disk backpressure is active, so VDM is holding off on extra ramp-up and work-steal pressure.",
        "warn",
      );
    }
    if (!download.writerBackpressure && download.diagnostics.checkpointDiskPressureEvents > 0) {
      push(
        AlertTriangle,
        `Disk pressure was detected ${download.diagnostics.checkpointDiskPressureEvents} time${download.diagnostics.checkpointDiskPressureEvents === 1 ? "" : "s"} during this transfer.`,
        "warn",
      );
    }
    const hostCooldown = formatCooldownLabel(
      download.hostDiagnostics.cooldownUntil ?? download.hostCooldownUntil,
    );
    if (hostCooldown) {
      push(
        AlertTriangle,
        `${hostCooldown} is active because the host is currently throttling or unstable.`,
        "warn",
      );
    }
    if (download.hostDiagnostics.concurrencyLocked) {
      push(
        Layers,
        `${hostLockLabel(download.hostDiagnostics.lockReason)} is limiting connection ramp-up for stability.`,
        "note",
      );
    }

    if (
      !download.capabilities.rangeSupported &&
      download.status !== "finished"
    ) {
      push(
        Layers,
        "Host is pinned to single-connection mode because byte-range support is unavailable or untrusted.",
        "note",
      );
    }

    for (const warning of download.diagnostics.warnings.slice(0, 2)) {
      push(AlertTriangle, warning, "warn");
    }

    for (const note of download.diagnostics.notes
      .filter(isUserFacingDiagnosticNote)
      .slice(0, 1)) {
      push(Info, note, "note");
    }

    if (rows.length <= 3) {
      return rows;
    }

    const overflowCount = rows.length - 2;
    return [
      ...rows.slice(0, 2),
      {
        icon: Info,
        message: `${overflowCount} more diagnostic ${overflowCount === 1 ? "message is" : "messages are"} available in Engine Log.`,
        title: undefined,
        tone: "note" as const,
      },
    ];
  })();

  const TAB_META: { id: DetailTab; label: string; icon: React.ElementType }[] =
    [
      { id: "general", label: "General", icon: Info },
      { id: "segments", label: "Segments", icon: Layers },
      { id: "log", label: "Log", icon: Activity },
    ];

  return (
    <section
      className="relative flex flex-col overflow-hidden border-t border-border/80"
      style={{ height: panelHeight }}
    >
      {/* Drag handle – sits at top, cursor:ns-resize, snaps to default height */}
      <div
        role="separator"
        aria-label="Drag to resize panel"
        onMouseDown={startPanelResize}
        className="absolute top-0 left-0 right-0 h-[5px] cursor-ns-resize z-10 hover:bg-primary/14 active:bg-primary/24 transition-colors"
        title="Drag to resize"
      />
      <div
        className="flex h-[32px] items-stretch shrink-0 border-b border-border/60"
        style={{ background: "hsl(0,0%,7.5%)" }}
      >
        <div className="flex flex-1 items-center">
          {TAB_META.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              onClick={() => setTab(id)}
              className={cn(
                "relative flex h-full items-center gap-1.5 px-4 text-[11px] tracking-wide transition-colors",
                tab === id
                  ? "font-semibold text-foreground/95 after:absolute after:bottom-0 after:left-0 after:right-0 after:h-[2px] after:rounded-t-sm after:bg-primary"
                  : "text-foreground/42 hover:text-foreground/72",
              )}
            >
              <Icon
                size={11}
                strokeWidth={tab === id ? 2.2 : 1.8}
                className="shrink-0"
              />
              {label}
            </button>
          ))}
        </div>
        <button
          type="button"
          onClick={onClearSelection}
          className="mr-2 my-auto flex h-6 w-6 items-center justify-center rounded text-muted-foreground/45 hover:bg-accent hover:text-foreground transition-colors"
        >
          <X size={11} strokeWidth={2} />
        </button>
      </div>

      <div
        className="flex-1 overflow-y-auto min-h-0 bg-[hsl(var(--background))]"
      >
        {tab === "general" && (
          <div className="flex items-start gap-3.5 px-4 py-3">
            <div
              className="flex h-[52px] w-[52px] shrink-0 items-center justify-center rounded-lg"
              style={{
                background:
                  "linear-gradient(145deg, hsl(var(--card)), hsl(var(--muted)))",
                border: "1px solid hsl(var(--border))",
              }}
            >
              <CategoryIcon
                size={22}
                strokeWidth={1.3}
                className={categoryIconColor}
              />
            </div>

            <div className="flex min-w-0 flex-1 flex-col gap-1.5">
              <div className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
                <h2 className="min-w-0 truncate text-[12.5px] font-semibold text-foreground/88 leading-tight">
                  {download.name}
                </h2>
                <StatusBadge status={download.status} />
                <div className="ml-1 flex items-center gap-0.5">
                  <ActionIconButton
                    icon={Play}
                    label={
                      download.diagnostics.restartRequired
                        ? "Resume unavailable"
                        : "Resume"
                    }
                    onClick={() => void onResume(download.id)}
                    disabled={!canResumeDownload(download)}
                    active={canResumeDownload(download)}
                    variant="green"
                  />
                  <ActionIconButton
                    icon={Pause}
                    label="Pause"
                    onClick={() => void onPause(download.id)}
                    disabled={!canPauseDownload(download)}
                    active={canPauseDownload(download)}
                    variant="amber"
                  />
                  <ActionIconButton
                    icon={RotateCcw}
                    label={
                      download.diagnostics.restartRequired
                        ? "Restart from zero"
                        : "Restart"
                    }
                    onClick={() => void onRestart(download.id)}
                    disabled={!canRestartDownload(download)}
                  />
                  <div className="mx-0.5 h-3.5 w-px bg-border/55" />
                  <ActionIconButton
                    icon={ArrowUp}
                    label="Move up in queue"
                    onClick={() => void onReorder(download.id, "up")}
                    disabled={!canMoveUp}
                  />
                  <ActionIconButton
                    icon={ArrowDown}
                    label="Move down in queue"
                    onClick={() => void onReorder(download.id, "down")}
                    disabled={!canMoveDown}
                  />
                  <ActionIconButton
                    icon={FolderOpen}
                    label="Open folder"
                    onClick={() => void onOpenFolder(download.id)}
                  />
                  <div className="mx-0.5 h-3.5 w-px bg-border/55" />
                  <ActionIconButton
                    icon={Trash2}
                    label="Delete download"
                    onClick={() => void onDelete(download.id)}
                    danger
                  />
                </div>
              </div>

              <div className="flex flex-col gap-1">
                <div className="flex items-center justify-between text-[10.5px]">
                  <span className="tabular-nums text-muted-foreground/58">
                    {download.size > 0
                      ? `${formatBytes(download.downloaded, DETAIL_BYTE_FORMAT)} of ${formatBytes(download.size, DETAIL_BYTE_FORMAT)}`
                      : formatBytes(download.downloaded, DETAIL_BYTE_FORMAT)}
                  </span>
                  <div className="flex items-center gap-2.5 tabular-nums text-muted-foreground/52">
                    {download.speed > 0 && (
                      <span>
                        ▼{" "}
                        {formatBytesPerSecond(
                          download.speed,
                          DETAIL_SPEED_FORMAT,
                        )}
                      </span>
                    )}
                    {progress > 0 && (
                      <span className="font-medium text-foreground/62">
                        {Math.round(progress)}%
                      </span>
                    )}
                    {(download.timeLeft ?? 0) > 0 && (
                      <span>
                        (
                        {formatTimeRemaining(
                          download.timeLeft,
                          DETAIL_TIME_FORMAT,
                        )}
                        )
                      </span>
                    )}
                  </div>
                </div>
                <div className="h-[6px] overflow-hidden rounded-sm bg-white/[0.06] relative">
                  <div
                    className={cn(
                      "h-full rounded-sm transition-[width] duration-300",
                      download.status === "error"
                        ? "bg-[hsl(var(--status-error))]"
                        : download.status === "paused"
                          ? "bg-[hsl(var(--status-paused))]"
                          : download.status === "finished"
                            ? "bg-[hsl(var(--status-finished))]"
                            : "bg-[hsl(var(--status-downloading))]",
                    )}
                    style={{ width: `${progress}%` }}
                  />
                </div>
              </div>

              <div className="flex flex-col gap-0.5">
                <div className="flex items-center gap-1.5 text-[10.5px]">
                  <FolderOpen
                    size={10}
                    className="shrink-0 text-muted-foreground/36"
                  />
                  <button
                    type="button"
                    onClick={() => void onOpenFolder(download.id)}
                    className="truncate text-left text-[hsl(205,72%,58%)] hover:underline"
                  >
                    {download.targetPath}
                  </button>
                </div>
                <div className="flex items-center gap-1.5 text-[10.5px]">
                  <Link
                    size={10}
                    className="shrink-0 text-muted-foreground/36"
                  />
                  <span
                    className="truncate text-muted-foreground/50"
                    title={displaySourceUrl}
                  >
                    {displaySourceUrl}
                  </span>
                </div>
              </div>

              {signalRows.length > 0 ? (
                <div className="flex flex-col gap-1.5">
                  {signalRows.map((signal, index) => (
                    <SignalRow
                      key={`${signal.message}-${index}`}
                      icon={signal.icon}
                      message={signal.message}
                      title={signal.title}
                      tone={signal.tone}
                    />
                  ))}
                </div>
              ) : null}

              {hostBadges.length > 0 || hostFields.length > 0 ? (
                <div className="flex flex-col gap-1.5">
                  <div className="flex items-center gap-2">
                    <span className="text-[9px] font-semibold uppercase tracking-[0.15em] text-muted-foreground/38">Host</span>
                    <div className="flex-1 h-px bg-border/25" />
                  </div>
                  {hostBadges.length > 0 ? (
                    <div className="flex flex-wrap gap-1">
                      {hostBadges.map((badge) => (
                        <CapabilityBadge
                          key={badge.label}
                          label={badge.label}
                          tone={badge.tone}
                        />
                      ))}
                    </div>
                  ) : null}
                  {hostFields.length > 0 ? (
                    <div className="grid grid-cols-2 gap-1">
                      {hostFields.map((field) => (
                        <Field
                          key={field.label}
                          label={field.label}
                          value={field.value}
                        />
                      ))}
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>
          </div>
        )}

        {tab === "segments" && (
          <div className="flex flex-col gap-3 px-4 py-3">
            <BlockProgressMap download={download} />
            {download.segments.length > 0 && (
              <>
                <div className="h-px bg-border/25" />
                <div className="flex flex-col">
                  <div className="mb-1.5 text-[9px] font-semibold uppercase tracking-[0.14em] text-muted-foreground/30">
                    {download.segments.length} Part
                    {download.segments.length !== 1 ? "s" : ""}
                  </div>
                  <div className="flex flex-col divide-y divide-border/15">
                    {download.segments.map((segment) => (
                      <SegmentRow
                        key={segment.id}
                        segment={segment}
                        sample={segmentSamplesById.get(segment.id)}
                      />
                    ))}
                  </div>
                </div>
              </>
            )}
          </div>
        )}

        {tab === "log" && (
          <div className="flex flex-col gap-2.5 px-4 py-3">
            {download.integrity.expected || download.integrity.actual || download.integrity.state === "verifying" ? (
              <Section title="Integrity">
                <div className="grid grid-cols-3 gap-1.5">
                  <Field
                    label="Status"
                    value={integrityStatusValue(download.integrity)}
                  />
                  <Field
                    label="Algorithm"
                    value={integrityAlgorithmValue(download.integrity)}
                  />
                  <Field
                    label="Hash"
                    value={integrityActualLabel(download.integrity)}
                  />
                </div>
                {integrityBadge ? (
                  <div className="mt-1">
                    <CapabilityBadge
                      label={integrityBadge}
                      tone={integrityBadgeTone(download.integrity)}
                    />
                  </div>
                ) : null}
              </Section>
            ) : null}

            <Section title="Engine Log">
              <div className="flex flex-col">
                {recentLogEntries.length > 0 ? (
                  recentLogEntries.map((entry, index) => (
                    <EngineLogRow
                      key={`${entry.timestamp}-${index}`}
                      entry={entry}
                    />
                  ))
                ) : (
                  <div className="py-1 text-[11px] text-muted-foreground/40">
                    No events yet.
                  </div>
                )}
                {download.engineLog.length > recentLogEntries.length ? (
                  <div className="pt-1 text-[9.5px] text-muted-foreground/30">
                    +{download.engineLog.length - recentLogEntries.length}{" "}
                    earlier
                  </div>
                ) : null}
              </div>
            </Section>
          </div>
        )}
      </div>
    </section>
  );
}

export function DownloadDetailsPanel({
  selectedDownloads,
  onOpenFolder,
  onPause,
  onResume,
  onRestart,
  onDelete,
  onReorder,
  canMoveUp,
  canMoveDown,
  onClearSelection,
}: DownloadDetailsPanelProps) {
  if (selectedDownloads.length === 0) {
    return null;
  }

  if (selectedDownloads.length > 1) {
    return (
      <SelectionSummary
        selectedDownloads={selectedDownloads}
        onClearSelection={onClearSelection}
      />
    );
  }

  return (
    <SingleSelection
      download={selectedDownloads[0]}
      onOpenFolder={onOpenFolder}
      onPause={onPause}
      onResume={onResume}
      onRestart={onRestart}
      onDelete={onDelete}
      onReorder={onReorder}
      canMoveUp={canMoveUp}
      canMoveDown={canMoveDown}
      onClearSelection={onClearSelection}
    />
  );
}
