type BinaryFormatOptions = {
  unknownLabel?: string;
  zeroLabel?: string;
  integerAbove?: number;
  fixedFractionDigits?: number;
  preserveWholeNumbers?: boolean;
};

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;
const SPEED_UNITS = ["B/s", "KB/s", "MB/s", "GB/s", "TB/s"] as const;

function formatBinaryValue(
  value: number | null | undefined,
  units: readonly string[],
  options: BinaryFormatOptions = {},
): string {
  const {
    unknownLabel = "Unknown",
    zeroLabel = `0 ${units[0]}`,
    integerAbove,
    fixedFractionDigits,
    preserveWholeNumbers = false,
  } = options;

  if (value === null || value === undefined || value < 0) {
    return unknownLabel;
  }

  if (value === 0) {
    return zeroLabel;
  }

  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  const scaled = value / Math.pow(1024, index);

  const rendered =
    fixedFractionDigits !== undefined
      ? scaled.toFixed(fixedFractionDigits)
      : preserveWholeNumbers && Number.isInteger(scaled)
        ? scaled.toFixed(0)
        : index === 0 || (integerAbove !== undefined && scaled >= integerAbove)
          ? scaled.toFixed(0)
          : scaled.toFixed(2);

  return `${rendered} ${units[index]}`;
}

export function formatBytes(value: number | null | undefined, options: BinaryFormatOptions = {}): string {
  return formatBinaryValue(value, BYTE_UNITS, options);
}

export function formatBytesPerSecond(
  value: number | null | undefined,
  options: BinaryFormatOptions & { idleLabel?: string } = {},
): string {
  const { idleLabel, ...rest } = options;
  if (value === 0) {
    return idleLabel ?? `0 ${SPEED_UNITS[0]}`;
  }

  return formatBinaryValue(value, SPEED_UNITS, {
    ...rest,
    zeroLabel: idleLabel ?? rest.zeroLabel ?? `0 ${SPEED_UNITS[0]}`,
  });
}

export function formatTimeRemaining(
  seconds: number | null | undefined,
  options: { emptyLabel?: string } = {},
): string {
  const { emptyLabel = "Unknown" } = options;
  if (seconds === null || seconds === undefined || seconds <= 0) {
    return emptyLabel;
  }
  if (seconds < 60) {
    return `${seconds}s`;
  }
  if (seconds < 3600) {
    return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  }
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}

export function formatDurationShort(seconds: number): string {
  if (seconds < 60) {
    return `${seconds}s`;
  }
  if (seconds < 3600) {
    return `${Math.floor(seconds / 60)}m`;
  }
  return `${Math.floor(seconds / 3600)}h`;
}

export function formatRelativeDate(date: Date): string {
  const now = new Date();
  const diffHours = (now.getTime() - date.getTime()) / (1000 * 60 * 60);
  if (diffHours < 1) {
    return "Just now";
  }
  if (diffHours < 24) {
    return `${Math.floor(diffHours)} hours ago`;
  }
  if (diffHours < 48) {
    return "Yesterday";
  }
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}