import { useEffect } from "react";
import { downloadDir } from "@tauri-apps/api/path";
import type { DownloadContentCategory } from "@/types/download";

export function getCaptureErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  if (typeof error === "string" && error.trim()) {
    return error;
  }

  return "The operation failed before VDM could explain why.";
}

export function guessCaptureCategory(
  mime: string | null,
  name: string,
): DownloadContentCategory {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (["zip", "rar", "7z", "tar", "gz", "bz2", "xz"].includes(ext)) return "compressed";
  if (["exe", "msi", "dmg", "pkg", "deb", "rpm", "apk"].includes(ext)) return "programs";
  if (["mp4", "mkv", "mov", "avi", "webm", "m4v"].includes(ext)) return "videos";
  if (["mp3", "flac", "wav", "ogg", "m4a", "aac"].includes(ext)) return "music";
  if (["jpg", "jpeg", "png", "gif", "bmp", "webp"].includes(ext)) return "pictures";
  if (mime?.startsWith("video/")) return "videos";
  if (mime?.startsWith("audio/")) return "music";
  if (mime?.startsWith("image/")) return "pictures";
  return "documents";
}

export function useDefaultCaptureSavePath(
  active: boolean,
  savePath: string,
  onChange: (path: string) => void,
) {
  useEffect(() => {
    if (!active || savePath.trim()) {
      return;
    }

    let cancelled = false;
    void downloadDir()
      .then((path) => {
        if (!cancelled) {
          onChange(path);
        }
      })
      .catch(() => null);

    return () => {
      cancelled = true;
    };
  }, [active, onChange, savePath]);
}
