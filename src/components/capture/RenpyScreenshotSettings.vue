<script setup lang="ts">
import { computed } from "vue";
import { AlertCircle } from "@lucide/vue";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import {
  pathFileName,
  type ProjectEngineIntegrationView,
  type RenpyScreenshotPolicyView,
} from "@/lib/capture-api";

const props = defineProps<{
  open: boolean;
  integrations: ProjectEngineIntegrationView[];
  policy: RenpyScreenshotPolicyView | null;
  loading: boolean;
  loadError: string | null;
  working: boolean;
  saving: boolean;
  warning: string | null;
}>();
const emit = defineEmits<{
  (event: "update:open", open: boolean): void;
  (event: "project-change", autoHideUi: boolean | null): void;
  (event: "installation-change", view: ProjectEngineIntegrationView, autoHideUi: boolean | null): void;
  (event: "retry"): void;
}>();

const disabled = computed(() =>
  props.working || props.saving || props.loading || Boolean(props.loadError) || !props.policy,
);

function directoryName(root: string) {
  return pathFileName(root.replace(/[\\/]+$/, ""));
}

function selection(value: boolean | null): string {
  return value === null ? "inherit" : value ? "on" : "off";
}

function selectedValue(event: Event, previous: boolean | null): boolean | null {
  const select = event.target as HTMLSelectElement;
  const value = select.value === "on" ? true : select.value === "off" ? false : null;
  // Keep the persisted choice visible until the parent receives a successful save.
  select.value = selection(previous);
  return value;
}

function changeProject(event: Event) {
  const value = selectedValue(event, props.policy?.projectAutoHideUi ?? null);
  if (!disabled.value) emit("project-change", value);
}

function changeInstallation(view: ProjectEngineIntegrationView, event: Event) {
  const value = selectedValue(event, view.integration.autoHideUiOverride);
  if (!disabled.value) emit("installation-change", view, value);
}
</script>

<template>
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <DialogContent class="renpy-screenshot-settings" aria-label="截图设置">
      <DialogTitle>截图设置</DialogTitle>
      <DialogDescription>项目可跟随全局，每个游戏可覆盖项目设置。全局默认关闭。</DialogDescription>
      <p class="screenshot-settings-help">
        启用后，按原截图键（通常为 S）自动隐藏标准剧情 UI。若当前界面不支持，不保存截图并在游戏中提示；
        请先按 H 隐藏 UI，再按 S 截图，或关闭此选项。
      </p>
      <p class="screenshot-settings-help">
        插件更新后需重启游戏一次；之后修改设置会在下一次截图时应用，无需再次安装。
        已有选择保留，可按项目或游戏覆盖。
      </p>
      <div v-if="loadError" class="screenshot-settings-error">
        <AlertCircle :size="16" />
        <span role="alert">截图设置加载失败：{{ loadError }}。请重试读取后再修改。</span>
        <button type="button" :disabled="working || saving || loading" @click="emit('retry')">重试读取截图设置</button>
      </div>
      <p v-else-if="loading" class="screenshot-settings-help" role="status">正在读取截图设置…</p>
      <label class="screenshot-settings-project">
        <span>项目默认</span>
        <select
          :value="selection(policy?.projectAutoHideUi ?? null)"
          :disabled="disabled"
          aria-label="项目截图设置"
          @change="changeProject"
        >
          <option value="inherit">跟随全局（{{ policy ? (policy.globalAutoHideUi ? "启用" : "关闭") : "读取中" }}）</option>
          <option value="on">启用</option>
          <option value="off">关闭</option>
        </select>
      </label>
      <p v-if="policy" class="screenshot-settings-state">
        当前项目有效：{{ policy.resolvedAutoHideUi ? "启用" : "关闭" }}；仅跟随项目的游戏会自动应用项目默认。
      </p>
      <p v-if="warning" class="screenshot-settings-warning" role="alert">
        <AlertCircle :size="16" />{{ warning }}
      </p>
      <ul class="renpy-screenshot-list" aria-label="已安装游戏的截图设置">
        <li v-for="view in integrations" :key="view.integration.id" class="renpy-screenshot-row">
          <div class="screenshot-game-info">
            <span class="screenshot-game-name" :title="view.integration.engineRoot">
              {{ directoryName(view.integration.engineRoot) }}
            </span>
            <span class="screenshot-game-version">Ren'Py {{ view.integration.engineVersion ?? "版本未知" }}</span>
            <span class="screenshot-game-state">当前有效：{{ view.resolvedAutoHideUi ? "启用" : "关闭" }}</span>
          </div>
          <label class="screenshot-settings-toggle">
            <span>自动隐藏 UI</span>
            <select
              :value="selection(view.integration.autoHideUiOverride)"
              :disabled="disabled"
              :aria-label="view.integration.engineRoot + '：自动隐藏 UI'"
              @change="changeInstallation(view, $event)"
            >
              <option value="inherit">跟随项目（{{ policy ? (policy.resolvedAutoHideUi ? "启用" : "关闭") : "读取中" }}）</option>
              <option value="on">启用</option>
              <option value="off">关闭</option>
            </select>
          </label>
          <p
            v-if="view.integration.bridgeVersion !== view.bundledBridgeVersion"
            class="screenshot-settings-warning screenshot-game-warning"
            role="alert"
          >
            <AlertCircle :size="16" />插件需要更新（{{ view.integration.bridgeVersion }} → {{ view.bundledBridgeVersion }}）：
            请重新安装并重启游戏一次，更新后的插件才会在截图时应用此设置。
          </p>
          <p v-if="view.warning" class="screenshot-settings-warning screenshot-game-warning" role="alert">
            <AlertCircle :size="16" />{{ view.warning }}
          </p>
        </li>
        <li v-if="!integrations.length" class="screenshot-settings-empty">
          当前项目还没有安装游戏插件。仍可保存项目默认，之后安装的游戏可跟随此设置。
        </li>
      </ul>
      <div class="screenshot-settings-footer">
        <span role="status" aria-live="polite">{{ saving ? "正在保存…" : "修改后自动保存" }}</span>
        <button type="button" @click="emit('update:open', false)">完成</button>
      </div>
    </DialogContent>
  </Dialog>
</template>

<style scoped>
.renpy-screenshot-settings {
  max-width: min(600px, calc(100vw - 32px));
  max-height: calc(100dvh - 64px);
  overflow-y: auto;
  background: var(--card);
}
.screenshot-settings-help {
  margin: 0;
  color: var(--muted-foreground);
  font-size: 12px;
  line-height: 1.6;
}
.screenshot-settings-project { display: grid; grid-template-columns: 64px minmax(0, 1fr); align-items: center; gap: 12px; color: var(--foreground); font-size: 13px; }
.screenshot-settings-project select, .screenshot-settings-toggle select {
  min-width: 0;
  height: 36px;
  padding: 0 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--card);
  color: var(--foreground);
  font-size: 12px;
}
.screenshot-settings-project select { width: 100%; }
.screenshot-settings-toggle select { max-width: 100%; }
.screenshot-settings-project select:disabled, .screenshot-settings-toggle select:disabled { cursor: not-allowed; opacity: .55; }
.screenshot-settings-project select:focus-visible, .screenshot-settings-toggle select:focus-visible, .screenshot-settings-error button:focus-visible { outline: 2px solid var(--ring); outline-offset: 2px; }
.screenshot-settings-state { margin: 0; color: var(--muted-foreground); font-size: 12px; line-height: 1.6; }
.screenshot-settings-error { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; color: var(--destructive); font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
.screenshot-settings-error > svg { flex: none; }
.screenshot-settings-error span { flex: 1; min-width: 0; }
.screenshot-settings-error button { padding: 6px 12px; border: 1px solid var(--destructive); border-radius: var(--radius-control); background: var(--card); color: inherit; }
.screenshot-settings-error button:disabled { cursor: not-allowed; opacity: .55; }
.renpy-screenshot-list {
  display: grid;
  gap: 8px;
  max-height: min(360px, 50vh);
  margin: 0;
  padding: 0;
  overflow-y: auto;
  list-style: none;
  background: var(--card);
}
.renpy-screenshot-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px 16px;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
}
.screenshot-game-info { display: grid; flex: 1; min-width: 0; gap: 4px; }
.screenshot-game-name { overflow: hidden; color: var(--foreground); font-size: 13px; text-overflow: ellipsis; white-space: nowrap; }
.screenshot-game-version, .screenshot-game-state { color: var(--muted-foreground); font-size: 11px; }
.screenshot-settings-toggle { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; max-width: 100%; color: var(--foreground); font-size: 12px; }
.screenshot-settings-warning { display: flex; align-items: flex-start; gap: 6px; margin: 0; color: var(--warn); font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
.screenshot-settings-warning > svg { flex: none; margin-top: 2px; }
.screenshot-game-warning { flex-basis: 100%; }
.screenshot-settings-empty { padding: 12px; color: var(--muted-foreground); font-size: 12px; }
.screenshot-settings-footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; color: var(--muted-foreground); font-size: 12px; }
.screenshot-settings-footer button { padding: 6px 12px; border: 1px solid var(--border); border-radius: var(--radius-control); background: var(--card); color: var(--foreground); }
</style>
