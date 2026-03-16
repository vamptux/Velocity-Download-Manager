import { memo, useMemo } from "react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { formatBytes } from "@/lib/format";
import { cn } from "@/lib/utils";
import { useSmoothedNumber } from "@/lib/downloadProgress";
import type { DownloadSegment } from "@/types/download";

const SEGMENT_BYTE_FORMAT = { zeroLabel: "0 B", integerAbove: 100 } as const;

function segmentLength(segment: DownloadSegment): number {
  return Math.max(segment.end - segment.start + 1, 0);
}

function segmentStatusLabel(segment: { status: DownloadSegment["status"] }): string {
  switch (segment.status) {
    case "finished":
      return "Finished";
    case "downloading":
      return "Receiving data";
    case "pending":
      return "Planned";
  }
}

interface SegmentViewModel {
  id: number;
  start: number;
  end: number;
  downloaded: number;
  status: DownloadSegment["status"];
  length: number;
  progress: number;
}

const SegmentCell = memo(function SegmentCell({
  segment,
  compact,
  barClassName,
}: {
  segment: SegmentViewModel;
  compact: boolean;
  barClassName?: string;
}) {
  const smoothedProgress = useSmoothedNumber(segment.progress, {
    durationMs: segment.status === "downloading" ? 620 : 240,
  });

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          className={cn(
            "relative min-w-[3px] flex-1 overflow-hidden rounded-[2px] bg-white/[0.05]",
            barClassName ?? (compact ? "h-1.5" : "h-2.5"),
          )}
          style={{ flexGrow: segment.length, flexBasis: 0 }}
        >
          <div
            className={cn(
              "absolute inset-y-0 left-0 transition-[width] duration-300",
              segment.status === "finished" && "bg-[hsl(var(--status-finished)/0.85)]",
              segment.status === "downloading" && "bg-[linear-gradient(90deg,hsl(var(--status-downloading)/0.9),hsl(198,80%,62%))]",
              segment.status === "pending" && "bg-white/[0.08]",
            )}
            style={{ width: `${smoothedProgress}%` }}
          />
          {segment.status === "downloading" ? (
            <div className="animate-segment-shimmer absolute top-0 bottom-0 w-[40%] bg-gradient-to-r from-transparent via-white/10 to-transparent" />
          ) : null}
        </div>
      </TooltipTrigger>
      <TooltipContent side="top" align="center" className="max-w-[220px]">
        <div className="space-y-1 text-[11px]">
          <div className="font-semibold text-foreground/88">Segment {segment.id}</div>
          <div className="text-muted-foreground/70">{segmentStatusLabel(segment)}</div>
          <div className="text-muted-foreground/70">
            {formatBytes(segment.downloaded, SEGMENT_BYTE_FORMAT)} of {formatBytes(segment.length, SEGMENT_BYTE_FORMAT)}
          </div>
          <div className="text-muted-foreground/60">
            Range {formatBytes(segment.start, SEGMENT_BYTE_FORMAT)} - {formatBytes(segment.end, SEGMENT_BYTE_FORMAT)}
          </div>
        </div>
      </TooltipContent>
    </Tooltip>
  );
});

export function TransferSegmentStrip({
  segments,
  compact = false,
  barClassName,
  className,
}: {
  segments: DownloadSegment[];
  compact?: boolean;
  barClassName?: string;
  className?: string;
}) {
  const prepared = useMemo<SegmentViewModel[]>(
    () =>
      segments.map((segment) => {
        const length = Math.max(segmentLength(segment), 1);
        return {
          id: segment.id,
          start: segment.start,
          end: segment.end,
          downloaded: segment.downloaded,
          status: segment.status,
          length,
          progress: Math.min(100, Math.max(0, (segment.downloaded / length) * 100)),
        };
      }),
    [segments],
  );

  if (prepared.length === 0) {
    return null;
  }

  return (
    <div className={cn("flex items-center", compact ? "gap-px" : "gap-0.5", className)}>
      {prepared.map((segment) => (
        <SegmentCell key={segment.id} segment={segment} compact={compact} barClassName={barClassName} />
      ))}
    </div>
  );
}
