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
    text = "Finalizing secure bridge…";
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

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

async function init() {
  await refreshStatus();
}

async function refreshStatus() {
  let swStatus = { enabled: true, connected: false, reachable: false, authState: "offline" };
  try {
    swStatus = await chrome.runtime.sendMessage({ type: "get-status", force: true });
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
    /* ignore and fall through */
  }
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
