// Shared theme bootstrap for every window (main app + popup workbench).
// The popup windows are separate Webviews, so localStorage changes alone do
// not reliably propagate: the main window broadcasts `theme:changed` through
// the Tauri event bridge and each window applies it live.
import { emit, listen } from "@tauri-apps/api/event";

export type ThemeValue = "light" | "dark";

export function resolveInitialTheme(): ThemeValue {
  const stored = localStorage.getItem("scene-vault-theme");
  if (stored === "dark" || stored === "light") return stored;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function applyTheme(value: ThemeValue): void {
  document.documentElement.classList.toggle("dark", value === "dark");
}

export function applyPersistedTheme(): ThemeValue {
  const theme = resolveInitialTheme();
  applyTheme(theme);
  return theme;
}

export async function broadcastTheme(value: ThemeValue): Promise<void> {
  try {
    await emit("theme:changed", value);
  } catch {
    // Browser preview / unit tests have no Tauri bridge.
  }
}

export async function initThemeSync(): Promise<void> {
  // Live sync from the main window while this popup/webview is open.
  try {
    await listen<ThemeValue>("theme:changed", (event) => {
      const value = event.payload;
      localStorage.setItem("scene-vault-theme", value);
      applyTheme(value);
    });
  } catch {
    // No Tauri bridge.
  }
  // Same-origin fallback for browsers/WebView2 storage events.
  window.addEventListener("storage", (event) => {
    if (event.key === "scene-vault-theme" && (event.newValue === "light" || event.newValue === "dark")) {
      applyTheme(event.newValue);
    }
  });
}
