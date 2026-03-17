export type ThemeId = "graphite" | "midnight" | "carbon" | "slate" | "dusk";
export type AccentId = "blue" | "teal" | "indigo" | "violet" | "sage" | "rose";
export type DensityId = "default" | "compact" | "cozy";

export interface UiPrefs {
  theme: ThemeId;
  accent: AccentId;
  density: DensityId;
  showStatusBar: boolean;
  smoothAnimations: boolean;
}

export const DEFAULT_UI_PREFS: UiPrefs = {
  theme: "carbon",
  accent: "blue",
  density: "cozy",
  showStatusBar: true,
  smoothAnimations: true,
};

const LS_KEY = "vdm-ui-prefs";

export function loadUiPrefs(): UiPrefs {
  try {
    const stored = localStorage.getItem(LS_KEY);
    if (stored) return { ...DEFAULT_UI_PREFS, ...JSON.parse(stored) };
  } catch {
    /* ignore */
  }
  return { ...DEFAULT_UI_PREFS };
}

export function saveUiPrefs(prefs: UiPrefs): void {
  try {
    localStorage.setItem(LS_KEY, JSON.stringify(prefs));
  } catch {
    /* ignore */
  }
}

export function applyUiPrefs(prefs: UiPrefs): void {
  const el = document.documentElement;
  el.dataset.theme = prefs.theme;
  el.dataset.accent = prefs.accent;
  el.dataset.density = prefs.density;
  el.style.zoom = ""; // clear any legacy inline zoom
  if (prefs.smoothAnimations) {
    delete el.dataset.reducedMotion;
  } else {
    el.dataset.reducedMotion = "true";
  }
}
