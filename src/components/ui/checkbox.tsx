import { Check, Minus } from "lucide-react";
import { cn } from "@/lib/utils";

interface CheckboxProps {
  checked: boolean;
  indeterminate?: boolean;
  onChange?: (checked: boolean) => void;
  onClick?: (e: React.MouseEvent) => void;
  className?: string;
  disabled?: boolean;
}

export function Checkbox({
  checked,
  indeterminate = false,
  onChange,
  onClick,
  className,
  disabled = false,
}: CheckboxProps) {
  const active = checked || indeterminate;

  return (
    <div
      role="checkbox"
      aria-checked={indeterminate ? "mixed" : checked}
      aria-disabled={disabled}
      tabIndex={disabled ? -1 : 0}
      onClick={(e) => {
        if (disabled) return;
        onClick?.(e);
        onChange?.(!checked);
      }}
      onKeyDown={(e) => {
        if (disabled) return;
        if (e.key === " ") { e.preventDefault(); onChange?.(!checked); }
      }}
      className={cn(
        "h-[14px] w-[14px] shrink-0 rounded-[3px] border transition-all duration-100 cursor-pointer",
        "flex items-center justify-center select-none outline-none",
        "focus-visible:ring-1 focus-visible:ring-primary/60 focus-visible:ring-offset-0",
        active
          ? "bg-primary border-primary shadow-[0_0_0_1px_hsl(var(--primary)/0.3)]"
          : "bg-transparent border-white/[0.18] hover:border-primary/55",
        disabled && "opacity-30 pointer-events-none",
        className,
      )}
    >
      {indeterminate ? (
        <Minus size={8} strokeWidth={3} className="text-primary-foreground" />
      ) : checked ? (
        <Check size={8} strokeWidth={3} className="text-primary-foreground" />
      ) : null}
    </div>
  );
}
