export type SpeedLimitUnit = "kb" | "mb" | "gb";

export const SPEED_LIMIT_UNIT_FACTORS: Record<SpeedLimitUnit, number> = {
  kb: 1024,
  mb: 1024 * 1024,
  gb: 1024 * 1024 * 1024,
};

function formatSpeedLimitEditorValue(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return "25";
  }

  const rounded = value >= 100 ? value.toFixed(0) : value >= 10 ? value.toFixed(1) : value.toFixed(2);
  return rounded.replace(/\.0+$/, "").replace(/(\.\d*[1-9])0+$/, "$1");
}

export function speedLimitDraftFromValue(limitBytesPerSecond: number | null | undefined): {
  enabled: boolean;
  value: string;
  unit: SpeedLimitUnit;
} {
  if (!limitBytesPerSecond || limitBytesPerSecond <= 0) {
    return { enabled: false, value: "25", unit: "mb" };
  }

  for (const unit of ["gb", "mb", "kb"] as const) {
    const scaled = limitBytesPerSecond / SPEED_LIMIT_UNIT_FACTORS[unit];
    if (scaled >= 1) {
      return {
        enabled: true,
        value: formatSpeedLimitEditorValue(scaled),
        unit,
      };
    }
  }

  return {
    enabled: true,
    value: formatSpeedLimitEditorValue(limitBytesPerSecond / SPEED_LIMIT_UNIT_FACTORS.kb),
    unit: "kb",
  };
}

export function parseSpeedLimitDraft(
  enabled: boolean,
  value: string,
  unit: SpeedLimitUnit,
): { limitBytesPerSecond: number | null; error: string | null } {
  if (!enabled) {
    return { limitBytesPerSecond: null, error: null };
  }

  const numeric = Number.parseFloat(value.trim());
  if (!Number.isFinite(numeric) || numeric <= 0) {
    return { limitBytesPerSecond: null, error: "Enter a positive bandwidth limit." };
  }

  const limitBytesPerSecond = Math.round(numeric * SPEED_LIMIT_UNIT_FACTORS[unit]);
  if (!Number.isSafeInteger(limitBytesPerSecond) || limitBytesPerSecond <= 0) {
    return { limitBytesPerSecond: null, error: "The selected bandwidth limit is too large." };
  }

  return { limitBytesPerSecond, error: null };
}

export function effectiveSpeedLimitBytesPerSecond(
  manualLimitBytesPerSecond: number | null | undefined,
  globalLimitBytesPerSecond: number | null | undefined,
): number | null {
  const manualLimit =
    manualLimitBytesPerSecond != null && manualLimitBytesPerSecond > 0
      ? manualLimitBytesPerSecond
      : null;
  const globalLimit =
    globalLimitBytesPerSecond != null && globalLimitBytesPerSecond > 0
      ? globalLimitBytesPerSecond
      : null;
  return manualLimit ?? globalLimit;
}