/**
 * VDM Catcher — Background Service Worker
 *
 * Responsibilities:
 *  1. Intercept browser-triggered downloads (chrome.downloads.onCreated).
 *  2. Smart-filter using file extension, MIME type, CDN host and size.
 *  3. Cancel the browser download and POST the URL to VDM's local capture
 *     server (127.0.0.1:17780).
 *  4. Provide a right-click "Download with VDM" context menu.
 *  5. Manage extension settings stored in chrome.storage.sync.
 *  6. Maintain a live VDM connection health badge on the extension icon.
 */

import {
  DEFAULT_SETTINGS,
  loadExtensionSettings,
  saveBridgeState,
  saveGeneralSettings,
} from "../shared/settings.js";
import { classifyDownload, normalizeResponseHeaders } from "./filter-rules.js";

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const VDM_BASE_URL = "http://127.0.0.1:17780";
const BRIDGE_PAIR_PATH = "/pair";
const HEALTH_CHECK_ALARM = "vdm-health-check";
const HEALTH_CHECK_PERIOD_MINUTES = 0.5; // 30 s — alarm minimum is 0.5 min
const MAINTENANCE_ALARM = "vdm-maintenance";
const MAINTENANCE_PERIOD_MINUTES = 1;
const HEALTH_CACHE_TTL_MS = 2_500;
const CAPTURE_TIMEOUT_MS = 3000;
const CAPTURE_DEDUPE_WINDOW_MS = 4500;
const OBSERVED_REQUEST_TTL_MS = 30_000;
const OBSERVED_RESPONSE_TTL_MS = 30_000;
const AUTH_STATE_OFFLINE = "offline";
const AUTH_STATE_NONE = "none";
const AUTH_STATE_MISSING = "missing";
const AUTH_STATE_PAIRED = "paired";
const AUTH_STATE_INVALID = "invalid";
const BRIDGE_HEADER_CLIENT = "X-VDM-Client";
const BRIDGE_HEADER_EXTENSION_ORIGIN = "X-VDM-Extension-Origin";

// ---------------------------------------------------------------------------
// In-memory state
// ---------------------------------------------------------------------------

// Dedupe windows for frequent double-fire paths (content click + downloads API).
const captureInFlight = new Set();
const captureRecent = new Map();
const observedRequests = new Map();
const observedResponses = new Map();

// Cached settings — refreshed whenever chrome.storage.sync changes.
let cachedSettings = {
  ...DEFAULT_SETTINGS,
  bridgePairingCode: "",
  bridgePairingRotatedAt: 0,
  bridgePairingSyncedAt: 0,
};

// Current VDM health status.
let vdmConnected = false;
let vdmReachable = false;
let vdmAuthState = AUTH_STATE_OFFLINE;
let bridgeSessionNonce = null;
let lastHealthCheckAt = 0;
let healthCheckPromise = null;

// ---------------------------------------------------------------------------
// Settings helpers
// ---------------------------------------------------------------------------

async function loadSettings() {
  cachedSettings = await loadExtensionSettings();
}

async function persistPairingState(pairingCode, rotatedAt = 0) {
  const normalizedCode = String(pairingCode ?? "").trim();
  const bridgeState = await saveBridgeState({
    bridgePairingCode: normalizedCode,
    bridgePairingRotatedAt: Number.isFinite(Number(rotatedAt)) ? Number(rotatedAt) : 0,
    bridgePairingSyncedAt: normalizedCode ? Date.now() : 0,
  });
  cachedSettings = {
    ...cachedSettings,
    ...bridgeState,
  };
}

function normalizedPairingCode() {
  return String(cachedSettings.bridgePairingCode ?? "").trim();
}

function extensionOrigin() {
  return chrome.runtime.getURL("").replace(/\/$/, "");
}

function bridgeStatusSnapshot() {
  return {
    enabled: !!cachedSettings.enabled,
    connected: vdmConnected,
    reachable: vdmReachable,
    authState: vdmAuthState,
    pairingRotatedAt: Number(cachedSettings.bridgePairingRotatedAt ?? 0) || 0,
    pairingSyncedAt: Number(cachedSettings.bridgePairingSyncedAt ?? 0) || 0,
  };
}

async function loadRequestCookies(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    if (!Array.isArray(cookies) || cookies.length === 0) {
      return null;
    }

    cookies.sort((left, right) => (right.path?.length ?? 0) - (left.path?.length ?? 0));
    return cookies
      .filter((cookie) => cookie.name)
      .map((cookie) => `${cookie.name}=${cookie.value}`)
      .join("; ");
  } catch {
    return null;
  }
}

async function enrichCapturePayload(payload) {
  return {
    ...payload,
    requestCookies: payload.requestCookies ?? await loadRequestCookies(payload.url),
    requestMethod: payload.requestMethod ?? "get",
    requestFormFields: Array.isArray(payload.requestFormFields) ? payload.requestFormFields : [],
  };
}

function normalizeCaptureKey(payload) {
  const url = String(payload.url ?? "").trim();
  const filename = String(payload.filename ?? "").trim().toLowerCase();
  return `${url}::${filename}`;
}

function bytesToHex(bytes) {
  return Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");
}

function randomHex(byteCount) {
  const bytes = new Uint8Array(byteCount);
  crypto.getRandomValues(bytes);
  return bytesToHex(bytes);
}

async function sha256Hex(value) {
  const bytes = value instanceof Uint8Array ? value : new TextEncoder().encode(String(value ?? ""));
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return bytesToHex(new Uint8Array(digest));
}

async function hmacSha256Hex(secret, message) {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signature = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(message));
  return bytesToHex(new Uint8Array(signature));
}

async function fetchBridge(path, options = {}) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), CAPTURE_TIMEOUT_MS);
  try {
    return await fetch(`${VDM_BASE_URL}${path}`, {
      cache: "no-store",
      ...options,
      signal: ctrl.signal,
    });
  } finally {
    clearTimeout(timer);
  }
}

async function fetchBridgeHealthSnapshot() {
  try {
    const response = await fetchBridge("/health", { method: "GET" });
    if (!response.ok) {
      vdmReachable = false;
      bridgeSessionNonce = null;
      return { ok: false, authRequired: true, authorized: false };
    }

    const payload = await response.json().catch(() => null);
    vdmReachable = true;
    bridgeSessionNonce = typeof payload?.sessionNonce === "string" ? payload.sessionNonce : null;
    return {
      ok: true,
      authRequired: payload?.authRequired !== false,
      authorized: payload?.authorized === true,
    };
  } catch {
    vdmReachable = false;
    bridgeSessionNonce = null;
    return { ok: false, authRequired: true, authorized: false };
  }
}

async function syncPairingFromVdm() {
  const client = chrome.runtime.id || "vdm-catcher";
  let response;
  try {
    response = await fetchBridge(BRIDGE_PAIR_PATH, {
      method: "GET",
      headers: {
        [BRIDGE_HEADER_CLIENT]: client,
        [BRIDGE_HEADER_EXTENSION_ORIGIN]: extensionOrigin(),
      },
    });
  } catch {
    return false;
  }

  if (!response.ok) {
    return false;
  }

  const payload = await response.json().catch(() => null);
  const pairingCode = String(payload?.pairingCode ?? "").trim();
  if (!pairingCode) {
    return false;
  }

  await persistPairingState(pairingCode, Number(payload?.rotatedAt ?? 0));
  if (typeof payload?.sessionNonce === "string" && payload.sessionNonce) {
    bridgeSessionNonce = payload.sessionNonce;
  }
  vdmReachable = true;
  return true;
}

async function buildBridgeAuthHeaders(method, path, bodyText = "") {
  const pairingCode = normalizedPairingCode();
  if (!pairingCode) {
    return null;
  }

  if (!bridgeSessionNonce) {
    const health = await fetchBridgeHealthSnapshot();
    if (!health.ok || !bridgeSessionNonce) {
      return null;
    }
  }

  const client = chrome.runtime.id || "vdm-catcher";
  const timestamp = String(Date.now());
  const requestNonce = randomHex(16);
  const bodyHash = await sha256Hex(bodyText);
  const payload = [method.toUpperCase(), path, bridgeSessionNonce, timestamp, requestNonce, client, bodyHash].join("\n");
  const signature = await hmacSha256Hex(pairingCode, payload);
  return {
    [BRIDGE_HEADER_CLIENT]: client,
    "X-VDM-Timestamp": timestamp,
    "X-VDM-Request-Nonce": requestNonce,
    "X-VDM-Auth": signature,
  };
}

async function fetchAuthorizedBridgeResponse(path, { method = "GET", bodyText = "", contentType = null } = {}) {
  const authHeaders = await buildBridgeAuthHeaders(method, path, bodyText);
  if (!authHeaders) {
    return null;
  }

  const headers = contentType ? { "Content-Type": contentType, ...authHeaders } : authHeaders;
  let response;
  try {
    response = await fetchBridge(path, {
      method,
      headers,
      body: bodyText ? bodyText : undefined,
    });
  } catch {
    return null;
  }

  if (response.status !== 401) {
    return response;
  }

  bridgeSessionNonce = null;
  const refreshed = await fetchBridgeHealthSnapshot();
  if (!refreshed.ok) {
    return response;
  }

  const retryAuthHeaders = await buildBridgeAuthHeaders(method, path, bodyText);
  if (!retryAuthHeaders) {
    return response;
  }

  const retryHeaders = contentType ? { "Content-Type": contentType, ...retryAuthHeaders } : retryAuthHeaders;
  try {
    return await fetchBridge(path, {
      method,
      headers: retryHeaders,
      body: bodyText ? bodyText : undefined,
    });
  } catch {
    return null;
  }
}

function pruneCaptureCache(now = Date.now()) {
  for (const [key, ts] of captureRecent) {
    if (now - ts > CAPTURE_DEDUPE_WINDOW_MS) {
      captureRecent.delete(key);
    }
  }
}

function normalizeObservedUrl(url) {
  try {
    const parsed = new URL(String(url ?? ""));
    parsed.hash = "";
    return parsed.href;
  } catch {
    return String(url ?? "").trim();
  }
}

function pruneTimedCache(cache, ttlMs, now = Date.now()) {
  for (const [key, value] of cache) {
    if (!value || now - value.capturedAt > ttlMs) {
      cache.delete(key);
    }
  }
}

function normalizeObservedFormFields(requestBody) {
  const formData = requestBody?.formData;
  if (!formData || typeof formData !== "object") {
    return [];
  }

  const fields = [];
  for (const [name, values] of Object.entries(formData)) {
    if (!Array.isArray(values)) {
      continue;
    }
    for (const value of values) {
      fields.push({ name, value: String(value) });
    }
  }
  return fields;
}

function rememberObservedRequest(details) {
  if (!details?.url || !/^https?:/i.test(details.url)) {
    return;
  }

  pruneTimedCache(observedRequests, OBSERVED_REQUEST_TTL_MS);
  observedRequests.set(normalizeObservedUrl(details.url), {
    capturedAt: Date.now(),
    requestMethod: String(details.method ?? "get").toLowerCase() === "post" ? "post" : "get",
    requestFormFields: normalizeObservedFormFields(details.requestBody),
    referrer: details.initiator ?? details.documentUrl ?? null,
  });
}

function rememberObservedResponse(details) {
  if (!details?.url || !/^https?:/i.test(details.url)) {
    return;
  }

  const headers = {};
  for (const header of details.responseHeaders ?? []) {
    if (!header?.name || typeof header.value !== "string" || !header.value) {
      continue;
    }
    const name = header.name.toLowerCase();
    if (!(name in headers)) {
      headers[name] = header.value;
    }
  }

  pruneTimedCache(observedResponses, OBSERVED_RESPONSE_TTL_MS);
  observedResponses.set(normalizeObservedUrl(details.url), {
    capturedAt: Date.now(),
    responseHeaders: headers,
  });
}

function latestObservedEntry(cache, ttlMs, urls) {
  pruneTimedCache(cache, ttlMs);
  for (const url of urls) {
    if (!url) {
      continue;
    }
    const entry = cache.get(normalizeObservedUrl(url));
    if (entry) {
      return entry;
    }
  }
  return null;
}

function findObservedRequest(urls) {
  return latestObservedEntry(observedRequests, OBSERVED_REQUEST_TTL_MS, urls);
}

function findObservedResponseHeaders(urls) {
  return latestObservedEntry(observedResponses, OBSERVED_RESPONSE_TTL_MS, urls)?.responseHeaders ?? null;
}

chrome.webRequest.onBeforeRequest.addListener(
  (details) => {
    rememberObservedRequest(details);
  },
  { urls: ["http://*/*", "https://*/*"] },
  ["requestBody"],
);

chrome.webRequest.onHeadersReceived.addListener(
  (details) => {
    rememberObservedResponse(details);
  },
  { urls: ["http://*/*", "https://*/*"] },
  ["responseHeaders"],
);

chrome.storage.onChanged.addListener((_, areaName) => {
  if (areaName !== "sync" && areaName !== "local") {
    return;
  }
  void (async () => {
    await loadSettings();
    await checkVdmHealth();
  })();
});

// ---------------------------------------------------------------------------
// Icon / badge rendering via OffscreenCanvas
// ---------------------------------------------------------------------------

function buildIconImageData(size, connected) {
  const canvas = new OffscreenCanvas(size, size);
  const ctx = canvas.getContext("2d");
  const s = size;

  // Background circle.
  ctx.beginPath();
  ctx.arc(s / 2, s / 2, s / 2 - 0.5, 0, Math.PI * 2);
  ctx.fillStyle = connected ? "#b66336" : "#6b7280";
  ctx.fill();

  // Download-arrow body (vertical line).
  ctx.strokeStyle = "#ffffff";
  ctx.lineWidth = Math.max(1, s / 8);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.beginPath();
  ctx.moveTo(s / 2, s * 0.2);
  ctx.lineTo(s / 2, s * 0.65);
  ctx.stroke();

  // Arrow-head chevron.
  ctx.beginPath();
  ctx.moveTo(s * 0.3, s * 0.5);
  ctx.lineTo(s / 2, s * 0.72);
  ctx.lineTo(s * 0.7, s * 0.5);
  ctx.stroke();

  return ctx.getImageData(0, 0, s, s);
}

function updateIcon() {
  chrome.action.setIcon({
    imageData: {
      16: buildIconImageData(16, vdmReachable),
      32: buildIconImageData(32, vdmReachable),
      48: buildIconImageData(48, vdmReachable),
      128: buildIconImageData(128, vdmReachable),
    },
  });

  let title = "VDM Download Catcher — Connected";
  let badgeText = "";
  let badgeColor = "#b66336";

  if (!cachedSettings.enabled) {
    title = "VDM Download Catcher — Disabled";
  } else if (!vdmReachable) {
    title = "VDM Download Catcher — VDM not running";
    badgeText = "!";
    badgeColor = "#ef4444";
  } else if (vdmAuthState === AUTH_STATE_MISSING) {
    title = "VDM Download Catcher — Preparing bridge";
    badgeText = "!";
    badgeColor = "#f59e0b";
  } else if (vdmAuthState === AUTH_STATE_INVALID) {
    title = "VDM Download Catcher — Refreshing bridge";
    badgeText = "!";
    badgeColor = "#ef4444";
  }

  chrome.action.setTitle({
    title,
  });
  chrome.action.setBadgeText({ text: badgeText });
  chrome.action.setBadgeBackgroundColor({ color: badgeColor });
}

// ---------------------------------------------------------------------------
// VDM Health Check
// ---------------------------------------------------------------------------

async function refreshVdmHealth({ allowAutoPair = true } = {}) {
  const health = await fetchBridgeHealthSnapshot();
  if (!health.ok) {
    vdmConnected = false;
    vdmAuthState = AUTH_STATE_OFFLINE;
    lastHealthCheckAt = Date.now();
    updateIcon();
    return bridgeStatusSnapshot();
  }

  if (!health.authRequired) {
    vdmConnected = true;
    vdmAuthState = AUTH_STATE_NONE;
    lastHealthCheckAt = Date.now();
    updateIcon();
    return bridgeStatusSnapshot();
  }

  if (!normalizedPairingCode()) {
    if (allowAutoPair && await syncPairingFromVdm()) {
      return refreshVdmHealth({ allowAutoPair: false });
    }
    vdmConnected = false;
    vdmAuthState = AUTH_STATE_MISSING;
    lastHealthCheckAt = Date.now();
    updateIcon();
    return bridgeStatusSnapshot();
  }

  const response = await fetchAuthorizedBridgeResponse("/health", { method: "GET" });
  if (!response) {
    vdmConnected = false;
    vdmAuthState = AUTH_STATE_OFFLINE;
    lastHealthCheckAt = Date.now();
    updateIcon();
    return bridgeStatusSnapshot();
  }

  if (response.ok) {
    const payload = await response.json().catch(() => null);
    if (typeof payload?.sessionNonce === "string") {
      bridgeSessionNonce = payload.sessionNonce;
    }
    vdmConnected = payload?.authorized === true;
    vdmAuthState = vdmConnected ? AUTH_STATE_PAIRED : AUTH_STATE_INVALID;
    lastHealthCheckAt = Date.now();
    updateIcon();
    return bridgeStatusSnapshot();
  }

  if (response.status === 401 && allowAutoPair && await syncPairingFromVdm()) {
    return refreshVdmHealth({ allowAutoPair: false });
  }

  vdmConnected = false;
  vdmAuthState = AUTH_STATE_INVALID;
  lastHealthCheckAt = Date.now();
  updateIcon();
  return bridgeStatusSnapshot();
}

async function checkVdmHealth({ allowAutoPair = true, force = false } = {}) {
  const now = Date.now();
  if (!force && healthCheckPromise) {
    return healthCheckPromise;
  }
  if (!force && now - lastHealthCheckAt < HEALTH_CACHE_TTL_MS) {
    return bridgeStatusSnapshot();
  }

  healthCheckPromise = refreshVdmHealth({ allowAutoPair });
  try {
    return await healthCheckPromise;
  } finally {
    healthCheckPromise = null;
  }
}

async function markCaptureDelivered(preparedPayload, dedupeKey) {
  vdmConnected = true;
  vdmReachable = true;
  vdmAuthState = normalizedPairingCode() ? AUTH_STATE_PAIRED : AUTH_STATE_NONE;
  updateIcon();
  captureRecent.set(dedupeKey, Date.now());

  if (cachedSettings.notifyOnCapture) {
    chrome.notifications.create({
      type: "basic",
      iconUrl: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVQI12NgAAIABQ==",
      title: "Sent to VDM",
      message: preparedPayload.filename ?? preparedPayload.url,
    });
  }
}

// ---------------------------------------------------------------------------
// Capture: send a URL to VDM
// ---------------------------------------------------------------------------

/**
 * POST a capture payload to the VDM capture bridge.
 * Returns true on success, false on failure.
 */
async function sendToVdm(payload) {
  if (!cachedSettings.enabled) return false;

  const preparedPayload = await enrichCapturePayload(payload);

  const dedupeKey = normalizeCaptureKey(preparedPayload);
  pruneCaptureCache();
  if (captureInFlight.has(dedupeKey)) return true;
  if (captureRecent.has(dedupeKey)) return true;
  captureInFlight.add(dedupeKey);

  try {
    const needsFreshHealth =
      !vdmReachable ||
      !bridgeSessionNonce ||
      !vdmConnected ||
      vdmAuthState === AUTH_STATE_MISSING ||
      vdmAuthState === AUTH_STATE_INVALID;

    if (needsFreshHealth) {
      await checkVdmHealth({ force: true });
    }
    if (vdmAuthState === AUTH_STATE_MISSING || vdmAuthState === AUTH_STATE_INVALID) {
      return false;
    }

    const bodyText = JSON.stringify(preparedPayload);
    let resp = await fetchAuthorizedBridgeResponse("/capture", {
      method: "POST",
      bodyText,
      contentType: "application/json",
    });

    if (!resp) {
      await checkVdmHealth({ force: true });
      return false;
    }

    if (resp.ok) {
      await markCaptureDelivered(preparedPayload, dedupeKey);
      return true;
    }

    if (resp.status === 401) {
      await checkVdmHealth({ force: true });
      if (vdmAuthState === AUTH_STATE_PAIRED) {
        resp = await fetchAuthorizedBridgeResponse("/capture", {
          method: "POST",
          bodyText,
          contentType: "application/json",
        });
        if (resp?.ok) {
          await markCaptureDelivered(preparedPayload, dedupeKey);
          return true;
        }
      }
      return false;
    }

    await checkVdmHealth({ force: true });
    return false;
  } catch {
    await checkVdmHealth({ force: true });
    return false;
  } finally {
    captureInFlight.delete(dedupeKey);
  }
}

async function initializeBridgeIntegration({ force = false } = {}) {
  await loadSettings();
  return checkVdmHealth({ force });
}

async function focusVdmApp() {
  const response = await fetchBridge("/focus", { method: "GET" }).catch(() => null);
  if (!response) {
    return false;
  }
  if (response.ok) {
    await checkVdmHealth({ force: true });
  }
  return response.ok;
}

async function restoreBrowserDownload(url) {
  try {
    await chrome.downloads.download({ url, saveAs: false, conflictAction: "uniquify" });
  } catch {
    chrome.tabs.create({ url, active: false });
  }
}

function notifyBridgeFallback(title, message) {
  void chrome.notifications.create({
    type: "basic",
    iconUrl: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVQI12NgAAIABQ==",
    title,
    message,
  });
}

// ---------------------------------------------------------------------------
// Download interception — chrome.downloads.onCreated
// ---------------------------------------------------------------------------

chrome.downloads.onCreated.addListener(async (downloadItem) => {
  if (!cachedSettings.enabled) return;

  const { url, finalUrl, mime, filename, totalBytes, tabUrl } = downloadItem;
  const observedUrls = [finalUrl, url].filter((value) => typeof value === "string" && value.trim());
  const observedRequest = findObservedRequest(observedUrls);
  const observedResponseHeaders = findObservedResponseHeaders(observedUrls);
  const observedResponse = normalizeResponseHeaders(observedResponseHeaders);
  const effectiveMime = observedResponse?.contentType || mime || null;
  const effectiveSize = totalBytes > 0 ? totalBytes : observedResponse?.contentLength ?? null;
  const effectiveFilename = filename || observedResponse?.filename || null;

  const decision = classifyDownload({
    url,
    mime: effectiveMime,
    filename: effectiveFilename,
    fileSizeBytes: effectiveSize,
    settings: cachedSettings,
    context: "download-api",
    responseHeaders: observedResponseHeaders,
  });

  if (decision === "no") return;

  // Cancel the browser download immediately.
  chrome.downloads.cancel(downloadItem.id, async () => {
    // Erase the (cancelled) download item from the browser's download bar.
    chrome.downloads.erase({ id: downloadItem.id });

    const sent = await sendToVdm({
      url,
      referrer: observedRequest?.referrer ?? tabUrl ?? null,
      filename: effectiveFilename,
      sizeHint: effectiveSize,
      mime: effectiveMime,
      requestMethod: observedRequest?.requestMethod ?? "get",
      requestFormFields: observedRequest?.requestFormFields ?? [],
      source: "download-api",
    });

    if (!sent) {
      await restoreBrowserDownload(finalUrl || url);
    }
  });
});

// ---------------------------------------------------------------------------
// Context menu — "Download with VDM"
// ---------------------------------------------------------------------------

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "vdm-download-link",
    title: "Download with VDM",
    contexts: ["link"],
  });
  chrome.contextMenus.create({
    id: "vdm-download-image",
    title: "Download Image with VDM",
    contexts: ["image"],
  });
  chrome.contextMenus.create({
    id: "vdm-download-video",
    title: "Download Video with VDM",
    contexts: ["video", "audio"],
  });

  // Schedule periodic health check.
  chrome.alarms.create(HEALTH_CHECK_ALARM, {
    periodInMinutes: HEALTH_CHECK_PERIOD_MINUTES,
  });
  chrome.alarms.create(MAINTENANCE_ALARM, {
    periodInMinutes: MAINTENANCE_PERIOD_MINUTES,
  });

  void initializeBridgeIntegration();
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.linkUrl ?? info.srcUrl ?? info.pageUrl;
  if (!url) return;

  const sent = await sendToVdm({
    url,
    referrer: tab?.url ?? null,
    filename: null,
    sizeHint: null,
    mime: null,
    source: "context-menu",
  });

  if (!sent) {
    await restoreBrowserDownload(url);
    notifyBridgeFallback(
      "VDM unavailable",
      "The desktop app is offline, so the download continued in the browser.",
    );
  }
});

// ---------------------------------------------------------------------------
// Message bus — communication from content script and popup
// ---------------------------------------------------------------------------

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (msg.type === "intercept-link") {
    // Content script detected a download-link click.
    handleInterceptedLink(msg, sender).then(sendResponse);
    return true; // keep channel open for async response
  }

  if (msg.type === "get-status") {
    void (async () => {
      await initializeBridgeIntegration({ force: msg.force !== false });
      sendResponse(bridgeStatusSnapshot());
    })();
    return true;
  }

  if (msg.type === "set-enabled") {
    void (async () => {
      cachedSettings.enabled = !!msg.enabled;
      await saveGeneralSettings(cachedSettings);
      lastHealthCheckAt = 0;
      if (cachedSettings.enabled) {
        await checkVdmHealth({ force: true });
      } else {
        vdmConnected = false;
        updateIcon();
      }
      sendResponse({ ok: true, ...bridgeStatusSnapshot() });
    })();
    return true;
  }

  if (msg.type === "add-url") {
    sendToVdm({
      url: msg.url,
      referrer: msg.referrer ?? null,
      filename: msg.filename ?? null,
      sizeHint: msg.sizeHint ?? null,
      mime: null,
      source: "manual",
    }).then((ok) => sendResponse({ ok }));
    return true;
  }

  if (msg.type === "focus-app") {
    focusVdmApp().then((ok) => sendResponse({ ok }));
    return true;
  }
});

async function handleInterceptedLink(msg, sender) {
  const { url, referrer, filename } = msg;
  const observedResponseHeaders = findObservedResponseHeaders([url]);

  const decision = classifyDownload({
    url,
    mime: null,
    filename,
    fileSizeBytes: null, // unknown at link-click time
    settings: cachedSettings,
    context: "link-click",
    referrerUrl: referrer ?? sender.tab?.url ?? null,
    explicitDownload: !!msg.downloadAttribute,
    mediaHint: !!msg.mediaHint,
    responseHeaders: observedResponseHeaders,
  });

  if (decision !== "yes") return { intercepted: false };

  const sent = await sendToVdm({
    url,
    referrer: referrer ?? sender.tab?.url ?? null,
    filename: filename ?? null,
    sizeHint: null,
    mime: null,
    source: "link-click",
  });

  return { intercepted: sent, shouldPrevent: decision === "yes" };
}

// ---------------------------------------------------------------------------
// Alarms — periodic health checks
// ---------------------------------------------------------------------------

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === HEALTH_CHECK_ALARM) checkVdmHealth({ force: true });
  if (alarm.name === MAINTENANCE_ALARM) {
    pruneCaptureCache();
    pruneTimedCache(observedRequests, OBSERVED_REQUEST_TTL_MS);
    pruneTimedCache(observedResponses, OBSERVED_RESPONSE_TTL_MS);
  }
});

chrome.runtime.onStartup?.addListener(() => {
  void initializeBridgeIntegration();
});

// ---------------------------------------------------------------------------
// Service worker activation
// ---------------------------------------------------------------------------

self.addEventListener("activate", () => {
  void initializeBridgeIntegration();
});
