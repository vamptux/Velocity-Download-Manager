import { listen } from "@tauri-apps/api/event";
import { startTransition, useEffect, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import {
  fromRawDownload,
  ipcGetEngineBootstrapState,
  ipcGetStartupSnapshot,
  type EngineBootstrapState,
  type RawDownload,
} from "@/lib/ipc";
import type {
  AppUpdateStartupHealth,
  Download,
  DownloadCompletedEvent,
  DownloadProgressDiffEvent,
  EngineSettings,
  QueueState,
} from "@/types/download";

interface UseEngineBridgeArgs {
  setBootstrapState: Dispatch<SetStateAction<EngineBootstrapState>>;
  setUpdateHealth: Dispatch<SetStateAction<AppUpdateStartupHealth | null>>;
  setSettings: Dispatch<SetStateAction<EngineSettings>>;
  setQueueState: Dispatch<SetStateAction<QueueState>>;
  setDownloads: Dispatch<SetStateAction<Download[]>>;
  refreshAppState: () => Promise<void>;
  upsertDownload: (download: Download) => void;
  removeDownloadLocally: (id: string) => void;
  enqueueCompletionNotice: (notice: DownloadCompletedEvent) => void;
  applyProgressDiff: (event: DownloadProgressDiffEvent) => void;
  eventBridgeAttached: MutableRefObject<boolean>;
  lastRealtimeSyncAt: MutableRefObject<number>;
}

export function useEngineBridge({
  setBootstrapState,
  setUpdateHealth,
  setSettings,
  setQueueState,
  setDownloads,
  refreshAppState,
  upsertDownload,
  removeDownloadLocally,
  enqueueCompletionNotice,
  applyProgressDiff,
  eventBridgeAttached,
  lastRealtimeSyncAt,
}: UseEngineBridgeArgs): void {
  useEffect(() => {
    let disposed = false;
    let bootstrapEventReceived = false;
    const unlisten: Array<() => void> = [];

    async function subscribe() {
      try {
        const startupSnapshot = await ipcGetStartupSnapshot();
        if (!disposed) {
          startTransition(() => {
            setBootstrapState(startupSnapshot.bootstrap);
            setUpdateHealth(startupSnapshot.updateHealth);
            setSettings(startupSnapshot.settings);
            setQueueState(startupSnapshot.queueState);
            setDownloads(startupSnapshot.activeDownloads);
          });
        }

        eventBridgeAttached.current = true;
        lastRealtimeSyncAt.current = Date.now();
        const unlistenBootstrap = await listen<EngineBootstrapState>("engine://bootstrap", (event) => {
          bootstrapEventReceived = true;
          setBootstrapState(event.payload);
          void ipcGetStartupSnapshot()
            .then((startupSnapshot) => {
              if (!disposed) {
                setUpdateHealth(startupSnapshot.updateHealth);
              }
            })
            .catch(() => null);
          void refreshAppState();
        });
        const unlistenUpsert = await listen<RawDownload>("downloads://upsert-row", (event) => {
          if (!disposed) {
            upsertDownload(fromRawDownload(event.payload));
          }
        });
        const unlistenRemove = await listen<{ id: string }>("downloads://remove", (event) => {
          if (!disposed) {
            removeDownloadLocally(event.payload.id);
          }
        });
        const unlistenCompleted = await listen<DownloadCompletedEvent>("downloads://completed", (event) => {
          if (!disposed) {
            enqueueCompletionNotice(event.payload);
          }
        });
        const unlistenProgress = await listen<DownloadProgressDiffEvent>("downloads://progress-diff", (event) => {
          if (!disposed) {
            lastRealtimeSyncAt.current = Date.now();
            applyProgressDiff(event.payload);
          }
        });

        if (disposed) {
          unlistenBootstrap();
          unlistenUpsert();
          unlistenRemove();
          unlistenCompleted();
          unlistenProgress();
          return;
        }

        unlisten.push(unlistenBootstrap, unlistenUpsert, unlistenRemove, unlistenCompleted, unlistenProgress);

        const bootstrap = await ipcGetEngineBootstrapState();
        if (disposed) {
          return;
        }
        setBootstrapState(bootstrap);

        if (bootstrap.ready && !bootstrapEventReceived) {
          const startupSnapshot = await ipcGetStartupSnapshot().catch(() => null);
          if (!disposed && startupSnapshot) {
            setUpdateHealth(startupSnapshot.updateHealth);
          }
          await refreshAppState();
        }
      } catch {
        eventBridgeAttached.current = false;
        setBootstrapState({ ready: true, error: null });
        setUpdateHealth(null);
        await refreshAppState();
      }
    }

    void subscribe();

    return () => {
      disposed = true;
      eventBridgeAttached.current = false;
      for (const stop of unlisten) {
        stop();
      }
    };
  }, [
    applyProgressDiff,
    enqueueCompletionNotice,
    eventBridgeAttached,
    lastRealtimeSyncAt,
    refreshAppState,
    removeDownloadLocally,
    setBootstrapState,
    setUpdateHealth,
    setDownloads,
    setQueueState,
    setSettings,
    upsertDownload,
  ]);
}
