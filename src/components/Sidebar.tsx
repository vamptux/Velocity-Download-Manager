import { useRef, useState } from "react";
import {
  LayoutGrid,
  Archive,
  Package,
  Film,
  Music,
  Image,
  FileText,
  CheckCircle2,
  Clock,
  ChevronDown,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { formatBytesPerSecond } from "@/lib/format";
import type { SidebarCategory, Download } from "@/types/download";

const SIDEBAR_DEFAULT_W = 182;
const SIDEBAR_MIN_W = 135;
const SIDEBAR_MAX_W = 268;
const SIDEBAR_SNAP_RANGE = 16;

interface SidebarItem {
  id: SidebarCategory;
  label: string;
  icon: LucideIcon;
}

const TYPE_CATEGORIES: SidebarItem[] = [
  { id: "all", label: "All", icon: LayoutGrid },
  { id: "compressed", label: "Compressed", icon: Archive },
  { id: "programs", label: "Programs", icon: Package },
  { id: "videos", label: "Videos", icon: Film },
  { id: "music", label: "Music", icon: Music },
  { id: "pictures", label: "Pictures", icon: Image },
  { id: "documents", label: "Documents", icon: FileText },
];

const STATUS_CATEGORIES: SidebarItem[] = [
  { id: "finished", label: "Finished", icon: CheckCircle2 },
  { id: "unfinished", label: "Unfinished", icon: Clock },
];

interface SidebarProps {
  activeCategory: SidebarCategory;
  onCategoryChange: (category: SidebarCategory) => void;
  downloads: Download[];
  activeCount: number;
  queuedCount: number;
  totalSpeed: number;
}

function countForCategory(cat: SidebarCategory, downloads: Download[]): number {
  if (cat === "all") return downloads.length;
  if (cat === "finished") return downloads.filter((d) => d.status === "finished").length;
  if (cat === "unfinished") return downloads.filter((d) => d.status !== "finished").length;
  return downloads.filter((d) => d.category === cat).length;
}

function SectionHeader({
  label,
  open,
  onToggle,
}: {
  label: string;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      onClick={onToggle}
      className="group flex w-full items-center gap-1.5 px-2.5 pt-3 pb-1 text-left transition-colors"
    >
      <ChevronDown
        size={9}
        strokeWidth={2.4}
        className={cn(
          "shrink-0 text-muted-foreground/44 transition-transform duration-150",
          !open && "-rotate-90",
        )}
      />
      <span className="text-[9px] font-semibold uppercase tracking-[0.13em] text-muted-foreground/52 group-hover:text-muted-foreground/70 transition-colors">
        {label}
      </span>
    </button>
  );
}

function NavItem({
  item,
  active,
  count,
  onClick,
}: {
  item: SidebarItem;
  active: boolean;
  count: number;
  onClick: () => void;
}) {
  const Icon = item.icon;
  return (
    <button
      onClick={onClick}
      className={cn(
        "relative flex w-full items-center gap-2.5 rounded-md py-[5px] pl-3 pr-2.5 text-[12.5px] transition-colors",
        active
          ? "bg-[hsl(var(--sidebar-active))] text-[hsl(var(--sidebar-active-foreground))] font-medium"
          : "text-[hsl(var(--sidebar-foreground))] hover:bg-[hsl(var(--sidebar-active)/0.65)] hover:text-[hsl(var(--sidebar-active-foreground))]",
      )}
    >
      {active && (
        <span className="absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-full bg-primary" />
      )}
      <Icon size={14} className={cn("shrink-0", active ? "text-primary" : "opacity-60")} />
      <span className="flex-1 truncate text-left">{item.label}</span>
      {count > 0 && (
        <span
          className={cn(
            "shrink-0 min-w-[18px] h-[15px] flex items-center justify-center rounded-full px-1 text-[9px] font-semibold tabular-nums",
            active
              ? "bg-primary/22 text-primary"
              : "bg-[hsl(var(--sidebar-active))] text-[hsl(var(--sidebar-foreground)/0.7)]",
          )}
        >
          {count}
        </span>
      )}
    </button>
  );
}

export function Sidebar({ activeCategory, onCategoryChange, downloads, activeCount, queuedCount, totalSpeed }: SidebarProps) {
  const [catOpen, setCatOpen] = useState(true);
  const [statusOpen, setStatusOpen] = useState(true);
  const [sidebarWidth, setSidebarWidth] = useState(SIDEBAR_DEFAULT_W);
  const isDragging = useRef(false);

  function startResize(e: React.MouseEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = sidebarWidth;
    isDragging.current = true;

    function onMove(ev: MouseEvent) {
      const delta = ev.clientX - startX;
      const raw = startW + delta;
      setSidebarWidth(Math.max(SIDEBAR_MIN_W, Math.min(SIDEBAR_MAX_W, raw)));
    }

    function onUp() {
      isDragging.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      setSidebarWidth((w) =>
        Math.abs(w - SIDEBAR_DEFAULT_W) <= SIDEBAR_SNAP_RANGE ? SIDEBAR_DEFAULT_W : w,
      );
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    }

    document.body.style.cursor = "ew-resize";
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }

  return (
    <aside
      className="relative flex shrink-0 flex-col overflow-hidden border-r border-border/50"
      style={{ width: sidebarWidth, background: "hsl(var(--sidebar))" }}
    >
      <nav className="flex flex-1 flex-col overflow-y-auto px-1.5 pb-3">
        <SectionHeader label="Categories" open={catOpen} onToggle={() => setCatOpen((v) => !v)} />
        {catOpen && (
          <div className="flex flex-col gap-px">
            {TYPE_CATEGORIES.map((item) => (
              <NavItem
                key={item.id}
                item={item}
                active={activeCategory === item.id}
                count={countForCategory(item.id, downloads)}
                onClick={() => onCategoryChange(item.id)}
              />
            ))}
          </div>
        )}

        <SectionHeader label="Status" open={statusOpen} onToggle={() => setStatusOpen((v) => !v)} />
        {statusOpen && (
          <div className="flex flex-col gap-px">
            {STATUS_CATEGORIES.map((item) => (
              <NavItem
                key={item.id}
                item={item}
                active={activeCategory === item.id}
                count={countForCategory(item.id, downloads)}
                onClick={() => onCategoryChange(item.id)}
              />
            ))}
          </div>
        )}
      </nav>

      <div className="border-t border-border/45 px-2.5 py-2.5">
        <div className="rounded-lg border border-border/50 bg-black/10 px-2.5 py-2">
          <div className="text-[9px] font-semibold uppercase tracking-[0.13em] text-muted-foreground/46">
            Queue Snapshot
          </div>
          <div className="mt-1.5 flex items-baseline justify-between gap-2">
            <div>
              <div className="text-[15px] font-semibold text-foreground/84">{activeCount}</div>
              <div className="text-[10px] text-muted-foreground/52">active</div>
            </div>
            <div className="text-right">
              <div className="text-[11px] font-medium text-foreground/76">
                {formatBytesPerSecond(totalSpeed, { idleLabel: "0 B/s", fixedFractionDigits: 1 })}
              </div>
              <div className="text-[10px] text-muted-foreground/52">{queuedCount} queued</div>
            </div>
          </div>
        </div>
      </div>

      {/* Right-edge resize handle – drag to resize sidebar width, snaps to default */}
      <div
        role="separator"
        aria-label="Drag to resize sidebar"
        onMouseDown={startResize}
        className="absolute right-0 top-0 bottom-0 w-[5px] cursor-ew-resize z-10 group transition-colors hover:bg-primary/14 active:bg-primary/24"
        title="Drag to resize"
      />
    </aside>
  );
}
