function normalizeWhitespace(message: string): string {
  return message.replace(/\s+/g, " ").trim();
}

const PROBE_NOISE_PATTERNS = [
  "host capability cache",
  "probe metadata source:",
  "planning with local hints",
  "planning fallback without fresh metadata",
  "planning.",
  "network probe unavailable",
  "probe request failed",
  "probe failed",
  "file extension was inferred",
  "inferred from content",
  "inferred from the reported",
  "suggested name was derived",
  "filename was inferred",
  "content-type suggests",
  "mime type",
  "content disposition",
  "content-disposition",
  "extension inferred",
  "no content-length",
  "content-length is unavailable",
  "size is unknown",
];

const MESSAGE_REWRITES: Array<{ match: RegExp; replace: string }> = [
  {
    match: /guarded single-stream/i,
    replace:
      "VDM is keeping this transfer on one connection until resume support is proven for this exact request.",
  },
  {
    match: /saved capabilities? .*expired/i,
    replace: "VDM is refreshing host capabilities before it starts this link.",
  },
  {
    match: /repeated probe failures .* planning it conservatively/i,
    replace:
      "Recent probes for this exact request were unstable, so VDM is starting conservatively.",
  },
  {
    match: /saved capability learning .* rejected byte-range requests/i,
    replace:
      "This link is not allowing resume or segmented transfer right now, so VDM will stay on one connection until a fresh probe succeeds.",
  },
  {
    match: /probe only reached a browser wrapper page/i,
    replace:
      "This link still passes through a wrapper page. VDM will stabilize the direct file URL when the transfer starts.",
  },
  {
    match: /probe detected an app-backed wrapper page/i,
    replace:
      "This link comes from a wrapper page. VDM will retry the direct file handoff during transfer.",
  },
  {
    match: /wrapper app api is cooling down/i,
    replace:
      "This wrapper link is cooling down after recent failures. Try again in a moment.",
  },
  {
    match: /throttled parallel requests/i,
    replace:
      "This host recently throttled parallel connections, so VDM is using a safer connection count.",
  },
  {
    match: /cooling down for about .*replay context/i,
    replace:
      "This request context is cooling down briefly after throttling or unstable responses.",
  },
  {
    match: /host cooldown active/i,
    replace:
      "This host is cooling down briefly after throttling or unstable responses.",
  },
  {
    match: /temporarily ramp-locked|low ramp-up gain|ramp-lock/i,
    replace:
      "VDM is holding the current connection count because more parallel streams were not helping.",
  },
  {
    match: /exclusive write lock on the temp file/i,
    replace:
      "Another app or antivirus is interfering with temp-file writes, so download speed may dip.",
  },
  {
    match: /checkpointed with segmented state.*guarded single-stream restart/i,
    replace:
      "This partial download can no longer resume safely. Use Restart to retry from the beginning.",
  },
  {
    match: /cannot resume safely after partial progress/i,
    replace:
      "This partial download can no longer resume safely. Use Restart to retry from the beginning.",
  },
  {
    match: /transfer-start metadata bootstrap failed:/i,
    replace:
      "VDM could not stabilize the direct file URL before the transfer started. Try again or use the final file URL if the share page keeps failing.",
  },
];

export function simplifyUserMessage(message: string): string {
  const normalized = normalizeWhitespace(message);
  for (const rule of MESSAGE_REWRITES) {
    if (rule.match.test(normalized)) {
      return rule.replace;
    }
  }

  return normalized;
}

export function isInternalProbeWarning(message: string): boolean {
  const lower = message.toLowerCase();
  return PROBE_NOISE_PATTERNS.some((pattern) => lower.includes(pattern));
}

export function getVisibleProbeWarnings(
  warnings: string[],
  limit?: number,
): string[] {
  const deduped = Array.from(
    new Set(
      warnings
        .filter((warning) => !isInternalProbeWarning(warning))
        .map(simplifyUserMessage)
        .filter((warning) => warning.length > 0),
    ),
  );

  return typeof limit === "number" ? deduped.slice(0, limit) : deduped;
}

export function firstVisibleProbeWarning(warnings: string[]): string | null {
  return getVisibleProbeWarnings(warnings, 1)[0] ?? null;
}