import { formatTimeRemaining } from "@/lib/format";
import {
  primaryIssueSummary,
  statusLabel,
  transferConstraintMeta,
  transferConstraintSummary,
  transferModeLabel,
} from "@/lib/downloadPresentation";
import {
  getVisibleDiagnosticNotes,
  getVisibleDownloadWarnings,
} from "@/lib/userFacingMessages";
import type { Download, DownloadLogEntry } from "@/types/download";

function logLevelLabel(level: DownloadLogEntry["level"]): string {
  switch (level) {
    case "info":
      return "Info";
    case "warn":
      return "Warn";
    case "error":
      return "Error";
  }
}

function formatLogTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function buildDownloadDiagnosticsSummary(
  download: Download,
  displaySourceUrl: string,
): string {
  const transferConstraint = transferConstraintMeta(download);
  const transferPressure = transferConstraintSummary(transferConstraint);
  const warnings = getVisibleDownloadWarnings(download.diagnostics.warnings);
  const notes = getVisibleDiagnosticNotes(download.diagnostics.notes);
  const primaryIssue = primaryIssueSummary(download);
  const recentLogLines = download.engineLog.slice(-5).map((entry) => (
    `- [${logLevelLabel(entry.level)} ${formatLogTime(entry.timestamp)}] ${entry.message}`
  ));

  const lines = [
    `Name: ${download.name}`,
    `Status: ${statusLabel(download.status)}`,
    `Mode: ${transferModeLabel(download)}`,
    `Target path: ${download.targetPath}`,
    `Source URL: ${displaySourceUrl}`,
    `Host: ${download.host}`,
    download.timeLeft != null
      ? `Time remaining: ${formatTimeRemaining(download.timeLeft, { emptyLabel: "Unknown" })}`
      : null,
    transferPressure ? `Transfer pressure: ${transferPressure}` : null,
    primaryIssue ? `Primary issue: ${primaryIssue}` : null,
  ].filter((line): line is string => line != null);

  if (warnings.length > 0) {
    lines.push("Warnings:");
    lines.push(...warnings.map((warning) => `- ${warning}`));
  }

  if (notes.length > 0) {
    lines.push("Notes:");
    lines.push(...notes.map((note) => `- ${note}`));
  }

  if (recentLogLines.length > 0) {
    lines.push("Recent log:");
    lines.push(...recentLogLines);
  }

  return lines.join("\n");
}

export function buildSelectionDiagnosticsSummary(selectedDownloads: Download[]): string {
  return selectedDownloads
    .map((download) => {
      const transferConstraint = transferConstraintMeta(download);
      const transferPressure = transferConstraintSummary(transferConstraint);
      const primaryIssue = primaryIssueSummary(download);
      return [
        `${download.name}`,
        `  Status: ${statusLabel(download.status)}`,
        `  Mode: ${transferModeLabel(download)}`,
        transferPressure ? `  Transfer pressure: ${transferPressure}` : null,
        primaryIssue ? `  Primary issue: ${primaryIssue}` : null,
      ].filter((line): line is string => line != null).join("\n");
    })
    .join("\n\n");
}