import { useEffect, useMemo, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { AlertTriangle, Gauge, Network, Settings2, X, Zap } from "lucide-react";
import { cn } from "@/lib/utils";
import type { EngineSettings, TrafficMode } from "@/types/download";

const CONNECTION_OPTIONS = [1, 2, 4, 6, 8, 10, 12, 16, 20] as const;
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

type SettingsPage = "connections" | "speed" | "advanced";

const PAGES: Array<{ id: SettingsPage; label: string; icon: React.ElementType }> = [
  { id: "connections", label: "Connections", icon: Network   },
  { id: "speed",       label: "Speed",       icon: Gauge     },
  { id: "advanced",    label: "Advanced",    icon: Zap       },
];

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-md border px-2.5 py-1.5 text-center text-[12px] font-medium transition-all duration-200",
        active
          ? "border-[hsl(var(--status-downloading)/0.45)] bg-[hsl(var(--status-downloading)/0.12)] text-foreground shadow-[0_0_8px_hsl(var(--status-downloading)/0.15)]"
          : "border-border/60 bg-black/10 text-foreground/60 hover:border-border/90 hover:text-foreground/80 hover:bg-accent/5",
      )}
    >
      {children}
    </button>
  );
}

function PageLabel({ icon: Icon, label }: { icon: React.ElementType; label: string }) {
  return (
    <div className="mb-3 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground/38">
      <Icon size={10} strokeWidth={2.2} />
      <span>{label}</span>
    </div>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-1 text-[11px] font-semibold text-foreground/55">{children}</div>
  );
}

interface SettingsDialogProps {
  open: boolean;
  settings: EngineSettings;
  saving: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onSave: (settings: EngineSettings) => Promise<void> | void;
}

export function SettingsDialog({ open, settings, saving, error, onOpenChange, onSave }: SettingsDialogProps) {
  const [draft, setDraft] = useState<EngineSettings>(settings);
  const [page, setPage] = useState<SettingsPage>("connections");

  useEffect(() => {
    if (open) {
      setDraft(settings);
      setPage("connections");
    }
  }, [open, settings]);

  const isDirty = useMemo(
    () => JSON.stringify(draft) !== JSON.stringify(settings),
    [draft, settings],
  );

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-black/55 backdrop-blur-[2px] data-[state=open]:animate-in data-[state=open]:fade-in-0" />
        <Dialog.Content
          className={cn(
            "fixed left-1/2 top-1/2 z-50 w-[540px] max-w-[calc(100vw-24px)] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-border bg-[linear-gradient(180deg,hsl(0,0%,10.5%),hsl(0,0%,8.8%))] shadow-2xl shadow-black/60 outline-none",
            "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
          )}
        >
          {/* Header */}
          <div className="flex items-center justify-between border-b border-border/60 px-4 py-3">
            <div className="flex items-center gap-2">
              <Settings2 size={13} className="text-muted-foreground/60" strokeWidth={2} />
              <Dialog.Title className="text-[13px] font-semibold text-foreground/88">Engine Settings</Dialog.Title>
              {isDirty && (
                <span className="ml-0.5 h-[6px] w-[6px] rounded-full bg-[hsl(var(--status-paused)/0.85)]" title="Unsaved changes" />
              )}
            </div>
            <Dialog.Close className="flex h-6 w-6 items-center justify-center rounded text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground">
              <X size={13} strokeWidth={2} />
            </Dialog.Close>
          </div>

          {/* Body: sidebar + content */}
          <div className="flex min-h-[260px]">
            {/* Sidebar nav */}
            <nav className="flex w-[110px] shrink-0 flex-col gap-0.5 border-r border-border/40 p-2.5">
              {PAGES.map(({ id, label, icon: Icon }) => (
                <button
                  key={id}
                  type="button"
                  onClick={() => setPage(id)}
                  className={cn(
                    "flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[12px] font-medium transition-colors",
                    page === id
                      ? "bg-white/[0.07] text-foreground/90"
                      : "text-muted-foreground/55 hover:bg-white/[0.04] hover:text-foreground/75",
                  )}
                >
                  <Icon size={13} strokeWidth={page === id ? 2.2 : 1.8} />
                  {label}
                </button>
              ))}
            </nav>

            {/* Page content */}
            <div className="flex-1 overflow-y-auto px-5 py-4" style={{ maxHeight: "calc(90vh - 130px)" }}>

              {page === "connections" && (
                <div className="flex flex-col gap-5">
                  <PageLabel icon={Network} label="Connections" />

                  <div>
                    <SectionTitle>Max connections per file</SectionTitle>
                    <div className="text-[10.5px] text-muted-foreground/40 mb-2.5">
                      VDM opens this many parallel HTTP connections per download.
                    </div>
                    <div className="grid grid-cols-9 gap-1.5">
                      {CONNECTION_OPTIONS.map((value) => (
                        <Chip
                          key={value}
                          active={draft.defaultMaxConnections === value}
                          onClick={() => setDraft((prev) => ({ ...prev, defaultMaxConnections: value }))}
                        >
                          {value}
                        </Chip>
                      ))}
                    </div>
                    {draft.defaultMaxConnections > 8 && (
                      <div className="mt-2.5 flex items-start gap-2 rounded-md border border-[hsl(var(--status-paused)/0.22)] bg-[hsl(var(--status-paused)/0.07)] px-2.5 py-1.5 text-[11px] text-foreground/70">
                        <AlertTriangle size={11} strokeWidth={2} className="mt-[1px] shrink-0 text-[hsl(var(--status-paused))]" />
                        <span>VDM downscales aggressively when a host throttles above 8.</span>
                      </div>
                    )}
                  </div>

                  <div>
                    <SectionTitle>Concurrent downloads</SectionTitle>
                    <div className="text-[10.5px] text-muted-foreground/40 mb-2.5">
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
              )}

              {page === "speed" && (
                <div className="flex flex-col gap-5">
                  <PageLabel icon={Gauge} label="Speed" />

                  <div>
                    <SectionTitle>Traffic mode</SectionTitle>
                    <div className="text-[10.5px] text-muted-foreground/40 mb-2.5">
                      Controls the in-memory I/O buffer size per download segment.
                    </div>
                    <div className="grid grid-cols-4 gap-1.5">
                      {TRAFFIC_OPTIONS.map((option) => (
                        <button
                          key={option.value}
                          type="button"
                          onClick={() => setDraft((prev) => ({ ...prev, trafficMode: option.value }))}
                          className={cn(
                            "rounded-md border px-2 py-2.5 text-left transition-all duration-200",
                            draft.trafficMode === option.value
                              ? "border-[hsl(var(--status-downloading)/0.45)] bg-[hsl(var(--status-downloading)/0.12)] text-foreground shadow-[0_0_8px_hsl(var(--status-downloading)/0.15)]"
                              : "border-border/60 bg-black/10 text-foreground/60 hover:border-border/90 hover:text-foreground/80 hover:bg-accent/5",
                          )}
                        >
                          <div className="text-[12px] font-semibold">{option.label}</div>
                          <div className="mt-0.5 text-[10px] text-muted-foreground/55">{option.hint}</div>
                        </button>
                      ))}
                    </div>
                  </div>

                  <div>
                    <SectionTitle>Checkpoint cadence</SectionTitle>
                    <div className="text-[10.5px] text-muted-foreground/40 mb-2.5">
                      How often segment progress is flushed to disk during an active download.
                    </div>
                    <div className="grid grid-cols-3 gap-1.5">
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
                            "rounded-md border px-2 py-2.5 text-left transition-all duration-200",
                            activeCheckpointPreset(
                              draft.segmentCheckpointMinIntervalMs,
                              draft.segmentCheckpointMaxIntervalMs,
                            ) === index
                              ? "border-[hsl(var(--status-downloading)/0.45)] bg-[hsl(var(--status-downloading)/0.12)] text-foreground shadow-[0_0_8px_hsl(var(--status-downloading)/0.15)]"
                              : "border-border/60 bg-black/10 text-foreground/60 hover:border-border/90 hover:text-foreground/80 hover:bg-accent/5",
                          )}
                        >
                          <div className="text-[12px] font-semibold">{preset.label}</div>
                          <div className="mt-0.5 text-[10px] text-muted-foreground/55">{preset.hint}</div>
                        </button>
                      ))}
                    </div>
                  </div>
                </div>
              )}

              {page === "advanced" && (
                <div className="flex flex-col gap-5">
                  <PageLabel icon={Zap} label="Advanced" />

                  <div>
                    <SectionTitle>Uncapped concurrency</SectionTitle>
                    <div className="text-[10.5px] text-muted-foreground/40 mb-2.5">
                      Removes the adaptive connection cap ceiling. VDM will open as many connections as the host tolerates.
                    </div>
                    <div
                      className="flex cursor-pointer items-center justify-between gap-3 rounded-md border border-border/50 bg-black/10 px-3 py-2.5 transition-colors hover:bg-accent/10"
                      onClick={() => setDraft((prev) => ({ ...prev, experimentalUncappedMode: !prev.experimentalUncappedMode }))}
                    >
                      <span className="text-[12px] font-medium text-foreground/85 select-none">Enable</span>
                      <div
                        className={cn(
                          "relative h-5 w-9 shrink-0 rounded-full border transition-colors duration-200",
                          draft.experimentalUncappedMode
                            ? "border-[hsl(var(--status-downloading)/0.45)] bg-[hsl(var(--status-downloading)/0.25)]"
                            : "border-border/60 bg-black/20",
                        )}
                      >
                        <span
                          className={cn(
                            "absolute top-[3px] h-3 w-3 rounded-full transition-all duration-200",
                            draft.experimentalUncappedMode
                              ? "left-[18px] bg-[hsl(var(--status-downloading))]"
                              : "left-[3px] bg-muted-foreground/35",
                          )}
                        />
                      </div>
                    </div>
                    {draft.experimentalUncappedMode && (
                      <div className="mt-2.5 flex items-start gap-2 rounded-md border border-[hsl(var(--status-paused)/0.22)] bg-[hsl(var(--status-paused)/0.07)] px-2.5 py-1.5 text-[11px] text-foreground/70">
                        <AlertTriangle size={11} strokeWidth={2} className="mt-[1px] shrink-0 text-[hsl(var(--status-paused))]" />
                        <span>Uncapped mode can saturate server connections. Hosts may throttle or temporarily block VDM.</span>
                      </div>
                    )}
                  </div>

                  <div>
                    <SectionTitle>Extension bridge</SectionTitle>
                    <div className="mb-2.5 text-[10.5px] text-muted-foreground/40">
                      VDM Catcher now auto-detects the desktop app over localhost. No browser linking or copy step is required.
                    </div>
                    <div className="rounded-md border border-border/50 bg-black/10 px-3 py-3">
                      <div className="text-[12px] font-medium text-foreground/82">
                        Seamless extension handoff
                      </div>
                      <div className="mt-2 text-[11px] leading-5 text-foreground/72">
                        When VDM is running, the extension discovers the local bridge automatically, keeps its secure session alive, and reconnects after browser or service-worker restarts.
                      </div>
                      <div className="mt-2 rounded-md border border-border/40 bg-white/[0.03] px-3 py-2 text-[10.5px] text-muted-foreground/52">
                        The popup, options page, and background worker all refresh bridge state automatically. Users should only need to keep the desktop app running.
                      </div>
                    </div>
                  </div>
                </div>
              )}

            </div>
          </div>

          {error && (
            <div className="mx-4 mb-3 rounded-md border border-[hsl(var(--status-error)/0.24)] bg-[hsl(var(--status-error)/0.08)] px-3 py-2 text-[11px] text-[hsl(var(--status-error))]">
              {error}
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
              disabled={saving}
              onClick={() => void onSave(draft)}
              style={!saving ? { background: "linear-gradient(90deg, hsl(20,58%,48%) 0%, hsl(12,42%,32%) 55%, hsl(0,0%,22%) 100%)" } : undefined}
              className={cn(
                "h-7 rounded-md px-4 text-[12px] font-semibold transition-all",
                saving
                  ? "bg-[hsl(0,0%,18%)] text-white/40 pointer-events-none"
                  : "text-[hsl(24,10%,95%)] hover:brightness-110",
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
