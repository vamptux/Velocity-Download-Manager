import type { Download } from "@/types/download";

function daysAgo(n: number): Date {
  const d = new Date();
  d.setDate(d.getDate() - n);
  return d;
}

type MockSegment = Omit<Download["segments"][number], "retryAttempts" | "retryBudget"> & {
  retryAttempts?: number;
  retryBudget?: number;
};

type MockDiagnostics = Partial<Download["diagnostics"]> &
  Pick<Download["diagnostics"], "warnings" | "notes" | "failureKind" | "restartRequired">;

function buildDownload(partial: Omit<Download, "capabilities" | "validators" | "integrity" | "diagnostics" | "queue" | "queuePosition" | "errorMessage" | "contentType" | "finalUrl" | "host" | "targetPath" | "tempPath" | "maxConnections" | "customMaxConnections" | "hostMaxConnections" | "hostCooldownUntil" | "hostAverageTtfbMs" | "hostAverageThroughputBytesPerSecond" | "hostProtocol" | "hostDiagnostics" | "trafficMode" | "speedLimitBytesPerSecond" | "openFolderOnCompletion" | "compatibility" | "segments" | "targetConnections" | "writerBackpressure" | "engineLog" | "manualStartRequested" | "runtimeCheckpoint"> & {
  finalUrl?: string;
  host?: string;
  targetPath?: string;
  tempPath?: string;
  maxConnections?: number;
  customMaxConnections?: number | null;
  hostMaxConnections?: number | null;
  hostCooldownUntil?: number | null;
  hostAverageTtfbMs?: number | null;
  hostAverageThroughputBytesPerSecond?: number | null;
  hostProtocol?: string | null;
  hostDiagnostics?: Download["hostDiagnostics"];
  trafficMode?: Download["trafficMode"];
  speedLimitBytesPerSecond?: number | null;
  openFolderOnCompletion?: boolean;
  errorMessage?: string | null;
  contentType?: string | null;
  capabilities?: Download["capabilities"];
  compatibility?: Download["compatibility"];
  validators?: Download["validators"];
  integrity?: Download["integrity"];
  diagnostics?: MockDiagnostics;
  segments?: MockSegment[];
  queuePosition?: number;
  targetConnections?: number;
  writerBackpressure?: boolean;
  engineLog?: Download["engineLog"];
  manualStartRequested?: boolean;
  runtimeCheckpoint?: Download["runtimeCheckpoint"];
}): Download {
  return {
    ...partial,
    finalUrl: partial.finalUrl ?? partial.url,
    host: partial.host ?? new URL(partial.url).host,
    targetPath: partial.targetPath ?? `${partial.savePath}/${partial.name}`,
    tempPath: partial.tempPath ?? `${partial.savePath}/${partial.name}.part`,
    queue: "default",
    queuePosition: partial.queuePosition ?? 1,
    maxConnections: partial.maxConnections ?? 8,
    customMaxConnections: partial.customMaxConnections ?? null,
    hostMaxConnections: partial.hostMaxConnections ?? null,
    hostCooldownUntil: partial.hostCooldownUntil ?? null,
    hostAverageTtfbMs: partial.hostAverageTtfbMs ?? null,
    hostAverageThroughputBytesPerSecond: partial.hostAverageThroughputBytesPerSecond ?? null,
    hostProtocol: partial.hostProtocol ?? null,
    hostDiagnostics: partial.hostDiagnostics ?? {
      hardNoRange: false,
      concurrencyLocked: false,
      lockReason: null,
      cooldownUntil: null,
      negotiatedProtocol: partial.hostProtocol ?? null,
      reuseConnections: null,
    },
    trafficMode: partial.trafficMode ?? "max",
    speedLimitBytesPerSecond: partial.speedLimitBytesPerSecond ?? null,
    openFolderOnCompletion: partial.openFolderOnCompletion ?? false,
    errorMessage: partial.errorMessage ?? null,
    contentType: partial.contentType ?? null,
    capabilities: partial.capabilities ?? {
      resumable: false,
      rangeSupported: false,
      segmented: false,
    },
    compatibility: partial.compatibility ?? {
      redirectChain: [],
      filenameSource: null,
      classification: null,
      wrapperDetected: false,
      directUrlRecovered: false,
      browserInterstitialOnly: false,
      requestReferer: null,
      requestCookies: null,
      requestMethod: "get",
      requestFormFields: [],
    },
    validators: partial.validators ?? {
      etag: null,
      lastModified: null,
      contentLength: partial.size > 0 ? partial.size : null,
      contentType: partial.contentType ?? null,
      contentDisposition: null,
    },
    integrity: partial.integrity ?? {
      expected: null,
      actual: null,
      state: "none",
      message: null,
      checkedAt: null,
    },
    diagnostics: partial.diagnostics
      ? {
          ...partial.diagnostics,
          terminalReason: partial.diagnostics.terminalReason ?? null,
          checkpointFlushes: partial.diagnostics.checkpointFlushes ?? 0,
          checkpointSkips: partial.diagnostics.checkpointSkips ?? 0,
          checkpointAvgFlushMs: partial.diagnostics.checkpointAvgFlushMs ?? 0,
          checkpointLastFlushMs: partial.diagnostics.checkpointLastFlushMs ?? 0,
          checkpointDiskPressureEvents: partial.diagnostics.checkpointDiskPressureEvents ?? 0,
          contiguousFsyncFlushes: partial.diagnostics.contiguousFsyncFlushes ?? 0,
          contiguousFsyncWindowBytes: partial.diagnostics.contiguousFsyncWindowBytes ?? 0,
        }
      : {
          warnings: [],
          notes: [
            partial.capabilities?.segmented
              ? "VDM is splitting this transfer into persisted byte-range part files."
              : "This job is running through VDM's stable single-stream pipeline.",
          ],
          failureKind: null,
          restartRequired: false,
          terminalReason: null,
          checkpointFlushes: 0,
          checkpointSkips: 0,
          checkpointAvgFlushMs: 0,
          checkpointLastFlushMs: 0,
          checkpointDiskPressureEvents: 0,
          contiguousFsyncFlushes: 0,
          contiguousFsyncWindowBytes: 0,
        },
    segments:
      partial.segments?.map((segment) => ({
        ...segment,
        retryAttempts: segment.retryAttempts ?? 0,
        retryBudget: segment.retryBudget ?? 6,
      })) ?? [],
    targetConnections:
      partial.targetConnections ??
      (partial.capabilities?.segmented
        ? Math.max(1, partial.segments?.length ?? partial.customMaxConnections ?? partial.maxConnections ?? 8)
        : 1),
    writerBackpressure: partial.writerBackpressure ?? false,
    manualStartRequested: partial.manualStartRequested ?? false,
    runtimeCheckpoint: partial.runtimeCheckpoint ?? {
      segmentSamples: [],
      activeRaces: [],
    },
    engineLog: partial.engineLog ?? [],
  };
}

export const MOCK_DOWNLOADS: Download[] = [
  buildDownload({
    id: "1",
    name: "Project Zomboid v42.14.1.rar",
    url: "https://example.com/zomboid.rar",
    size: 4_694_021_734,
    downloaded: 4_694_021_734,
    status: "finished",
    category: "compressed",
    speed: 0,
    timeLeft: null,
    dateAdded: daysAgo(0),
    savePath: "C:/Downloads",
  }),
  buildDownload({
    id: "2",
    name: "Ubuntu 24.04.1 LTS Desktop.iso",
    url: "https://releases.ubuntu.com/ubuntu.iso",
    size: 2_147_483_648,
    downloaded: 1_288_490_189,
    status: "downloading",
    category: "programs",
    speed: 3_145_728,
    timeLeft: 275,
    dateAdded: daysAgo(0),
    savePath: "C:/Downloads",
    capabilities: {
      resumable: true,
      rangeSupported: true,
      segmented: true,
    },
    maxConnections: 8,
    customMaxConnections: 4,
    speedLimitBytesPerSecond: 12 * 1024 * 1024,
    validators: {
      etag: '"ubuntu-2404"',
      lastModified: null,
      contentLength: 2_147_483_648,
      contentType: "application/octet-stream",
      contentDisposition: null,
    },
    contentType: "application/octet-stream",
    segments: [
      { id: 1, start: 0, end: 536_870_911, downloaded: 536_870_912, status: "finished" },
      { id: 2, start: 536_870_912, end: 1_073_741_823, downloaded: 402_653_184, status: "downloading" },
      { id: 3, start: 1_073_741_824, end: 1_610_612_735, downloaded: 214_748_364, status: "downloading" },
      { id: 4, start: 1_610_612_736, end: 2_147_483_647, downloaded: 134_217_729, status: "downloading" },
    ],
  }),
  buildDownload({
    id: "3",
    name: "Premiere Pro 2024 Full.zip",
    url: "https://example.com/premiere.zip",
    size: 1_073_741_824,
    downloaded: 536_870_912,
    status: "paused",
    category: "programs",
    speed: 0,
    timeLeft: null,
    dateAdded: daysAgo(1),
    savePath: "C:/Downloads",
    capabilities: {
      resumable: true,
      rangeSupported: true,
      segmented: true,
    },
    maxConnections: 8,
    customMaxConnections: 2,
    validators: {
      etag: '"premiere-zip"',
      lastModified: null,
      contentLength: 1_073_741_824,
      contentType: "application/zip",
      contentDisposition: null,
    },
    contentType: "application/zip",
    segments: [
      { id: 1, start: 0, end: 268_435_455, downloaded: 268_435_456, status: "finished" },
      { id: 2, start: 268_435_456, end: 536_870_911, downloaded: 268_435_456, status: "finished" },
      { id: 3, start: 536_870_912, end: 805_306_367, downloaded: 0, status: "pending" },
      { id: 4, start: 805_306_368, end: 1_073_741_823, downloaded: 0, status: "pending" },
    ],
  }),
  buildDownload({
    id: "4",
    name: "The Dark Knight 4K HDR.mkv",
    url: "https://example.com/tdk.mkv",
    size: 5_368_709_120,
    downloaded: 0,
    status: "queued",
    category: "videos",
    speed: 0,
    timeLeft: null,
    dateAdded: daysAgo(1),
    savePath: "C:/Downloads/Videos",
    contentType: "video/x-matroska",
  }),
  buildDownload({
    id: "5",
    name: "Cyberpunk 2077 OST.zip",
    url: "https://example.com/cp77ost.zip",
    size: 524_288_000,
    downloaded: 524_288_000,
    status: "finished",
    category: "music",
    speed: 0,
    timeLeft: null,
    dateAdded: daysAgo(3),
    savePath: "C:/Downloads/Music",
    contentType: "application/zip",
  }),
  buildDownload({
    id: "6",
    name: "Wallpaper Engine Collection.rar",
    url: "https://example.com/wallpapers.rar",
    size: 786_432_000,
    downloaded: 0,
    status: "error",
    category: "pictures",
    speed: 0,
    timeLeft: null,
    dateAdded: daysAgo(2),
    savePath: "C:/Downloads",
    errorMessage: "Host rejected the request with HTTP 429.",
    contentType: "application/x-rar-compressed",
    diagnostics: {
      warnings: ["The host ignored byte-range requests, so VDM is pinned to a single connection."],
      notes: ["Retry after the host cooldown window or restart the transfer from a clean session."],
      failureKind: "http",
      restartRequired: false,
    },
  }),
];
