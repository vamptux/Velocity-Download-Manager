import { useEffect, useRef, useState } from "react";
import type { DownloadStatus } from "@/types/download";

export function calculateDisplayProgress(
  downloaded: number,
  size: number,
  status: DownloadStatus,
): number {
  if (size <= 0) {
    return 0;
  }

  const raw = Math.min(100, Math.max(0, (downloaded / size) * 100));
  if (status === "finished") {
    return 100;
  }

  return Math.min(raw, 99.4);
}

export function useSmoothedNumber(
  target: number,
  { durationMs = 500, epsilon = 0.05 }: { durationMs?: number; epsilon?: number } = {},
): number {
  const [display, setDisplay] = useState(target);
  const displayRef = useRef(target);
  const frameRef = useRef<number | null>(null);
  const startRef = useRef(0);
  const fromRef = useRef(target);

  useEffect(() => {
    displayRef.current = display;
  }, [display]);

  useEffect(() => {
    if (Math.abs(target - displayRef.current) <= epsilon) {
      setDisplay(target);
      displayRef.current = target;
      return;
    }

    if (frameRef.current !== null) {
      cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }

    fromRef.current = displayRef.current;
    startRef.current = performance.now();

    const tick = (timestamp: number) => {
      const elapsed = timestamp - startRef.current;
      const t = Math.min(1, elapsed / Math.max(1, durationMs));
      const eased = 1 - Math.pow(1 - t, 3);
      const next = fromRef.current + (target - fromRef.current) * eased;
      setDisplay(next);
      displayRef.current = next;
      if (t < 1) {
        frameRef.current = requestAnimationFrame(tick);
      } else {
        frameRef.current = null;
      }
    };

    frameRef.current = requestAnimationFrame(tick);

    return () => {
      if (frameRef.current !== null) {
        cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
    };
  }, [durationMs, epsilon, target]);

  return display;
}
