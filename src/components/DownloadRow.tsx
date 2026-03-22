import { memo, useState } from "react";
import * as ContextMenu from "@radix-ui/react-context-menu";
import * as Dialog from "@radix-ui/react-dialog";
import * as Select from "@radix-ui/react-select";
import {
  ArrowDown,
  ArrowUp,
  StopCircle,
  FolderOpen,
  Copy,
  Play,
  Pause,
  RotateCcw,
  Trash2,
  Shield,
  ShieldOff,
  ChevronDown,
  Check,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Checkbox } from "@/components/ui/checkbox";
import {
  canPauseDownload,
  canRestartDownload,
  canResumeDownload,
  restartRequirementLabel,
} from "@/lib/downloadActions";
import {
  activeConnectionCount,
  CATEGORY_ICONS,
  CATEGORY_ICON_BG,
  CATEGORY_ICON_COLORS,
  CATEGORY_LABELS,
  integrityStatusDetail,
  integritySummaryLabel,
  stallReasonLabel,
  STATUS_META,
  targetConnectionCount,
} from "@/lib/downloadPresentation";
import {
  formatBytes,
  formatBytesPerSecond,
  formatRelativeDate,
  formatTimeRemaining,
} from "@/lib/format";
import {
  calculateDisplayProgress,
  useSmoothedNumber,
} from "@/lib/downloadProgress";
import { simplifyUserMessage } from "@/lib/userFacingMessages";
import {
  ipcPauseDownload,
  ipcRestartDownload,
  ipcResumeDownload,
  ipcSetDownloadChecksum,
} from "@/lib/ipc";
import { writeClipboardText } from "@/lib/clipboard";
import type { ChecksumAlgorithm, Download } from "@/types/download";

const CHECKSUM_ALGORITHMS: { value: ChecksumAlgorithm; label: string }[] = [
  { value: "sha256", label: "SHA-256" },
  { value: "sha512", label: "SHA-512" },
  { value: "sha1", label: "SHA-1" },
  { value: "md5", label: "MD5" },
];

function AlgorithmSelect({
  value,
  onChange,
}: {
  value: ChecksumAlgorithm;
  onChange: (v: ChecksumAlgorithm) => void;
}) {
  return (
    <Select.Root
      value={value}
      onValueChange={(v) => onChange(v as ChecksumAlgorithm)}
    >
      <Select.Trigger
        className={cn(
          "flex items-center justify-between rounded-md border border-border bg-[hsl(var(--card))] px-3 h-8 text-[12.5px] text-foreground outline-none w-full",
          "focus:border-primary/60 focus:ring-1 focus:ring-primary/30 transition-colors",
        )}
      >
        <Select.Value />
        <Select.Icon>
          <ChevronDown size={13} className="opacity-50" />
        </Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Content
          position="popper"
          sideOffset={4}
          className={cn(
            "z-[200] w-[var(--radix-select-trigger-width)] rounded-md border border-border py-1",
            "bg-[hsl(var(--card))] shadow-xl shadow-black/40",
            "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
          )}
        >
          <Select.Viewport>
            {CHECKSUM_ALGORITHMS.map((algo) => (
              <Select.Item
                key={algo.value}
                value={algo.value}
                className={cn(
                  "flex items-center justify-between px-3 py-[5px] text-[12px] cursor-default outline-none rounded-sm mx-0.5",
                  "text-foreground/80 data-[highlighted]:bg-accent data-[highlighted]:text-foreground",
                )}
              >
                <Select.ItemText>{algo.label}</Select.ItemText>
                <Select.ItemIndicator>
                  <Check size={12} className="text-primary" />
                </Select.ItemIndicator>
              </Select.Item>
            ))}
          </Select.Viewport>
        </Select.Content>
      </Select.Portal>
    </Select.Root>
  );
}

function transferModeLabel(download: Download): string {
  const restartLabel = restartRequirementLabel(download);
  if (restartLabel) {
    return restartLabel === "Replay-only"
      ? "Guarded single stream • replay-only"
      : "Guarded single stream • restart only";
  }

  if (download.capabilities.segmented && download.segments.length > 0) {
    const activeConnections = activeConnectionCount(download);
    const targetConnections = targetConnectionCount(download);
    return download.status === "downloading"
      ? `Segmented • ${activeConnections}/${targetConnections} parts active`
      : `Segmented • ${targetConnections} planned parts`;
  }

  if (download.capabilities.resumable) {
    return "Single stream • resume ready";
  }

  if (download.capabilities.rangeSupported) {
    return "Single-session range";
  }

  return "Single connection";
}

function ProgressBar({
  pct,
  paused,
  active,
}: {
  pct: number;
  paused?: boolean;
  active?: boolean;
}) {
  return (
    <div className="absolute bottom-0 left-0 right-0 h-[2px] bg-white/[0.03]">
      <div
        className={cn(
          "h-full transition-[width] duration-200 relative overflow-hidden",
          paused
            ? "bg-[hsl(var(--status-paused)/0.55)]"
            : "bg-gradient-to-r from-[hsl(var(--status-downloading)/0.5)] via-[hsl(var(--status-downloading)/0.85)] to-[hsl(var(--primary))]",
        )}
        style={{ width: `${Math.min(100, Math.max(0, pct))}%` }}
      >
        {active && !paused && (
          <span
            className="absolute inset-0"
            style={{
              background:
                "linear-gradient(90deg, transparent 0%, rgba(255,255,255,0.28) 50%, transparent 100%)",
              backgroundSize: "400px 100%",
              animation: "progress-shimmer 1.8s linear infinite",
            }}
          />
        )}
      </div>
    </div>
  );
}

function MenuItem({
  icon: Icon,
  label,
  danger,
  tone,
  shortcut,
  onSelect,
  disabled,
}: {
  icon: React.ElementType;
  label: string;
  danger?: boolean;
  tone?: "green" | "amber";
  shortcut?: string;
  onSelect?: () => void;
  disabled?: boolean;
}) {
  return (
    <ContextMenu.Item
      onSelect={onSelect}
      disabled={disabled}
      className={cn(
        "flex items-center gap-2 px-3 py-[5px] text-[12px] rounded-sm outline-none cursor-default select-none",
        "data-[disabled]:opacity-25 data-[disabled]:pointer-events-none",
        danger
          ? "text-[hsl(var(--status-error))] data-[highlighted]:bg-[hsl(var(--status-error)/0.12)] data-[highlighted]:text-[hsl(var(--status-error))]"
          : tone === "green"
            ? "text-[hsl(var(--status-finished)/0.8)] data-[highlighted]:bg-[hsl(var(--status-finished)/0.12)] data-[highlighted]:text-[hsl(var(--status-finished))]"
            : tone === "amber"
              ? "text-[hsl(var(--status-paused)/0.85)] data-[highlighted]:bg-[hsl(var(--status-paused)/0.12)] data-[highlighted]:text-[hsl(var(--status-paused))]"
              : "text-foreground/78 data-[highlighted]:bg-accent data-[highlighted]:text-foreground",
      )}
    >
      <Icon size={13} strokeWidth={1.7} className="shrink-0" />
      <span className="flex-1">{label}</span>
      {shortcut ? (
        <span className="shrink-0 text-[9.5px] text-muted-foreground/30 tabular-nums">
          {shortcut}
        </span>
      ) : null}
    </ContextMenu.Item>
  );
}

export interface DownloadRowProps {
  download: Download;
  index: number;
  selected: boolean;
  canMoveUp: boolean;
  canMoveDown: boolean;
  onSelect: (id: string, checked: boolean) => void;
  onActivate: (id: string) => void;
  onDelete: (id: string) => Promise<void>;
  onReorder: (id: string, direction: "up" | "down") => Promise<void> | void;
  onOpenFolder: (id: string) => Promise<void> | void;
  onRefresh: () => void;
}

export const DownloadRow = memo(function DownloadRow({
  download,
  index,
  selected,
  canMoveUp,
  canMoveDown,
  onSelect,
  onActivate,
  onDelete,
  onReorder,
  onOpenFolder,
  onRefresh,
}: DownloadRowProps) {
  const { label: baseLabel, color, Icon } = STATUS_META[download.status];
  const CategoryIcon = CATEGORY_ICONS[download.category];
  const categoryIconColor = CATEGORY_ICON_COLORS[download.category];
  const isFinalizing =
    download.status === "downloading"
    && download.size > 0
    && download.downloaded >= download.size;
  const label = isFinalizing ? "Finalizing" : baseLabel;
  const pct = calculateDisplayProgress(
    download.downloaded,
    download.size,
    download.status,
  );
  const smoothedPct = useSmoothedNumber(pct, {
    durationMs: download.status === "downloading" ? 760 : 240,
  });
  const isDownloading = download.status === "downloading";
  const showProgress =
    pct > 0 && download.status !== "finished" && download.status !== "error";
  const canResume = canResumeDownload(download);
  const canPause = canPauseDownload(download);
  const canRestart = canRestartDownload(download);
  const canStop =
    download.status === "downloading" || download.status === "paused";
  const queueLabel =
    download.queuePosition > 0 ? `#${download.queuePosition}` : null;
  const transferMode = transferModeLabel(download);
  const restartLabel = restartRequirementLabel(download);
  const integritySummary = integritySummaryLabel(download);
  const isAlt = index % 2 !== 0;
  const simplifiedErrorMessage = download.errorMessage
    ? simplifyUserMessage(download.errorMessage)
    : null;
  const summaryText =
    download.status === "error" && download.integrity.state === "mismatch"
      ? `${integritySummary ?? "Checksum mismatch"} · ${download.host || CATEGORY_LABELS[download.category]}`
      : download.status === "error" && simplifiedErrorMessage
      ? simplifiedErrorMessage
      : download.diagnostics.restartRequired
        ? `${restartLabel ?? "Restart only"} · ${download.host || CATEGORY_LABELS[download.category]}`
      : integritySummary && download.host
        ? `${download.host} · ${transferMode} · ${integritySummary}`
      : integritySummary
        ? `${CATEGORY_LABELS[download.category]} · ${transferMode} · ${integritySummary}`
      : download.host
        ? `${download.host} · ${transferMode}`
        : `${CATEGORY_LABELS[download.category]} · ${transferMode}`;
  const secondaryText = queueLabel ? `${queueLabel} · ${summaryText}` : summaryText;
  const stallReason = stallReasonLabel(download);
  const statusDetail =
    stallReason
    ?? integrityStatusDetail(download)
    ?? restartLabel
    ?? (isFinalizing ? "Flushing to disk" : null)
    ?? (showProgress && pct > 0 ? `${Math.round(smoothedPct)}%` : null);

  const [csOpen, setCsOpen] = useState(false);
  const [csAlgorithm, setCsAlgorithm] = useState<ChecksumAlgorithm>("sha256");
  const [csValue, setCsValue] = useState("");
  const [csSaving, setCsSaving] = useState(false);
  const [csError, setCsError] = useState<string | null>(null);

  function openChecksumDialog() {
    const existing = download.integrity.expected;
    setCsAlgorithm(existing?.algorithm ?? "sha256");
    setCsValue(existing?.value ?? "");
    setCsError(null);
    setCsOpen(true);
  }

  async function handleChecksumSave(e: React.FormEvent) {
    e.preventDefault();
    setCsSaving(true);
    setCsError(null);
    try {
      await ipcSetDownloadChecksum(
        download.id,
        csValue.trim()
          ? { algorithm: csAlgorithm, value: csValue.trim() }
          : null,
      );
      setCsOpen(false);
      onRefresh();
    } catch (err) {
      setCsError(
        err instanceof Error
          ? err.message
          : typeof err === "string"
            ? err
            : "Failed to set checksum.",
      );
    } finally {
      setCsSaving(false);
    }
  }

  function copyUrl() {
    void writeClipboardText(download.url).catch(() => null);
  }

  async function handleResume() {
    await ipcResumeDownload(download.id).catch(() => null);
    onRefresh();
  }

  async function handlePause() {
    await ipcPauseDownload(download.id).catch(() => null);
    onRefresh();
  }

  async function handleRestart() {
    await ipcRestartDownload(download.id).catch(() => null);
    onRefresh();
  }

  async function handleStop() {
    await ipcPauseDownload(download.id).catch(() => null);
    onRefresh();
  }

  async function handleOpenFolder() {
    await onOpenFolder(download.id);
  }

  async function handleReorder(direction: "up" | "down") {
    await onReorder(download.id, direction);
  }

  return (
    <>
      <ContextMenu.Root>
        <ContextMenu.Trigger asChild>
          <div
            className={cn(
              "relative flex items-center text-[12px] border-b border-border/25 cursor-default select-none transition-colors group/row",
              selected
                ? "bg-accent/70"
                : isAlt
                  ? "bg-[hsl(var(--row-alt))] hover:bg-accent/20"
                  : "hover:bg-accent/15",
              isDownloading && !selected && "shadow-[inset_2px_0_0_hsl(var(--status-downloading)/0.3)]",
            )}
            style={{ minHeight: "38px" }}
            onClick={() => onActivate(download.id)}
          >
            <span
              className={cn(
                "absolute left-0 top-[3px] bottom-[3px] w-[3px] rounded-full bg-primary transition-all duration-150",
                selected ? "opacity-100" : "opacity-0",
              )}
            />

            <div className="flex w-8 shrink-0 items-center justify-center">
              <Checkbox
                checked={selected}
                onChange={(checked) => onSelect(download.id, checked)}
                onClick={(e) => e.stopPropagation()}
              />
            </div>

            {/* Category icon with gradient badge */}
            <div className="flex w-7 shrink-0 items-center justify-center">
              <div
                className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[4px]"
                style={{ background: CATEGORY_ICON_BG[download.category] }}
              >
                <CategoryIcon
                  size={11.5}
                  className={cn("shrink-0", categoryIconColor)}
                  strokeWidth={1.6}
                />
              </div>
            </div>

            <div className="flex flex-1 flex-col justify-center min-w-0 px-2 py-[2px]">
              <span className="truncate text-[11.5px] text-foreground/90 leading-tight font-medium">
                {download.name}
              </span>
              <span
                className={cn(
                  "truncate text-[9.5px] leading-tight mt-[1px]",
                  download.status === "error"
                    ? "text-[hsl(var(--status-error)/0.75)]"
                    : "text-muted-foreground/52",
                )}
                title={secondaryText}
              >
                {secondaryText}
              </span>
            </div>

            <div className="w-[72px] shrink-0 px-2 text-right text-[11px] text-muted-foreground/55 tabular-nums">
              {formatBytes(download.size, {
                unknownLabel: "—",
                preserveWholeNumbers: true,
              })}
            </div>

            <div
              className={cn(
                "w-[88px] shrink-0 px-2 flex items-center gap-1.5",
                color,
              )}
            >
              <Icon
                size={11}
                strokeWidth={1.8}
                className={cn(
                  "shrink-0",
                  isDownloading && "animate-spin",
                )}
                style={isDownloading ? { filter: "drop-shadow(0 0 4px hsl(var(--status-downloading) / 0.6))" } : undefined}
              />
              <div className="flex flex-col min-w-0">
                <span className="text-[11px] font-medium leading-tight">
                  {label}
                </span>
                {statusDetail ? (
                  <span className="text-[9.5px] opacity-55 leading-tight tabular-nums">
                    {statusDetail}
                  </span>
                ) : null}
              </div>
            </div>

            <div
              className={cn(
                "w-[80px] shrink-0 px-2 text-right text-[11px] tabular-nums",
                isDownloading && download.speed > 0
                  ? "text-[hsl(var(--status-downloading)/0.88)] font-medium"
                  : "text-muted-foreground/45",
              )}
            >
              {formatBytesPerSecond(download.speed, {
                idleLabel: "—",
                preserveWholeNumbers: true,
              })}
            </div>

            <div className="w-[68px] shrink-0 px-2 text-right text-[11px] text-muted-foreground/50 tabular-nums">
              {formatTimeRemaining(download.timeLeft, { emptyLabel: "—" })}
            </div>

            <div className="w-[90px] shrink-0 px-2 text-right text-[11px] text-muted-foreground/45">
              {formatRelativeDate(download.dateAdded)}
            </div>

            <div className="w-1 shrink-0" />

            {showProgress ? (
              <ProgressBar
                pct={smoothedPct}
                paused={download.status === "paused"}
                active={isDownloading}
              />
            ) : null}
          </div>
        </ContextMenu.Trigger>

        <ContextMenu.Portal>
          <ContextMenu.Content
            className={cn(
              "z-50 min-w-[210px] rounded-lg border border-border/75 py-1",
              "bg-popover shadow-[0_12px_32px_rgba(0,0,0,0.55)]",
              "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95 data-[state=open]:slide-in-from-top-1",
            )}
          >
            <div className="px-3 pt-2 pb-1.5 border-b border-border/40 mb-0.5">
              <div className="flex items-center justify-between gap-2 min-w-0">
                <span className="truncate text-[11.5px] font-medium text-foreground/82 leading-none">
                  {download.name}
                </span>
                <span
                  className={cn(
                    "shrink-0 text-[9.5px] font-semibold uppercase tracking-wide leading-none",
                    color,
                  )}
                >
                  {label}
                </span>
              </div>
            </div>

            <MenuItem
              icon={FolderOpen}
              label="Open folder"
              onSelect={() => void handleOpenFolder()}
            />
            <MenuItem
              icon={Copy}
              label="Copy URL"
              shortcut="Ctrl+C"
              onSelect={copyUrl}
            />
            <MenuItem
              icon={Shield}
              label="Set checksum…"
              onSelect={openChecksumDialog}
              disabled={download.integrity.state === "verifying"}
            />

            <ContextMenu.Separator className="my-1 h-px bg-border/40 mx-2" />

            <MenuItem
              icon={Play}
              label={
                download.diagnostics.restartRequired
                  ? "Resume unavailable"
                  : "Resume"
              }
              tone="green"
              shortcut="Space"
              disabled={!canResume}
              onSelect={() => void handleResume()}
            />
            <MenuItem
              icon={Pause}
              label="Pause"
              tone="amber"
              shortcut="Space"
              disabled={!canPause}
              onSelect={() => void handlePause()}
            />
            <MenuItem
              icon={RotateCcw}
              label={
                download.diagnostics.restartRequired
                  ? "Restart from zero"
                  : "Restart"
              }
              disabled={!canRestart}
              onSelect={() => void handleRestart()}
            />
            <MenuItem
              icon={StopCircle}
              label="Stop"
              disabled={!canStop}
              onSelect={() => void handleStop()}
            />

            <ContextMenu.Separator className="my-1 h-px bg-border/40 mx-2" />

            <MenuItem
              icon={ArrowUp}
              label="Move up in queue"
              disabled={!canMoveUp}
              onSelect={() => void handleReorder("up")}
            />
            <MenuItem
              icon={ArrowDown}
              label="Move down in queue"
              disabled={!canMoveDown}
              onSelect={() => void handleReorder("down")}
            />

            <ContextMenu.Separator className="my-1 h-px bg-border/40 mx-2" />

            <MenuItem
              icon={Trash2}
              label="Delete"
              shortcut="Del"
              danger
              onSelect={() => void onDelete(download.id)}
            />
          </ContextMenu.Content>
        </ContextMenu.Portal>
      </ContextMenu.Root>

      <Dialog.Root open={csOpen} onOpenChange={setCsOpen}>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-[100] bg-black/50 backdrop-blur-[2px] data-[state=open]:animate-in data-[state=open]:fade-in-0" />
          <Dialog.Content
            className={cn(
              "fixed left-1/2 top-1/2 z-[101] -translate-x-1/2 -translate-y-1/2",
              "w-[420px] rounded-lg border border-border bg-[hsl(var(--background))] shadow-2xl shadow-black/60 outline-none",
              "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
            )}
          >
            <div className="flex items-center justify-between border-b border-border px-4 py-3">
              <div className="flex items-center gap-2">
                <Shield
                  size={13}
                  className="text-muted-foreground/60"
                  strokeWidth={1.8}
                />
                <Dialog.Title className="text-[13px] font-semibold text-foreground">
                  Set Checksum
                </Dialog.Title>
              </div>
              <Dialog.Close className="flex h-6 w-6 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground transition-colors">
                <X size={13} strokeWidth={2} />
              </Dialog.Close>
            </div>
            <form
              onSubmit={(e) => void handleChecksumSave(e)}
              className="flex flex-col gap-3 px-4 py-4"
            >
              <p className="text-[11.5px] text-muted-foreground/70 leading-relaxed">
                If the file is already finished, VDM will verify it immediately.
                Otherwise it will verify after the download completes. Leave the
                hash blank to remove the current checksum.
              </p>
              <div className="grid grid-cols-[120px_minmax(0,1fr)] gap-2">
                <div className="flex flex-col gap-1.5">
                  <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wide">
                    Algorithm
                  </label>
                  <AlgorithmSelect
                    value={csAlgorithm}
                    onChange={setCsAlgorithm}
                  />
                </div>
                <div className="flex flex-col gap-1.5">
                  <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wide">
                    Expected hash
                  </label>
                  <input
                    type="text"
                    placeholder="Leave blank to clear"
                    value={csValue}
                    onChange={(e) => {
                      setCsValue(e.target.value);
                      setCsError(null);
                    }}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoCorrect="off"
                    className={cn(
                      "w-full rounded-md border border-border bg-[hsl(var(--card))] px-3 h-8 text-[12px] text-foreground placeholder:text-muted-foreground/40 outline-none",
                      "focus:border-primary/60 focus:ring-1 focus:ring-primary/30 transition-colors",
                    )}
                  />
                </div>
              </div>
              {download.integrity.expected ? (
                <button
                  type="button"
                  onClick={() => {
                    setCsValue("");
                  }}
                  className="flex items-center gap-1.5 self-start text-[11px] text-muted-foreground/55 hover:text-muted-foreground transition-colors"
                >
                  <ShieldOff size={11} strokeWidth={1.8} />
                  Clear existing checksum
                </button>
              ) : null}
              {csError ? (
                <div className="rounded-md border border-[hsl(var(--status-error)/0.26)] bg-[hsl(var(--status-error)/0.08)] px-3 py-2 text-[11.5px] text-[hsl(var(--status-error))]">
                  {csError}
                </div>
              ) : null}
              <div className="flex items-center justify-end gap-2 pt-1">
                <Dialog.Close asChild>
                  <button
                    type="button"
                    className="h-7 px-3 rounded-md border border-border text-[12px] text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
                  >
                    Cancel
                  </button>
                </Dialog.Close>
                <button
                  type="submit"
                  disabled={csSaving}
                  className="h-7 px-4 rounded-md bg-primary hover:bg-primary/90 text-primary-foreground text-[12px] font-medium transition-colors disabled:opacity-40"
                >
                  {csSaving ? "Saving…" : "Save"}
                </button>
              </div>
            </form>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </>
  );
});
