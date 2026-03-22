import { useMemo, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { X, ListPlus } from "lucide-react";
import { InlineNotice } from "@/components/ui/inline-notice";
import { cn } from "@/lib/utils";
import { ipcAddDownload } from "@/lib/ipc";
import { getCaptureErrorMessage, useDefaultCaptureSavePath } from "@/lib/captureUtils";
import { parseBatchImportInput } from "@/lib/batchImport";
import { describeDuplicateMatch, findDuplicateDownload } from "@/lib/downloadDuplicates";
import type { Download } from "@/types/download";

interface BatchDownloadDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDownloadsAdded?: (downloads: Download[]) => void;
  existingDownloads?: Download[];
}

interface BatchImportResult {
  successCount: number;
  failureCount: number;
  failures: Array<{ lineNumber: number; label: string; message: string }>;
}

export function BatchDownloadDialog({
  open,
  onOpenChange,
  onDownloadsAdded,
  existingDownloads = [],
}: BatchDownloadDialogProps) {
  const [urls, setUrls] = useState("");
  const [defaultSavePath, setDefaultSavePath] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ total: number; done: number } | null>(null);
  const [importResult, setImportResult] = useState<BatchImportResult | null>(null);

  useDefaultCaptureSavePath(open, defaultSavePath, setDefaultSavePath);

  const preview = useMemo(() => {
    const base = parseBatchImportInput(urls, defaultSavePath);
    const rows = base.rows.map((row) => {
      const duplicateMatch = findDuplicateDownload(existingDownloads, {
        url: row.url,
        targetPath: row.targetPath,
      });

      if (!duplicateMatch) {
        return row;
      }

      return {
        ...row,
        errors: [...row.errors, describeDuplicateMatch(duplicateMatch)],
      };
    });

    const invalidCount = rows.filter((row) => row.errors.length > 0).length;
    return {
      ...base,
      rows,
      validCount: rows.length - invalidCount,
      invalidCount,
    };
  }, [defaultSavePath, existingDownloads, urls]);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!urls.trim() || submitting) return;

    const validRows = preview.rows.filter((row) => row.errors.length === 0);
    if (validRows.length === 0) return;

    setSubmitting(true);
    setSubmitError(null);
    setImportResult(null);
    setProgress({ total: validRows.length, done: 0 });

    const addedDownloads: Download[] = [];
    const failures: BatchImportResult["failures"] = [];

    try {
      for (let index = 0; index < validRows.length; index += 1) {
        const row = validRows[index];
        try {
          const download = await ipcAddDownload({
            url: row.url,
            name: row.filename,
            category: row.category,
            savePath: row.folder,
            checksum: row.checksum ?? undefined,
            startImmediately: row.startImmediately,
          });
          addedDownloads.push(download);
        } catch (err) {
          failures.push({
            lineNumber: row.lineNumber,
            label: row.filename || row.url,
            message: getCaptureErrorMessage(err),
          });
        }
        setProgress({ total: validRows.length, done: index + 1 });
      }

      onDownloadsAdded?.(addedDownloads);

      if (failures.length === 0 && preview.invalidCount === 0) {
        onOpenChange(false);
        setUrls("");
        setImportResult(null);
      } else {
        setImportResult({
          successCount: addedDownloads.length,
          failureCount: failures.length,
          failures,
        });
      }
    } catch (error) {
      setSubmitError(getCaptureErrorMessage(error));
    } finally {
      setProgress(null);
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
          setImportResult(null);
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
                Paste URLs, CSV, or TSV
              </label>
              <textarea
                id="batch-urls"
                value={urls}
                onChange={(e) => {
                  setUrls(e.target.value);
                  setSubmitError(null);
                  setImportResult(null);
                }}
                placeholder={"https://example.com/file1.zip\nhttps://example.com/file2.zip\n\nurl,folder,filename,category,start mode\nhttps://example.com/file3.zip,C:\\Downloads,file3.zip,compressed,queued"}
                className="min-h-[150px] w-full rounded-md border border-border bg-black/20 p-2.5 text-[12px] text-foreground placeholder:text-muted-foreground/40 outline-none focus:border-primary/50 focus:bg-black/40 transition-colors resize-none shadow-inner"
                disabled={submitting}
                autoFocus
              />
              <div className="text-[11px] text-muted-foreground/55">
                Rows without a folder use {defaultSavePath || "your default download directory"}. Supported columns: url, folder, filename, checksum, category, start mode.
              </div>
            </div>

            {submitError && (
              <div className="rounded border border-[hsl(var(--status-error)/0.2)] bg-[hsl(var(--status-error)/0.1)] px-3 py-2 text-[11.5px] text-[hsl(var(--status-error))]">
                {submitError}
              </div>
            )}

            {preview.rows.length > 0 ? (
              <div className="rounded-lg border border-border/60 bg-black/10 p-3">
                <div className="flex items-center justify-between gap-3 text-[11px] text-muted-foreground/70">
                  <span>Detected format: {preview.format.toUpperCase()}</span>
                  <span>{preview.validCount} ready, {preview.invalidCount} need review</span>
                </div>
                <div className="mt-2 max-h-[190px] overflow-y-auto rounded-md border border-border/40 bg-black/10">
                  <div className="grid grid-cols-[52px_minmax(0,1.4fr)_minmax(0,1.1fr)_88px] gap-x-2 border-b border-border/40 px-3 py-2 text-[10px] uppercase tracking-[0.12em] text-muted-foreground/45">
                    <span>Row</span>
                    <span>File</span>
                    <span>Folder</span>
                    <span>Status</span>
                  </div>
                  {preview.rows.slice(0, 8).map((row) => {
                    const hasErrors = row.errors.length > 0;
                    return (
                      <div
                        key={`${row.lineNumber}:${row.url}`}
                        className="grid grid-cols-[52px_minmax(0,1.4fr)_minmax(0,1.1fr)_88px] gap-x-2 border-b border-border/20 px-3 py-2 text-[11px] last:border-b-0"
                      >
                        <span className="text-muted-foreground/60">{row.lineNumber}</span>
                        <div className="min-w-0">
                          <div className="truncate text-foreground/84">{row.filename || row.url}</div>
                          <div className="truncate text-muted-foreground/50">{row.url}</div>
                          {hasErrors ? (
                            <div className="mt-1 text-[10.5px] text-[hsl(var(--status-error))]">
                              {row.errors[0]}
                            </div>
                          ) : null}
                        </div>
                        <div className="min-w-0 truncate text-muted-foreground/60">{row.folder || "Missing"}</div>
                        <div className={hasErrors ? "text-[hsl(var(--status-error))]" : "text-[hsl(var(--status-finished))]"}>
                          {hasErrors ? "Review" : row.startImmediately ? "Start now" : "Queued"}
                        </div>
                      </div>
                    );
                  })}
                </div>
                {preview.rows.length > 8 ? (
                  <div className="mt-2 text-[11px] text-muted-foreground/55">
                    Showing 8 of {preview.rows.length} parsed rows.
                  </div>
                ) : null}
              </div>
            ) : null}

            {preview.invalidCount > 0 ? (
              <InlineNotice
                tone="warning"
                title="Some rows need review"
                message="VDM will only import rows that validate cleanly and do not duplicate existing downloads."
              />
            ) : null}

            {importResult ? (
              <InlineNotice
                tone={importResult.failureCount > 0 ? "warning" : "success"}
                title={importResult.failureCount > 0 ? "Import finished with issues" : "Import complete"}
                message={importResult.failureCount > 0
                  ? `${importResult.successCount} downloads were added and ${importResult.failureCount} rows failed.`
                  : `${importResult.successCount} downloads were added.`}
              />
            ) : null}

            {importResult?.failures.length ? (
              <div className="max-h-[100px] overflow-y-auto rounded-md border border-[hsl(var(--status-paused)/0.2)] bg-[hsl(var(--status-paused)/0.07)] px-3 py-2 text-[11px] text-foreground/74">
                {importResult.failures.slice(0, 5).map((failure) => (
                  <div key={`${failure.lineNumber}:${failure.label}`} className="py-0.5">
                    Row {failure.lineNumber}: {failure.message}
                  </div>
                ))}
              </div>
            ) : null}

            {progress && (
              <div className="flex items-center justify-between text-[11.5px] text-muted-foreground">
                <span>Importing rows...</span>
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
                disabled={preview.validCount === 0 || submitting}
                style={{ background: "linear-gradient(90deg, hsl(var(--accent-h) 22% 32%) 0%, hsl(var(--accent-h) 15% 25%) 55%, hsl(0,0%,18%) 100%)" }}
                className="h-8 px-5 rounded-md text-[12.5px] font-semibold text-[hsl(0,0%,93%)] hover:brightness-110 transition-all disabled:opacity-40 disabled:pointer-events-none"
              >
                {submitting ? "Importing..." : preview.validCount > 0 ? `Import ${preview.validCount}` : "Nothing to import"}
              </button>
            </div>
          </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
