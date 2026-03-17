import { useEffect, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { X, Link, ArrowDownToLine } from "lucide-react";
import { cn } from "@/lib/utils";
import { ipcAddDownload, ipcProbeDownload } from "@/lib/ipc";
import type { Download, DownloadContentCategory, DownloadProbe } from "@/types/download";
import {
  DialogFormField,
  DialogInput,
  DownloadCapturePane,
  ProbeSummaryStrip,
} from "@/components/DownloadCapturePane";
import { getCaptureErrorMessage, useDefaultCaptureSavePath } from "@/lib/captureUtils";

interface NewDownloadDialogProps {
  open: boolean;
  initialUrl?: string;
  onOpenChange: (open: boolean) => void;
  onDownloadAdded?: (download: Download) => void;
}

export function NewDownloadDialog({ open, initialUrl, onOpenChange, onDownloadAdded }: NewDownloadDialogProps) {
  const [url, setUrl] = useState("");
  const [filename, setFilename] = useState("");
  const [category, setCategory] = useState<DownloadContentCategory>("compressed");
  const [savePath, setSavePath] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [probe, setProbe] = useState<DownloadProbe | null>(null);
  const [probeLoading, setProbeLoading] = useState(false);
  const [probeError, setProbeError] = useState<string | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [categoryDirty, setCategoryDirty] = useState(false);
  const [filenameDirty, setFilenameDirty] = useState(false);

  useDefaultCaptureSavePath(open, savePath, setSavePath);

  useEffect(() => {
    if (open) {
      setUrl(initialUrl ?? "");
    }
  }, [initialUrl, open]);

  useEffect(() => {
    if (!open) {
      setProbe(null);
      setProbeLoading(false);
      setProbeError(null);
      setSubmitError(null);
      setCategoryDirty(false);
      setFilenameDirty(false);
      setFilename("");
      return;
    }

    const trimmedUrl = url.trim();
    if (!trimmedUrl) {
      setProbe(null);
      setProbeLoading(false);
      setProbeError(null);
      return;
    }

    setProbe(null);
    setProbeError(null);

    let cancelled = false;
    const timeoutId = window.setTimeout(async () => {
      setProbeLoading(true);
      setProbeError(null);

      try {
        const result = await ipcProbeDownload(trimmedUrl, savePath.trim() || undefined);
        if (cancelled) return;
        setProbe(result);
        if (!categoryDirty) {
          setCategory(result.suggestedCategory);
        }
        if (!filenameDirty) {
          setFilename(result.suggestedName);
        }
      } catch (error) {
        if (cancelled) return;
        setProbe(null);
        setProbeError(getCaptureErrorMessage(error));
      } finally {
        if (!cancelled) {
          setProbeLoading(false);
        }
      }
    }, 420);

    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
    };
  }, [categoryDirty, filenameDirty, open, savePath, url]);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!url.trim() || submitting) return;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const createdDownload = await ipcAddDownload({
        url: url.trim(),
        name: filename.trim() || undefined,
        category,
        savePath: savePath.trim(),
        requestReferer: probe?.compatibility.requestReferer ?? undefined,
        requestCookies: probe?.compatibility.requestCookies ?? undefined,
        requestMethod: probe?.compatibility.requestMethod ?? undefined,
        requestFormFields: probe?.compatibility.requestFormFields ?? undefined,
        sizeHintBytes: probe?.size ?? undefined,
        rangeSupportedHint: probe?.rangeSupported ?? undefined,
        resumableHint: probe?.resumable ?? undefined,
        startImmediately: true,
      });
      onDownloadAdded?.(createdDownload);
      onOpenChange(false);
      setUrl("");
      setFilename("");
      setProbe(null);
      setProbeError(null);
      setSubmitError(null);
      setCategoryDirty(false);
      setFilenameDirty(false);
    } catch (error) {
      setSubmitError(getCaptureErrorMessage(error));
    } finally {
      setSubmitting(false);
    }
  }

  function handleBrowseSavePath() {
    setSubmitError(null);
    void openDialog({
      directory: true,
      multiple: false,
      defaultPath: savePath.trim() || undefined,
      title: "Choose download folder",
    })
      .then((selected) => {
        if (typeof selected === "string" && selected.trim()) {
          setSavePath(selected);
        }
      })
      .catch((error) => {
        setSubmitError(getCaptureErrorMessage(error));
      });
  }

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-black/50 backdrop-blur-[2px] data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=closed]:animate-out data-[state=closed]:fade-out-0" />
        <Dialog.Content
          className={cn(
            "fixed left-1/2 top-1/2 z-50 -translate-x-1/2 -translate-y-1/2",
            "w-[500px] rounded-xl border border-border bg-[linear-gradient(180deg,hsl(var(--card)),hsl(var(--background)))] shadow-2xl shadow-black/60 p-0 outline-none",
            "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95 data-[state=open]:slide-in-from-top-2",
            "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
          )}
        >
          {/* Header */}
          <div className="flex items-center justify-between border-b border-border/60 px-5 py-3">
            <div className="flex items-center gap-2.5">
              <div
                className="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded"
                style={{ background: "linear-gradient(135deg, hsl(var(--accent-h) 25% 38%), hsl(var(--accent-h) 18% 24%))", boxShadow: "0 1px 4px rgba(0,0,0,0.35)" }}
              >
                <ArrowDownToLine size={11} className="text-white" strokeWidth={2.2} />
              </div>
              <Dialog.Title className="text-[13px] font-medium tracking-[-0.01em] text-foreground/92">
                New Download
              </Dialog.Title>
            </div>
            <Dialog.Close className="flex h-6 w-6 items-center justify-center rounded text-muted-foreground/60 hover:bg-accent hover:text-foreground transition-colors">
              <X size={13} strokeWidth={2} />
            </Dialog.Close>
          </div>

          {/* Body */}
          <form onSubmit={handleSubmit} className="flex flex-col gap-2.5 px-5 py-3.5">
            {/* URL */}
            <DialogFormField label="Download URL" id="nd-url">
              <div className="relative">
                <Link
                  size={12}
                  className="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground/40 pointer-events-none"
                />
                <DialogInput
                  id="nd-url"
                  type="url"
                  placeholder="https://"
                  value={url}
                  onChange={(e) => {
                    setUrl(e.target.value);
                    setSubmitError(null);
                  }}
                  className="pl-7"
                  autoFocus
                  required
                />
              </div>
            </DialogFormField>

            <ProbeSummaryStrip loading={probeLoading} probe={probe} error={probeError} />

            <DownloadCapturePane
              variant="dialog"
              category={category}
              onCategoryChange={(nextCategory) => {
                setCategory(nextCategory);
                setCategoryDirty(true);
              }}
              savePath={savePath}
              onSavePathChange={setSavePath}
              onBrowseSavePath={handleBrowseSavePath}
              filename={filename}
              onFilenameChange={(value) => {
                setFilename(value);
                setFilenameDirty(value.trim().length > 0);
              }}
              filenamePlaceholder={probeLoading ? "Detecting…" : "Auto-detected from URL"}
              filenameResetVisible={filenameDirty}
              onFilenameReset={() => {
                setFilename(probe?.suggestedName ?? "");
                setFilenameDirty(false);
              }}
              errorMessage={submitError}
              onErrorDismiss={() => setSubmitError(null)}
              fieldIds={{
                category: "nd-category",
                savePath: "nd-savepath",
                filename: "nd-filename",
              }}
            />

            {/* Actions */}
            <div className="flex items-center justify-end gap-2.5 pt-1">
              <Dialog.Close asChild>
                <button
                  type="button"
                  className="h-8 px-4 rounded-md border border-border text-[12.5px] text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
                >
                  Cancel
                </button>
              </Dialog.Close>
              <button
                type="submit"
                style={{ background: "linear-gradient(90deg, hsl(var(--accent-h) 22% 32%) 0%, hsl(var(--accent-h) 15% 25%) 55%, hsl(0,0%,18%) 100%)" }}
                className="h-8 px-5 rounded-md text-[12.5px] font-semibold text-[hsl(0,0%,93%)] hover:brightness-110 transition-all disabled:opacity-40 disabled:pointer-events-none"
                disabled={!url.trim() || submitting}
              >
                {submitting ? "Adding…" : "Download"}
              </button>
            </div>
          </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
