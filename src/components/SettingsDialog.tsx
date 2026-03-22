import { useEffect, useMemo, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { AlertTriangle, AlignCenter, AlignJustify, Check, Cpu, Globe, LayoutGrid, Palette, X } from "lucide-react";
import { InlineNotice } from "@/components/ui/inline-notice";
import { cn } from "@/lib/utils";
import {
  type AccentId,
  type DensityId,
  type ThemeId,
  type UiPrefs,
  DEFAULT_UI_PREFS,
  applyUiPrefs,
  loadUiPrefs,
  saveUiPrefs,
} from "@/lib/uiPrefs";
import type { AppUpdateChannel, EngineSettings, TrafficMode } from "@/types/download";

/* ─── THEME METADATA ────────────────────────────────────────────────────── */

interface ThemeMeta {
  id: ThemeId;
  label: string;
  bg: string;
  sidebar: string;
  border: string;
}

const THEMES: ThemeMeta[] = [
  { id: "carbon",   label: "Carbon",   bg: "hsl(0,0%,7%)",       sidebar: "hsl(0,0%,4%)",        border: "hsl(0,0%,14%)"    },
  { id: "graphite", label: "Graphite", bg: "hsl(0,0%,10%)",      sidebar: "hsl(0,0%,7%)",        border: "hsl(0,0%,20%)"    },
  { id: "midnight", label: "Midnight", bg: "hsl(220,35%,8.5%)",  sidebar: "hsl(220,38%,6%)",     border: "hsl(220,28%,17%)" },
  { id: "slate",    label: "Slate",    bg: "hsl(215,18%,11%)",   sidebar: "hsl(215,22%,8%)",     border: "hsl(215,20%,20%)" },
  { id: "dusk",     label: "Dusk",     bg: "hsl(265,14%,10%)",   sidebar: "hsl(265,18%,7%)",     border: "hsl(265,15%,20%)" },
];

interface AccentMeta {
  id: AccentId;
  label: string;
  color: string;
}

const ACCENTS: AccentMeta[] = [
  { id: "blue",   label: "Blue",   color: "hsl(215,60%,56%)" },
  { id: "teal",   label: "Teal",   color: "hsl(175,55%,46%)" },
  { id: "indigo", label: "Indigo", color: "hsl(238,58%,64%)" },
  { id: "violet", label: "Violet", color: "hsl(265,52%,62%)" },
  { id: "sage",   label: "Sage",   color: "hsl(145,38%,48%)" },
  { id: "rose",   label: "Rose",   color: "hsl(0,62%,60%)"   },
];

const DENSITIES: Array<{ id: DensityId; label: string; hint: string }> = [
  { id: "compact", label: "Compact", hint: "Smaller" },
  { id: "default", label: "Default", hint: "Standard" },
  { id: "cozy",    label: "Cozy",    hint: "Larger" },
];

/* ─── SMALL UI HELPERS ───────────────────────────────────────────────────── */

/** Mini theme preview card used in the card-grid theme picker */
function ThemeCard({
  theme,
  accentColor,
  selected,
  onClick,
}: {
  theme: ThemeMeta;
  accentColor: string;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={theme.label}
      className={cn(
        "group relative flex flex-col overflow-hidden rounded-md border transition-all duration-150 focus-visible:outline-none",
        selected
          ? "border-[hsl(var(--primary)/0.55)] ring-1 ring-[hsl(var(--primary)/0.22)] shadow-[0_0_10px_hsl(var(--primary)/0.1)]"
          : "border-border/50 hover:border-border/80",
      )}
      style={{ background: theme.bg }}
    >
      {/* Mini sidebar + content preview */}
      <div className="flex h-[40px] w-full">
        <div className="w-[12px] shrink-0" style={{ background: theme.sidebar }} />
        <div className="flex flex-1 flex-col gap-[3px] p-[5px]">
          {/* fake title bar */}
          <div className="h-[3px] w-[60%] rounded-[1px]" style={{ background: `hsl(0 0% 100% / 0.08)` }} />
          {/* fake rows */}
          <div className="flex gap-[2px]">
            <div className="h-[2.5px] w-[5px] rounded-[1px]" style={{ background: accentColor, opacity: 0.85 }} />
            <div className="h-[2.5px] flex-1 rounded-[1px]" style={{ background: `hsl(0 0% 100% / 0.07)` }} />
          </div>
          <div className="flex gap-[2px]">
            <div className="h-[2.5px] w-[5px] rounded-[1px]" style={{ background: `hsl(0 0% 100% / 0.06)` }} />
            <div className="h-[2.5px] flex-1 rounded-[1px]" style={{ background: `hsl(0 0% 100% / 0.05)` }} />
          </div>
          {/* accent bar at bottom */}
          <div className="mt-auto h-[2px] w-full rounded-[1px]" style={{ background: theme.border }} />
        </div>
      </div>
      {/* Label row */}
      <div
        className="flex items-center justify-between px-[6px] py-[4px]"
        style={{ background: theme.sidebar, borderTop: `1px solid ${theme.border}` }}
      >
        <span
          className="text-[9.5px] font-semibold tracking-wide"
          style={{ color: `hsl(0 0% 100% / 0.55)` }}
        >
          {theme.label}
        </span>
        {selected && (
          <Check size={8} strokeWidth={3} style={{ color: accentColor }} />
        )}
      </div>
    </button>
  );
}

function AccentSwatch({ accent, selected, onClick }: {
  accent: AccentMeta;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      title={accent.label}
      onClick={onClick}
      className={cn(
        "relative flex h-[28px] w-[28px] shrink-0 items-center justify-center rounded-full transition-all duration-150",
        selected
          ? "ring-2 ring-offset-2 ring-offset-[hsl(var(--background))] scale-110"
          : "hover:scale-110 opacity-70 hover:opacity-100",
      )}
      style={{
        background: accent.color,
        ...(selected ? { boxShadow: `0 2px 8px ${accent.color}55` } : {}),
      }}
    >
      {selected && <Check size={10} strokeWidth={3} className="text-white drop-shadow" />}
    </button>
  );
}
const ACTIVE_OPTIONS = [1, 2, 3, 4, 5, 6] as const;

const CHECKPOINT_PRESETS = [
  { label: "Fast",    hint: "250 ms",  min: 250,  max: 900  },
  { label: "Normal",  hint: "500 ms",  min: 500,  max: 1500 },
  { label: "Relaxed", hint: "1500 ms", min: 1500, max: 3500 },
] as const;

function activeCheckpointPreset(min: number, max: number): number {
  return CHECKPOINT_PRESETS.findIndex((p) => p.min === min && p.max === max);
}

const TRAFFIC_OPTIONS: Array<{ value: TrafficMode; label: string; hint: string }> = [
  { value: "low",    label: "Low",    hint: "5 MiB/s"   },
  { value: "medium", label: "Medium", hint: "12 MiB/s"  },
  { value: "high",   label: "High",   hint: "100 MiB/s" },
  { value: "max",    label: "Max",    hint: "Unlimited" },
];

const UPDATE_CHANNEL_OPTIONS: Array<{ value: AppUpdateChannel; label: string; hint: string }> = [
  { value: "stable", label: "Stable", hint: "Production releases only" },
  { value: "preview", label: "Preview", hint: "Early builds with fallback to stable on failure" },
];

type SettingsPage = "appearance" | "engine" | "browser";

const PAGES: Array<{ id: SettingsPage; label: string; icon: React.ElementType }> = [
  { id: "appearance", label: "Appearance",          icon: Palette },
  { id: "engine",     label: "Download Engine",     icon: Cpu     },
  { id: "browser",    label: "Browser Integration", icon: Globe   },
];

/* ─── REUSABLE CONTROLS ──────────────────────────────────────────────────── */

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-md border px-2.5 py-1.5 text-center text-[12px] font-medium transition-all duration-150",
        active
          ? "border-[hsl(var(--primary)/0.4)] bg-[hsl(var(--primary)/0.12)] text-foreground"
          : "border-border/60 bg-black/10 text-foreground/55 hover:border-border/90 hover:text-foreground/80 hover:bg-accent/5",
      )}
    >
      {children}
    </button>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-2 mt-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/40">{children}</div>
  );
}

function PillToggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative h-[22px] w-10 shrink-0 rounded-full border shadow-inner transition-colors duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background",
        checked
          ? "border-[hsl(var(--primary)/0.6)] bg-[hsl(var(--primary)/0.3)] shadow-[inset_0_1px_2px_rgba(0,0,0,0.2)]"
          : "border-border/60 bg-black/30 shadow-[inset_0_1px_2px_rgba(0,0,0,0.15)]",
      )}
    >
      <span
        className={cn(
          "absolute top-[2px] h-4 w-4 rounded-full transition-all duration-200 shadow-[0_1px_2px_rgba(0,0,0,0.25)]",
          checked ? "left-[19px] bg-[hsl(var(--primary))] shadow-[0_0_8px_hsl(var(--primary)/0.5)]" : "left-[3px] bg-muted-foreground/40",
        )}
      />
    </button>
  );
}

function AppearanceRow({
  label,
  description,
  control,
}: {
  label: string;
  description?: string;
  control: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4 py-2.5 px-1">
      <div className="min-w-0">
        <div className="text-[12.5px] font-medium text-foreground/88">{label}</div>
        {description && <div className="mt-0.5 text-[11px] text-muted-foreground/50">{description}</div>}
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  );
}

/* ─── PROPS + COMPONENT ──────────────────────────────────────────────────── */

interface SettingsDialogProps {
  open: boolean;
  settings: EngineSettings;
  saving: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onSave: (settings: EngineSettings) => Promise<void> | void;
  onClearError?: () => void;
  onUiPrefsChange?: (prefs: UiPrefs) => void;
}

export function SettingsDialog({
  open,
  settings,
  saving,
  error,
  onOpenChange,
  onSave,
  onClearError,
  onUiPrefsChange,
}: SettingsDialogProps) {
  const [draft, setDraft] = useState<EngineSettings>(settings);
  const [page, setPage] = useState<SettingsPage>("appearance");
  const [uiPrefs, setUiPrefsState] = useState<UiPrefs>(() => ({ ...DEFAULT_UI_PREFS, ...loadUiPrefs() }));

  function updateUiPrefs(patch: Partial<UiPrefs>) {
    setUiPrefsState((prev) => {
      const next = { ...prev, ...patch };
      saveUiPrefs(next);
      applyUiPrefs(next);
      onUiPrefsChange?.(next);
      return next;
    });
  }

  useEffect(() => {
    if (open) {
      setDraft(settings);
      setPage("appearance");
      const prefs = { ...DEFAULT_UI_PREFS, ...loadUiPrefs() };
      setUiPrefsState(prefs);
    }
  }, [open, settings]);

  const isDirty = useMemo(
    () => JSON.stringify(draft) !== JSON.stringify(settings),
    [draft, settings],
  );

  // Current accent color for theme preview stripe
  const currentAccentColor = ACCENTS.find((a) => a.id === uiPrefs.accent)?.color ?? ACCENTS[0].color;

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-black/55 backdrop-blur-[2px] data-[state=open]:animate-in data-[state=open]:fade-in-0" />
        <Dialog.Content
          className={cn(
            "fixed left-1/2 top-1/2 z-50 w-[645px] max-w-[calc(100vw-24px)] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-border bg-[linear-gradient(180deg,hsl(var(--card)),hsl(var(--background)))] shadow-2xl shadow-black/60 outline-none",
            "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
          )}
        >
          {/* Header */}
          <div className="flex items-center justify-between border-b border-border/60 px-4 py-3">
            <div className="flex items-center gap-2">
              <Dialog.Title className="text-[13px] font-semibold text-foreground/88">Settings</Dialog.Title>
              {isDirty && page !== "appearance" && (
                <span className="h-[6px] w-[6px] rounded-full bg-[hsl(var(--status-paused)/0.85)]" title="Unsaved engine changes" />
              )}
            </div>
            <Dialog.Close className="flex h-6 w-6 items-center justify-center rounded text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground">
              <X size={13} strokeWidth={2} />
            </Dialog.Close>
          </div>

          {/* Body — fixed height prevents resize jump between pages */}
          <div className="flex" style={{ height: "420px" }}>
            {/* Sidebar nav */}
            <nav className="flex w-[188px] shrink-0 flex-col gap-0.5 border-r border-border/40 p-3">
              {PAGES.map(({ id, label, icon: Icon }) => (
                <button
                  key={id}
                  type="button"
                  onClick={() => setPage(id)}
                  className={cn(
                    "relative flex items-center gap-2.5 rounded-md px-3 py-2 text-left text-[12.5px] font-medium transition-colors",
                    page === id
                      ? "bg-white/[0.07] text-foreground/90"
                      : "text-muted-foreground/50 hover:bg-white/[0.04] hover:text-foreground/75",
                  )}
                >
                  <span
                    className={cn(
                      "absolute left-0 top-1/2 -translate-y-1/2 h-4 w-[3px] rounded-r transition-all duration-200",
                      page === id ? "bg-[hsl(var(--primary)/0.75)] opacity-100" : "opacity-0",
                    )}
                  />
                  <Icon size={14} strokeWidth={page === id ? 2.1 : 1.7} className="shrink-0" />
                  {label}
                </button>
              ))}
            </nav>

            {/* Page content */}
            <div className="flex-1 overflow-y-auto px-5 py-4">

              {/* ── APPEARANCE ─────────────────────────────────────────── */}
              {page === "appearance" && (
                <div className="flex flex-col gap-5">

                  {/* Theme */}
                  <div>
                    <div className="mb-2.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/40">Theme</div>
                    <div className="grid grid-cols-5 gap-2">
                      {THEMES.map((theme) => (
                        <ThemeCard
                          key={theme.id}
                          theme={theme}
                          accentColor={currentAccentColor}
                          selected={uiPrefs.theme === theme.id}
                          onClick={() => updateUiPrefs({ theme: theme.id })}
                        />
                      ))}
                    </div>
                  </div>

                  {/* Accent */}
                  <div>
                    <div className="mb-2.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/40">Accent color</div>
                    <div className="flex items-center gap-3">
                      {ACCENTS.map((accent) => (
                        <AccentSwatch
                          key={accent.id}
                          accent={accent}
                          selected={uiPrefs.accent === accent.id}
                          onClick={() => updateUiPrefs({ accent: accent.id })}
                        />
                      ))}
                      <span className="ml-0.5 text-[11px] font-medium text-muted-foreground/45">
                        {ACCENTS.find((a) => a.id === uiPrefs.accent)?.label}
                      </span>
                    </div>
                  </div>

                  {/* Density */}
                  <div>
                    <div className="mb-2.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted-foreground/40">Density</div>
                    <div className="flex gap-2">
                      {DENSITIES.map((d) => {
                        const DensityIcon = d.id === "compact" ? AlignJustify : d.id === "cozy" ? LayoutGrid : AlignCenter;
                        const isActive = uiPrefs.density === d.id;
                        return (
                          <button
                            key={d.id}
                            type="button"
                            onClick={() => updateUiPrefs({ density: d.id })}
                            className={cn(
                              "flex flex-1 flex-col items-center gap-1.5 rounded-md border px-3 py-2.5 text-center transition-all duration-150",
                              isActive
                                ? "border-[hsl(var(--primary)/0.45)] bg-[hsl(var(--primary)/0.08)] text-foreground"
                                : "border-border/45 bg-black/[0.08] text-foreground/50 hover:border-border/70 hover:text-foreground/75 hover:bg-white/[0.02]",
                            )}
                          >
                            <DensityIcon
                              size={13}
                              strokeWidth={isActive ? 2.2 : 1.7}
                              className={isActive ? "text-[hsl(var(--primary))]" : ""}
                            />
                            <div className="text-[11.5px] font-semibold">{d.label}</div>
                            <div className="text-[9.5px] text-muted-foreground/45">{d.hint}</div>
                          </button>
                        );
                      })}
                    </div>
                  </div>

                  {/* Toggles */}
                  <div className="rounded-md border border-border/35 bg-black/[0.08]">
                    <AppearanceRow
                      label="Status bar"
                      description="Show connection and queue stats at the bottom"
                      control={
                        <PillToggle
                          checked={uiPrefs.showStatusBar}
                          onChange={(v) => updateUiPrefs({ showStatusBar: v })}
                        />
                      }
                    />
                    <div className="mx-3 h-px bg-border/25" />
                    <AppearanceRow
                      label="Smooth animations"
                      description="Transitions and animated progress indicators"
                      control={
                        <PillToggle
                          checked={uiPrefs.smoothAnimations}
                          onChange={(v) => updateUiPrefs({ smoothAnimations: v })}
                        />
                      }
                    />
                  </div>

                </div>
              )}

              {/* ── ENGINE ─────────────────────────────────────────────── */}
              {page === "engine" && (
                <div className="flex flex-col gap-5">
                  {/* Connections */}
                  <div>
                    <SectionTitle>Queue & Concurrency</SectionTitle>
                    <div className="rounded-md border border-border/40 bg-black/[0.12] p-3.5 flex flex-col gap-4">
                      <div>
                        <div className="mb-0.5 text-[12px] font-medium text-foreground/80">Concurrent downloads</div>
                        <div className="mb-2.5 text-[10.5px] text-muted-foreground/50">
                          Maximum number of files downloading at the same time.
                        </div>
                        <div className="grid grid-cols-6 gap-1.5">
                          {ACTIVE_OPTIONS.map((value) => (
                            <Chip
                              key={value}
                              active={draft.maxActiveDownloads === value}
                              onClick={() => setDraft((prev) => ({ ...prev, maxActiveDownloads: value }))}
                            >
                              {value}
                            </Chip>
                          ))}
                        </div>
                      </div>
                    </div>
                  </div>

                  {/* Speed & I/O */}
                  <div>
                    <SectionTitle>Speed &amp; I/O</SectionTitle>
                    <div className="rounded-md border border-border/40 bg-black/[0.12] p-3.5 flex flex-col gap-4">
                      <div>
                        <div className="mb-0.5 text-[12px] font-medium text-foreground/80">Traffic mode</div>
                        <div className="mb-2.5 text-[10.5px] text-muted-foreground/50">
                          Controls the in-memory I/O buffer size per download segment.
                        </div>
                        <div className="flex overflow-hidden rounded-md border border-border/50">
                          {TRAFFIC_OPTIONS.map((option, i) => (
                            <button
                              key={option.value}
                              type="button"
                              onClick={() => setDraft((prev) => ({ ...prev, trafficMode: option.value }))}
                              className={cn(
                                "flex-1 px-2.5 py-2 text-center transition-colors duration-150",
                                i > 0 && "border-l border-border/50",
                                draft.trafficMode === option.value
                                  ? "bg-[hsl(var(--primary)/0.12)] text-foreground"
                                  : "text-foreground/55 hover:bg-white/[0.03] hover:text-foreground/80",
                              )}
                            >
                              <div className="text-[12px] font-semibold">{option.label}</div>
                              <div className="mt-0.5 text-[10px] text-muted-foreground/55">{option.hint}</div>
                            </button>
                          ))}
                        </div>
                      </div>
                      <div>
                        <div className="mb-0.5 text-[12px] font-medium text-foreground/80">Checkpoint cadence</div>
                        <div className="mb-2.5 text-[10.5px] text-muted-foreground/50">
                          How often segment progress is flushed to disk during an active download.
                        </div>
                        <div className="flex overflow-hidden rounded-md border border-border/50">
                          {CHECKPOINT_PRESETS.map((preset, index) => (
                            <button
                              key={preset.label}
                              type="button"
                              onClick={() =>
                                setDraft((prev) => ({
                                  ...prev,
                                  segmentCheckpointMinIntervalMs: preset.min,
                                  segmentCheckpointMaxIntervalMs: preset.max,
                                }))
                              }
                              className={cn(
                                "flex-1 px-2.5 py-2 text-center transition-colors duration-150",
                                index > 0 && "border-l border-border/50",
                                activeCheckpointPreset(
                                  draft.segmentCheckpointMinIntervalMs,
                                  draft.segmentCheckpointMaxIntervalMs,
                                ) === index
                                  ? "bg-[hsl(var(--primary)/0.12)] text-foreground"
                                  : "text-foreground/55 hover:bg-white/[0.03] hover:text-foreground/80",
                              )}
                            >
                              <div className="text-[12px] font-semibold">{preset.label}</div>
                              <div className="mt-0.5 text-[10px] text-muted-foreground/55">{preset.hint}</div>
                            </button>
                          ))}
                        </div>
                      </div>
                    </div>
                  </div>

                  {/* Advanced */}
                  <div>
                    <SectionTitle>Advanced</SectionTitle>
                    <div className="rounded-md border border-border/40 bg-black/[0.12] p-3.5">
                      <div className="mb-0.5 text-[12px] font-medium text-foreground/80">Uncapped concurrency</div>
                      <div className="mb-2.5 text-[10.5px] text-muted-foreground/50">
                        Removes the adaptive connection cap ceiling. VDM will open as many connections as the host tolerates.
                      </div>
                      <div className="flex items-center justify-between gap-3 rounded-md border border-border/50 bg-black/10 px-3 py-2.5">
                        <span className="text-[12px] font-medium text-foreground/80 select-none">Enable</span>
                        <PillToggle
                          checked={draft.experimentalUncappedMode}
                          onChange={(v) => setDraft((prev) => ({ ...prev, experimentalUncappedMode: v }))}
                        />
                      </div>
                      {draft.experimentalUncappedMode && (
                        <div className="mt-2.5 flex items-start gap-2 rounded-md border border-[hsl(var(--status-paused)/0.22)] bg-[hsl(var(--status-paused)/0.07)] px-2.5 py-1.5 text-[11px] text-foreground/70">
                          <AlertTriangle size={11} strokeWidth={2} className="mt-[1px] shrink-0 text-[hsl(var(--status-paused))]" />
                          <span>Uncapped mode can saturate server connections. Hosts may throttle or temporarily block VDM.</span>
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              )}

              {/* ── BROWSER ────────────────────────────────────────────── */}
              {page === "browser" && (
                <div className="flex flex-col gap-3">
                  <div className="rounded-md border border-border/40 bg-black/[0.12] p-4">
                    <div className="mb-2 flex items-center gap-2">
                      <Globe size={14} strokeWidth={1.8} className="text-muted-foreground/50" />
                      <div className="text-[12.5px] font-semibold text-foreground/80">App Updates</div>
                    </div>
                    <div className="text-[11.5px] leading-[1.65] text-muted-foreground/60">
                      Stable keeps VDM on release builds. Preview checks prerelease manifests first and automatically falls back to stable if the first restart after an update fails.
                    </div>
                    <div className="mt-3 grid grid-cols-2 gap-2">
                      {UPDATE_CHANNEL_OPTIONS.map((option) => {
                        const active = draft.updateChannel === option.value;
                        return (
                          <button
                            key={option.value}
                            type="button"
                            onClick={() => setDraft((prev) => ({ ...prev, updateChannel: option.value }))}
                            className={cn(
                              "rounded-md border px-3 py-2.5 text-left transition-colors duration-150",
                              active
                                ? "border-[hsl(var(--primary)/0.4)] bg-[hsl(var(--primary)/0.12)] text-foreground"
                                : "border-border/60 bg-black/10 text-foreground/60 hover:border-border/90 hover:text-foreground/80 hover:bg-accent/5",
                            )}
                          >
                            <div className="text-[12px] font-semibold">{option.label}</div>
                            <div className="mt-0.5 text-[10.5px] text-muted-foreground/55">{option.hint}</div>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                  <div className="rounded-md border border-border/40 bg-black/[0.12] p-4">
                    <div className="mb-2 flex items-center gap-2">
                      <Globe size={14} strokeWidth={1.8} className="text-muted-foreground/50" />
                      <div className="text-[12.5px] font-semibold text-foreground/80">VDM Catcher Extension</div>
                    </div>
                    <div className="text-[11.5px] leading-[1.65] text-muted-foreground/60">
                      VDM Catcher auto-detects the desktop app over localhost. No pairing step is required — keep the app running
                      and the extension reconnects automatically after browser or service-worker restarts.
                    </div>
                  </div>
                  <div className="flex items-center gap-2.5 rounded-md border border-border/40 bg-black/[0.08] px-3.5 py-3">
                    <div className="h-2 w-2 shrink-0 rounded-full bg-[hsl(var(--status-finished)/0.75)]" />
                    <span className="text-[11.5px] text-foreground/60">
                      Bridge listening on{" "}
                      <span className="font-mono text-foreground/48">localhost:6670</span>
                    </span>
                  </div>
                </div>
              )}

            </div>
          </div>

          {error && (
            <div className="mx-4 mb-3">
              <InlineNotice tone="error" message={error} onDismiss={onClearError} />
            </div>
          )}

          {/* Footer */}
          <div className="flex items-center justify-end gap-2 border-t border-border/60 px-4 py-3">
            <Dialog.Close
              type="button"
              className="h-7 rounded-md border border-border/70 bg-black/10 px-3 text-[12px] text-foreground/65 transition-colors hover:bg-accent hover:text-foreground"
            >
              Cancel
            </Dialog.Close>
            <button
              type="button"
              disabled={saving || !isDirty || page === "appearance"}
              onClick={() => void onSave(draft)}
              style={!(saving || !isDirty || page === "appearance")
                ? { background: "linear-gradient(90deg, hsl(var(--accent-h) 22% 32%) 0%, hsl(var(--accent-h) 16% 25%) 55%, hsl(0,0%,18%) 100%)" }
                : undefined}
              className={cn(
                "h-7 rounded-md px-4 text-[12px] font-semibold transition-all",
                saving || !isDirty || page === "appearance"
                  ? "bg-[hsl(0,0%,18%)] text-white/35 pointer-events-none"
                  : "text-[hsl(0,0%,92%)] hover:brightness-110",
              )}
            >
              {saving ? "Saving…" : "Save"}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
