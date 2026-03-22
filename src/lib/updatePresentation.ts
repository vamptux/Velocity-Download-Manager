const ENGINE_HIGHLIGHT_PATTERN = /engine|backend|segment|resume|queue|host|probe|range|retry|checkpoint|disk|throughput|stability|integrity|scheduler|connection/i;

function normalizeUpdateLine(line: string): string {
  return line
    .replace(/^\s*[-*+>]+\s*/, "")
    .replace(/^\s*\d+[.)]\s*/, "")
    .replace(/^#+\s*/, "")
    .trim();
}

function truncateLine(line: string, maxLength: number): string {
  if (line.length <= maxLength) {
    return line;
  }

  return `${line.slice(0, maxLength - 3).trimEnd()}...`;
}

export function summarizeUpdateNotes(notes: string | null | undefined): string | null {
  if (!notes) {
    return null;
  }

  const firstMeaningfulLine = notes
    .split(/\r?\n/)
    .map(normalizeUpdateLine)
    .find((line) => line.length > 0);

  if (!firstMeaningfulLine) {
    return null;
  }

  return truncateLine(firstMeaningfulLine, 160);
}

export function extractUpdateHighlights(
  notes: string | null | undefined,
  maxHighlights = 3,
): string[] {
  if (!notes) {
    return [];
  }

  const lines = notes
    .split(/\r?\n/)
    .map(normalizeUpdateLine)
    .filter((line) => line.length > 0);

  if (lines.length === 0) {
    return [];
  }

  const uniqueLines = [...new Set(lines)];
  const prioritized = uniqueLines.filter((line) => ENGINE_HIGHLIGHT_PATTERN.test(line));
  const remaining = uniqueLines.filter((line) => !ENGINE_HIGHLIGHT_PATTERN.test(line));

  return [...prioritized, ...remaining]
    .slice(0, maxHighlights)
    .map((line) => truncateLine(line, 120));
}