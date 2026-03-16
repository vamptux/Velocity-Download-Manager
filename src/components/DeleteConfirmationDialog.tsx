import * as Dialog from "@radix-ui/react-dialog";
import { X, Trash2, AlertTriangle } from "lucide-react";
import { cn } from "@/lib/utils";
import { useState, useEffect } from "react";

interface DeleteConfirmationDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (deleteFile: boolean) => void;
  count: number;
}

export function DeleteConfirmationDialog({
  open,
  onOpenChange,
  onConfirm,
  count,
}: DeleteConfirmationDialogProps) {
  const [deleteFile, setDeleteFile] = useState(false);

  useEffect(() => {
    if (open) setDeleteFile(false);
  }, [open]);

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[60] bg-black/60 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=closed]:animate-out data-[state=closed]:fade-out-0" />
        <Dialog.Content
          className={cn(
            "fixed left-1/2 top-1/2 z-[70] -translate-x-1/2 -translate-y-1/2 outline-none",
            "w-[360px] rounded-lg border border-border/70 bg-[hsl(var(--background))] shadow-[0_24px_48px_rgba(0,0,0,0.55)]",
            "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
            "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
          )}
        >
          {/* Header */}
          <div className="flex items-center justify-between border-b border-border/50 px-4 py-2.5">
            <div className="flex items-center gap-2">
              <Trash2 size={13} className="text-[hsl(var(--status-error)/0.7)] shrink-0" strokeWidth={1.7} />
              <Dialog.Title className="text-[12.5px] font-semibold text-foreground/90">
                Delete {count > 1 ? `${count} downloads` : "download"}
              </Dialog.Title>
            </div>
            <Dialog.Close className="flex h-[22px] w-[22px] items-center justify-center rounded text-muted-foreground/40 hover:bg-white/[0.07] hover:text-foreground/70 transition-colors">
              <X size={11} strokeWidth={1.7} />
            </Dialog.Close>
          </div>

          {/* Body */}
          <div className="px-4 py-3 flex flex-col gap-3">
            <p className="text-[12px] text-muted-foreground/65 leading-relaxed">
              {count > 1
                ? `Remove ${count} downloads from the list?`
                : "Remove this download from the list?"}
            </p>

            {/* Delete-files toggle */}
            <button
              type="button"
              onClick={() => setDeleteFile((v) => !v)}
              className={cn(
                "flex items-center gap-3 w-full rounded-md border px-3 py-2 text-left transition-colors",
                deleteFile
                  ? "border-[hsl(var(--status-error)/0.35)] bg-[hsl(var(--status-error)/0.07)]"
                  : "border-border/50 bg-white/[0.02] hover:bg-white/[0.04]",
              )}
            >
              {/* custom toggle indicator */}
              <span
                className={cn(
                  "flex h-[14px] w-[14px] shrink-0 items-center justify-center rounded-[3px] border transition-colors",
                  deleteFile
                    ? "border-[hsl(var(--status-error)/0.6)] bg-[hsl(var(--status-error)/0.18)]"
                    : "border-white/[0.18] bg-transparent",
                )}
              >
                {deleteFile && (
                  <X size={8} strokeWidth={2.5} className="text-[hsl(var(--status-error))]" />
                )}
              </span>
              <div className="flex flex-col min-w-0">
                <span className={cn(
                  "text-[11.5px] font-medium leading-tight",
                  deleteFile ? "text-[hsl(var(--status-error)/0.9)]" : "text-foreground/75",
                )}>
                  Also delete files from disk
                </span>
                <span className="text-[10px] text-muted-foreground/45 mt-[2px]">Cannot be undone</span>
              </div>
              {deleteFile && (
                <AlertTriangle size={12} className="ml-auto shrink-0 text-[hsl(var(--status-error)/0.65)]" strokeWidth={1.7} />
              )}
            </button>

            {/* Actions */}
            <div className="flex justify-end gap-1.5 pt-0.5">
              <Dialog.Close asChild>
                <button className="h-[26px] px-3 rounded-[3px] border border-border/70 text-[11.5px] text-muted-foreground/65 hover:bg-accent hover:text-foreground transition-colors">
                  Cancel
                </button>
              </Dialog.Close>
              <button
                type="button"
                onClick={() => {
                  onConfirm(deleteFile);
                  onOpenChange(false);
                }}
                className={cn(
                  "h-[26px] px-3 rounded-[3px] text-[11.5px] font-medium transition-all",
                  deleteFile
                    ? "bg-[hsl(var(--status-error))] text-white hover:brightness-110"
                    : "border border-[hsl(var(--status-error)/0.4)] text-[hsl(var(--status-error)/0.85)] bg-[hsl(var(--status-error)/0.08)] hover:bg-[hsl(var(--status-error)/0.14)] hover:text-[hsl(var(--status-error))]",
                )}
              >
                {deleteFile ? "Delete from disk" : "Remove"}
              </button>
            </div>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
