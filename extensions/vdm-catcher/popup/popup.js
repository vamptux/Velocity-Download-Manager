/**
 * VDM Catcher — Popup Script
 */

// DOM refs
// ---------------------------------------------------------------------------

const toggleInput = /** @type {HTMLInputElement} */ (document.getElementById("toggle-enabled"));
const statusBanner = document.getElementById("status-banner");
const statusDot = document.getElementById("status-dot");
const statusText = document.getElementById("status-text");
const manualUrl = /** @type {HTMLInputElement} */ (document.getElementById("manual-url"));
const btnAdd = document.getElementById("btn-add");
const recentList = document.getElementById("recent-list");
const STATUS_POLL_MS = 5000;
let currentStatus = { enabled: true, connected: false, reachable: false, authState: "offline" };
let statusResetTimer = null;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function applyStatusTone(tone) {
  statusBanner.className = `status-banner status-${tone}`;
  statusDot.className = `status-dot dot-${tone}`;
}

function setStatus(connected, enabled, reachable = connected, authState = "none", overrideText = null, overrideTone = null) {
  let tone = overrideTone;
  let text = overrideText;

  if (!enabled) {
    tone = "disabled";
    text = "Disabled — not intercepting";
  } else if (!reachable) {
    tone = "error";
    text = "Waiting for VDM desktop app";
  } else if (authState === "missing") {
    tone = "checking";
    text = "Preparing secure bridge…";
  } else if (authState === "invalid") {
    tone = "checking";
    text = "Refreshing secure bridge…";
  } else if (!tone) {
    tone = connected ? "ok" : "checking";
    text = connected ? "VDM ready" : "Checking bridge…";
  }

  applyStatusTone(tone);
  statusText.textContent = text ?? "Checking connection…";
}

function clearTransientStatusTimer() {
  if (statusResetTimer !== null) {
    window.clearTimeout(statusResetTimer);
    statusResetTimer = null;
  }
}

function setTransientStatus(message, tone, ttlMs = 2200) {
  clearTransientStatusTimer();
  setStatus(currentStatus.connected, currentStatus.enabled, currentStatus.reachable, currentStatus.authState, message, tone);
  if (ttlMs > 0) {
    statusResetTimer = window.setTimeout(() => {
      statusResetTimer = null;
      setStatus(currentStatus.connected, currentStatus.enabled, currentStatus.reachable, currentStatus.authState);
    }, ttlMs);
  }
}

function truncate(str, max = 48) {
  if (str.length <= max) return str;
  return str.slice(0, max - 1) + "…";
}

function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return null;
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const precision = value >= 100 || unitIndex === 0 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(precision)} ${units[unitIndex]}`;
}

function sourceLabel(source) {
  switch (source) {
    case "download-api":
      return "Browser";
    case "context-menu":
      return "Context";
    case "link-click":
      return "Link";
    case "manual":
      return "Manual";
    default:
      return null;
  }
}

/** Returns { ext, color } for a file extension, or null if indeterminate. */
function getExtBadge(filename) {
  if (!filename) return null;
  const ext = filename.split(".").pop()?.toLowerCase();
  if (!ext || ext.length > 5) return null;
  const catColors = [
    [["zip", "rar", "7z", "tar", "gz", "bz2", "xz"], "#f4a252"],
    [["exe", "msi", "dmg", "pkg", "deb", "rpm", "apk"], "#4fa8e8"],
    [["mp4", "mkv", "mov", "avi", "webm", "m4v"], "#e07070"],
    [["mp3", "flac", "wav", "ogg", "m4a", "aac"], "#a47fd4"],
    [["jpg", "jpeg", "png", "gif", "bmp", "webp", "svg"], "#4ec87a"],
    [["pdf", "doc", "docx", "txt", "md"], "#8ab0e0"],
  ];
  for (const [exts, color] of catColors) {
    if (exts.includes(ext)) return { ext, color };
  }
  return { ext, color: "#8ea2ca" };
}

async function loadRecent() {
  const { recentCaptures = [] } = await chrome.storage.session.get("recentCaptures");
  recentList.innerHTML = "";
  if (recentCaptures.length === 0) {
    const li = document.createElement("li");
    li.className = "empty-state";
    li.textContent = "No captures yet this session.";
    recentList.appendChild(li);
    return;
  }
  for (const item of [...recentCaptures].reverse()) {
    const li = document.createElement("li");
    li.className = "recent-item";
    li.title = item.url;

    const filename = item.filename ?? new URL(item.url).pathname.split("/").pop() ?? item.url;
    const badge = getExtBadge(filename);

    if (badge) {
      const extEl = document.createElement("span");
      extEl.className = "recent-ext-badge";
      extEl.textContent = badge.ext;
      extEl.style.color = badge.color;
      extEl.style.borderColor = `${badge.color}44`;
      extEl.style.background = `${badge.color}14`;
      li.appendChild(extEl);
    }

    const name = document.createElement("span");
    name.className = "recent-name";
    name.textContent = truncate(filename);

    const time = document.createElement("span");
    time.className = "recent-time";
    time.textContent = new Date(item.ts).toLocaleTimeString();

    const textWrap = document.createElement("div");
    textWrap.className = "recent-text";

    const mainRow = document.createElement("div");
    mainRow.className = "recent-main";
    mainRow.append(name, time);

    const metaRow = document.createElement("div");
    metaRow.className = "recent-meta";
    const source = sourceLabel(item.source);
    const sizeLabel = formatBytes(Number(item.sizeHint ?? 0));
    if (source) {
      const sourceEl = document.createElement("span");
      sourceEl.className = "recent-source";
      sourceEl.textContent = source;
      metaRow.appendChild(sourceEl);
    }
    if (sizeLabel) {
      const sizeEl = document.createElement("span");
      sizeEl.className = "recent-size";
      sizeEl.textContent = sizeLabel;
      metaRow.appendChild(sizeEl);
    }
    if (metaRow.childElementCount > 0) {
      textWrap.append(mainRow, metaRow);
    } else {
      textWrap.append(mainRow);
    }

    li.append(textWrap);
    recentList.appendChild(li);
  }
}

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

async function init() {
  await refreshStatus();
  await loadRecent();
}

async function refreshStatus() {
  let swStatus = { enabled: true, connected: false, reachable: false, authState: "offline" };
  try {
    swStatus = await chrome.runtime.sendMessage({ type: "get-status" });
  } catch {
    /* SW may be restarting */
  }

  currentStatus = {
    enabled: !!swStatus.enabled,
    connected: !!swStatus.connected,
    reachable: !!swStatus.reachable,
    authState: String(swStatus.authState ?? "offline"),
  };
  toggleInput.checked = currentStatus.enabled;
  setStatus(currentStatus.connected, currentStatus.enabled, currentStatus.reachable, currentStatus.authState);
}

async function focusVdmApp() {
  try {
    const response = await chrome.runtime.sendMessage({ type: "focus-app" });
    if (response?.ok) {
      await refreshStatus();
      return true;
    }
  } catch {
    /* ignore and use fallback below */
  }

  chrome.tabs.create({ url: "http://127.0.0.1:17780/health", active: true });
  return false;
}

async function openOptionsPage() {
  try {
    await chrome.runtime.openOptionsPage();
  } catch {
    chrome.tabs.create({ url: chrome.runtime.getURL("options/options.html"), active: true });
  }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

toggleInput.addEventListener("change", () => {
  currentStatus.enabled = toggleInput.checked;
  chrome.runtime.sendMessage({ type: "set-enabled", enabled: toggleInput.checked });
  clearTransientStatusTimer();
  setStatus(currentStatus.connected, currentStatus.enabled, currentStatus.reachable, currentStatus.authState);
});

btnAdd.addEventListener("click", async () => {
  const url = manualUrl.value.trim();
  if (!url) return;
  try {
    new URL(url); // basic validation
  } catch {
    manualUrl.classList.add("input-error");
    return;
  }
  manualUrl.classList.remove("input-error");
  btnAdd.disabled = true;
  manualUrl.disabled = true;
  btnAdd.textContent = "…";
  setStatus(
    currentStatus.connected,
    currentStatus.enabled,
    currentStatus.reachable,
    currentStatus.authState,
    "Sending capture to VDM…",
    "checking",
  );
  try {
    const resp = await chrome.runtime.sendMessage({ type: "add-url", url });
    if (resp?.ok) {
      manualUrl.value = "";
      await refreshStatus();
      await loadRecent();
      setTransientStatus("Sent to VDM — capture queued", "ok");
    } else {
      await refreshStatus();
      if (!currentStatus.reachable) {
        setTransientStatus("Send failed — start VDM first", "error", 2800);
      } else if (currentStatus.authState === "missing" || currentStatus.authState === "invalid") {
        setTransientStatus("Bridge is reconnecting — try again in a moment", "checking", 2800);
      } else {
        setTransientStatus("Send failed — secure bridge unavailable", "error", 2800);
      }
    }
  } catch {
    await refreshStatus();
    setTransientStatus("Extension bridge restarted — try again", "error", 2800);
  } finally {
    btnAdd.disabled = false;
    manualUrl.disabled = false;
    btnAdd.textContent = "Send";
  }
});

manualUrl.addEventListener("input", () => {
  manualUrl.classList.remove("input-error");
});

manualUrl.addEventListener("keydown", (e) => {
  if (e.key === "Enter") btnAdd.click();
});

document.getElementById("open-vdm")?.addEventListener("click", async (e) => {
  e.preventDefault();
  const focused = await focusVdmApp();
  if (!focused) {
    setTransientStatus("VDM is offline — start the desktop app first", "error", 2600);
  }
});

document.getElementById("open-options")?.addEventListener("click", async (e) => {
  e.preventDefault();
  await openOptionsPage();
});

init();
setInterval(() => {
  void refreshStatus();
}, STATUS_POLL_MS);
