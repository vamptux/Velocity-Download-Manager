import type { ChecksumAlgorithm, ChecksumSpec } from "@/types/download";

const HEX_ONLY_PATTERN = /^[a-f0-9]+$/i;

const LENGTH_TO_ALGORITHM: Record<number, ChecksumAlgorithm> = {
  32: "md5",
  40: "sha1",
  64: "sha256",
  128: "sha512",
};

const PREFIX_TO_ALGORITHM: Record<string, ChecksumAlgorithm> = {
  md5: "md5",
  sha1: "sha1",
  "sha-1": "sha1",
  sha256: "sha256",
  "sha-256": "sha256",
  sha512: "sha512",
  "sha-512": "sha512",
};

export interface ParsedChecksumInput {
  checksum: ChecksumSpec | null;
  error: string | null;
}

export function checksumAlgorithmLabel(algorithm: ChecksumAlgorithm): string {
  switch (algorithm) {
    case "md5":
      return "MD5";
    case "sha1":
      return "SHA-1";
    case "sha256":
      return "SHA-256";
    case "sha512":
      return "SHA-512";
  }
}

function normalizeAlgorithm(rawValue: string): ChecksumAlgorithm | null {
  return PREFIX_TO_ALGORITHM[rawValue.trim().toLowerCase()] ?? null;
}

function inferAlgorithmFromLength(hash: string): ChecksumAlgorithm | null {
  return LENGTH_TO_ALGORITHM[hash.length] ?? null;
}

function normalizeHash(hash: string): string {
  return hash.trim().toLowerCase();
}

function invalidHexError(): ParsedChecksumInput {
  return {
    checksum: null,
    error: "Checksum must contain only hexadecimal characters.",
  };
}

function parseHashForAlgorithm(
  rawHash: string,
  algorithm: ChecksumAlgorithm,
): ParsedChecksumInput {
  const hash = normalizeHash(rawHash);
  if (!hash) {
    return {
      checksum: null,
      error: "Checksum cannot be empty.",
    };
  }
  if (!HEX_ONLY_PATTERN.test(hash)) {
    return invalidHexError();
  }

  const inferred = inferAlgorithmFromLength(hash);
  if (inferred !== algorithm) {
    const expectedLength = Object.entries(LENGTH_TO_ALGORITHM).find(
      ([, value]) => value === algorithm,
    )?.[0];
    return {
      checksum: null,
      error: `${checksumAlgorithmLabel(algorithm)} checksums must be exactly ${expectedLength} hexadecimal characters.`,
    };
  }

  return {
    checksum: { algorithm, value: hash },
    error: null,
  };
}

function parsePrefixedChecksum(input: string): ParsedChecksumInput | null {
  const normalized = input.trim();
  const opensslMatch = normalized.match(/^([a-z0-9-]+)\s*\([^)]*\)\s*=\s*([a-f0-9]+)$/i);
  if (opensslMatch) {
    const algorithm = normalizeAlgorithm(opensslMatch[1]);
    if (!algorithm) {
      return {
        checksum: null,
        error: `Unsupported checksum algorithm '${opensslMatch[1]}'.`,
      };
    }
    return parseHashForAlgorithm(opensslMatch[2], algorithm);
  }

  const prefixedMatch = normalized.match(/^([a-z0-9-]+)\s*[:=]\s*([a-f0-9]+)$/i);
  if (prefixedMatch) {
    const algorithm = normalizeAlgorithm(prefixedMatch[1]);
    if (!algorithm) {
      return {
        checksum: null,
        error: `Unsupported checksum algorithm '${prefixedMatch[1]}'.`,
      };
    }
    return parseHashForAlgorithm(prefixedMatch[2], algorithm);
  }

  const spacedMatch = normalized.match(/^([a-z0-9-]+)\s+([a-f0-9]+)$/i);
  if (spacedMatch) {
    const algorithm = normalizeAlgorithm(spacedMatch[1]);
    if (algorithm) {
      return parseHashForAlgorithm(spacedMatch[2], algorithm);
    }
  }

  return null;
}

function parseChecksumFileLine(input: string): ParsedChecksumInput | null {
  const normalized = input.trim();
  const checksumFileMatch = normalized.match(/^([a-f0-9]+)\s+[ *].+$/i);
  if (!checksumFileMatch) {
    return null;
  }

  const hash = normalizeHash(checksumFileMatch[1]);
  const algorithm = inferAlgorithmFromLength(hash);
  if (!algorithm) {
    return {
      checksum: null,
      error: "Checksum length does not match a supported algorithm.",
    };
  }

  return {
    checksum: { algorithm, value: hash },
    error: null,
  };
}

export function parseChecksumInput(rawValue: string): ParsedChecksumInput {
  const firstLine = rawValue
    .split(/\r?\n/)
    .map((value) => value.trim())
    .find((value) => value.length > 0);

  if (!firstLine) {
    return { checksum: null, error: null };
  }

  const prefixed = parsePrefixedChecksum(firstLine);
  if (prefixed) {
    return prefixed;
  }

  const checksumFileLine = parseChecksumFileLine(firstLine);
  if (checksumFileLine) {
    return checksumFileLine;
  }

  const hash = normalizeHash(firstLine);
  if (!HEX_ONLY_PATTERN.test(hash)) {
    return invalidHexError();
  }

  const algorithm = inferAlgorithmFromLength(hash);
  if (!algorithm) {
    return {
      checksum: null,
      error: "Checksum length does not match a supported algorithm.",
    };
  }

  return {
    checksum: { algorithm, value: hash },
    error: null,
  };
}