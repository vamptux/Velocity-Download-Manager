import { useDeferredValue, useEffect, useMemo, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { X, ListPlus } from "lucide-react";
import { InlineNotice } from "@/components/ui/inline-notice";
import { cn } from "@/lib/utils";
import { ipcAddDownload } from "@/lib/ipc";
import { getCaptureErrorMessage, useDefaultCaptureSavePath } from "@/lib/captureUtils";
import {
  createBatchImportDrafts,
  type BatchImportDraftRow,
  validateBatchImportRows,
} from "@/lib/batchImport";
import {
  buildDuplicateLookupInput,
  describeDuplicateMatch,
  findDuplicateDownload,
} from "@/lib/downloadDuplicates";
import type { Download, DownloadContentCategory } from "@/types/download";

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

const CATEGORY_OPTIONS: DownloadContentCategory[] = [
  "compressed",
  "programs",
  "videos",
  "music",
  "pictures",
  "documents",
];

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
  const [draftRows, setDraftRows] = useState<BatchImportDraftRow[]>([]);

  useDefaultCaptureSavePath(open, defaultSavePath, setDefaultSavePath);
  const deferredUrls = useDeferredValue(urls);

  const parsedDraftPreview = useMemo(
    () => createBatchImportDrafts(deferredUrls, defaultSavePath),
    [defaultSavePath, deferredUrls],
  );

  useEffect(() => {
    setDraftRows(parsedDraftPreview.rows);
  }, [parsedDraftPreview.rows]);

  const preview = useMemo(() => {
    const base = validateBatchImportRows(draftRows, parsedDraftPreview.format, defaultSavePath);
    const rows = base.rows.map((row) => {
      const duplicateMatch = findDuplicateDownload(existingDownloads, buildDuplicateLookupInput({
        url: row.url,
        targetPath: row.targetPath,
      }));

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
  }, [defaultSavePath, draftRows, existingDownloads, parsedDraftPreview.format]);

  const parsedRowByLine = useMemo(
    () => new Map(parsedDraftPreview.rows.map((row) => [row.lineNumber, row])),
    [parsedDraftPreview.rows],
  );
  const draftRowByLine = useMemo(
    () => new Map(draftRows.map((row) => [row.lineNumber, row])),
    [draftRows],
  );

  function updateDraftRow(lineNumber: number, patch: Partial<BatchImportDraftRow>) {
    setDraftRows((prev) => prev.map((row) => (
      row.lineNumber === lineNumber
        ? {
          ...row,
          ...patch,
          seedErrors: patch.category !== undefined || patch.startImmediately !== undefined
            ? []
            : row.seedErrors,
        }
        : row
    )));
    setSubmitError(null);
    setImportResult(null);
  }

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
          setDraftRows([]);
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
                Rows without a folder use {defaultSavePath || "your default download directory"}. Supported columns: url, folder, filename, category, start mode.
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
                <div className="mt-2 max-h-[320px] space-y-2 overflow-y-auto rounded-md border border-border/40 bg-black/10 p-2">
                  {preview.rows.map((row) => {
                    const hasErrors = row.errors.length > 0;
                    return (
                      <div
                        key={row.lineNumber}
                        className={cn(
                          "rounded-md border px-3 py-2.5",
                          hasErrors
                            ? "border-[hsl(var(--status-error)/0.28)] bg-[hsl(var(--status-error)/0.07)]"
                            : "border-border/40 bg-black/10",
                        )}
                      >
                        <div className="flex items-center justify-between gap-3">
                          <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground/46">
                            Row {row.lineNumber}
                          </div>
                          <div className="flex items-center gap-2">
                            <span className={cn(
                              "rounded-full border px-2 py-0.5 text-[10px] uppercase tracking-[0.1em]",
                              hasErrors
                                ? "border-[hsl(var(--status-error)/0.28)] text-[hsl(var(--status-error))]"
                                : "border-[hsl(var(--status-finished)/0.28)] text-[hsl(var(--status-finished))]",
                            )}>
                              {hasErrors ? "Review" : row.startImmediately ? "Start now" : "Queued"}
                            </span>
                            <button
                              type="button"
                              onClick={() => {
                                const sourceRow = parsedRowByLine.get(row.lineNumber);
                                if (sourceRow) {
                                  updateDraftRow(row.lineNumber, sourceRow);
                                }
                              }}
                              className="rounded-md border border-border/60 px-2 py-1 text-[10.5px] text-muted-foreground/64 transition-colors hover:bg-accent hover:text-foreground"
                            >
                              Reset row
                            </button>
                          </div>
                        </div>

                        <div className="mt-2 grid gap-2 md:grid-cols-2">
                          <label className="flex flex-col gap-1 text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground/42">
                            URL
                            <input
                              value={draftRowByLine.get(row.lineNumber)?.url ?? row.url}
                              onChange={(event) => updateDraftRow(row.lineNumber, { url: event.target.value })}
                              className="rounded-md border border-border/60 bg-black/20 px-2.5 py-2 text-[11.5px] normal-case tracking-normal text-foreground outline-none transition-colors focus:border-primary/50 focus:bg-black/35"
                              disabled={submitting}
                            />
                          </label>
                          <label className="flex flex-col gap-1 text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground/42">
                            Filename
                            <input
                              value={draftRowByLine.get(row.lineNumber)?.filename ?? row.filename}
                              onChange={(event) => updateDraftRow(row.lineNumber, { filename: event.target.value })}
                              className="rounded-md border border-border/60 bg-black/20 px-2.5 py-2 text-[11.5px] normal-case tracking-normal text-foreground outline-none transition-colors focus:border-primary/50 focus:bg-black/35"
                              disabled={submitting}
                            />
                          </label>
                          <label className="flex flex-col gap-1 text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground/42">
                            Folder
                            <input
                              value={draftRowByLine.get(row.lineNumber)?.folder ?? row.folder}
                              onChange={(event) => updateDraftRow(row.lineNumber, { folder: event.target.value })}
                              className="rounded-md border border-border/60 bg-black/20 px-2.5 py-2 text-[11.5px] normal-case tracking-normal text-foreground outline-none transition-colors focus:border-primary/50 focus:bg-black/35"
                              disabled={submitting}
                            />
                          </label>
                        </div>

                        <div className="mt-2 grid gap-2 md:grid-cols-[minmax(0,1fr)_140px]">
                          <label className="flex flex-col gap-1 text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground/42">
                            Category
                            <select
                              value={draftRowByLine.get(row.lineNumber)?.category ?? row.category}
                              onChange={(event) => updateDraftRow(row.lineNumber, {
                                category: event.target.value as DownloadContentCategory,
                              })}
                              className="rounded-md border border-border/60 bg-black/20 px-2.5 py-2 text-[11.5px] text-foreground outline-none transition-colors focus:border-primary/50 focus:bg-black/35"
                              disabled={submitting}
                            >
                              {CATEGORY_OPTIONS.map((category) => (
                                <option key={category} value={category}>
                                  {category}
                                </option>
                              ))}
                            </select>
                          </label>
                          <label className="flex flex-col gap-1 text-[10.5px] uppercase tracking-[0.08em] text-muted-foreground/42">
                            Start mode
                            <select
                              value={(draftRowByLine.get(row.lineNumber)?.startImmediately ?? row.startImmediately) ? "now" : "queued"}
                              onChange={(event) => updateDraftRow(row.lineNumber, {
                                startImmediately: event.target.value === "now",
                              })}
                              className="rounded-md border border-border/60 bg-black/20 px-2.5 py-2 text-[11.5px] text-foreground outline-none transition-colors focus:border-primary/50 focus:bg-black/35"
                              disabled={submitting}
                            >
                              <option value="now">Start now</option>
                              <option value="queued">Queued</option>
                            </select>
                          </label>
                        </div>

                        {hasErrors ? (
                          <div className="mt-2 space-y-1 text-[10.5px] text-[hsl(var(--status-error))]">
                            {row.errors.map((error) => (
                              <div key={`${row.lineNumber}:${error}`}>{error}</div>
                            ))}
                          </div>
                        ) : null}
                      </div>
                    );
                  })}
                </div>
                <div className="mt-2 text-[11px] text-muted-foreground/55">
                  Edit any parsed row before import. VDM revalidates duplicates and required fields live.
                </div>
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
