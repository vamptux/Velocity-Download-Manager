export const DEFAULT_SETTINGS = {
  enabled: true,
  minSizeBytes: 1 * 1024 * 1024,
  blockedHosts: [],
  notifyOnCapture: false,
};

export const DEFAULT_BRIDGE_STATE = {
  bridgePairingCode: "",
  bridgePairingRotatedAt: 0,
  bridgePairingSyncedAt: 0,
};

const LEGACY_SYNC_DEFAULTS = {
  ...DEFAULT_SETTINGS,
  bridgePairingCode: "",
};

function normalizeBlockedHosts(value) {
  if (!Array.isArray(value)) {
    return [];
  }

  return value
    .map((entry) => String(entry ?? "").trim())
    .filter(Boolean);
}

function normalizeBridgeState(raw, legacyPairingCode = "") {
  const pairingCode = String(raw?.bridgePairingCode ?? legacyPairingCode ?? "").trim();
  const bridgePairingRotatedAt = Number(raw?.bridgePairingRotatedAt ?? 0);
  const bridgePairingSyncedAt = Number(raw?.bridgePairingSyncedAt ?? 0);

  return {
    bridgePairingCode: pairingCode,
    bridgePairingRotatedAt: Number.isFinite(bridgePairingRotatedAt) ? bridgePairingRotatedAt : 0,
    bridgePairingSyncedAt: Number.isFinite(bridgePairingSyncedAt) ? bridgePairingSyncedAt : 0,
  };
}

export async function loadExtensionSettings() {
  const [syncStored, localStored] = await Promise.all([
    chrome.storage.sync.get(LEGACY_SYNC_DEFAULTS),
    chrome.storage.local.get(DEFAULT_BRIDGE_STATE),
  ]);

  const legacyPairingCode = String(syncStored.bridgePairingCode ?? "").trim();
  const bridgeState = normalizeBridgeState(localStored, legacyPairingCode);
  if (!String(localStored.bridgePairingCode ?? "").trim() && legacyPairingCode) {
    await chrome.storage.local.set(bridgeState);
    await chrome.storage.sync.remove("bridgePairingCode");
  }

  const minSizeBytes = Number(syncStored.minSizeBytes ?? DEFAULT_SETTINGS.minSizeBytes);

  return {
    ...DEFAULT_SETTINGS,
    ...syncStored,
    minSizeBytes: Number.isFinite(minSizeBytes) ? minSizeBytes : DEFAULT_SETTINGS.minSizeBytes,
    blockedHosts: normalizeBlockedHosts(syncStored.blockedHosts),
    ...bridgeState,
  };
}

export async function saveGeneralSettings(settings) {
  const minSizeBytes = Number(settings?.minSizeBytes ?? DEFAULT_SETTINGS.minSizeBytes);
  await chrome.storage.sync.set({
    enabled: !!settings?.enabled,
    minSizeBytes: Number.isFinite(minSizeBytes) ? Math.max(0, minSizeBytes) : DEFAULT_SETTINGS.minSizeBytes,
    blockedHosts: normalizeBlockedHosts(settings?.blockedHosts),
    notifyOnCapture: !!settings?.notifyOnCapture,
  });
}

export async function saveBridgeState(update) {
  const current = await chrome.storage.local.get(DEFAULT_BRIDGE_STATE);
  const nextState = normalizeBridgeState({
    ...current,
    ...update,
  });
  await chrome.storage.local.set(nextState);
  return nextState;
}