import {
  AlertCircle,
  Archive,
  CheckCircle2,
  Clock,
  FileText,
  Film,
  Image,
  LayoutGrid,
  Loader2,
  Music,
  Package,
  PauseCircle,
  StopCircle,
  type LucideIcon,
} from "lucide-react";
import {
  restartRequirementLabel,
  restartRequirementReason,
} from "@/lib/downloadActions";
import { formatDurationShort } from "@/lib/format";
import { sameVisibleMessage, simplifyUserMessage } from "@/lib/userFacingMessages";
import type {
  Download,
  DownloadCategory,
  DownloadFailureKind,
  DownloadIntegrityStatus,
  DownloadProbe,
  DownloadStatus,
} from "@/types/download";

export type StatusMeta = {
  label: string;
  color: string;
  Icon: LucideIcon;
};

export type IntegrityMeta = {
  label: string;
  color: string;
};

export type TransferConstraintMeta = {
  label: string;
  summary: string;
  detail: string;
  tone: "neutral" | "warn";
};

export type TransferConstraintNotice = {
  message: string;
  tone: "note" | "warn";
};

export type HostBadgeMeta = {
  label: string;
  tone: SemanticBadgeTone;
};

export type TransferConstraintSource = Pick<
  Download,
  | "status"
  | "scheduledFor"
  | "writerBackpressure"
  | "hostCooldownUntil"
  | "hostDiagnostics"
  | "diagnostics"
  | "capabilities"
  | "targetConnections"
  | "segments"
>;

export type SemanticBadgeTone = "neutral" | "good" | "warn" | "error";

type HostBadgeSource = {
  compatibility: Pick<
    Download["compatibility"],
    "directUrlRecovered" | "browserInterstitialOnly" | "requestReferer"
  >;
  hostDiagnostics: Pick<
    Download["hostDiagnostics"],
    | "hardNoRange"
    | "cooldownUntil"
    | "concurrencyLocked"
    | "lockReason"
    | "negotiatedProtocol"
    | "reuseConnections"
  >;
  rangeSupported: boolean;
  hostCooldownUntil?: number | null;
  hostProtocol?: string | null;
  hostMaxConnections?: number | null;
  restartLabel?: string | null;
};

export const CATEGORY_ICONS: Record<DownloadCategory, LucideIcon> = {
  all: LayoutGrid,
  compressed: Archive,
  programs: Package,
  videos: Film,
  music: Music,
  pictures: Image,
  documents: FileText,
};

export const CATEGORY_ICON_COLORS: Record<DownloadCategory, string> = {
  all: "text-muted-foreground/50",
  compressed: "text-[hsl(38,68%,52%)]",
  programs: "text-[hsl(205,62%,52%)]",
  videos: "text-[hsl(270,52%,62%)]",
  music: "text-[hsl(152,46%,44%)]",
  pictures: "text-[hsl(188,58%,48%)]",
  documents: "text-[hsl(220,48%,58%)]",
};

/** Subtle gradient backgrounds for the download row category icon badge */
export const CATEGORY_ICON_BG: Record<DownloadCategory, string> = {
  all:        "linear-gradient(135deg, hsl(0 0% 20% / 0.6), hsl(0 0% 14% / 0.4))",
  compressed: "linear-gradient(135deg, hsl(38 60% 24% / 0.55), hsl(38 48% 14% / 0.35))",
  programs:   "linear-gradient(135deg, hsl(205 55% 22% / 0.55), hsl(205 45% 13% / 0.35))",
  videos:     "linear-gradient(135deg, hsl(270 45% 22% / 0.55), hsl(270 38% 13% / 0.35))",
  music:      "linear-gradient(135deg, hsl(152 40% 18% / 0.55), hsl(152 34% 11% / 0.35))",
  pictures:   "linear-gradient(135deg, hsl(188 50% 20% / 0.55), hsl(188 42% 12% / 0.35))",
  documents:  "linear-gradient(135deg, hsl(220 42% 22% / 0.55), hsl(220 36% 13% / 0.35))",
};

export const CATEGORY_LABELS: Record<DownloadCategory, string> = {
  all: "General",
  compressed: "Compressed",
  programs: "Programs",
  videos: "Videos",
  music: "Music",
  pictures: "Pictures",
  documents: "Documents",
};

export const STATUS_META: Record<DownloadStatus, StatusMeta> = {
  finished: {
    label: "Finished",
    color: "text-[hsl(var(--status-finished))]",
    Icon: CheckCircle2,
  },
  downloading: {
    label: "Downloading",
    color: "text-[hsl(var(--status-downloading))]",
    Icon: Loader2,
  },
  paused: {
    label: "Paused",
    color: "text-[hsl(var(--status-paused))]",
    Icon: PauseCircle,
  },
  queued: {
    label: "Queued",
    color: "text-[hsl(var(--status-queued))]",
    Icon: Clock,
  },
  error: {
    label: "Error",
    color: "text-[hsl(var(--status-error))]",
    Icon: AlertCircle,
  },
  stopped: {
    label: "Stopped",
    color: "text-[hsl(var(--status-queued))]",
    Icon: StopCircle,
  },
};

export const INTEGRITY_STATUS_META: Record<DownloadIntegrityStatus, IntegrityMeta> = {
  unavailable: {
    label: "Unavailable",
    color: "text-muted-foreground/50",
  },
  pending: {
    label: "Pending",
    color: "text-muted-foreground/66",
  },
  computed: {
    label: "Ready",
    color: "text-[hsl(var(--status-finished))]",
  },
  verified: {
    label: "Verified",
    color: "text-[hsl(var(--status-finished))]",
  },
  mismatch: {
    label: "Mismatch",
    color: "text-[hsl(var(--status-error))]",
  },
  failed: {
    label: "Failed",
    color: "text-[hsl(var(--status-error))]",
  },
};

export function statusLabel(status: DownloadStatus): string {
  return STATUS_META[status].label;
}

export function statusBadgeClassName(status: DownloadStatus): string {
  switch (status) {
    case "downloading":
      return "bg-[hsl(var(--status-downloading)/0.14)] text-[hsl(var(--status-downloading))]";
    case "paused":
      return "bg-[hsl(var(--status-paused)/0.14)] text-[hsl(var(--status-paused))]";
    case "error":
      return "bg-[hsl(var(--status-error)/0.14)] text-[hsl(var(--status-error))]";
    case "finished":
      return "bg-[hsl(var(--status-finished)/0.14)] text-[hsl(var(--status-finished))]";
    case "queued":
    case "stopped":
      return "bg-white/6 text-muted-foreground/78";
    default:
      return "bg-white/6 text-muted-foreground/78";
  }
}

export function semanticBadgeToneClassName(tone: SemanticBadgeTone): string {
  switch (tone) {
    case "good":
      return "bg-[hsl(var(--status-downloading)/0.12)] text-[hsl(var(--status-downloading))]";
    case "warn":
      return "bg-[hsl(var(--status-paused)/0.14)] text-[hsl(var(--status-paused))]";
    case "error":
      return "bg-[hsl(var(--status-error)/0.12)] text-[hsl(var(--status-error))]";
    case "neutral":
    default:
      return "bg-white/[0.065] text-foreground/62";
  }
}

export function integrityStatusLabel(status: DownloadIntegrityStatus): string {
  return INTEGRITY_STATUS_META[status].label;
}

export function integritySummaryLabel(status: DownloadIntegrityStatus): string | null {
  switch (status) {
    case "pending":
      return "SHA-256 pending";
    case "computed":
      return "SHA-256 ready";
    case "verified":
      return "SHA-256 verified";
    case "mismatch":
      return "SHA-256 mismatch";
    case "failed":
      return "SHA-256 failed";
    default:
      return null;
  }
}

export function transferConstraintSummary(
  constraint: TransferConstraintMeta | null,
): string | null {
  if (!constraint) {
    return null;
  }

  return constraint.summary === constraint.label
    ? constraint.label
    : `${constraint.label} · ${constraint.summary}`;
}

function diskLimitedConstraintMeta(
  download: TransferConstraintSource,
): TransferConstraintMeta {
  const targetConnections = Math.max(download.targetConnections, 1);
  const constrainedSegmentedTransfer =
    download.capabilities.segmented && targetConnections > 1;

  if (!constrainedSegmentedTransfer) {
    return {
      label: "Disk-limited",
      summary: "Disk pressure",
      detail:
        "Writer backpressure is active, so disk writes are limiting further ramp-up and work stealing.",
      tone: "warn",
    };
  }

  const activeConnections = Math.max(
    download.segments.filter((segment) => segment.status === "downloading").length,
    1,
  );
  const summary = download.status === "downloading"
    ? `Holding ${activeConnections}/${targetConnections} parts`
    : `Holding ${targetConnections} planned parts`;
  const detail = download.status === "downloading"
    ? `Writer backpressure is active, so disk writes are holding this transfer at ${activeConnections} of ${targetConnections} planned parts while ramp-up and work stealing stay limited.`
    : `Writer backpressure is active, so disk writes are holding this transfer below its ${targetConnections}-part plan until storage pressure recovers.`;

  return {
    label: "Disk-limited",
    summary,
    detail,
    tone: "warn",
  };
}

export function activeConnectionCount(download: Download): number {
  if (download.capabilities.segmented && download.segments.length > 0) {
    return download.segments.filter(
      (segment) => segment.status === "downloading",
    ).length;
  }

  return download.status === "downloading" ? 1 : 0;
}

export function targetConnectionCount(download: Download): number {
  if (download.status === "finished") {
    return 0;
  }

  return Math.max(download.targetConnections, 1);
}

export function failureKindLabel(
  kind: DownloadFailureKind | null,
): string | null {
  switch (kind) {
    case "http":
      return "HTTP failure";
    case "network":
      return "Network failure";
    case "validation":
      return "Validation failure";
    case "fileSystem":
      return "File system failure";
    default:
      return null;
  }
}

export function transferModeLabel(download: Download): string {
  const restartLabel = restartRequirementLabel(download);
  if (restartLabel) {
    return restartLabel === "Replay-only"
      ? "Guarded single stream - replay-only"
      : "Guarded single stream - restart only";
  }

  if (download.capabilities.segmented && download.segments.length > 0) {
    const activeConnections = activeConnectionCount(download);
    const targetConnections = targetConnectionCount(download);
    return download.status === "downloading"
      ? `Segmented - ${activeConnections}/${targetConnections} parts active`
      : `Segmented - ${targetConnections} planned parts`;
  }

  if (download.capabilities.resumable) {
    return "Single stream - resume ready";
  }

  if (download.capabilities.rangeSupported) {
    return "Single-session range";
  }

  return "Single connection";
}

export function primaryIssueSummary(
  download: Pick<
    Download,
    | "status"
    | "errorMessage"
    | "diagnostics"
    | "capabilities"
    | "compatibility"
  >,
): string | null {
  const restartReason = restartRequirementReason(download);
  const terminalReason = download.diagnostics.terminalReason;

  if (terminalReason) {
    return simplifyUserMessage(terminalReason);
  }

  if (
    restartReason
    && !sameVisibleMessage(restartReason, download.errorMessage)
  ) {
    return simplifyUserMessage(restartReason);
  }

  if (download.errorMessage) {
    return simplifyUserMessage(download.errorMessage);
  }

  return restartReason ? simplifyUserMessage(restartReason) : null;
}

function formatScheduledStartLabel(timestamp: number): string {
  const diffSeconds = Math.ceil((timestamp - Date.now()) / 1000);
  if (diffSeconds <= 0) {
    return "Starting soon";
  }
  if (diffSeconds < 8 * 60 * 60) {
    return `Starts in ${formatDurationShort(diffSeconds)}`;
  }

  const scheduledAt = new Date(timestamp);
  const now = new Date();
  const sameDay = scheduledAt.toDateString() === now.toDateString();
  const timeLabel = scheduledAt.toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
  });
  if (sameDay) {
    return `Starts at ${timeLabel}`;
  }

  return `Starts ${scheduledAt.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  })} ${timeLabel}`;
}

export function hostLockLabel(reason: string | null): string {
  switch (reason) {
    case "probe-failures":
      return "Probe lock";
    case "ramp-no-gain":
      return "Ramp lock";
    case "cooldown-active":
      return "Cooldown lock";
    default:
      return "Host lock";
  }
}

export function formatCooldownLabel(timestamp: number | null): string | null {
  if (timestamp === null) {
    return null;
  }

  const remainingSeconds = Math.ceil((timestamp - Date.now()) / 1000);
  if (remainingSeconds <= 0) {
    return null;
  }

  return `Cooldown ${formatDurationShort(remainingSeconds)}`;
}

export function hostBadgeItems(
  source: HostBadgeSource,
  maxItems = 6,
): HostBadgeMeta[] {
  const rows: HostBadgeMeta[] = [];
  if (source.restartLabel) {
    rows.push({ label: source.restartLabel, tone: "warn" });
  }
  if (source.compatibility.directUrlRecovered) {
    rows.push({ label: "Wrapper recovered", tone: "good" });
  } else if (source.compatibility.browserInterstitialOnly) {
    rows.push({ label: "Browser interstitial", tone: "warn" });
  }
  if (source.compatibility.requestReferer) {
    rows.push({ label: "Wrapper referer", tone: "neutral" });
  }
  if (source.hostDiagnostics.hardNoRange || !source.rangeSupported) {
    rows.push({ label: "No-range host", tone: "warn" });
  }
  const cooldown = formatCooldownLabel(
    source.hostDiagnostics.cooldownUntil ?? source.hostCooldownUntil ?? null,
  );
  if (cooldown) {
    rows.push({ label: cooldown, tone: "warn" });
  }
  if (source.hostDiagnostics.concurrencyLocked) {
    rows.push({
      label: hostLockLabel(source.hostDiagnostics.lockReason),
      tone: "warn",
    });
  }
  const protocol = source.hostDiagnostics.negotiatedProtocol ?? source.hostProtocol;
  if (protocol) {
    rows.push({ label: protocol.toUpperCase(), tone: "neutral" });
  }
  if (source.hostDiagnostics.reuseConnections != null) {
    rows.push({
      label: source.hostDiagnostics.reuseConnections
        ? "Keep-alive reuse"
        : "Fresh sockets",
      tone: source.hostDiagnostics.reuseConnections ? "good" : "neutral",
    });
  }
  if (source.hostMaxConnections != null) {
    rows.push({ label: `Cap ${source.hostMaxConnections}`, tone: "neutral" });
  }
  return rows.slice(0, maxItems);
}

export function probeHostBadgeItems(
  probe: DownloadProbe,
  maxItems = 4,
): HostBadgeMeta[] {
  return hostBadgeItems(probe, maxItems);
}

export function transferConstraintMeta(
  download: TransferConstraintSource,
): TransferConstraintMeta | null {
  if (
    download.scheduledFor != null
    && download.scheduledFor > Date.now()
    && download.status !== "finished"
  ) {
    const summary = formatScheduledStartLabel(download.scheduledFor);
    return {
      label: "Scheduled",
      summary,
      detail: `${summary}. VDM is holding the transfer until its planned queue start time.`,
      tone: "neutral",
    };
  }

  if (download.writerBackpressure) {
    return diskLimitedConstraintMeta(download);
  }

  const cooldown = formatCooldownLabel(
    download.hostDiagnostics.cooldownUntil ?? download.hostCooldownUntil,
  );
  if (cooldown) {
    return {
      label: "Host-limited",
      summary: cooldown,
      detail: `${cooldown} is active because the host or request context is throttling or unstable.`,
      tone: "warn",
    };
  }

  if (download.hostDiagnostics.concurrencyLocked) {
    const summary = hostLockLabel(download.hostDiagnostics.lockReason);
    const plannerLimited = download.hostDiagnostics.lockReason === "ramp-no-gain";
    return {
      label: plannerLimited ? "Planner-limited" : "Host-limited",
      summary,
      detail: plannerLimited
        ? "VDM is holding the current connection count because extra parallel streams were not improving throughput."
        : `${summary} is capping connection ramp-up for stability on this host.`,
      tone: plannerLimited ? "neutral" : "warn",
    };
  }

  if (
    download.diagnostics.restartRequired
    && (download.status === "paused" || download.status === "error")
  ) {
    return {
      label: "Restart required",
      summary: "Restart required",
      detail:
        "This partial state can no longer resume safely, so the transfer must restart from zero.",
      tone: "warn",
    };
  }

  if (
    !download.capabilities.rangeSupported
    && download.status !== "finished"
    && download.status !== "downloading"
  ) {
    return {
      label: "Guarded single stream",
      summary: "Single-stream guarded",
      detail:
        "Byte-range support is unavailable or untrusted, so VDM is holding this transfer to a single connection.",
      tone: "warn",
    };
  }

  return null;
}

export function transferConstraintNotice(
  download: TransferConstraintSource,
): TransferConstraintNotice | null {
  const constraint = transferConstraintMeta(download);
  if (!constraint || constraint.label === "Restart required") {
    return null;
  }

  return {
    message: constraint.detail,
    tone: constraint.tone === "warn" ? "warn" : "note",
  };
}
