import type { AppSettings } from "@/lib/capture-api";

// Capture-cache draft shared across ResourceStoragePanel mounts: the settings
// pane unmounts when another category is shown, and switching categories must
// not silently reset the user's unsaved edits. Cleared on a successful save.
let draft: AppSettings | null = null;

export function rememberCaptureCleanupDraft(value: AppSettings): void {
  draft = { ...value };
}

export function captureCleanupDraft(): AppSettings | null {
  return draft;
}

export function clearCaptureCleanupDraft(): void {
  draft = null;
}
