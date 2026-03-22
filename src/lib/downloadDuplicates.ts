import type { Download } from "@/types/download";

export interface DuplicateLookupInput {
  url?: string | null;
  finalUrl?: string | null;
  targetPath?: string | null;
}

export interface DuplicateMatch {
  download: Download;
  reason: "url" | "targetPath";
}

export function normalizeComparableUrl(url: string | null | undefined): string | null {
  const trimmed = url?.trim();
  if (!trimmed) {
    return null;
  }

  try {
    const parsed = new URL(trimmed);
    parsed.hash = "";
    return parsed.toString();
  } catch {
    return trimmed.split("#", 1)[0] || null;
  }
}

export function normalizeComparablePath(path: string | null | undefined): string | null {
  const trimmed = path?.trim();
  if (!trimmed) {
    return null;
  }

  const collapsed = trimmed.replace(/[\\/]+/g, "\\");
  const looksWindows = /^[a-z]:\\/i.test(collapsed) || collapsed.includes("\\");
  return looksWindows ? collapsed.toLowerCase() : collapsed;
}

export function suggestedNameFromUrl(url: string): string {
  try {
    const parsed = new URL(url);
    const hinted = ["filename", "file", "download", "attachment", "name", "title"]
      .map((key) => parsed.searchParams.get(key)?.trim())
      .find((value): value is string => Boolean(value));
    if (hinted) {
      return hinted;
    }

    const segments = parsed.pathname.split("/").filter((segment) => segment.trim().length > 0);
    if (segments.length > 0) {
      return decodeURIComponent(segments[segments.length - 1]);
    }
  } catch {
    const fallbackSegments = url.split(/[?#]/, 1)[0]?.split("/").filter(Boolean) ?? [];
    const fallback = fallbackSegments.length > 0
      ? fallbackSegments[fallbackSegments.length - 1]?.trim()
      : null;
    if (fallback) {
      return fallback;
    }
  }

  return "download.bin";
}

export function joinTargetPathPreview(savePath: string, name: string): string | null {
  const trimmedSavePath = savePath.trim();
  const trimmedName = name.trim();
  if (!trimmedSavePath || !trimmedName) {
    return null;
  }

  const separator = trimmedSavePath.includes("\\") || /^[a-z]:/i.test(trimmedSavePath) ? "\\" : "/";
  const base = trimmedSavePath.replace(/[\\/]+$/, "");
  return `${base}${separator}${trimmedName}`;
}

export function findDuplicateDownload(
  downloads: Download[],
  input: DuplicateLookupInput,
): DuplicateMatch | null {
  const candidateUrls = new Set(
    [normalizeComparableUrl(input.url), normalizeComparableUrl(input.finalUrl)].filter(
      (value): value is string => value != null,
    ),
  );
  const candidateTargetPath = normalizeComparablePath(input.targetPath);

  for (const download of downloads) {
    if (candidateTargetPath) {
      const existingTargetPath = normalizeComparablePath(download.targetPath);
      if (existingTargetPath && existingTargetPath === candidateTargetPath) {
        return { download, reason: "targetPath" };
      }
    }

    if (candidateUrls.size > 0) {
      const existingUrls = [download.url, download.finalUrl]
        .map((value) => normalizeComparableUrl(value))
        .filter((value): value is string => value != null);
      if (existingUrls.some((value) => candidateUrls.has(value))) {
        return { download, reason: "url" };
      }
    }
  }

  return null;
}

export function describeDuplicateMatch(match: DuplicateMatch): string {
  if (match.reason === "targetPath") {
    return `${match.download.name} is already using this target path.`;
  }

  return `${match.download.name} already tracks this source URL.`;
}