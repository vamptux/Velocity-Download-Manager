import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";

export async function readClipboardText(): Promise<string> {
  try {
    const value = await readText();
    if (typeof value === "string") {
      return value;
    }
  } catch {
    // Fall through to the browser clipboard API when Tauri is unavailable.
  }

  if (typeof navigator !== "undefined" && navigator.clipboard?.readText) {
    return navigator.clipboard.readText();
  }

  return "";
}

export async function writeClipboardText(value: string): Promise<void> {
  try {
    await writeText(value);
    return;
  } catch {
    // Fall through to the browser clipboard API when Tauri is unavailable.
  }

  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
  }
}