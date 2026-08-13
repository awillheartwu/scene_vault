<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { ChevronsLeft, ChevronsRight, Pin, PinOff, X } from "@lucide/vue";
import { captureApi } from "@/lib/capture-api";
import PopupClassify from "./PopupClassify.vue";
import PopupNote from "./PopupNote.vue";

type NoteComponent = InstanceType<typeof PopupNote>;

// Split-pane workbench: classify on the left, notes on the right, with
// chevron buttons in the middle to collapse either pane. At least one pane
// stays visible: once one side is collapsed, the other cannot be collapsed.
type PaneMode = "both" | "classify" | "note";

const pane = ref<PaneMode>("both");
const leftCollapsed = ref(false);
const rightCollapsed = ref(false);
const noteRef = ref<NoteComponent | null>(null);
const projectTitle = ref("截图分类 · 快速笔记");
const windowLabel = ref<string | null>(null);
const pinned = ref(false);
const unlisteners: UnlistenFn[] = [];

async function syncPane() {
  let split = false;
  try {
    split = (await captureApi.getAppSettings()).splitPopupWindows;
  } catch {
    // Browser previews have no Tauri bridge; keep the combined layout.
    return;
  }
  if (!split) {
    pane.value = "both";
  } else if (windowLabel.value === "note-popup") {
    pane.value = "note";
  } else {
    pane.value = "classify";
  }
}

async function syncProjectTitle() {
  const projectId = localStorage.getItem("scene-vault.capture.project");
  if (!projectId) {
    projectTitle.value = "截图分类 · 快速笔记";
    return;
  }
  try {
    const projects = await captureApi.listProjects();
    const project = projects.find((value) => value.id === projectId);
    projectTitle.value = project?.name ?? "截图分类 · 快速笔记";
  } catch {
    // Keep the default subtitle when the bridge is unavailable.
  }
}

async function closeWindow() {
  try {
    await getCurrentWindow().hide();
  } catch {
    window.close();
  }
}

async function syncPinState() {
  try {
    pinned.value = await getCurrentWindow().isAlwaysOnTop();
  } catch {
    pinned.value = false;
  }
}

async function togglePin() {
  const next = !pinned.value;
  try {
    await invoke("set_window_always_on_top", {
      windowLabel: getCurrentWindow().label,
      always: next,
    });
    pinned.value = next;
  } catch (error) {
    // Keep the button in its current state on failure.
    console.error("[workbench] pin toggle failed:", error);
  }
}

function toggleLeft() {
  if (leftCollapsed.value) {
    leftCollapsed.value = false;
  } else if (!rightCollapsed.value) {
    leftCollapsed.value = true;
  }
}

function toggleRight() {
  if (rightCollapsed.value) {
    rightCollapsed.value = false;
  } else if (!leftCollapsed.value) {
    // Collapsing the notes pane: save any pending draft first so nothing is
    // lost while the pane is hidden.
    noteRef.value?.saveIfDirty();
    rightCollapsed.value = true;
  }
}

onMounted(async () => {
  try {
    windowLabel.value = getCurrentWindow().label;
  } catch {
    // Browser preview.
  }
  void syncPinState();
  await syncPane();
  void syncProjectTitle();
  try {
    unlisteners.push(
      await listen("popup:configure", () => void syncPane()),
      await listen("note:refresh", () => void syncProjectTitle()),
    );
  } catch {
    // Event bridge unavailable in browser previews.
  }
});

onBeforeUnmount(() => {
  unlisteners.forEach((unlisten) => unlisten());
});
</script>

<template>
  <section class="workbench">
    <header class="workbench-header" data-tauri-drag-region="deep">
      <div class="workbench-title" data-tauri-drag-region="deep">
        <span class="workbench-dot" />
        <strong>Scene Vault</strong>
        <span class="workbench-sub">{{ projectTitle }}</span>
      </div>
      <button
        type="button"
        class="workbench-pin"
        :class="{ pinned }"
        :aria-label="pinned ? '取消置顶' : '置顶'"
        :title="pinned ? '取消置顶' : '置顶'"
        @click="togglePin"
      >
        <Pin v-if="pinned" :size="16" /><PinOff v-else :size="16" />
      </button>
      <button type="button" class="workbench-close" aria-label="关闭" title="关闭" @click="closeWindow">
        <X :size="16" />
      </button>
    </header>

    <div v-if="pane === 'classify'" class="workbench-single">
      <PopupClassify embedded />
    </div>
    <div v-else-if="pane === 'note'" class="workbench-single">
      <PopupNote embedded />
    </div>
    <div v-else class="workbench-split">
      <div class="panel" :class="{ collapsed: leftCollapsed }">
        <PopupClassify embedded />
      </div>

      <div class="splitter">
        <button
          type="button"
          class="splitter-btn"
          :class="{ on: leftCollapsed }"
          :disabled="!leftCollapsed && rightCollapsed"
          :title="leftCollapsed ? '展开左侧截图分类' : '收起左侧截图分类'"
          @click="toggleLeft"
        >
          <ChevronsLeft v-if="!leftCollapsed" :size="15" />
          <ChevronsRight v-else :size="15" />
        </button>
        <button
          type="button"
          class="splitter-btn"
          :class="{ on: rightCollapsed }"
          :disabled="!rightCollapsed && leftCollapsed"
          :title="rightCollapsed ? '展开右侧笔记' : '收起右侧笔记'"
          @click="toggleRight"
        >
          <ChevronsRight v-if="!rightCollapsed" :size="15" />
          <ChevronsLeft v-else :size="15" />
        </button>
      </div>

      <div class="panel" :class="{ collapsed: rightCollapsed }">
        <PopupNote ref="noteRef" embedded />
      </div>
    </div>
  </section>
</template>

<style scoped>
.workbench {
  display: flex;
  width: 100%;
  height: 100vh;
  min-height: 0;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-gradient);
  color: var(--foreground);
}

.workbench-header {
  display: flex;
  height: 48px;
  flex: none;
  align-items: center;
  justify-content: space-between;
  padding: 0 10px 0 14px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 72%, transparent);
  background: color-mix(in srgb, var(--sidebar) 72%, transparent);
  backdrop-filter: var(--panel-blur);
  user-select: none;
}

.workbench-title {
  display: flex;
  flex: 1;
  min-width: 0;
  align-items: center;
  gap: 9px;
  font-size: 12.5px;
}

.workbench-title strong {
  flex: none;
  font-weight: 700;
  letter-spacing: 0.04em;
}

.workbench-sub {
  overflow: hidden;
  color: var(--muted-foreground);
  font-size: 11.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workbench-dot {
  width: 8px;
  height: 8px;
  flex: none;
  border-radius: 50%;
  background: var(--ok);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ok) 18%, transparent);
}

.workbench-close,
.workbench-pin {
  display: grid;
  width: 40px;
  height: 40px;
  margin-left: 4px;
  place-items: center;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
  transition:
    background-color var(--motion-fast),
    border-color var(--motion-fast),
    color var(--motion-fast);
}

.workbench-close:hover {
  background: color-mix(in srgb, var(--destructive) 14%, transparent);
  color: var(--destructive);
}

.workbench-close:focus-visible,
.workbench-pin:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.workbench-pin:hover {
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--accent);
}

.workbench-pin.pinned {
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--accent);
}

.workbench-single {
  display: flex;
  min-height: 0;
  flex: 1;
  overflow: hidden;
}

.workbench-single > * {
  min-width: 0;
  min-height: 0;
  flex: 1;
}

.workbench-split {
  display: flex;
  min-height: 0;
  flex: 1;
  overflow: hidden;
}

.panel {
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1 1 0;
  overflow: hidden;
  transition: flex-basis 0.22s ease;
}

.panel.collapsed {
  flex: 0 0 0;
}

.panel > * {
  min-width: 0;
  min-height: 0;
  flex: 1;
}

.splitter {
  display: flex;
  width: 44px;
  flex: none;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  border-left: 1px solid color-mix(in srgb, var(--border) 75%, transparent);
  border-right: 1px solid color-mix(in srgb, var(--border) 75%, transparent);
  background: color-mix(in srgb, var(--sidebar) 70%, transparent);
  backdrop-filter: var(--panel-blur);
  user-select: none;
}

.splitter-btn {
  display: grid;
  width: 40px;
  height: 40px;
  place-items: center;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
  transition: background-color 0.18s, color 0.18s, border-color 0.18s;
}

.splitter-btn:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.splitter-btn:hover:not(:disabled) {
  border-color: color-mix(in srgb, var(--accent) 35%, var(--border));
  background: color-mix(in srgb, var(--accent) 10%, transparent);
  color: var(--accent);
}

.splitter-btn:disabled {
  opacity: 0.3;
  cursor: not-allowed;
}

.splitter-btn.on {
  border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--accent);
}
</style>
