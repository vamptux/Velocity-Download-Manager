import { lazy, Suspense } from "react";
import { createRoot } from "react-dom/client";
import "./globals.css";
import { App } from "./App";
import { TooltipProvider } from "@/components/ui/tooltip";

const CompactCaptureWindow = lazy(() =>
  import("@/components/CompactCaptureWindow").then((module) => ({ default: module.CompactCaptureWindow })),
);

const windowMode = new URLSearchParams(window.location.search).get("window");

const Root =
  windowMode === "capture" ? (
    <TooltipProvider>
      <Suspense fallback={null}>
        <CompactCaptureWindow />
      </Suspense>
    </TooltipProvider>
  ) : (
    <App />
  );

createRoot(document.getElementById("root")!).render(
  Root,
);
