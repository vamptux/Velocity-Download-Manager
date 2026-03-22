import type { Download } from "@/types/download";

export interface BatchActionFailure {
  id: string;
  message: string;
}

export interface BatchActionResult {
  requested: number;
  succeeded: string[];
  failed: BatchActionFailure[];
}

function batchActionErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  if (typeof error === "string" && error.trim()) {
    return error;
  }

  return "The action could not be completed.";
}

function isReplayOnlyRequest(download: Download): boolean {
  return download.compatibility.requestMethod !== "get" || download.compatibility.requestFormFields.length > 0;
}

export function restartRequirementLabel(download: Download): string | null {
  if (!download.diagnostics.restartRequired) {
    return null;
  }

  if (isReplayOnlyRequest(download)) {
    return "Replay-only";
  }

  return "Restart only";
}

export function restartRequirementReason(download: Download): string | null {
  if (!download.diagnostics.restartRequired) {
    return null;
  }

  if (download.diagnostics.terminalReason) {
    return download.diagnostics.terminalReason;
  }

  if (isReplayOnlyRequest(download)) {
    return "This download depends on a replayed request shape, so VDM requires a clean restart after partial progress.";
  }

  if (!download.capabilities.rangeSupported) {
    return "This download is pinned to guarded single-stream mode without verified byte-range support. Use Restart to retry from byte 0.";
  }

  return "VDM requires a clean restart before this download can continue.";
}

export function canPauseDownload(download: Download): boolean {
  return download.status === "downloading";
}

export function canResumeDownload(download: Download): boolean {
  if (download.diagnostics.restartRequired) {
    return false;
  }

  if (download.status === "paused" || download.status === "stopped" || download.status === "queued") {
    return true;
  }

  if (download.status === "error") {
    return download.downloaded === 0 || download.capabilities.resumable;
  }

  return false;
}

export function canRestartDownload(download: Download): boolean {
  if (
    download.status === "finished" ||
    download.status === "downloading" ||
    download.status === "queued"
  ) {
    return false;
  }

  return download.diagnostics.restartRequired || download.downloaded > 0 || download.status === "error";
}

export function selectDownloadIds(
  downloads: Download[],
  predicate: (download: Download) => boolean,
): string[] {
  return downloads.filter(predicate).map((download) => download.id);
}

export async function runDownloadActionBatch(
  ids: string[],
  action: (id: string) => Promise<void>,
) : Promise<BatchActionResult> {
  if (ids.length === 0) {
    return {
      requested: 0,
      succeeded: [],
      failed: [],
    };
  }

  const outcomes = await Promise.allSettled(ids.map((id) => action(id)));
  const succeeded: string[] = [];
  const failed: BatchActionFailure[] = [];

  outcomes.forEach((outcome, index) => {
    const id = ids[index];
    if (outcome.status === "fulfilled") {
      succeeded.push(id);
      return;
    }

    failed.push({
      id,
      message: batchActionErrorMessage(outcome.reason),
    });
  });

  return {
    requested: ids.length,
    succeeded,
    failed,
  };
}
