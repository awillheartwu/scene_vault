import { createApp } from "vue";
import { createPinia } from "pinia";

import App from "./App.vue";
import { router } from "./router";
import { installContextMenuGuard } from "@/composables/useContextMenu";
import { applyPersistedTheme, initThemeSync } from "@/lib/theme";

import "./assets/main.css";

// Apply persisted/system theme before first paint and keep every window
// (main app + popup workbench) in sync live.
applyPersistedTheme();
void initThemeSync();

// The WebView native context menu must not appear in the desktop app; input
// controls keep it for copy/paste. Custom per-item menus are unaffected.
installContextMenuGuard();

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

if (isPopup) {
  void import("./pages/PopupWorkbench.vue").then(({ default: PopupWorkbench }) => {
    createApp(PopupWorkbench).use(createPinia()).mount("#app");
  });
} else {
  createApp(App).use(createPinia()).use(router).mount("#app");
}
