import { canRestartDownload, canResumeDownload } from "@/lib/downloadActions";
import type { Download, ResumeValidators } from "@/types/download";

export interface DuplicateLookupInput {
  url?: string | null;
  finalUrl?: string | null;
  targetPath?: string | null;
  validators?: ResumeValidators | null;
}

export interface DuplicateMatch {
  download: Download;
  reason: "url" | "validators" | "targetPath";
}

export type DuplicateResolutionKind = "resume" | "restart" | "reveal" | "inspect" | "keepBoth";

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

function normalizeValidatorToken(
  value: string | null | undefined,
  lowerCase: boolean = false,
): string | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }

  return lowerCase ? trimmed.toLowerCase() : trimmed;
}

function validatorsMatch(
  existing: ResumeValidators | null | undefined,
  candidate: ResumeValidators | null | undefined,
): boolean {
  if (!existing || !candidate) {
    return false;
  }

  if (existing.contentLength == null || candidate.contentLength == null) {
    return false;
  }

  if (existing.contentLength !== candidate.contentLength) {
    return false;
  }

  const existingEtag = normalizeValidatorToken(existing.etag);
  const candidateEtag = normalizeValidatorToken(candidate.etag);
  if (existingEtag && candidateEtag && existingEtag === candidateEtag) {
    return true;
  }

  const existingLastModified = normalizeValidatorToken(existing.lastModified, true);
  const candidateLastModified = normalizeValidatorToken(candidate.lastModified, true);
  return Boolean(
    existingLastModified
      && candidateLastModified
      && existingLastModified === candidateLastModified,
  );
}

export function findDuplicateDownload(
  downloads: Download[],
  input: DuplicateLookupInput,
): DuplicateMatch | null {
  const candidateTargetPath = normalizeComparablePath(input.targetPath);

  if (candidateTargetPath) {
    for (const download of downloads) {
      const existingTargetPath = normalizeComparablePath(download.targetPath);
      if (existingTargetPath && existingTargetPath === candidateTargetPath) {
        return { download, reason: "targetPath" };
      }
    }

    return null;
  }

  const candidateUrls = new Set(
    [normalizeComparableUrl(input.url), normalizeComparableUrl(input.finalUrl)].filter(
      (value): value is string => value != null,
    ),
  );
  const candidateValidators = input.validators;

  for (const download of downloads) {
    if (candidateUrls.size > 0) {
      const existingUrls = [download.url, download.finalUrl]
        .map((value) => normalizeComparableUrl(value))
        .filter((value): value is string => value != null);
      if (existingUrls.some((value) => candidateUrls.has(value))) {
        return { download, reason: "url" };
      }
    }

    if (validatorsMatch(download.validators, candidateValidators)) {
      return { download, reason: "validators" };
    }
  }

  return null;
}

export function describeDuplicateMatch(match: DuplicateMatch): string {
  if (match.reason === "targetPath") {
    return "This file already exists in that folder.";
  }

  if (match.reason === "validators") {
    return "This file is already in VDM.";
  }

  return "This URL is already in VDM.";
}

function getExistingDuplicateResolution(match: DuplicateMatch): DuplicateResolutionKind {
  if (match.download.status === "finished") {
    return "reveal";
  }

  if (canResumeDownload(match.download)) {
    return "resume";
  }

  if (canRestartDownload(match.download)) {
    return "restart";
  }

  return "inspect";
}

export function getDuplicateResolution(match: DuplicateMatch): DuplicateResolutionKind {
  if (match.reason === "targetPath") {
    return "keepBoth";
  }

  return getExistingDuplicateResolution(match);
}

export function getSecondaryDuplicateResolution(match: DuplicateMatch): DuplicateResolutionKind | null {
  if (match.reason !== "targetPath") {
    return null;
  }

  return getExistingDuplicateResolution(match);
}

export function duplicateResolutionLabel(
  kind: DuplicateResolutionKind,
  surface: "dialog" | "compact",
): string {
  switch (kind) {
    case "keepBoth":
      return surface === "dialog" ? "Use new name" : "Rename copy";
    case "resume":
      return "Resume existing";
    case "restart":
      return "Restart existing";
    case "reveal":
      return "Open folder";
    case "inspect":
      return surface === "dialog" ? "Select existing" : "Monitor existing";
    default:
      return "Review existing";
  }
}

export function suggestAlternativeFilename(name: string): string {
  const trimmedName = name.trim();
  if (!trimmedName) {
    return "download (2).bin";
  }

  const dotIndex = trimmedName.lastIndexOf(".");
  if (dotIndex > 0) {
    return `${trimmedName.slice(0, dotIndex)} (2)${trimmedName.slice(dotIndex)}`;
  }

  return `${trimmedName} (2)`;
}