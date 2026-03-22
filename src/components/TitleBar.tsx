import { useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { CheckCircle2, ExternalLink, Minus, RefreshCw, Search, Maximize2, X } from "lucide-react";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { readClipboardText } from "@/lib/clipboard";
import { ipcOpenExternalUrl } from "@/lib/ipc";

const GITHUB_REPOSITORY_URL = "https://github.com/vamptux/Velocity-Download-Manager";

function tauriWindow() {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

interface TitleBarProps {
  onSearch?: (q: string) => void;
  onNewDownload?: (url?: string) => void;
  onOpenSettings?: () => void;
  onStartQueue?: () => void;
  onStopQueue?: () => void;
  onBatchDownload?: () => void;
  onCheckForUpdates?: () => void;
  checkingForUpdates?: boolean;
  queueRunning?: boolean;
}

export function TitleBar({ 
  onSearch,
  onNewDownload,
  onOpenSettings,
  onStartQueue,
  onStopQueue,
  onBatchDownload,
  onCheckForUpdates,
  checkingForUpdates = false,
  queueRunning
}: TitleBarProps) {
  const [search, setSearch] = useState("");
  const minimize = async () => tauriWindow()?.minimize();
  const maximize = async () => tauriWindow()?.toggleMaximize();
  const close = async () => tauriWindow()?.close();

  const handlePasteAndAdd = async () => {
    try {
      const text = (await readClipboardText()).trim();
      if (text && onNewDownload) {
        onNewDownload(text);
      } else {
        onNewDownload?.();
      }
    } catch (err) {
      console.error("Failed to read clipboard:", err);
      onNewDownload?.();
    }
  };

  const handleOpenRepository = async () => {
    try {
      await ipcOpenExternalUrl(GITHUB_REPOSITORY_URL);
    } catch (error) {
      console.error("Failed to open repository URL:", error);
      window.open(GITHUB_REPOSITORY_URL, "_blank", "noopener,noreferrer");
    }
  };

  const itemClass = "relative flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-xs outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50 transition-colors";

  return (
    <header
      data-tauri-drag-region
      className="flex h-11 shrink-0 items-center border-b border-border/60 select-none"
      style={{ background: "hsl(var(--toolbar))" }}
    >
      <div className="z-50 flex items-center pl-2">
        <img
          src="/veloicon.ico"
          alt="Velocity DM"
          className="mr-2 h-[24px] w-[24px] shrink-0 object-contain select-none pointer-events-none"
        />
        
        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <button className="h-11 px-2.5 text-[11px] text-muted-foreground/55 transition-colors outline-none hover:bg-white/[0.055] hover:text-foreground/85 data-[state=open]:bg-white/[0.055] data-[state=open]:text-foreground/85">
              File
            </button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content className="z-50 min-w-[180px] overflow-hidden rounded-md border bg-popover/90 backdrop-blur-md p-1 text-popover-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2" sideOffset={0} align="start">
              <DropdownMenu.Item className={itemClass} onSelect={() => onNewDownload?.()}>
                New Download...
              </DropdownMenu.Item>
              <DropdownMenu.Item className={itemClass} onSelect={() => void handlePasteAndAdd()}>
                Paste Link from Clipboard
              </DropdownMenu.Item>
              <DropdownMenu.Separator className="-mx-1 my-1 h-px bg-border/50" />
              <DropdownMenu.Item className={itemClass} onSelect={() => void close()}>
                Exit
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>

        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <button className="h-11 px-2.5 text-[11px] text-muted-foreground/55 transition-colors outline-none hover:bg-white/[0.055] hover:text-foreground/85 data-[state=open]:bg-white/[0.055] data-[state=open]:text-foreground/85">
              Tasks
            </button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content className="z-50 min-w-[180px] overflow-hidden rounded-md border bg-popover/90 backdrop-blur-md p-1 text-popover-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2" sideOffset={0} align="start">
              <DropdownMenu.Item className={itemClass} onSelect={() => queueRunning ? onStopQueue?.() : onStartQueue?.()}>
                {queueRunning ? "Stop Main Queue" : "Start Main Queue"}
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>

        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <button className="h-11 px-2.5 text-[11px] text-muted-foreground/55 transition-colors outline-none hover:bg-white/[0.055] hover:text-foreground/85 data-[state=open]:bg-white/[0.055] data-[state=open]:text-foreground/85">
              Tools
            </button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content className="z-50 min-w-[180px] overflow-hidden rounded-md border bg-popover/90 backdrop-blur-md p-1 text-popover-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2" sideOffset={0} align="start">
              <DropdownMenu.Item className={itemClass} onSelect={() => onOpenSettings?.()}>
                Settings
              </DropdownMenu.Item>
              <DropdownMenu.Item className={itemClass} onSelect={() => onBatchDownload?.()}>
                Batch Download
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>

        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <button className="h-11 px-2.5 text-[11px] text-muted-foreground/55 transition-colors outline-none hover:bg-white/[0.055] hover:text-foreground/85 data-[state=open]:bg-white/[0.055] data-[state=open]:text-foreground/85">
              Help
            </button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content className="z-50 min-w-[180px] overflow-hidden rounded-md border bg-popover/90 backdrop-blur-md p-1 text-popover-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2" sideOffset={0} align="start">
              <DropdownMenu.Item className={itemClass} onSelect={() => void handleOpenRepository()}>
                <ExternalLink size={12} strokeWidth={1.8} className="mr-2 shrink-0" />
                GitHub Repository
              </DropdownMenu.Item>
              <DropdownMenu.Separator className="-mx-1 my-1 h-px bg-border/50" />
              <DropdownMenu.Item
                className={itemClass}
                disabled={checkingForUpdates}
                onSelect={() => onCheckForUpdates?.()}
              >
                {checkingForUpdates ? (
                  <RefreshCw size={12} strokeWidth={1.8} className="mr-2 shrink-0 animate-spin" />
                ) : (
                  <CheckCircle2 size={12} strokeWidth={1.8} className="mr-2 shrink-0" />
                )}
                {checkingForUpdates ? "Checking for Updates..." : "Check for Updates"}
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      </div>

      <div data-tauri-drag-region className="flex flex-1 items-center justify-center gap-2">
        <span
          className="pointer-events-none select-none text-[12px] font-semibold tracking-[0.04em] text-foreground/30"
          style={{ letterSpacing: "0.03em" }}
        >
          Velocity{" "}
          <span className="text-[hsl(var(--primary)/0.55)]">DM</span>
        </span>
      </div>

      <div className="relative mr-4 flex items-center">
        <Search
          size={12}
          className="absolute left-2.5 text-muted-foreground/40 pointer-events-none"
        />
        <input
          type="text"
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            onSearch?.(e.target.value);
          }}
          placeholder="Search downloads…"
          className="h-[28px] w-[140px] rounded-[6px] border border-white/[0.08] bg-black/20 pl-7 pr-2.5 text-[11.5px] text-foreground placeholder:text-muted-foreground/35 outline-none shadow-inner transition-all duration-200 ease-out focus:w-[200px] focus:border-primary/50 focus:bg-black/40"
        />
        {search && (
          <button 
            onClick={() => { setSearch(""); onSearch?.(""); }}
            className="absolute right-1.5 flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground/50 hover:bg-white/10 hover:text-foreground transition-colors"
          >
            <X size={10} strokeWidth={2.5} />
          </button>
        )}
      </div>

      <div className="flex items-stretch self-stretch border-l border-white/[0.06]">
        <button
          onClick={minimize}
          aria-label="Minimize"
          className="group flex w-[44px] items-center justify-center text-muted-foreground/58 transition-colors hover:bg-white/[0.1] hover:text-foreground/90"
        >
          <Minus size={14} strokeWidth={1.8} />
        </button>
        <button
          onClick={maximize}
          aria-label="Maximize"
          className="group flex w-[44px] items-center justify-center text-muted-foreground/58 transition-colors hover:bg-white/[0.1] hover:text-foreground/90"
        >
          <Maximize2 size={11} strokeWidth={1.8} />
        </button>
        <button
          onClick={close}
          aria-label="Close"
          className="group flex w-[48px] items-center justify-center text-muted-foreground/58 transition-colors hover:bg-[hsl(0,66%,46%)] hover:text-white"
        >
          <X size={13} strokeWidth={1.9} />
        </button>
      </div>
    </header>
  );
}
