import { lazy, Suspense } from "react";
import { createRoot } from "react-dom/client";
import "./globals.css";
import { App } from "./App";
import { TooltipProvider } from "@/components/ui/tooltip";
import { loadUiPrefs, applyUiPrefs } from "@/lib/uiPrefs";

// Apply saved visual prefs synchronously before first render to prevent FOUC.
applyUiPrefs(loadUiPrefs());

// Re-apply prefs whenever another window (e.g. settings dialog in main window)
// writes to localStorage so all open Tauri windows stay in sync.
window.addEventListener("storage", (e) => {
  if (e.key === "vdm-ui-prefs") {
    applyUiPrefs(loadUiPrefs());
  }
});

const CompactCaptureWindow = lazy(() =>
  import("@/components/CompactCaptureWindow").then((module) => ({ default: module.CompactCaptureWindow })),
);

const windowMode = new URLSearchParams(window.location.search).get("window");

function Root() {
  return windowMode === "capture" ? (
    <TooltipProvider>
      <Suspense fallback={null}>
        <CompactCaptureWindow />
      </Suspense>
    </TooltipProvider>
  ) : (
    <App />
  );
}

createRoot(document.getElementById("root")!).render(<Root />);
