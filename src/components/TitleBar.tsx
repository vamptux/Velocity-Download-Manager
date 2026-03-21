import { useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, X, Search, Maximize2 } from "lucide-react";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { readText } from "@tauri-apps/plugin-clipboard-manager";

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
  queueRunning?: boolean;
}

export function TitleBar({ 
  onSearch,
  onNewDownload,
  onOpenSettings,
  onStartQueue,
  onStopQueue,
  onBatchDownload,
  queueRunning
}: TitleBarProps) {
  const [search, setSearch] = useState("");
  const minimize = async () => tauriWindow()?.minimize();
  const maximize = async () => tauriWindow()?.toggleMaximize();
  const close = async () => tauriWindow()?.close();

  const handlePasteAndAdd = async () => {
    try {
      const text = await readText();
      if (text && onNewDownload) {
        onNewDownload(text);
      }
    } catch (err) {
      console.error("Failed to read clipboard:", err);
    }
  };

  const itemClass = "relative flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-xs outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50 transition-colors";

  return (
    <header
      data-tauri-drag-region
      className="flex h-9 shrink-0 items-center border-b border-border/60 select-none"
      style={{ background: "hsl(var(--toolbar))" }}
    >
      <div className="flex items-center pl-2.5 z-50">
        <img
          src="/veloicon.ico"
          alt="Velocity DM"
          className="mr-2 h-[20px] w-[20px] shrink-0 object-contain select-none pointer-events-none"
        />
        
        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <button className="h-9 px-2.5 text-[11px] text-muted-foreground/55 hover:bg-white/[0.055] hover:text-foreground/85 transition-colors data-[state=open]:bg-white/[0.055] data-[state=open]:text-foreground/85 outline-none">
              File
            </button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content className="z-50 min-w-[180px] overflow-hidden rounded-md border bg-popover/90 backdrop-blur-md p-1 text-popover-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2" sideOffset={0} align="start">
              <DropdownMenu.Item className={itemClass} onClick={() => onNewDownload?.()}>
                New Download...
              </DropdownMenu.Item>
              <DropdownMenu.Item className={itemClass} onClick={handlePasteAndAdd}>
                Paste Link from Clipboard
              </DropdownMenu.Item>
              <DropdownMenu.Separator className="-mx-1 my-1 h-px bg-border/50" />
              <DropdownMenu.Item className={itemClass} onClick={close}>
                Exit
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>

        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <button className="h-9 px-2.5 text-[11px] text-muted-foreground/55 hover:bg-white/[0.055] hover:text-foreground/85 transition-colors data-[state=open]:bg-white/[0.055] data-[state=open]:text-foreground/85 outline-none">
              Tasks
            </button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content className="z-50 min-w-[180px] overflow-hidden rounded-md border bg-popover/90 backdrop-blur-md p-1 text-popover-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2" sideOffset={0} align="start">
              <DropdownMenu.Item className={itemClass} onClick={() => queueRunning ? onStopQueue?.() : onStartQueue?.()}>
                {queueRunning ? "Stop Main Queue" : "Start Main Queue"}
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>

        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <button className="h-9 px-2.5 text-[11px] text-muted-foreground/55 hover:bg-white/[0.055] hover:text-foreground/85 transition-colors data-[state=open]:bg-white/[0.055] data-[state=open]:text-foreground/85 outline-none">
              Tools
            </button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content className="z-50 min-w-[180px] overflow-hidden rounded-md border bg-popover/90 backdrop-blur-md p-1 text-popover-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2" sideOffset={0} align="start">
              <DropdownMenu.Item className={itemClass} onClick={() => onOpenSettings?.()}>
                Settings
              </DropdownMenu.Item>
              <DropdownMenu.Item className={itemClass} onClick={() => onBatchDownload?.()}>
                Batch Download
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>

        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <button className="h-9 px-2.5 text-[11px] text-muted-foreground/55 hover:bg-white/[0.055] hover:text-foreground/85 transition-colors data-[state=open]:bg-white/[0.055] data-[state=open]:text-foreground/85 outline-none">
              Help
            </button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content className="z-50 min-w-[180px] overflow-hidden rounded-md border bg-popover/90 backdrop-blur-md p-1 text-popover-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2" sideOffset={0} align="start">
              <DropdownMenu.Item className={itemClass} onClick={() => window.open("https://github.com/vamptux/Velocity-Download-Manager", "_blank")}>
                GitHub Repository
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
          className="h-[24px] w-[140px] rounded-[6px] border border-white/[0.08] bg-black/20 pl-7 pr-2.5 text-[11.5px] text-foreground placeholder:text-muted-foreground/35 outline-none focus:w-[200px] focus:border-primary/50 focus:bg-black/40 transition-all duration-200 ease-out shadow-inner"
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
          className="group flex w-[42px] items-center justify-center text-muted-foreground/58 hover:bg-white/[0.1] hover:text-foreground/90 transition-colors"
        >
          <Minus size={14} strokeWidth={1.8} />
        </button>
        <button
          onClick={maximize}
          aria-label="Maximize"
          className="group flex w-[42px] items-center justify-center text-muted-foreground/58 hover:bg-white/[0.1] hover:text-foreground/90 transition-colors"
        >
          <Maximize2 size={11} strokeWidth={1.8} />
        </button>
        <button
          onClick={close}
          aria-label="Close"
          className="group flex w-[46px] items-center justify-center text-muted-foreground/58 hover:bg-[hsl(0,66%,46%)] hover:text-white transition-colors"
        >
          <X size={13} strokeWidth={1.9} />
        </button>
      </div>
    </header>
  );
}
