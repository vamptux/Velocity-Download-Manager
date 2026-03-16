import { useState, useRef, useEffect } from "react";
import {
  Zap,
  ChevronDown,
  Play,
  Pause,
  RotateCcw,
  PlayCircle,
  StopCircle,
  Trash2,
  Settings,
  Clipboard,
  Link,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

export interface ToolbarProps {
  onNewDownload: (prefillUrl?: string) => void;
  onOpenSettings: () => void;
  queueRunning: boolean;
  canPause: boolean;
  canResume: boolean;
  canRestart: boolean;
  canDelete: boolean;
  resumeTooltip: string;
  restartTooltip: string;
  onStartQueue: () => void;
  onStopQueue: () => void;
  onPause: () => void;
  onResume: () => void;
  onRestart: () => void;
  onDelete: () => void;
}

interface ToolbarButtonProps {
  icon: LucideIcon;
  label: string;
  tooltip: string;
  onClick?: () => void;
  disabled?: boolean;
  danger?: boolean;
  active?: boolean;
  variant?: "blue" | "green" | "amber";
}

function ToolbarButton({
  icon: Icon,
  label,
  tooltip,
  onClick,
  disabled = false,
  danger = false,
  active = false,
  variant = "blue",
}: ToolbarButtonProps) {
  const activeClass =
    variant === "green"
      ? "text-[hsl(var(--status-finished))] bg-[hsl(var(--status-finished)/0.09)] hover:bg-[hsl(var(--status-finished)/0.15)] border border-[hsl(var(--status-finished)/0.2)]"
      : variant === "amber"
      ? "text-[hsl(var(--status-paused))] bg-[hsl(var(--status-paused)/0.09)] hover:bg-[hsl(var(--status-paused)/0.16)] border border-[hsl(var(--status-paused)/0.2)]"
      : "text-[hsl(var(--status-downloading)/0.9)] bg-[hsl(var(--status-downloading)/0.08)] hover:bg-[hsl(var(--status-downloading)/0.14)] border border-[hsl(var(--status-downloading)/0.18)]";

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          onClick={onClick}
          disabled={disabled}
          className={cn(
            "flex flex-col items-center justify-center gap-[3px]",
            "min-w-[48px] h-[38px] px-2.5 rounded-md transition-all duration-200",
            disabled
              ? "text-muted-foreground/18 pointer-events-none border border-transparent"
              : danger
              ? "text-[hsl(var(--status-error)/0.55)] hover:text-[hsl(var(--status-error))] hover:bg-[hsl(var(--status-error)/0.1)]"
              : active
              ? activeClass
              : "text-muted-foreground/45 hover:bg-[hsl(0,0%,17%)] hover:text-foreground/85 border border-transparent hover:border-white/[0.06]",
          )}
        >
          <Icon size={15} strokeWidth={1.6} />
          <span className="text-[9.5px] leading-none tracking-wide">{label}</span>
        </button>
      </TooltipTrigger>
      <TooltipContent>{tooltip}</TooltipContent>
    </Tooltip>
  );
}

function Sep() {
  return <div className="mx-1 h-6 w-px shrink-0 bg-border/50" />;
}

const DROPDOWN_ITEMS = [
  { id: "paste", icon: Clipboard, label: "Paste URL from Clipboard", shortcut: "Ctrl+V" },
  { id: "manual", icon: Link, label: "Add URL Manually", shortcut: "Ctrl+N" },
] as const;

function NewDownloadSplitButton({ onNewDownload }: { onNewDownload: (prefillUrl?: string) => void }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function handler(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  async function pasteFromClipboard() {
    setOpen(false);
    try {
      const text = await navigator.clipboard.readText();
      onNewDownload(text.trim() || undefined);
    } catch {
      onNewDownload();
    }
  }

  return (
    <div ref={ref} className="relative flex items-center mr-2.5">
      {/* Main button — copper left fading to grey right */}
      <button
        onClick={() => onNewDownload()}
        style={{ background: "linear-gradient(90deg, hsl(20,60%,46%) 0%, hsl(12,42%,31%) 55%, hsl(0,0%,20%) 100%)" }}
        className="flex items-center gap-[5px] px-3 h-[26px] rounded-l text-[11.5px] font-semibold tracking-tight text-[hsl(24,10%,95%)] hover:brightness-110 transition-all"
      >
        <Zap size={11} strokeWidth={2.5} fill="currentColor" />
        New Download
      </button>

      {/* Chevron — grey, matching the tail of the main button */}
      <button
        onClick={() => setOpen((v) => !v)}
        style={{ background: "hsl(0,0%,20%)" }}
        className={cn(
          "flex items-center justify-center w-[18px] h-[26px] rounded-r border-l border-black/30 text-[hsl(24,10%,68%)] hover:brightness-125 transition-all",
          open && "brightness-125",
        )}
      >
        <ChevronDown size={11} strokeWidth={2.3} className={cn("transition-transform duration-150", open && "rotate-180")} />
      </button>

      {/* Dropdown */}
      {open && (
        <div
          className="absolute left-0 top-[calc(100%+4px)] z-50 min-w-[220px] rounded-md border border-border bg-card shadow-xl py-1 animate-in fade-in-0 zoom-in-95 duration-100"
          style={{ boxShadow: "0 8px 24px hsl(0,0%,0%,0.55)" }}
        >
          {DROPDOWN_ITEMS.map(({ id, icon: Icon, label, shortcut }) => (
            <button
              key={label}
              onClick={id === "paste" ? pasteFromClipboard : () => { setOpen(false); onNewDownload(); }}
              className="flex w-full items-center gap-2.5 px-3 py-[7px] text-[12px] text-foreground/80 hover:bg-accent hover:text-foreground transition-colors"
            >
              <Icon size={13} strokeWidth={1.6} className="shrink-0 text-muted-foreground" />
              <span className="flex-1 text-left">{label}</span>
              {shortcut && (
                <kbd className="text-[10px] text-muted-foreground/50 font-mono">{shortcut}</kbd>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function Toolbar({
  onNewDownload,
  onOpenSettings,
  queueRunning,
  canPause,
  canResume,
  canRestart,
  canDelete,
  resumeTooltip,
  restartTooltip,
  onStartQueue,
  onStopQueue,
  onPause,
  onResume,
  onRestart,
  onDelete,
}: ToolbarProps) {
  return (
    <div
      className="flex items-center border-b border-border/50 px-2.5 h-[46px] shrink-0"
      style={{
        background: "hsl(var(--toolbar))",
        boxShadow: "inset 0 1px 0 hsl(0,0%,100%,0.04)",
      }}
    >
      <NewDownloadSplitButton onNewDownload={onNewDownload} />

      <Sep />

      <ToolbarButton icon={Play} label="Resume" variant="green" disabled={!canResume} active={canResume} onClick={onResume} tooltip={resumeTooltip} />
      <ToolbarButton icon={Pause} label="Pause" variant="amber" disabled={!canPause} active={canPause} onClick={onPause} tooltip="Pause selected (Space)" />
      <ToolbarButton icon={RotateCcw} label="Restart" variant="blue" disabled={!canRestart} active={canRestart} onClick={onRestart} tooltip={restartTooltip} />

      <Sep />

      <ToolbarButton icon={PlayCircle} label="Run Queue" variant="green" disabled={queueRunning} active={!queueRunning} onClick={onStartQueue} tooltip="Resume scheduling for the default queue" />
      <ToolbarButton icon={StopCircle} label="Pause Queue" variant="amber" disabled={!queueRunning} active={queueRunning} onClick={onStopQueue} tooltip="Pause scheduling for the default queue" />

      <Sep />

      <ToolbarButton icon={Trash2} label="Delete" disabled={!canDelete} danger onClick={onDelete} tooltip="Delete selected (Del)" />

      <div className="flex-1" />

      <ToolbarButton icon={Settings} label="Settings" tooltip="Open settings" onClick={onOpenSettings} />
    </div>
  );
}
