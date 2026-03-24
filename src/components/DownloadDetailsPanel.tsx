import { useRef, useState } from "react";
import {
  Activity,
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  Copy,
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
} from "@/lib/downloadActions";
import { writeClipboardText } from "@/lib/clipboard";
import {
  buildDownloadDiagnosticsSummary,
  buildSelectionDiagnosticsSummary,
} from "@/lib/downloadDiagnostics";
import {
  CATEGORY_ICONS,
  CATEGORY_ICON_COLORS,
  failureKindLabel,
  hostBadgeItems,
  hostLockLabel,
  primaryIssueSummary,
  semanticBadgeToneClassName,
  statusBadgeClassName,
  statusLabel,
  transferConstraintNotice,
} from "@/lib/downloadPresentation";
import {
  formatBytes,
  formatBytesPerSecond,
  formatTimeRemaining,
} from "@/lib/format";
import { calculateDisplayProgress } from "@/lib/downloadProgress";
import {
  getVisibleDiagnosticNotes,
  getVisibleDownloadWarnings,
  sameVisibleMessage,
  simplifyUserMessage,
} from "@/lib/userFacingMessages";
import type {
  Download as DownloadItem,
  DownloadLogEntry,
  DownloadLogLevel,
  DownloadStatus,
} from "@/types/download";
import { SegmentRuntimePanel } from "@/components/download-details/SegmentRuntimePanel";

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

function StatusBadge({ status }: { status: DownloadStatus }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide",
        statusBadgeClassName(status),
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
        semanticBadgeToneClassName(tone),
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

function QuickActionButton({
  icon: Icon,
  label,
  onClick,
  active = false,
  disabled = false,
}: {
  icon: React.ElementType;
  label: string;
  onClick: () => void;
  active?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-[10px] font-medium transition-colors",
        disabled && "pointer-events-none opacity-45",
        active
          ? "border-[hsl(var(--status-finished)/0.28)] bg-[hsl(var(--status-finished)/0.12)] text-[hsl(var(--status-finished))]"
          : "border-border/55 bg-black/10 text-foreground/72 hover:bg-accent hover:text-foreground",
      )}
    >
      <Icon size={11} strokeWidth={1.9} className="shrink-0" />
      {label}
    </button>
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
  const [summaryCopied, setSummaryCopied] = useState(false);
  const errorCount = selectedDownloads.filter(
    (download) => download.status === "error",
  ).length;
  const restartCount = selectedDownloads.filter(
    (download) => download.diagnostics.restartRequired,
  ).length;

  async function handleCopySummary() {
    await writeClipboardText(buildSelectionDiagnosticsSummary(selectedDownloads));
    setSummaryCopied(true);
    window.setTimeout(() => setSummaryCopied(false), 1800);
  }

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
        <div className="flex items-center gap-2">
          <QuickActionButton
            icon={Copy}
            label={summaryCopied ? "Summary copied" : "Copy summary"}
            onClick={() => {
              void handleCopySummary();
            }}
            active={summaryCopied}
          />
          <CloseButton onClick={onClearSelection} />
        </div>
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
  const [copiedAction, setCopiedAction] = useState<null | "url" | "path" | "diagnostics">(null);
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

  const progress = calculateDisplayProgress(
    download.downloaded,
    download.size,
    download.status,
  );
  const failureLabel = failureKindLabel(download.diagnostics.failureKind);
  const sourceUrl =
    download.finalUrl !== download.url ? download.finalUrl : download.url;
  const displaySourceUrl = redactUrlDisplay(sourceUrl);
  const CategoryIcon = CATEGORY_ICONS[download.category];
  const categoryIconColor = CATEGORY_ICON_COLORS[download.category];
  const recentLogEntries = download.engineLog.slice(-20).reverse();
  const restartLabel = restartRequirementLabel(download);
  const primaryIssue = primaryIssueSummary(download);
  const visibleWarnings = getVisibleDownloadWarnings(
    download.diagnostics.warnings,
    2,
  );
  const visibleNotes = getVisibleDiagnosticNotes(download.diagnostics.notes, 1);

  async function handleCopyAction(
    action: "url" | "path" | "diagnostics",
    value: string,
  ) {
    await writeClipboardText(value);
    setCopiedAction(action);
    window.setTimeout(() => setCopiedAction(null), 1800);
  }

  const hostBadges = hostBadgeItems({
    compatibility: download.compatibility,
    hostDiagnostics: download.hostDiagnostics,
    rangeSupported: download.capabilities.rangeSupported,
    hostCooldownUntil: download.hostCooldownUntil,
    hostProtocol: download.hostProtocol,
    hostMaxConnections: download.hostMaxConnections,
    restartLabel,
  });
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

    if (primaryIssue) {
      push(
        download.status === "error" ? AlertTriangle : RotateCcw,
        primaryIssue,
        download.status === "error" ? "error" : "warn",
      );
    }

    if (download.errorMessage && !sameVisibleMessage(download.errorMessage, primaryIssue)) {
      push(AlertTriangle, download.errorMessage, "error");
    } else if (failureLabel) {
      push(Info, failureLabel, "note");
    }

    if (
      download.diagnostics.terminalReason &&
      !sameVisibleMessage(download.diagnostics.terminalReason, primaryIssue) &&
      !sameVisibleMessage(download.diagnostics.terminalReason, download.errorMessage)
    ) {
      push(
        download.diagnostics.failureKind ? AlertTriangle : Info,
        download.diagnostics.terminalReason,
        download.diagnostics.failureKind ? "warn" : "note",
      );
    }

    const constraintNotice = transferConstraintNotice(download);
    if (constraintNotice) {
      push(
        constraintNotice.tone === "warn" ? AlertTriangle : Layers,
        constraintNotice.message,
        constraintNotice.tone,
      );
    }

    if (!download.writerBackpressure && download.diagnostics.checkpointDiskPressureEvents > 0) {
      push(
        AlertTriangle,
        `Disk pressure was detected ${download.diagnostics.checkpointDiskPressureEvents} time${download.diagnostics.checkpointDiskPressureEvents === 1 ? "" : "s"} during this transfer.`,
        "warn",
      );
    }

    for (const warning of visibleWarnings) {
      push(AlertTriangle, warning, "warn");
    }

    for (const note of visibleNotes) {
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
                  <button
                    type="button"
                    onClick={() => {
                      void handleCopyAction("url", sourceUrl);
                    }}
                    className="min-w-0 truncate text-left text-foreground/60 hover:text-foreground/85 transition-colors"
                    title={copiedAction === "url" ? "Copied!" : `${displaySourceUrl}\nClick to copy`}
                  >
                    {copiedAction === "url" ? "Copied!" : displaySourceUrl}
                  </button>
                </div>
              </div>

              <div className="flex flex-wrap gap-1.5">
                <QuickActionButton
                  icon={Copy}
                  label={copiedAction === "url" ? "Final URL copied" : "Copy final URL"}
                  onClick={() => {
                    void handleCopyAction("url", sourceUrl);
                  }}
                  active={copiedAction === "url"}
                />
                <QuickActionButton
                  icon={FolderOpen}
                  label={copiedAction === "path" ? "Target path copied" : "Copy target path"}
                  onClick={() => {
                    void handleCopyAction("path", download.targetPath);
                  }}
                  active={copiedAction === "path"}
                />
                <QuickActionButton
                  icon={Info}
                  label={copiedAction === "diagnostics" ? "Diagnostics copied" : "Copy diagnostics"}
                  onClick={() => {
                    void handleCopyAction(
                      "diagnostics",
                      buildDownloadDiagnosticsSummary(download, displaySourceUrl),
                    );
                  }}
                  active={copiedAction === "diagnostics"}
                />
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
          <SegmentRuntimePanel download={download} />
        )}

        {tab === "log" && (
          <div className="flex flex-col gap-2.5 px-4 py-3">
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
