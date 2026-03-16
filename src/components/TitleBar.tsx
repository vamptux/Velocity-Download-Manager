import { useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, X, ArrowDownToLine, Search, Maximize2 } from "lucide-react";

const MENU_ITEMS = ["File", "Tasks", "Tools", "Help"] as const;

function tauriWindow() {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

export function TitleBar({ onSearch }: { onSearch?: (q: string) => void }) {
  const [search, setSearch] = useState("");
  const minimize = async () => tauriWindow()?.minimize();
  const maximize = async () => tauriWindow()?.toggleMaximize();
  const close = async () => tauriWindow()?.close();

  return (
    <header
      data-tauri-drag-region
      className="flex h-9 shrink-0 items-center border-b border-border/60 select-none"
      style={{ background: "hsl(var(--toolbar))" }}
    >
      <div className="flex items-center pl-2.5">
        <div
          className="mr-2 flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-[4px]"
          style={{ background: "linear-gradient(135deg, hsl(24,62%,54%), hsl(12,52%,34%))" }}
        >
          <ArrowDownToLine size={9} className="text-white" strokeWidth={2.5} />
        </div>
        {MENU_ITEMS.map((item) => (
          <button
            key={item}
            className="h-9 px-2.5 text-[11px] text-muted-foreground/55 hover:bg-white/[0.055] hover:text-foreground/85 transition-colors"
          >
            {item}
          </button>
        ))}
      </div>

      <div data-tauri-drag-region className="flex flex-1 items-center justify-center gap-2">
        <span
          className="pointer-events-none select-none text-[12px] font-semibold tracking-[0.04em] text-foreground/38"
          style={{ letterSpacing: "0.03em" }}
        >
          Velocity{" "}
          <span className="text-[hsl(var(--primary)/0.7)]">DM</span>
        </span>
      </div>

      <div className="relative mr-3" data-tauri-drag-region={undefined}>
        <Search
          size={11}
          className="absolute left-2 top-1/2 -translate-y-1/2 text-muted-foreground/30 pointer-events-none"
        />
        <input
          type="text"
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            onSearch?.(e.target.value);
          }}
          placeholder="Search…"
          className="h-[22px] w-[130px] rounded-[5px] border border-border/35 bg-background/35 pl-[22px] pr-2 text-[11px] text-foreground placeholder:text-muted-foreground/28 outline-none focus:w-[175px] focus:border-primary/50 focus:bg-muted/50 transition-[width,border-color,background-color] duration-150 ease-out"
        />
      </div>

      <div className="flex items-stretch self-stretch">
        <button
          onClick={minimize}
          aria-label="Minimize"
          className="group flex w-11 items-center justify-center text-muted-foreground/40 hover:bg-white/[0.07] hover:text-foreground/75 transition-colors"
        >
          <Minus size={12} strokeWidth={1.6} />
        </button>
        <button
          onClick={maximize}
          aria-label="Maximize"
          className="group flex w-11 items-center justify-center text-muted-foreground/40 hover:bg-white/[0.07] hover:text-foreground/75 transition-colors"
        >
          <Maximize2 size={10} strokeWidth={1.6} />
        </button>
        <button
          onClick={close}
          aria-label="Close"
          className="group flex w-[46px] items-center justify-center text-muted-foreground/40 hover:bg-[hsl(0,62%,42%)] hover:text-white transition-colors"
        >
          <X size={12} strokeWidth={1.6} />
        </button>
      </div>
    </header>
  );
}
