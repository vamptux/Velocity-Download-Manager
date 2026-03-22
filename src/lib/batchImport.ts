import { guessCaptureCategory } from "@/lib/captureUtils";
import {
  joinTargetPathPreview,
  normalizeComparablePath,
  normalizeComparableUrl,
  suggestedNameFromUrl,
} from "@/lib/downloadDuplicates";
import type { ChecksumAlgorithm, ChecksumSpec, DownloadContentCategory } from "@/types/download";

export type BatchImportFormat = "lines" | "csv" | "tsv";

export interface BatchImportRow {
  lineNumber: number;
  source: string;
  url: string;
  folder: string;
  filename: string;
  category: DownloadContentCategory;
  startImmediately: boolean;
  checksum: ChecksumSpec | null;
  targetPath: string | null;
  errors: string[];
}

export interface BatchImportPreview {
  format: BatchImportFormat;
  rows: BatchImportRow[];
  validCount: number;
  invalidCount: number;
}

const HEADER_ALIASES: Record<string, string> = {
  url: "url",
  link: "url",
  href: "url",
  folder: "folder",
  directory: "folder",
  savepath: "folder",
  path: "folder",
  filename: "filename",
  file: "filename",
  name: "filename",
  checksum: "checksum",
  hash: "checksum",
  category: "category",
  type: "category",
  startmode: "startMode",
  start: "startMode",
  mode: "startMode",
};

const CATEGORY_VALUES = new Set<DownloadContentCategory>([
  "compressed",
  "programs",
  "videos",
  "music",
  "pictures",
  "documents",
]);

function splitDelimitedLine(line: string, delimiter: string): string[] {
  const values: string[] = [];
  let current = "";
  let inQuotes = false;

  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];
    const next = line[index + 1];

    if (char === '"') {
      if (inQuotes && next === '"') {
        current += '"';
        index += 1;
        continue;
      }

      inQuotes = !inQuotes;
      continue;
    }

    if (char === delimiter && !inQuotes) {
      values.push(current.trim());
      current = "";
      continue;
    }

    current += char;
  }

  values.push(current.trim());
  return values;
}

function normalizeHeader(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]/g, "");
}

function detectFormat(lines: string[]): BatchImportFormat {
  const firstContentLine = lines.find((line) => line.trim().length > 0);
  if (!firstContentLine) {
    return "lines";
  }

  for (const [delimiter, format] of [["\t", "tsv"], [",", "csv"]] as const) {
    if (!firstContentLine.includes(delimiter)) {
      continue;
    }

    const headers = splitDelimitedLine(firstContentLine, delimiter)
      .map(normalizeHeader)
      .map((header) => HEADER_ALIASES[header] ?? header);
    if (headers.includes("url")) {
      return format;
    }
  }

  return "lines";
}

function parseChecksum(rawValue: string): { checksum: ChecksumSpec | null; error: string | null } {
  const trimmed = rawValue.trim();
  if (!trimmed) {
    return { checksum: null, error: null };
  }

  const [algorithm, value] = trimmed.split(/:(.+)/, 2);
  const normalizedAlgorithm = algorithm?.trim().toLowerCase();
  const normalizedValue = value?.trim();
  if (!normalizedAlgorithm || !normalizedValue) {
    return {
      checksum: null,
      error: "Checksum must use the format algorithm:value.",
    };
  }

  if (!["md5", "sha1", "sha256", "sha512"].includes(normalizedAlgorithm)) {
    return {
      checksum: null,
      error: `Unsupported checksum algorithm '${algorithm}'.`,
    };
  }

  return {
    checksum: {
      algorithm: normalizedAlgorithm as ChecksumAlgorithm,
      value: normalizedValue,
    },
    error: null,
  };
}

function parseStartMode(rawValue: string): { startImmediately: boolean; error: string | null } {
  const trimmed = rawValue.trim().toLowerCase();
  if (!trimmed) {
    return { startImmediately: true, error: null };
  }

  if (["start", "now", "immediate", "true", "yes", "1"].includes(trimmed)) {
    return { startImmediately: true, error: null };
  }

  if (["queue", "queued", "later", "manual", "paused", "false", "no", "0"].includes(trimmed)) {
    return { startImmediately: false, error: null };
  }

  return {
    startImmediately: true,
    error: `Unsupported start mode '${rawValue}'.`,
  };
}

function validateUrl(url: string): string | null {
  try {
    const parsed = new URL(url);
    if (!parsed.protocol || !parsed.hostname) {
      return "URL must include a valid protocol and host.";
    }
    return null;
  } catch {
    return "URL is invalid.";
  }
}

function finalizeRow(
  lineNumber: number,
  source: string,
  values: {
    url?: string;
    folder?: string;
    filename?: string;
    checksum?: string;
    category?: string;
    startMode?: string;
  },
  defaultSavePath: string,
): BatchImportRow {
  const errors: string[] = [];
  const url = values.url?.trim() ?? "";
  if (!url) {
    errors.push("URL is required.");
  } else {
    const urlError = validateUrl(url);
    if (urlError) {
      errors.push(urlError);
    }
  }

  const filename = (values.filename?.trim() || suggestedNameFromUrl(url)).trim();
  const folder = (values.folder?.trim() || defaultSavePath).trim();
  if (!folder) {
    errors.push("Folder is required or a default download directory must be available.");
  }

  const categoryValue = values.category?.trim().toLowerCase();
  const category = categoryValue
    ? (CATEGORY_VALUES.has(categoryValue as DownloadContentCategory)
      ? categoryValue as DownloadContentCategory
      : null)
    : guessCaptureCategory(null, filename || suggestedNameFromUrl(url));
  if (!category) {
    errors.push(`Unsupported category '${values.category?.trim() ?? ""}'.`);
  }

  const { checksum, error: checksumError } = parseChecksum(values.checksum ?? "");
  if (checksumError) {
    errors.push(checksumError);
  }

  const { startImmediately, error: startModeError } = parseStartMode(values.startMode ?? "");
  if (startModeError) {
    errors.push(startModeError);
  }

  const targetPath = joinTargetPathPreview(folder, filename);

  return {
    lineNumber,
    source,
    url,
    folder,
    filename,
    category: category ?? "documents",
    startImmediately,
    checksum,
    targetPath,
    errors,
  };
}

function addIntraBatchDuplicateErrors(rows: BatchImportRow[]): BatchImportRow[] {
  const firstUrlLine = new Map<string, number>();
  const firstTargetLine = new Map<string, number>();

  return rows.map((row) => {
    const errors = [...row.errors];
    const normalizedUrl = normalizeComparableUrl(row.url);
    const normalizedTargetPath = normalizeComparablePath(row.targetPath);

    if (normalizedUrl) {
      const firstLine = firstUrlLine.get(normalizedUrl);
      if (firstLine != null) {
        errors.push(`Matches the same URL already listed on row ${firstLine}.`);
      } else {
        firstUrlLine.set(normalizedUrl, row.lineNumber);
      }
    }

    if (normalizedTargetPath) {
      const firstLine = firstTargetLine.get(normalizedTargetPath);
      if (firstLine != null) {
        errors.push(`Matches the same target path already listed on row ${firstLine}.`);
      } else {
        firstTargetLine.set(normalizedTargetPath, row.lineNumber);
      }
    }

    return { ...row, errors };
  });
}

export function parseBatchImportInput(input: string, defaultSavePath: string): BatchImportPreview {
  const lines = input.split(/\r?\n/);
  const format = detectFormat(lines);
  const rows: BatchImportRow[] = [];

  if (format === "lines") {
    lines.forEach((line, index) => {
      const source = line.trim();
      if (!source) {
        return;
      }
      rows.push(
        finalizeRow(index + 1, line, { url: source }, defaultSavePath),
      );
    });
  } else {
    const delimiter = format === "tsv" ? "\t" : ",";
    const firstContentIndex = lines.findIndex((line) => line.trim().length > 0);
    if (firstContentIndex >= 0) {
      const rawHeaders = splitDelimitedLine(lines[firstContentIndex], delimiter);
      const headers = rawHeaders.map((header) => HEADER_ALIASES[normalizeHeader(header)] ?? normalizeHeader(header));

      for (let index = firstContentIndex + 1; index < lines.length; index += 1) {
        const line = lines[index];
        if (!line.trim()) {
          continue;
        }
        const values = splitDelimitedLine(line, delimiter);
        const record = headers.reduce<Record<string, string>>((accumulator, header, headerIndex) => {
          accumulator[header] = values[headerIndex] ?? "";
          return accumulator;
        }, {});

        rows.push(
          finalizeRow(index + 1, line, {
            url: record.url,
            folder: record.folder,
            filename: record.filename,
            checksum: record.checksum,
            category: record.category,
            startMode: record.startMode,
          }, defaultSavePath),
        );
      }
    }
  }

  const rowsWithDuplicateChecks = addIntraBatchDuplicateErrors(rows);
  const invalidCount = rowsWithDuplicateChecks.filter((row) => row.errors.length > 0).length;

  return {
    format,
    rows: rowsWithDuplicateChecks,
    validCount: rowsWithDuplicateChecks.length - invalidCount,
    invalidCount,
  };
}