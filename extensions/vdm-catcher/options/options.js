/**
 * VDM Catcher — Options Page Script
 */

import { DEFAULT_SETTINGS, loadExtensionSettings, saveGeneralSettings } from "../shared/settings.js";

const elEnabled = /** @type {HTMLInputElement} */ (document.getElementById("opt-enabled"));
const elNotify  = /** @type {HTMLInputElement} */ (document.getElementById("opt-notify"));
const elMinSize = /** @type {HTMLInputElement} */ (document.getElementById("opt-min-size"));
const elBlocklist = /** @type {HTMLTextAreaElement} */ (document.getElementById("opt-blocklist"));
const elPairingStatus = document.getElementById("pairing-status");
const bridgeStatusDetail = document.getElementById("bridge-status-detail");
const btnFocusVdm = document.getElementById("btn-focus-vdm");
const btnSave   = document.getElementById("btn-save");
const saveStatus = document.getElementById("save-status");
let saveStatusTimer = null;
let currentBridgeStatus = { enabled: true, connected: false, reachable: false, authState: "offline", pairingSyncedAt: 0 };

function setSaveStatus(message, tone = "default") {
  if (!saveStatus) {
    return;
  }

  if (saveStatusTimer !== null) {
    window.clearTimeout(saveStatusTimer);
    saveStatusTimer = null;
  }

  saveStatus.textContent = message;
  saveStatus.dataset.tone = tone;
  if (!message) {
    return;
  }

  saveStatusTimer = window.setTimeout(() => {
    saveStatus.textContent = "";
    saveStatus.dataset.tone = "default";
    saveStatusTimer = null;
  }, 2600);
}

function formatTimestamp(value) {
  if (!Number.isFinite(value) || value <= 0) {
    return null;
  }

  return new Date(value).toLocaleString();
}

function updatePairingStatus() {
  if (!elPairingStatus) {
    return;
  }

  if (!currentBridgeStatus.reachable) {
    elPairingStatus.dataset.state = "missing";
    elPairingStatus.textContent = "VDM desktop app is offline.";
    if (bridgeStatusDetail) {
      bridgeStatusDetail.textContent = "Start the app and the extension will reconnect automatically over the local bridge.";
    }
    return;
  }

  if (currentBridgeStatus.authState === "invalid") {
    elPairingStatus.dataset.state = "missing";
    elPairingStatus.textContent = "VDM is online. The secure bridge is refreshing.";
    if (bridgeStatusDetail) {
      bridgeStatusDetail.textContent = "This usually resolves on its own after the next automatic bridge refresh.";
    }
    return;
  }

  if (currentBridgeStatus.authState === "missing") {
    elPairingStatus.dataset.state = "missing";
    elPairingStatus.textContent = "VDM is online. The secure bridge is finalizing.";
    if (bridgeStatusDetail) {
      bridgeStatusDetail.textContent = "The extension is syncing the local bridge secret and should be ready in a moment.";
    }
    return;
  }

  if (currentBridgeStatus.connected) {
    const syncedAt = formatTimestamp(currentBridgeStatus.pairingSyncedAt);
    elPairingStatus.dataset.state = "ready";
    elPairingStatus.textContent = syncedAt
      ? `VDM detected. Secure bridge active since ${syncedAt}.`
      : "VDM detected. Secure bridge active.";
    if (bridgeStatusDetail) {
      bridgeStatusDetail.textContent = "The extension will hand off captures automatically whenever the desktop app is reachable.";
    }
    return;
  }

  elPairingStatus.dataset.state = "missing";
  elPairingStatus.textContent = "Checking bridge status…";
  if (bridgeStatusDetail) {
    bridgeStatusDetail.textContent = "The extension is probing the local bridge in the background.";
  }
}

async function refreshBridgeStatus() {
  try {
    const status = await chrome.runtime.sendMessage({ type: "get-status", force: true });
    currentBridgeStatus = {
      enabled: !!status?.enabled,
      connected: !!status?.connected,
      reachable: !!status?.reachable,
      authState: String(status?.authState ?? "offline"),
      pairingSyncedAt: Number(status?.pairingSyncedAt ?? 0) || 0,
    };
  } catch {
    currentBridgeStatus = {
      enabled: true,
      connected: false,
      reachable: false,
      authState: "offline",
      pairingSyncedAt: 0,
    };
  }

  updatePairingStatus();
}

async function load() {
  const stored = await loadExtensionSettings();
  elEnabled.checked = !!stored.enabled;
  elNotify.checked  = !!stored.notifyOnCapture;
  elMinSize.value   = String((stored.minSizeBytes ?? DEFAULT_SETTINGS.minSizeBytes) / (1024 * 1024));
  elBlocklist.value = (stored.blockedHosts ?? []).join("\n");
  await refreshBridgeStatus();
  updatePairingStatus();
}

btnSave?.addEventListener("click", async () => {
  const minMb = parseFloat(elMinSize.value);
  const settings = {
    enabled:           elEnabled.checked,
    notifyOnCapture:   elNotify.checked,
    minSizeBytes:      isNaN(minMb) ? DEFAULT_SETTINGS.minSizeBytes : Math.max(0, minMb * 1024 * 1024),
    blockedHosts:      elBlocklist.value
                         .split("\n")
                         .map((l) => l.trim())
                         .filter(Boolean),
  };
  await saveGeneralSettings(settings);
  setSaveStatus("General settings saved.");
});

btnFocusVdm?.addEventListener("click", async () => {
  try {
    const response = await chrome.runtime.sendMessage({ type: "focus-app" });
    if (response?.ok) {
      await refreshBridgeStatus();
      setSaveStatus("VDM focused.");
      return;
    }
  } catch {
    // fall through to status hint below
  }

  await refreshBridgeStatus();
  setSaveStatus("VDM is offline. Start the desktop app first.", "warning");
});

load();
window.setInterval(() => {
  void refreshBridgeStatus();
}, 4000);
