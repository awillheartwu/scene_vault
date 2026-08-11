import { createApp } from "vue";
import { createPinia } from "pinia";
import { invoke } from "@tauri-apps/api/core";

import App from "./App.vue";
import { router } from "./router";
import { installContextMenuGuard } from "@/composables/useContextMenu";
import { applyPersistedTheme, initThemeSync } from "@/lib/theme";
import { installClientErrorLogging } from "@/lib/client-log";
import type { DatabaseStartupStatus } from "@/lib/capture-api";

import "./assets/main.css";

// Apply persisted/system theme before first paint and keep every window
// (main app + popup workbench) in sync live.
applyPersistedTheme();
void initThemeSync();

// The WebView native context menu must not appear in the desktop app; input
// controls keep it for copy/paste. Custom per-item menus are unaffected.
installContextMenuGuard();
installClientErrorLogging();

// The workbench popup is a separate always-on-top window that reuses the same
// frontend bundle. Detect it by Tauri window label (or ?popup=workbench in
// browser previews) and mount the workbench instead of the main app.
const tauriInternals = (
  window as unknown as {
    __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } };
  }
).__TAURI_INTERNALS__;
const isPopup =
  tauriInternals?.metadata?.currentWindow?.label === "classify-popup" ||
  tauriInternals?.metadata?.currentWindow?.label === "note-popup" ||
  new URLSearchParams(location.search).get("popup") === "workbench";

async function queryStartupStatus(): Promise<DatabaseStartupStatus | null> {
  // Browser previews have no Tauri IPC; keep the normal flow untouched.
  if (!tauriInternals) return null;
  try {
    return await invoke<DatabaseStartupStatus>("get_database_startup_status");
  } catch (error) {
    console.error("[app.startup] could not query database startup status", error);
    return null;
  }
}

async function bootstrap() {
  const status = await queryStartupStatus();
  if (status?.mode === "recovery") {
    // Database is unavailable: mount the minimal recovery page so no normal
    // screen can invoke commands that need the real connection pool.
    const { default: RecoveryPage } = await import("./pages/Recovery.vue");
    createApp(RecoveryPage).use(createPinia()).mount("#app");
    return;
  }

  if (isPopup) {
    void import("./pages/PopupWorkbench.vue").then(({ default: PopupWorkbench }) => {
      createApp(PopupWorkbench).use(createPinia()).mount("#app");
    });
  } else {
    createApp(App).use(createPinia()).use(router).mount("#app");
  }
}

void bootstrap();
