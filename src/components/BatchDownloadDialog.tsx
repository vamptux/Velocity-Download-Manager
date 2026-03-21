import { useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { X, ListPlus } from "lucide-react";
import { cn } from "@/lib/utils";
import { ipcAddDownload } from "@/lib/ipc";
import { getCaptureErrorMessage } from "@/lib/captureUtils";
import type { Download } from "@/types/download";

interface BatchDownloadDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDownloadsAdded?: (downloads: Download[]) => void;
}

export function BatchDownloadDialog({ open, onOpenChange, onDownloadsAdded }: BatchDownloadDialogProps) {
  const [urls, setUrls] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ total: number; done: number } | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!urls.trim() || submitting) return;

    const lines = urls.split("\n").map(line => line.trim()).filter(line => line.length > 0);
    if (lines.length === 0) return;

    setSubmitting(true);
    setSubmitError(null);
    setProgress({ total: lines.length, done: 0 });

    const addedDownloads: Download[] = [];

    try {
      for (let i = 0; i < lines.length; i++) {
        const url = lines[i];
        try {
          const download = await ipcAddDownload({
            url,
            category: "compressed",
            savePath: "",
            startImmediately: true,
          });
          addedDownloads.push(download);
        } catch (err) {
          console.error("Failed to add download for", url, err);
          // Continue with others even if one fails
        }
        setProgress({ total: lines.length, done: i + 1 });
      }

      onDownloadsAdded?.(addedDownloads);
      onOpenChange(false);
      setUrls("");
      setProgress(null);
    } catch (error) {
      setSubmitError(getCaptureErrorMessage(error));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Dialog.Root open={open} onOpenChange={(val) => {
      if (!submitting) {
        onOpenChange(val);
        if (!val) {
          setUrls("");
          setSubmitError(null);
          setProgress(null);
        }
      }
    }}>
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
                <ListPlus size={11} className="text-white" strokeWidth={2.2} />
              </div>
              <Dialog.Title className="text-[13px] font-medium tracking-[-0.01em] text-foreground/92">
                Batch Download
              </Dialog.Title>
            </div>
            <Dialog.Close disabled={submitting} className="flex h-6 w-6 items-center justify-center rounded text-muted-foreground/60 hover:bg-accent hover:text-foreground transition-colors disabled:opacity-50">
              <X size={13} strokeWidth={2} />
            </Dialog.Close>
          </div>

          {/* Body */}
          <form onSubmit={handleSubmit} className="flex flex-col gap-3 px-5 py-4">
            <div className="flex flex-col gap-1.5">
              <label htmlFor="batch-urls" className="text-[11.5px] font-medium text-foreground/80">
                Download URLs (one per line)
              </label>
              <textarea
                id="batch-urls"
                value={urls}
                onChange={(e) => setUrls(e.target.value)}
                placeholder="https://example.com/file1.zip&#10;https://example.com/file2.zip"
                className="min-h-[150px] w-full rounded-md border border-border bg-black/20 p-2.5 text-[12px] text-foreground placeholder:text-muted-foreground/40 outline-none focus:border-primary/50 focus:bg-black/40 transition-colors resize-none shadow-inner"
                disabled={submitting}
                autoFocus
              />
            </div>

            {submitError && (
              <div className="rounded border border-[hsl(var(--status-error)/0.2)] bg-[hsl(var(--status-error)/0.1)] px-3 py-2 text-[11.5px] text-[hsl(var(--status-error))]">
                {submitError}
              </div>
            )}

            {progress && (
              <div className="flex items-center justify-between text-[11.5px] text-muted-foreground">
                <span>Processing links...</span>
                <span>{progress.done} / {progress.total}</span>
              </div>
            )}

            {/* Actions */}
            <div className="flex items-center justify-end gap-2.5 pt-2">
              <Dialog.Close asChild>
                <button
                  type="button"
                  disabled={submitting}
                  className="h-8 px-4 rounded-md border border-border text-[12.5px] text-muted-foreground hover:bg-accent hover:text-foreground transition-colors disabled:opacity-50"
                >
                  Cancel
                </button>
              </Dialog.Close>
              <button
                type="submit"
                disabled={!urls.trim() || submitting}
                style={{ background: "linear-gradient(90deg, hsl(var(--accent-h) 22% 32%) 0%, hsl(var(--accent-h) 15% 25%) 55%, hsl(0,0%,18%) 100%)" }}
                className="h-8 px-5 rounded-md text-[12.5px] font-semibold text-[hsl(0,0%,93%)] hover:brightness-110 transition-all disabled:opacity-40 disabled:pointer-events-none"
              >
                {submitting ? "Adding..." : "Add Downloads"}
              </button>
            </div>
          </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
