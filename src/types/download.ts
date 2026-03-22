import type { AddDownloadArgs as BackendAddDownloadArgs } from "./generated/backend/AddDownloadArgs";
import type { AppUpdateInfo as BackendAppUpdateInfo } from "./generated/backend/AppUpdateInfo";
import type { AppUpdateProgressEvent as BackendAppUpdateProgressEvent } from "./generated/backend/AppUpdateProgressEvent";
import type { AppUpdateStartupHealth as BackendAppUpdateStartupHealth } from "./generated/backend/AppUpdateStartupHealth";
import type { AppUpdateStartupHealthStatus as BackendAppUpdateStartupHealthStatus } from "./generated/backend/AppUpdateStartupHealthStatus";
import type { CapturePayload as BackendCapturePayload } from "./generated/backend/CapturePayload";
import type { ChecksumAlgorithm as BackendChecksumAlgorithm } from "./generated/backend/ChecksumAlgorithm";
import type { ChecksumSpec as BackendChecksumSpec } from "./generated/backend/ChecksumSpec";
import type { DownloadCapabilities as BackendDownloadCapabilities } from "./generated/backend/DownloadCapabilities";
import type { DownloadCategory as BackendDownloadContentCategory } from "./generated/backend/DownloadCategory";
import type { DownloadCompatibility as BackendDownloadCompatibility } from "./generated/backend/DownloadCompatibility";
import type { DownloadCompletedEvent as BackendDownloadCompletedEvent } from "./generated/backend/DownloadCompletedEvent";
import type { DownloadDiagnostics as BackendDownloadDiagnostics } from "./generated/backend/DownloadDiagnostics";
import type { DownloadFailureKind as BackendDownloadFailureKind } from "./generated/backend/DownloadFailureKind";
import type { DownloadIntegrity as BackendDownloadIntegrity } from "./generated/backend/DownloadIntegrity";
import type { DownloadLogEntry as BackendDownloadLogEntry } from "./generated/backend/DownloadLogEntry";
import type { DownloadLogLevel as BackendDownloadLogLevel } from "./generated/backend/DownloadLogLevel";
import type { DownloadProgressDiffEvent as BackendDownloadProgressDiffEvent } from "./generated/backend/DownloadProgressDiffEvent";
import type { DownloadRecord as BackendDownloadRecord } from "./generated/backend/DownloadRecord";
import type { DownloadRequestField as BackendDownloadRequestField } from "./generated/backend/DownloadRequestField";
import type { DownloadRequestMethod as BackendDownloadRequestMethod } from "./generated/backend/DownloadRequestMethod";
import type { DownloadRuntimeCheckpoint as BackendDownloadRuntimeCheckpoint } from "./generated/backend/DownloadRuntimeCheckpoint";
import type { DownloadRuntimeRaceState as BackendDownloadRuntimeRaceState } from "./generated/backend/DownloadRuntimeRaceState";
import type { DownloadRuntimeSegmentSample as BackendDownloadRuntimeSegmentSample } from "./generated/backend/DownloadRuntimeSegmentSample";
import type { DownloadSegment as BackendDownloadSegment } from "./generated/backend/DownloadSegment";
import type { DownloadSegmentStatus as BackendDownloadSegmentStatus } from "./generated/backend/DownloadSegmentStatus";
import type { DownloadStatus as BackendDownloadStatus } from "./generated/backend/DownloadStatus";
import type { EngineSettings as BackendEngineSettings } from "./generated/backend/EngineSettings";
import type { HostDiagnosticsSummary as BackendHostDiagnosticsSummary } from "./generated/backend/HostDiagnosticsSummary";
import type { IntegrityState as BackendIntegrityState } from "./generated/backend/IntegrityState";
import type { ProbeDownloadArgs as BackendProbeDownloadArgs } from "./generated/backend/ProbeDownloadArgs";
import type { ProbeResult as BackendProbeResult } from "./generated/backend/ProbeResult";
import type { QueueState as BackendQueueState } from "./generated/backend/QueueState";
import type { ReorderDirection as BackendReorderDirection } from "./generated/backend/ReorderDirection";
import type { ResumeValidators as BackendResumeValidators } from "./generated/backend/ResumeValidators";
import type { SegmentProgressDiff as BackendSegmentProgressDiff } from "./generated/backend/SegmentProgressDiff";
import type { TrafficMode as BackendTrafficMode } from "./generated/backend/TrafficMode";

type Jsonify<T> =
  T extends bigint ? number
  : T extends Array<infer Item> ? Array<Jsonify<Item>>
  : T extends object ? { [Key in keyof T]: Jsonify<T[Key]> }
  : T;

export type DownloadStatus = Jsonify<BackendDownloadStatus>;
export type DownloadContentCategory = Jsonify<BackendDownloadContentCategory>;
export type DownloadCategory = "all" | DownloadContentCategory;
export type SidebarCategory = DownloadCategory | "finished" | "unfinished";
export type DownloadCapabilities = Jsonify<BackendDownloadCapabilities>;
export type TrafficMode = Jsonify<BackendTrafficMode>;
export type DownloadRequestMethod = Jsonify<BackendDownloadRequestMethod>;
export type DownloadRequestField = Jsonify<BackendDownloadRequestField>;
export type EngineSettings = Jsonify<BackendEngineSettings>;
export type AppUpdateInfo = Jsonify<BackendAppUpdateInfo>;
export type AppUpdateProgressEvent = Jsonify<BackendAppUpdateProgressEvent>;
export type AppUpdateStartupHealthStatus = Jsonify<BackendAppUpdateStartupHealthStatus>;
export type AppUpdateStartupHealth = Jsonify<BackendAppUpdateStartupHealth>;
export type QueueState = Jsonify<BackendQueueState>;
export type HostDiagnosticsSummary = Jsonify<BackendHostDiagnosticsSummary>;
export type DownloadCompatibility = Jsonify<BackendDownloadCompatibility>;
export type DownloadSegmentStatus = Jsonify<BackendDownloadSegmentStatus>;
export type DownloadSegment = Jsonify<BackendDownloadSegment>;
export type DownloadRuntimeSegmentSample = Jsonify<BackendDownloadRuntimeSegmentSample>;
export type DownloadRuntimeRaceState = Jsonify<BackendDownloadRuntimeRaceState>;
export type DownloadRuntimeCheckpoint = Jsonify<BackendDownloadRuntimeCheckpoint>;
export type ResumeValidators = Jsonify<BackendResumeValidators>;
export type ChecksumAlgorithm = Jsonify<BackendChecksumAlgorithm>;
export type ChecksumSpec = Jsonify<BackendChecksumSpec>;
export type IntegrityState = Jsonify<BackendIntegrityState>;
export type DownloadIntegrity = Jsonify<BackendDownloadIntegrity>;
export type DownloadFailureKind = Jsonify<BackendDownloadFailureKind>;
export type DownloadLogLevel = Jsonify<BackendDownloadLogLevel>;
export type DownloadLogEntry = Jsonify<BackendDownloadLogEntry>;
export type DownloadDiagnostics = Jsonify<BackendDownloadDiagnostics>;
export type DownloadRecord = Jsonify<BackendDownloadRecord>;
export type Download = Omit<DownloadRecord, "dateAdded"> & { dateAdded: Date };
export type DownloadProbe = Jsonify<BackendProbeResult>;
export type ReorderDirection = Jsonify<BackendReorderDirection>;
export type DownloadCompletedEvent = Jsonify<BackendDownloadCompletedEvent>;
export type SegmentProgressDiff = Jsonify<BackendSegmentProgressDiff>;
export type DownloadProgressDiffEvent = Jsonify<BackendDownloadProgressDiffEvent>;
export type ProbeDownloadArgs = Jsonify<BackendProbeDownloadArgs>;
export type AddDownloadArgs = Jsonify<BackendAddDownloadArgs>;

export type CaptureSource = "download-api" | "context-menu" | "link-click" | "manual";
export type CapturePayload = Omit<Jsonify<BackendCapturePayload>, "source"> & {
  source: CaptureSource | null;
};

export interface QueueStats {
  queuedCount: number;
  downloadSpeed: number;
}
