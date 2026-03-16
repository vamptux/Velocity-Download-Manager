import "../shared/download-rules.js";

const rules = globalThis.VDMDownloadRules;

if (!rules) {
  throw new Error("VDM shared download rules failed to initialize.");
}

export const ALWAYS_INTERCEPT_EXTENSIONS = rules.ALWAYS_INTERCEPT_EXTENSIONS;
export const ALWAYS_INTERCEPT_MIMES = rules.ALWAYS_INTERCEPT_MIMES;
export const DEFAULT_MIN_SIZE_BYTES = rules.DEFAULT_MIN_SIZE_BYTES;
export const CDN_PATTERNS = rules.CDN_PATTERNS;

export function extensionFromUrl(url, baseUrl) {
  return rules.extensionFromUrl(url, baseUrl);
}

export function hasDownloadPathHint(url, baseUrl) {
  return rules.hasDownloadPathHint(url, baseUrl);
}

export function isCdnHost(url, baseUrl) {
  return rules.isCdnHost(url, baseUrl);
}

export function normalizeResponseHeaders(responseHeaders) {
  return rules.normalizeResponseHeaders(responseHeaders);
}

export function classifyDownload(options) {
  return rules.classifyDownload(options);
}