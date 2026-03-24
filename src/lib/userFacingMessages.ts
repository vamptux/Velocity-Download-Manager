function normalizeWhitespace(message: string): string {
  return message.replace(/\s+/g, " ").trim();
}

function readStructuredMessage(error: Record<string, unknown>): string | null {
  const message = error.message;
  if (typeof message === "string" && message.trim()) {
    return message;
  }

  const nestedError = error.error;
  if (nestedError && typeof nestedError === "object") {
    return readStructuredMessage(nestedError as Record<string, unknown>);
  }

  return null;
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
  "redirected to a different final url",
  "transfer startup redirected",
  "live transfer bootstrap",
];

const DIAGNOSTIC_NOTE_NOISE_PATTERNS = [
  "runtime worker orchestration enabled",
  "live transfer bootstrap",
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
    match: /segmented checkpoint no longer included a recoverable byte-range map/i,
    replace:
      "The saved partial download no longer had enough segment state for a safe resume, so VDM restarted it from zero.",
  },
  {
    match: /saved segmented (resume plan|checkpoint) .* restarted (this transfer|it) from zero/i,
    replace:
      "The saved segmented partial state no longer matched its byte-range plan, so VDM restarted it from zero.",
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
  {
    match: /unknown-size transfer wrote .*temp file reports .*after disk flush/i,
    replace:
      "VDM stopped this guarded single-stream transfer because the flushed temp file no longer matched the streamed byte count.",
  },
  {
    match: /unknown-size transfer reached eof .*final stream reported content-length/i,
    replace:
      "VDM stopped this guarded single-stream transfer because the host ended the stream at a different size than the final Content-Length it reported.",
  },
  {
    match: /available disk space dropped to .*unknown-size transfer stopped before the target volume ran out of space/i,
    replace:
      "VDM stopped this guarded single-stream transfer because free disk space dropped below the safety margin.",
  },
  {
    match: /unknown-size stream dropped after .*restart is required because the host did not expose a resumable content length/i,
    replace:
      "The host dropped this guarded single-stream transfer after partial progress, and VDM now requires a clean restart because no safe resume boundary was available.",
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

export function userVisibleMessageKey(message: string): string {
  return simplifyUserMessage(message).toLocaleLowerCase();
}

export function sameVisibleMessage(
  left: string | null | undefined,
  right: string | null | undefined,
): boolean {
  if (!left || !right) {
    return false;
  }

  return userVisibleMessageKey(left) === userVisibleMessageKey(right);
}

export function extractErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  if (typeof error === "string" && error.trim()) {
    const normalized = error.trim();
    if (normalized.startsWith("{") && normalized.endsWith("}")) {
      try {
        const parsed = JSON.parse(normalized) as Record<string, unknown>;
        return readStructuredMessage(parsed) ?? normalized;
      } catch {
        return normalized;
      }
    }
    return normalized;
  }

  if (error && typeof error === "object") {
    return readStructuredMessage(error as Record<string, unknown>) ?? fallback;
  }

  return fallback;
}

export function isInternalProbeWarning(message: string): boolean {
  const lower = message.toLowerCase();
  return PROBE_NOISE_PATTERNS.some((pattern) => lower.includes(pattern));
}

export function isUserFacingDiagnosticNote(message: string): boolean {
  const lower = message.toLowerCase();
  return !DIAGNOSTIC_NOTE_NOISE_PATTERNS.some((pattern) => lower.includes(pattern));
}

function dedupeVisibleMessages(messages: string[]): string[] {
  const seen = new Set<string>();
  const deduped: string[] = [];

  for (const message of messages) {
    const simplified = simplifyUserMessage(message);
    if (!simplified) {
      continue;
    }

    const key = simplified.toLocaleLowerCase();
    if (seen.has(key)) {
      continue;
    }

    seen.add(key);
    deduped.push(simplified);
  }

  return deduped;
}

export function getVisibleProbeWarnings(
  warnings: string[],
  limit?: number,
): string[] {
  const deduped = dedupeVisibleMessages(
    warnings.filter((warning) => !isInternalProbeWarning(warning)),
  );

  return typeof limit === "number" ? deduped.slice(0, limit) : deduped;
}

export function getVisibleDownloadWarnings(
  warnings: string[],
  limit?: number,
): string[] {
  const deduped = dedupeVisibleMessages(
    warnings.filter((warning) => !isInternalProbeWarning(warning)),
  );

  return typeof limit === "number" ? deduped.slice(0, limit) : deduped;
}

export function getVisibleDiagnosticNotes(
  notes: string[],
  limit?: number,
): string[] {
  const deduped = dedupeVisibleMessages(
    notes.filter(isUserFacingDiagnosticNote),
  );

  return typeof limit === "number" ? deduped.slice(0, limit) : deduped;
}

export function firstVisibleProbeWarning(warnings: string[]): string | null {
  return getVisibleProbeWarnings(warnings, 1)[0] ?? null;
}