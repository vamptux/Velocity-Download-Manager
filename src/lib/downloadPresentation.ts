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
import { formatDurationShort } from "@/lib/format";
import type {
  ChecksumAlgorithm,
  Download,
  DownloadCategory,
  DownloadFailureKind,
  DownloadIntegrity,
  DownloadStatus,
  IntegrityState,
} from "@/types/download";

export type StatusMeta = {
  label: string;
  color: string;
  Icon: LucideIcon;
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

export function statusLabel(status: DownloadStatus): string {
  return STATUS_META[status].label;
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

export function checksumAlgorithmLabel(algorithm: ChecksumAlgorithm): string {
  switch (algorithm) {
    case "md5":
      return "MD5";
    case "sha1":
      return "SHA-1";
    case "sha256":
      return "SHA-256";
    case "sha512":
      return "SHA-512";
  }
}

export function integritySummaryLabel(
  download: Pick<Download, "integrity">,
): string | null {
  const expected = download.integrity.expected;
  if (!expected) {
    return null;
  }

  const algorithm = checksumAlgorithmLabel(expected.algorithm);
  switch (download.integrity.state) {
    case "verified":
      return `${algorithm} verified`;
    case "verifying":
      return `Verifying ${algorithm}`;
    case "mismatch":
      return `${algorithm} mismatch`;
    case "pending":
    case "none":
      return null;
  }
}

export function integrityStatusDetail(
  download: Pick<Download, "integrity">,
): string | null {
  switch (download.integrity.state) {
    case "verified":
      return "Verified";
    case "verifying":
      return "Verifying";
    case "mismatch":
      return "Mismatch";
    case "pending":
    case "none":
      return null;
  }
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

export function integrityStateLabel(state: IntegrityState): string {
  switch (state) {
    case "none":
      return "Not requested";
    case "pending":
      return "Queued";
    case "verifying":
      return "Verifying";
    case "verified":
      return "Verified";
    case "mismatch":
      return "Mismatch";
  }
}

export function integrityBadgeLabel(
  integrity: DownloadIntegrity,
): string | null {
  if (!integrity.expected) {
    return null;
  }

  switch (integrity.state) {
    case "pending":
      return "Checksum queued";
    case "verifying":
      return "Verifying checksum";
    case "verified":
      return "Checksum verified";
    case "mismatch":
      return "Checksum mismatch";
    case "none":
      return null;
  }
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

export function stallReasonLabel(
  download: Pick<
    Download,
    | "status"
    | "writerBackpressure"
    | "hostCooldownUntil"
    | "hostDiagnostics"
    | "diagnostics"
    | "capabilities"
  >,
): string | null {
  if (download.writerBackpressure) {
    return "Disk pressure";
  }

  const cooldown = formatCooldownLabel(
    download.hostDiagnostics.cooldownUntil ?? download.hostCooldownUntil,
  );
  if (cooldown) {
    return cooldown;
  }

  if (download.hostDiagnostics.concurrencyLocked) {
    return hostLockLabel(download.hostDiagnostics.lockReason);
  }

  if (
    download.diagnostics.restartRequired
    && (download.status === "paused" || download.status === "error")
  ) {
    return "Restart required";
  }

  if (
    !download.capabilities.rangeSupported
    && download.status !== "finished"
    && download.status !== "downloading"
  ) {
    return "Single-stream guarded";
  }

  return null;
}
