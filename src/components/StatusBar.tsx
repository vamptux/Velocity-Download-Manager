import { useEffect, useMemo, useRef, useState } from "react";
import { Activity, ArrowDown, ChevronDown, ChevronUp, Cpu } from "lucide-react";
import { formatBytesPerSecond } from "@/lib/format";
import {
  parseSpeedLimitDraft,
  speedLimitDraftFromValue,
  type SpeedLimitUnit,
} from "@/lib/speedLimits";
import { cn } from "@/lib/utils";

interface StatusBarProps {
  queuedCount: number;
  pausedCount: number;
  finishedCount: number;
  queueRunning: boolean;
  activeCount: number;
  activeLimit: number;
  connectionCount: number;
  /** Active transfer speed in bytes/sec */
  downloadSpeed: number;
  speedLimitBytesPerSecond: number | null;
  manualOverrideCount: number;
  engineBootstrapReady: boolean;
  engineBootstrapError: string | null;
  onRetryEngineBootstrap?: () => void;
  onUpdateGlobalSpeedLimit?: (limitBytesPerSecond: number | null) => Promise<void> | void;
}

const GLOBAL_SPEED_PRESETS_MB = [10, 25, 50, 100, 250, 500] as const;

function Dot({ color }: { color: string }) {
  return <span className={cn("h-[5px] w-[5px] shrink-0 rounded-full", color)} />;
}

function Sep() {
  return <span className="h-3 w-px shrink-0 bg-border/35" />;
}

function getErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  if (typeof error === "string" && error.trim()) {
    return error;
  }

  return "VDM could not update the global speed cap.";
}

export function StatusBar({
  queuedCount,
  pausedCount,
  finishedCount,
  queueRunning,
  activeCount,
  activeLimit,
  connectionCount,
  downloadSpeed,
  speedLimitBytesPerSecond,
  manualOverrideCount,
  engineBootstrapReady,
  engineBootstrapError,
  onRetryEngineBootstrap,
  onUpdateGlobalSpeedLimit,
}: StatusBarProps) {
  const [limiterOpen, setLimiterOpen] = useState(false);
  const [savingLimit, setSavingLimit] = useState(false);
  const [limitError, setLimitError] = useState<string | null>(null);
  const [draftValue, setDraftValue] = useState("25");
  const [draftUnit, setDraftUnit] = useState<SpeedLimitUnit>("mb");
  const limiterRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const isActive = activeCount > 0;
  const speedLimitLabel = speedLimitBytesPerSecond != null
    ? formatBytesPerSecond(speedLimitBytesPerSecond, { idleLabel: "Unlimited", fixedFractionDigits: 1 })
    : "Unlimited";

  const queueState = useMemo(() => {
    if (activeCount > 0) {
      return {
        text: queueRunning ? "Running" : "Manual",
        className: queueRunning
          ? "bg-[hsl(var(--status-downloading)/0.12)] text-[hsl(var(--status-downloading)/0.8)]"
          : "bg-white/[0.05] text-foreground/55",
      };
    }

    if (queuedCount > 0) {
      return {
        text: queueRunning ? "Queued" : "Paused",
        className: queueRunning
          ? "bg-[hsl(var(--status-downloading)/0.1)] text-[hsl(var(--status-downloading)/0.72)]"
          : "bg-[hsl(var(--status-paused)/0.1)] text-[hsl(var(--status-paused)/0.75)]",
      };
    }

    if (pausedCount > 0) {
      return {
        text: "Paused",
        className: "bg-[hsl(var(--status-paused)/0.1)] text-[hsl(var(--status-paused)/0.75)]",
      };
    }

    if (finishedCount > 0) {
      return {
        text: "Finished",
        className: "bg-[hsl(var(--status-finished)/0.1)] text-[hsl(var(--status-finished)/0.75)]",
      };
    }

    return {
      text: "Idle",
      className: "bg-white/[0.04] text-muted-foreground/55",
    };
  }, [activeCount, finishedCount, pausedCount, queuedCount, queueRunning]);

  const engineColor = engineBootstrapError
    ? "bg-[hsl(var(--status-error))]"
    : engineBootstrapReady
      ? "bg-[hsl(var(--status-finished)/0.7)]"
      : "bg-[hsl(var(--status-downloading)/0.6)]";

  const parsedDraft = parseSpeedLimitDraft(true, draftValue, draftUnit);
  const draftDirty = parsedDraft.error == null && parsedDraft.limitBytesPerSecond !== speedLimitBytesPerSecond;

  useEffect(() => {
    if (limiterOpen) {
      return;
    }

    const draft = speedLimitDraftFromValue(speedLimitBytesPerSecond);
    setDraftValue(draft.value);
    setDraftUnit(draft.unit);
    setLimitError(null);
  }, [limiterOpen, speedLimitBytesPerSecond]);

  useEffect(() => {
    if (!limiterOpen) {
      return;
    }

    function handlePointerDown(event: MouseEvent) {
      if (limiterRef.current && !limiterRef.current.contains(event.target as Node)) {
        setLimiterOpen(false);
      }
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setLimiterOpen(false);
      }
    }

    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [limiterOpen]);

  useEffect(() => {
    if (!limiterOpen) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    return () => {
      window.cancelAnimationFrame(frame);
    };
  }, [limiterOpen]);

  async function applyLimit(nextLimitBytesPerSecond: number | null) {
    if (!onUpdateGlobalSpeedLimit) {
      return;
    }

    setSavingLimit(true);
    setLimitError(null);
    try {
      await onUpdateGlobalSpeedLimit(nextLimitBytesPerSecond);
      setLimiterOpen(false);
    } catch (error) {
      setLimitError(getErrorMessage(error));
    } finally {
      setSavingLimit(false);
    }
  }

  async function handleApplyCustom() {
    if (parsedDraft.error) {
      setLimitError(parsedDraft.error);
      return;
    }

    await applyLimit(parsedDraft.limitBytesPerSecond);
  }

  async function handlePreset(limitBytesPerSecond: number) {
    const draft = speedLimitDraftFromValue(limitBytesPerSecond);
    setDraftValue(draft.value);
    setDraftUnit(draft.unit);
    await applyLimit(limitBytesPerSecond);
  }

  return (
    <div
      className="flex h-[22px] shrink-0 items-center gap-2.5 border-t border-border/40 px-3 text-[10.5px] tabular-nums"
      style={{ background: "hsl(var(--sidebar))" }}
    >
      {/* Active & queued */}
      <span className={cn("flex items-center gap-1.5 transition-colors", isActive ? "text-[hsl(var(--status-downloading))]" : "text-muted-foreground/40")}>
        <Activity size={9.5} strokeWidth={2} />
        <span>{activeCount}&thinsp;/&thinsp;{activeLimit}</span>
      </span>

      {queuedCount > 0 && (
        <>
          <Sep />
          <span className="text-muted-foreground/45">{queuedCount} queued</span>
        </>
      )}

      <Sep />

      {/* Connections */}
      <span className={cn("transition-colors", isActive ? "text-foreground/55" : "text-muted-foreground/38")}>
        {connectionCount} conn
      </span>

      <Sep />

      {/* Queue state pill */}
      <span
        className={cn(
          "rounded-[3px] px-1.5 py-[1px] text-[9.5px] font-medium tracking-wide transition-colors",
          queueState.className,
        )}
      >
        {queueState.text}
      </span>

      <Sep />

      <div ref={limiterRef} className="relative">
        <button
          type="button"
          onClick={() => {
            if (!onUpdateGlobalSpeedLimit) {
              return;
            }
            setLimiterOpen((prev) => !prev);
            setLimitError(null);
          }}
          disabled={!onUpdateGlobalSpeedLimit}
          className={cn(
            "flex items-center gap-1 rounded-[4px] px-1.5 py-[1px] transition-colors",
            onUpdateGlobalSpeedLimit
              ? "text-muted-foreground/42 hover:bg-white/[0.04] hover:text-foreground/78"
              : "text-muted-foreground/26",
            limiterOpen && "bg-white/[0.05] text-foreground/82",
          )}
          title={`Global speed cap — click to change`}
        >
          <span>{speedLimitLabel}</span>
          {manualOverrideCount > 0 && (
            <span className="rounded-full bg-[hsl(var(--status-paused)/0.1)] px-1 py-px text-[8.5px] font-semibold text-[hsl(var(--status-paused)/0.82)]">
              {manualOverrideCount}
            </span>
          )}
          {limiterOpen ? <ChevronUp size={9} strokeWidth={2} /> : <ChevronDown size={9} strokeWidth={2} />}
        </button>

        {limiterOpen && (
          <div className="absolute bottom-[calc(100%+6px)] left-0 z-[70] w-[264px] overflow-hidden rounded-[10px] border border-white/[0.09] bg-[hsl(var(--card))] text-left shadow-[0_20px_52px_rgba(0,0,0,0.6),0_0_0_1px_rgba(0,0,0,0.2)] animate-in fade-in-0 zoom-in-95 slide-in-from-bottom-1 duration-120">
            {/* Header: current cap + unlimited action */}
            <div className="flex items-center justify-between gap-2 px-3 pt-2.5 pb-2">
              <div>
                <div className="text-[8.5px] font-semibold uppercase tracking-[0.15em] text-muted-foreground/35">Global cap</div>
                <div className="mt-0.5 text-[13px] font-semibold tabular-nums leading-none text-foreground/88">{speedLimitLabel}</div>
              </div>
              <button
                type="button"
                onClick={() => void applyLimit(null)}
                disabled={savingLimit || speedLimitBytesPerSecond == null}
                className={cn(
                  "rounded-[5px] border px-2 py-[3px] text-[9.5px] font-medium transition-colors",
                  savingLimit || speedLimitBytesPerSecond == null
                    ? "cursor-not-allowed border-border/40 bg-transparent text-muted-foreground/28"
                    : "border-[hsl(var(--status-finished)/0.3)] bg-[hsl(var(--status-finished)/0.09)] text-[hsl(var(--status-finished)/0.85)] hover:bg-[hsl(var(--status-finished)/0.16)]",
                )}
              >
                Unlimited
              </button>
            </div>

            <div className="h-px bg-border/30 mx-2.5" />

            {/* Preset pills — 3 × 2 grid */}
            <div className="grid grid-cols-3 gap-1 p-2.5">
              {GLOBAL_SPEED_PRESETS_MB.map((preset) => {
                const presetBytes = preset * 1024 * 1024;
                const isActive = speedLimitBytesPerSecond === presetBytes;
                return (
                  <button
                    key={preset}
                    type="button"
                    onClick={() => void handlePreset(presetBytes)}
                    disabled={savingLimit}
                    className={cn(
                      "rounded-[6px] border py-1.5 text-[10px] font-medium tabular-nums transition-all duration-100",
                      isActive
                        ? "border-[hsl(var(--primary)/0.4)] bg-[hsl(var(--primary)/0.14)] text-foreground shadow-[0_0_0_1px_hsl(var(--primary)/0.12)]"
                        : "border-border/45 bg-white/[0.025] text-muted-foreground/52 hover:border-border/75 hover:bg-white/[0.06] hover:text-foreground/80",
                    )}
                  >
                    {preset >= 1000 ? `${preset / 1000}G` : `${preset}M`}
                  </button>
                );
              })}
            </div>

            <div className="h-px bg-border/30 mx-2.5" />

            {/* Custom value row */}
            <div className="flex items-center gap-1.5 px-2.5 py-2.5">
              <input
                ref={inputRef}
                type="number"
                min="0"
                step="0.1"
                disabled={savingLimit}
                value={draftValue}
                onChange={(event) => {
                  setDraftValue(event.target.value);
                  setLimitError(null);
                }}
                className="h-[26px] min-w-0 flex-1 rounded-[5px] border border-border/50 bg-black/30 px-2 text-[11.5px] tabular-nums text-foreground/85 outline-none transition-colors focus:border-[hsl(var(--primary)/0.5)]"
              />
              <select
                disabled={savingLimit}
                value={draftUnit}
                onChange={(event) => {
                  setDraftUnit(event.target.value as SpeedLimitUnit);
                  setLimitError(null);
                }}
                className="h-[26px] w-[58px] shrink-0 rounded-[5px] border border-border/50 bg-black/30 px-1 text-[10.5px] text-foreground/75 outline-none transition-colors"
              >
                <option value="kb">KB/s</option>
                <option value="mb">MB/s</option>
                <option value="gb">GB/s</option>
              </select>
              <button
                type="button"
                onClick={() => void handleApplyCustom()}
                disabled={savingLimit || !draftDirty}
                className={cn(
                  "h-[26px] shrink-0 rounded-[5px] border px-2.5 text-[10.5px] font-medium transition-colors",
                  savingLimit || !draftDirty
                    ? "cursor-not-allowed border-border/40 bg-black/10 text-muted-foreground/28"
                    : "border-[hsl(var(--primary)/0.4)] bg-[hsl(var(--primary)/0.12)] text-[hsl(var(--primary))] hover:bg-[hsl(var(--primary)/0.2)]",
                )}
              >
                {savingLimit ? "…" : "Set"}
              </button>
            </div>

            {/* Error / override note */}
            {(limitError || parsedDraft.error || manualOverrideCount > 0) && (
              <div className={cn(
                "border-t border-border/25 px-3 py-1.5 text-[9.5px] leading-[1.4]",
                limitError || parsedDraft.error
                  ? "text-[hsl(var(--status-error)/0.85)]"
                  : "text-muted-foreground/42",
              )}>
                {limitError
                  ?? parsedDraft.error
                  ?? `${manualOverrideCount} download${manualOverrideCount === 1 ? "" : "s"} use a manual cap override.`}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Spacer → right-aligned items */}
      <span className="flex-1" />

      {/* Engine status */}
      <span
        className={cn(
          "flex items-center gap-1.5 transition-colors",
          engineBootstrapError ? "text-[hsl(var(--status-error))]" : "text-muted-foreground/40",
        )}
        title={engineBootstrapError ?? undefined}
      >
        <Dot color={engineColor} />
        <Cpu size={9.5} strokeWidth={2} />
        {engineBootstrapError && onRetryEngineBootstrap && (
          <button
            type="button"
            onClick={onRetryEngineBootstrap}
            className="rounded border border-[hsl(var(--status-error)/0.3)] px-1 py-[1px] text-[9px] font-semibold text-[hsl(var(--status-error))] transition-colors hover:bg-[hsl(var(--status-error)/0.1)]"
          >
            Retry
          </button>
        )}
      </span>

      <Sep />

      {/* Speed — right-most */}
      <span
        className={cn(
          "flex items-center gap-1 font-medium transition-colors",
          isActive ? "text-[hsl(var(--status-downloading))]" : "text-muted-foreground/30",
        )}
      >
        <ArrowDown size={9.5} strokeWidth={2.2} />
        {formatBytesPerSecond(downloadSpeed, { idleLabel: "0 B/s", fixedFractionDigits: 1 })}
      </span>
    </div>
  );
}
