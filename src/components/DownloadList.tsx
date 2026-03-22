import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { Download, ChevronUp, ChevronDown, ChevronsUpDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { Checkbox } from "@/components/ui/checkbox";
import { getQueueMoveState } from "@/lib/downloadQueue";
import type { SidebarCategory, Download as DownloadItem } from "@/types/download";
import { DownloadRow } from "@/components/DownloadRow";

const ROW_HEIGHT_PX = 39;
const OVERSCAN_ROWS = 10;

type SortDir = "asc" | "desc" | "none";
type SortCol = "name" | "size" | "status" | "dateAdded";

interface Column {
  id: SortCol | "speed" | "timeLeft";
  label: string;
  width?: string;
  flex?: boolean;
  sortable?: boolean;
}

const COLUMNS: Column[] = [
  { id: "name", label: "Name", flex: true, sortable: true },
  { id: "size", label: "Size", width: "72px", sortable: true },
  { id: "status", label: "Status", width: "88px", sortable: true },
  { id: "speed", label: "Speed", width: "80px" },
  { id: "timeLeft", label: "ETA", width: "68px" },
  { id: "dateAdded", label: "Added", width: "90px", sortable: true },
];

function SortIcon({ col, sort }: { col: Column; sort: { col: SortCol; dir: SortDir } }) {
  if (!col.sortable) return null;
  const active = sort.col === col.id;
  if (!active) return <ChevronsUpDown size={11} className="opacity-0 group-hover:opacity-30 transition-opacity" />;
  return sort.dir === "asc" ? (
    <ChevronUp size={11} className="text-primary" />
  ) : (
    <ChevronDown size={11} className="text-primary" />
  );
}

interface DownloadListProps {
  downloads: DownloadItem[];
  activeCategory: SidebarCategory;
  searchQuery?: string;
  selectedIds: Set<string>;
  onSelectedChange: (ids: Set<string>) => void;
  onDelete: (id: string) => Promise<void>;
  onReorder: (id: string, direction: "up" | "down") => Promise<void> | void;
  onOpenFolder: (id: string) => Promise<void> | void;
  onRefresh: () => void;
}

export function DownloadList({
  downloads,
  activeCategory,
  searchQuery,
  selectedIds,
  onSelectedChange,
  onDelete,
  onReorder,
  onOpenFolder,
  onRefresh,
}: DownloadListProps) {
  const [sort, setSort] = useState<{ col: SortCol; dir: SortDir }>({ col: "dateAdded", dir: "desc" });
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const selectionAnchorId = useRef<string | null>(null);
  const deferredSearchQuery = useDeferredValue(searchQuery);
  const normalizedSearchQuery = useMemo(
    () => deferredSearchQuery?.trim().toLowerCase() ?? "",
    [deferredSearchQuery],
  );

  const filtered = useMemo(() => {
    return downloads.filter((download) => {
      if (normalizedSearchQuery) {
        if (
          !download.name.toLowerCase().includes(normalizedSearchQuery)
          && !download.url.toLowerCase().includes(normalizedSearchQuery)
        ) {
          return false;
        }
      }
      if (activeCategory === "all") return true;
      if (activeCategory === "finished") return download.status === "finished";
      if (activeCategory === "unfinished") return download.status !== "finished";
      return download.category === activeCategory;
    });
  }, [activeCategory, downloads, normalizedSearchQuery]);

  const sorted = useMemo(() => {
    return [...filtered].sort((left, right) => {
      if (sort.dir === "none") return 0;
      const direction = sort.dir === "asc" ? 1 : -1;
      switch (sort.col) {
        case "name":
          return direction * left.name.localeCompare(right.name);
        case "size":
          return direction * (left.size - right.size);
        case "status":
          return direction * left.status.localeCompare(right.status);
        case "dateAdded":
          return direction * (left.dateAdded.getTime() - right.dateAdded.getTime());
        default:
          return 0;
      }
    });
  }, [filtered, sort]);

  const queueMoveState = useMemo(() => getQueueMoveState(downloads), [downloads]);

  useEffect(() => {
    const node = scrollContainerRef.current;
    if (!node) {
      return;
    }

    const updateViewportHeight = () => {
      setViewportHeight(node.clientHeight);
      setScrollTop(node.scrollTop);
    };

    updateViewportHeight();

    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", updateViewportHeight);
      return () => window.removeEventListener("resize", updateViewportHeight);
    }

    const observer = new ResizeObserver(() => updateViewportHeight());
    observer.observe(node);
    return () => observer.disconnect();
  }, [sorted.length]);

  const visibleWindow = useMemo(() => {
    const fallbackHeight = ROW_HEIGHT_PX * 12;
    const effectiveViewportHeight = Math.max(viewportHeight, fallbackHeight);
    const visibleRowCount = Math.max(1, Math.ceil(effectiveViewportHeight / ROW_HEIGHT_PX));
    const maxStartIndex = Math.max(0, sorted.length - visibleRowCount);
    const startIndex = Math.min(
      Math.max(0, Math.floor(scrollTop / ROW_HEIGHT_PX) - OVERSCAN_ROWS),
      maxStartIndex,
    );
    const endIndex = Math.min(
      sorted.length,
      startIndex + visibleRowCount + OVERSCAN_ROWS * 2,
    );

    return {
      startIndex,
      endIndex,
      items: sorted.slice(startIndex, endIndex),
      topSpacerHeight: startIndex * ROW_HEIGHT_PX,
      bottomSpacerHeight: Math.max(0, (sorted.length - endIndex) * ROW_HEIGHT_PX),
    };
  }, [scrollTop, sorted, viewportHeight]);

  const someChecked = sorted.some((d) => selectedIds.has(d.id));
  const allChecked = sorted.length > 0 && sorted.every((d) => selectedIds.has(d.id));
  const indeterminate = someChecked && !allChecked;

  const handleSort = useCallback((col: Column) => {
    if (!col.sortable) return;
    const colId = col.id as SortCol;
    setSort((prev) => {
      if (prev.col !== colId) return { col: colId, dir: "asc" };
      if (prev.dir === "asc") return { col: colId, dir: "desc" };
      return { col: colId, dir: "none" };
    });
  }, []);

  const handleSelectAll = useCallback((checked: boolean) => {
    onSelectedChange(checked ? new Set(sorted.map((d) => d.id)) : new Set());
  }, [onSelectedChange, sorted]);

  const handleSelect = useCallback((id: string, checked: boolean) => {
    const next = new Set(selectedIds);
    if (checked) next.add(id); else next.delete(id);
    selectionAnchorId.current = id;
    onSelectedChange(next);
  }, [onSelectedChange, selectedIds]);

  const handleActivate = useCallback((id: string, options?: { toggle?: boolean; range?: boolean }) => {
    const orderedIds = sorted.map((download) => download.id);
    const anchorId = selectionAnchorId.current;

    if (options?.range && anchorId && orderedIds.includes(anchorId)) {
      const startIndex = orderedIds.indexOf(anchorId);
      const endIndex = orderedIds.indexOf(id);
      if (endIndex >= 0) {
        const [from, to] = startIndex <= endIndex ? [startIndex, endIndex] : [endIndex, startIndex];
        onSelectedChange(new Set(orderedIds.slice(from, to + 1)));
        return;
      }
    }

    if (options?.toggle) {
      const next = new Set(selectedIds);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      selectionAnchorId.current = id;
      onSelectedChange(next);
      return;
    }

    selectionAnchorId.current = id;
    onSelectedChange(new Set([id]));
  }, [onSelectedChange, selectedIds, sorted]);

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div
        className="flex items-center shrink-0 border-b border-border/50 text-[9px] font-semibold text-muted-foreground/40 uppercase tracking-[0.1em]"
        style={{ background: "hsl(var(--toolbar))", boxShadow: "inset 0 1px 0 hsl(0,0%,100%,0.04)" }}
      >
        <div className="flex w-8 shrink-0 items-center justify-center px-2 py-[7px]">
          <Checkbox
            checked={allChecked}
            indeterminate={indeterminate}
            onChange={(checked) => handleSelectAll(checked)}
            disabled={sorted.length === 0}
          />
        </div>

        {COLUMNS.map((col) => (
          <button
            key={col.id}
            onClick={() => handleSort(col)}
            style={col.flex ? undefined : { width: col.width, flexShrink: 0 }}
            className={cn(
              "group flex items-center gap-1 px-2 py-[7px] text-left transition-colors",
              col.flex && "flex-1",
              col.id === "name" && "pl-7",
              col.sortable && "hover:text-foreground/60 cursor-pointer",
              !col.sortable && "cursor-default",
              sort.col === col.id && "text-foreground/55",
            )}
          >
            <span>{col.label}</span>
            <SortIcon col={col} sort={sort} />
          </button>
        ))}

        <div className="w-1 shrink-0" />
      </div>

      <div
        ref={scrollContainerRef}
        className="flex flex-1 flex-col overflow-y-auto"
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      >
        {sorted.length === 0 ? (
          <EmptyState activeCategory={activeCategory} hasDownloads={downloads.length > 0} searchQuery={normalizedSearchQuery} />
        ) : (
          <>
            {visibleWindow.topSpacerHeight > 0 ? (
              <div aria-hidden style={{ height: visibleWindow.topSpacerHeight }} />
            ) : null}

            {visibleWindow.items.map((download, index) => {
              const moveState = queueMoveState.get(download.id);
              return (
                <DownloadRow
                  key={download.id}
                  index={visibleWindow.startIndex + index}
                  download={download}
                  selected={selectedIds.has(download.id)}
                  canMoveUp={moveState?.canMoveUp ?? false}
                  canMoveDown={moveState?.canMoveDown ?? false}
                  onSelect={handleSelect}
                  onActivate={handleActivate}
                  onDelete={onDelete}
                  onReorder={onReorder}
                  onOpenFolder={onOpenFolder}
                  onRefresh={onRefresh}
                />
              );
            })}

            {visibleWindow.bottomSpacerHeight > 0 ? (
              <div aria-hidden style={{ height: visibleWindow.bottomSpacerHeight }} />
            ) : null}
          </>
        )}
      </div>
    </div>
  );
}

function EmptyState({
  activeCategory,
  hasDownloads,
  searchQuery,
}: {
  activeCategory: SidebarCategory;
  hasDownloads: boolean;
  searchQuery: string;
}) {
  const isFiltered = searchQuery.length > 0 || activeCategory !== "all";
  const title = !hasDownloads
    ? "Nothing downloading yet"
    : searchQuery.length > 0
      ? "No downloads match this search"
      : activeCategory === "finished"
        ? "No finished downloads yet"
        : activeCategory === "unfinished"
          ? "Everything is finished"
          : `No ${activeCategory} downloads right now`;
  const message = !hasDownloads
    ? "Start with Ctrl+N or New Download to add your first transfer."
    : isFiltered
      ? "Try a broader search or switch back to All to see more transfers."
      : "New transfers in this section will appear here automatically.";

  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-4 py-16">
      <div
        className="flex h-[52px] w-[52px] items-center justify-center rounded-xl"
        style={{ background: "linear-gradient(135deg, hsl(var(--accent-h) 12% 16%), hsl(var(--background)))", border: "1px solid hsl(var(--border))" }}
      >
        <Download size={22} strokeWidth={1.4} className="text-muted-foreground/40" />
      </div>
      <div className="flex flex-col items-center gap-1.5">
        <p className="text-[13px] font-semibold text-foreground/65 tracking-tight">{title}</p>
        <p className="text-[11.5px] text-muted-foreground/42 text-center leading-relaxed">
          {!hasDownloads ? (
            <>
              Start with
              <kbd className="mx-0.5 rounded bg-muted px-1 py-px text-[10px] font-mono text-muted-foreground/65">Ctrl+N</kbd>
              or click <span className="font-medium text-[hsl(var(--primary)/0.7)]">New Download</span> to add your first transfer.
            </>
          ) : (
            message
          )}
        </p>
      </div>
    </div>
  );
}
