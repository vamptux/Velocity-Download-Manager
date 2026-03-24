import { useMemo } from "react";
import { AlertTriangle, Gauge, Layers, TimerReset } from "lucide-react";
import { TransferSegmentStrip } from "@/components/TransferSegmentStrip";
import { formatBytes, formatBytesPerSecond, formatTimeRemaining } from "@/lib/format";
import { cn } from "@/lib/utils";
import type {
  Download,
  DownloadRuntimeSegmentSample,
  DownloadSegment,
} from "@/types/download";

const DETAIL_BYTE_FORMAT = { unknownLabel: "Unknown", integerAbove: 100 } as const;
const DETAIL_SPEED_FORMAT = { idleLabel: "Idle", integerAbove: 100 } as const;

type BlockState = "complete" | "active" | "pending";

function segmentLength(segment: DownloadSegment): number {
  return Math.max(segment.end - segment.start + 1, 0);
}

function computeBlockStates(
  size: number,
  segments: DownloadSegment[],
  totalBlocks: number,
): BlockState[] {
  const states: BlockState[] = new Array(totalBlocks).fill("pending") as BlockState[];
  const bytesPerBlock = size / totalBlocks;

  for (const segment of segments) {
    const completedUpTo =
      segment.status === "finished" ? segment.end + 1 : segment.start + segment.downloaded;

    if (completedUpTo > segment.start) {
      const firstBlock = Math.floor(segment.start / bytesPerBlock);
      const lastBlock = Math.min(totalBlocks - 1, Math.floor((completedUpTo - 1) / bytesPerBlock));
      for (let blockIndex = firstBlock; blockIndex <= lastBlock; blockIndex += 1) {
        states[blockIndex] = "complete";
      }
    }

    if (segment.status === "downloading") {
      const edgeBlock = Math.min(totalBlocks - 1, Math.floor(completedUpTo / bytesPerBlock));
      if (states[edgeBlock] !== "complete") {
        states[edgeBlock] = "active";
      }
    }
  }

  return states;
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2 rounded-lg border border-border/60 bg-black/10 p-2.5">
      <div className="text-[9.5px] font-semibold uppercase tracking-[0.14em] text-muted-foreground/42">
        {title}
      </div>
      {children}
    </section>
  );
}

function BlockProgressMap({ download }: { download: Download }) {
  const totalBlocks = 768;
  const hasSegments = download.segments.length > 0 && download.size > 0;
  const blockStates = useMemo(
    () => (hasSegments ? computeBlockStates(download.size, download.segments, totalBlocks) : null),
    [download.segments, download.size, hasSegments],
  );
  const finishedSegments = download.segments.filter((segment) => segment.status === "finished").length;
  const activeSegments = download.segments.filter((segment) => segment.status === "downloading").length;

  return (
    <div className="flex flex-col gap-2">
      {blockStates ? (
        <div
          className="w-full overflow-hidden rounded-sm"
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, 7px)",
            gap: "2px",
          }}
        >
          {blockStates.map((state, index) => (
            <div
              key={index}
              className={cn(
                "h-[7px] w-[7px] rounded-[2px]",
                state === "complete" && "bg-[hsl(var(--status-downloading))]",
                state === "active" && "bg-[hsl(var(--status-downloading)/0.4)]",
                state === "pending" && "bg-white/[0.06]",
              )}
            />
          ))}
        </div>
      ) : (
        <div className="rounded-md border border-border/55 bg-black/10 px-3 py-2 text-[11px] text-muted-foreground/58">
          Segmented runtime telemetry appears once the engine has planned ranges for this transfer.
        </div>
      )}

      <div className="flex items-center gap-3 text-[10px] text-muted-foreground/40">
        <span>
          <span className="font-medium text-foreground/55 tabular-nums">{finishedSegments}</span>
          <span className="ml-0.5 text-muted-foreground/32">/ {download.segments.length} done</span>
        </span>
        <span className="h-2.5 w-px bg-border/30" />
        <span>
          <span className="font-medium text-foreground/55 tabular-nums">{activeSegments}</span>
          <span className="ml-0.5 text-muted-foreground/32">active</span>
        </span>
        {download.runtimeCheckpoint.activeRaces.length > 0 ? (
          <>
            <span className="h-2.5 w-px bg-border/30" />
            <span className="text-[hsl(var(--status-paused)/0.8)]">
              {download.runtimeCheckpoint.activeRaces.length} race{download.runtimeCheckpoint.activeRaces.length === 1 ? "" : "s"}
            </span>
          </>
        ) : null}
      </div>
    </div>
  );
}

function formatRangeLabel(segment: DownloadSegment): string {
  return `${formatBytes(segment.start, DETAIL_BYTE_FORMAT)} - ${formatBytes(segment.end, DETAIL_BYTE_FORMAT)}`;
}

function formatProgressLabel(segment: DownloadSegment): string {
  const length = Math.max(segmentLength(segment), 1);
  const progress = Math.min(100, Math.max(0, (segment.downloaded / length) * 100));
  return `${formatBytes(segment.downloaded, DETAIL_BYTE_FORMAT)} of ${formatBytes(length, DETAIL_BYTE_FORMAT)} · ${Math.round(progress)}%`;
}

function segmentStatusTone(segment: DownloadSegment): string {
  switch (segment.status) {
    case "finished":
      return "text-[hsl(var(--status-finished))]";
    case "downloading":
      return "text-[hsl(var(--status-downloading))]";
    default:
      return "text-muted-foreground/54";
  }
}

function segmentStatusLabel(segment: DownloadSegment): string {
  switch (segment.status) {
    case "finished":
      return "Finished";
    case "downloading":
      return "Active";
    default:
      return "Planned";
  }
}

function describeSample(sample: DownloadRuntimeSegmentSample | null): string {
  if (!sample) {
    return "Waiting for runtime samples";
  }
  if (sample.terminalFailureReason) {
    return sample.terminalFailureReason;
  }
  if (sample.throughputBytesPerSecond != null && sample.etaSeconds != null) {
    return `${formatBytesPerSecond(sample.throughputBytesPerSecond, DETAIL_SPEED_FORMAT)} · ETA ${formatTimeRemaining(sample.etaSeconds, { emptyLabel: "Unknown" })}`;
  }
  if (sample.throughputBytesPerSecond != null) {
    return formatBytesPerSecond(sample.throughputBytesPerSecond, DETAIL_SPEED_FORMAT);
  }
  if (sample.etaSeconds != null) {
    return `ETA ${formatTimeRemaining(sample.etaSeconds, { emptyLabel: "Unknown" })}`;
  }
  return "Waiting for throughput sample";
}

export function SegmentRuntimePanel({ download }: { download: Download }) {
  const sampleBySegmentId = useMemo(
    () => new Map(download.runtimeCheckpoint.segmentSamples.map((sample) => [sample.segmentId, sample] as const)),
    [download.runtimeCheckpoint.segmentSamples],
  );
  const raceCompanionBySegmentId = useMemo(() => {
    const map = new Map<number, number>();
    for (const race of download.runtimeCheckpoint.activeRaces) {
      map.set(race.slowSegmentId, race.companionSegmentId);
      map.set(race.companionSegmentId, race.slowSegmentId);
    }
    return map;
  }, [download.runtimeCheckpoint.activeRaces]);

  const segmentRows = useMemo(
    () => download.segments.map((segment) => ({
      segment,
      sample: sampleBySegmentId.get(segment.id) ?? null,
      raceCompanionId: raceCompanionBySegmentId.get(segment.id) ?? null,
    })),
    [download.segments, raceCompanionBySegmentId, sampleBySegmentId],
  );

  return (
    <div className="flex flex-col gap-3 px-4 py-3">
      <BlockProgressMap download={download} />

      {download.segments.length > 0 ? (
        <Section title="Live Segments">
          <div className="flex items-center justify-between text-[9px] text-muted-foreground/30">
            <span className="font-semibold uppercase tracking-[0.14em]">
              {download.segments.length} Segment{download.segments.length === 1 ? "" : "s"}
            </span>
            <span className="tabular-nums">
              {download.runtimeCheckpoint.segmentSamples.length} sampled
            </span>
          </div>
          <TransferSegmentStrip
            segments={download.segments}
            compact={false}
            barClassName="h-3"
            className="gap-[2px]"
            samplesBySegmentId={sampleBySegmentId}
            raceCompanionBySegmentId={raceCompanionBySegmentId}
          />
          <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
            {segmentRows.map(({ segment, sample, raceCompanionId }) => (
              <div key={segment.id} className="rounded-md border border-border/55 bg-black/8 p-2.5">
                <div className="flex items-center justify-between gap-2">
                  <div className="text-[11px] font-semibold text-foreground/82">Segment {segment.id}</div>
                  <div className={cn("text-[10px] font-medium uppercase tracking-[0.08em]", segmentStatusTone(segment))}>
                    {segmentStatusLabel(segment)}
                  </div>
                </div>
                <div className="mt-1 text-[10.5px] text-muted-foreground/58">{formatRangeLabel(segment)}</div>
                <div className="mt-2 text-[11px] text-foreground/76">{formatProgressLabel(segment)}</div>
                <div className="mt-2 flex items-start gap-1.5 text-[10.5px] text-muted-foreground/62">
                  <Gauge size={11} strokeWidth={1.8} className="mt-0.5 shrink-0" />
                  <span>{describeSample(sample)}</span>
                </div>
                <div className="mt-2 flex flex-wrap gap-1.5 text-[10px] text-muted-foreground/54">
                  <span>Retries {sample?.retryAttempts ?? segment.retryAttempts}</span>
                  {raceCompanionId != null ? <span>Race with {raceCompanionId}</span> : null}
                  {sample?.remainingBytes != null ? (
                    <span>Remain {formatBytes(sample.remainingBytes, DETAIL_BYTE_FORMAT)}</span>
                  ) : null}
                </div>
                {sample?.terminalFailureReason ? (
                  <div className="mt-2 rounded-md border border-[hsl(var(--status-error)/0.2)] bg-[hsl(var(--status-error)/0.06)] px-2 py-1.5 text-[10.5px] text-[hsl(var(--status-error))]">
                    {sample.terminalFailureReason}
                  </div>
                ) : null}
              </div>
            ))}
          </div>
        </Section>
      ) : null}

      {download.runtimeCheckpoint.activeRaces.length > 0 ? (
        <Section title="Race Recovery">
          <div className="flex flex-col gap-1.5">
            {download.runtimeCheckpoint.activeRaces.map((race) => (
              <div key={`${race.slowSegmentId}-${race.companionSegmentId}`} className="flex items-start gap-2 rounded-md border border-[hsl(var(--status-paused)/0.18)] bg-[hsl(var(--status-paused)/0.08)] px-3 py-2 text-[11px] text-foreground/78">
                <Layers size={12} strokeWidth={1.8} className="mt-0.5 shrink-0 text-[hsl(var(--status-paused))]" />
                <span>
                  Segment {race.companionSegmentId} is shadowing slower segment {race.slowSegmentId}. If it wins, VDM will retire the slower range and keep the faster path.
                </span>
              </div>
            ))}
          </div>
        </Section>
      ) : null}

      {download.runtimeCheckpoint.segmentSamples.length === 0 && download.runtimeCheckpoint.activeRaces.length === 0 ? (
        <div className="flex items-start gap-2 rounded-lg border border-border/55 bg-black/10 px-3 py-2.5 text-[11px] text-muted-foreground/58">
          <TimerReset size={12} strokeWidth={1.8} className="mt-0.5 shrink-0" />
          <span>Runtime samples are populated once segments have enough live data to estimate speed and ETA.</span>
        </div>
      ) : null}

      {download.runtimeCheckpoint.segmentSamples.some((sample) => sample.terminalFailureReason) ? (
        <div className="flex items-start gap-2 rounded-lg border border-[hsl(var(--status-error)/0.2)] bg-[hsl(var(--status-error)/0.06)] px-3 py-2.5 text-[11px] text-foreground/78">
          <AlertTriangle size={12} strokeWidth={1.8} className="mt-0.5 shrink-0 text-[hsl(var(--status-error))]" />
          <span>One or more segments recorded a terminal failure reason. Review the affected segment cards for the exact cause.</span>
        </div>
      ) : null}
    </div>
  );
}