import type { Download } from "@/types/download";

export interface QueueMoveState {
  canMoveUp: boolean;
  canMoveDown: boolean;
}

function queuePositionValue(download: Download): number {
  return download.queuePosition || Number.MAX_SAFE_INTEGER;
}

export function getQueueOrderedDownloads(downloads: Download[]): Download[] {
  return [...downloads]
    .filter((download) => download.queue === "default" && download.status !== "finished")
    .sort((left, right) => {
      const leftPosition = queuePositionValue(left);
      const rightPosition = queuePositionValue(right);
      if (leftPosition !== rightPosition) {
        return leftPosition - rightPosition;
      }

      return left.dateAdded.getTime() - right.dateAdded.getTime();
    });
}

export function getQueueMoveState(downloads: Download[]): Map<string, QueueMoveState> {
  const ordered = getQueueOrderedDownloads(downloads);
  return new Map(
    ordered.map((download, index) => [
      download.id,
      {
        canMoveUp: index > 0,
        canMoveDown: index < ordered.length - 1,
      },
    ]),
  );
}