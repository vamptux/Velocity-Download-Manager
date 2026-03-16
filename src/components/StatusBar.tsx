import { ListOrdered, ArrowDown, Activity, Network } from "lucide-react";
import { formatBytesPerSecond } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { TrafficMode } from "@/types/download";

interface StatusBarProps {
  queuedCount: number;
  queueRunning: boolean;
  activeCount: number;
  activeLimit: number;
  connectionCount: number;
  /** Active transfer speed in bytes/sec */
  downloadSpeed: number;
  trafficMode: TrafficMode;
  engineBootstrapReady: boolean;
  engineBootstrapError: string | null;
}

function trafficModeLabel(mode: TrafficMode): string {
  switch (mode) {
    case "low":
      return "Low";
    case "medium":
      return "Medium";
    case "high":
      return "High";
    case "max":
      return "Max";
  }
}

export function StatusBar({
  queuedCount,
  queueRunning,
  activeCount,
  activeLimit,
  connectionCount,
  downloadSpeed,
  trafficMode,
  engineBootstrapReady,
  engineBootstrapError,
}: StatusBarProps) {
  const isActive = activeCount > 0;
  const manualTransferActive = !queueRunning && isActive;
  return (
    <div
      className="flex h-[20px] shrink-0 items-center gap-3.5 border-t border-border/50 px-3"
      style={{ background: "hsl(var(--sidebar))" }}
    >
      <span
        className={cn(
          "flex items-center gap-1.5 text-[10.5px] tabular-nums transition-colors",
          isActive ? "text-[hsl(var(--status-downloading))]" : "text-muted-foreground/50",
        )}
      >
        <Activity size={10} strokeWidth={2} />
        <span>{activeCount} active</span>
      </span>
      <div className="h-2.5 w-px bg-border/50" />
      <span className="flex items-center gap-1.5 text-[10.5px] text-muted-foreground/50 tabular-nums">
        <ListOrdered size={10} strokeWidth={2} />
        <span>{queuedCount} queued</span>
      </span>
      <div className="h-2.5 w-px bg-border/50" />
      <span
        className={cn(
          "flex items-center gap-1.5 text-[10.5px] tabular-nums transition-colors",
          queueRunning
            ? "text-muted-foreground/58"
            : manualTransferActive
              ? "text-foreground/68"
              : "text-[hsl(var(--status-paused))]",
        )}
      >
        <span>{queueRunning ? "Queue running" : manualTransferActive ? "Queue paused • manual transfer active" : "Queue paused"}</span>
      </span>
      <div className="h-2.5 w-px bg-border/50" />
      <span
        className={cn(
          "flex items-center gap-1.5 text-[10.5px] tabular-nums transition-colors",
          isActive ? "text-foreground/65" : "text-muted-foreground/40",
        )}
      >
        <Network size={10} strokeWidth={2} />
        <span>{connectionCount} connections</span>
      </span>
      <div className="h-2.5 w-px bg-border/50" />
      <span className="flex items-center gap-1.5 text-[10.5px] text-muted-foreground/50 tabular-nums">
        <span>{activeCount}/{activeLimit} slots</span>
      </span>
      <div className="h-2.5 w-px bg-border/50" />
      <span
        className={cn(
          "flex items-center gap-1.5 text-[10.5px] tabular-nums transition-colors",
          engineBootstrapError
            ? "text-[hsl(var(--status-error))]"
            : engineBootstrapReady
              ? "text-muted-foreground/50"
              : "text-[hsl(var(--status-downloading))]",
        )}
      >
        <span>
          {engineBootstrapError
            ? "Engine degraded"
            : engineBootstrapReady
              ? "Engine ready"
              : "Engine starting"}
        </span>
      </span>
      <div className="h-2.5 w-px bg-border/50" />
      <span className="flex items-center gap-1.5 text-[10.5px] text-muted-foreground/50 tabular-nums">
        <span>{trafficModeLabel(trafficMode)} mode</span>
      </span>
      <div className="flex-1" />
      <span
        className={cn(
          "flex items-center gap-1.5 text-[10.5px] font-medium tabular-nums transition-colors",
          isActive ? "text-[hsl(var(--status-downloading))]" : "text-muted-foreground/35",
        )}
      >
        <ArrowDown size={10} strokeWidth={2} />
        <span>{formatBytesPerSecond(downloadSpeed, { idleLabel: "0 B/s", fixedFractionDigits: 1 })}</span>
      </span>
    </div>
  );
}
