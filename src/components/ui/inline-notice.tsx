import type { ReactNode } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Info,
  TriangleAlert,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";

type NoticeTone = "error" | "warning" | "info" | "success";

const NOTICE_STYLES: Record<
  NoticeTone,
  { icon: typeof Info; container: string; iconClass: string }
> = {
  error: {
    icon: AlertTriangle,
    container:
      "border-[hsl(var(--status-error)/0.24)] bg-[hsl(var(--status-error)/0.08)] text-[hsl(var(--status-error))]",
    iconClass: "text-[hsl(var(--status-error))]",
  },
  warning: {
    icon: TriangleAlert,
    container:
      "border-[hsl(var(--status-paused)/0.24)] bg-[hsl(var(--status-paused)/0.08)] text-foreground/78",
    iconClass: "text-[hsl(var(--status-paused))]",
  },
  info: {
    icon: Info,
    container: "border-border/60 bg-black/10 text-foreground/74",
    iconClass: "text-muted-foreground/56",
  },
  success: {
    icon: CheckCircle2,
    container:
      "border-[hsl(var(--status-finished)/0.24)] bg-[hsl(var(--status-finished)/0.08)] text-[hsl(var(--status-finished))]",
    iconClass: "text-[hsl(var(--status-finished))]",
  },
};

interface InlineNoticeProps {
  tone: NoticeTone;
  message: ReactNode;
  title?: string;
  onDismiss?: () => void;
  actionLabel?: string;
  onAction?: () => void;
  className?: string;
}

export function InlineNotice({
  tone,
  message,
  title,
  onDismiss,
  actionLabel,
  onAction,
  className,
}: InlineNoticeProps) {
  const style = NOTICE_STYLES[tone];
  const Icon = style.icon;

  return (
    <div
      className={cn(
        "flex items-start gap-2 rounded-md border px-3 py-2 text-[11px] leading-relaxed",
        style.container,
        className,
      )}
    >
      <Icon size={12} strokeWidth={1.9} className={cn("mt-0.5 shrink-0", style.iconClass)} />
      <div className="min-w-0 flex-1">
        {title ? <div className="font-semibold">{title}</div> : null}
        <div>{message}</div>
        {actionLabel && onAction ? (
          <button
            type="button"
            onClick={onAction}
            className="mt-1.5 text-[10.5px] font-medium underline underline-offset-2 transition-opacity hover:opacity-100"
          >
            {actionLabel}
          </button>
        ) : null}
      </div>
      {onDismiss ? (
        <button
          type="button"
          onClick={onDismiss}
          className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-current/60 transition-colors hover:bg-black/10 hover:text-current"
          aria-label="Dismiss notice"
        >
          <X size={11} strokeWidth={2} />
        </button>
      ) : null}
    </div>
  );
}