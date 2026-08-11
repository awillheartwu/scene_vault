<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  Activity,
  Database,
  FolderOpen,
  HardDrive,
  LoaderCircle,
  RefreshCw,
  Save,
  Trash2,
} from "@lucide/vue";
import {
  captureApi,
  openPathExternal,
  revealPath,
  type ProcessResourceGroup,
  type ProcessResourceStatus,
  type LogPolicySettings,
  type StorageResourceEntry,
  type StorageResourceStatus,
} from "@/lib/capture-api";
import { toast } from "@/lib/toast";

const PROCESS_SAMPLE_INTERVAL_MS = 2_000;
const props = withDefaults(defineProps<{ recognizer?: "sface" | "arcface" | null }>(), {
  recognizer: null,
});

const processStatus = ref<ProcessResourceStatus | null>(null);
const storageStatus = ref<StorageResourceStatus | null>(null);
const processLoading = ref(true);
const storageLoading = ref(true);
const processError = ref<string | null>(null);
const storageError = ref<string | null>(null);
const cleanupBusy = ref<string | null>(null);
const logSettings = ref<LogPolicySettings>({
  retentionDays: 14,
  maxFileSizeMb: 5,
  maxArchivedFiles: 20,
  automaticCleanup: true,
});
const logSettingsLoading = ref(true);
const logSettingsBusy = ref(false);
let processTimer: number | null = null;
let processRequestActive = false;

const processRole = {
  rust: { label: "Rust 核心", detail: "任务调度、数据库与文件服务" },
  webview: { label: "Vue / WebView", detail: "界面渲染及 WebView2 子进程" },
  python: { label: "Python AI Worker", detail: "常驻或单次视觉处理进程" },
  helper: { label: "辅助进程", detail: "应用启动的其他子进程" },
} as const;

const cleanupKind: Record<string, "thumbnail_cache" | "capture_output" | "expired_logs"> = {
  thumbnail_cache: "thumbnail_cache",
  capture_output: "capture_output",
  logs: "expired_logs",
};

const cleanupPrompt: Record<string, string> = {
  thumbnail_cache: "确定清理全部缩略图缓存吗？原图不会受影响，之后浏览时会按需重新生成。",
  capture_output: "确定清理已成功归档截图的本地处理缓存吗？源图和归档文件不会受影响。",
  logs: "确定按保留策略清理过期日志吗？当前日志会保留。",
};

const totalCpu = computed(() => formatPercent(processStatus.value?.totalCpuPercent));
const displayedProcessGroups = computed<ProcessResourceGroup[]>(() => {
  const groups = processStatus.value?.groups ?? [];
  if (groups.some((group) => group.role === "python")) return groups;
  const python: ProcessResourceGroup = {
    role: "python",
    processCount: 0,
    pids: [],
    cpuPercent: null,
    workingSetBytes: 0,
    peakWorkingSetBytes: 0,
    privateBytes: null,
  };
  const rustIndex = groups.findIndex((group) => group.role === "rust");
  let webviewIndex = -1;
  groups.forEach((group, index) => {
    if (group.role === "webview") webviewIndex = index;
  });
  const insertAt = webviewIndex >= 0 ? webviewIndex + 1 : rustIndex >= 0 ? rustIndex + 1 : 0;
  return [...groups.slice(0, insertAt), python, ...groups.slice(insertAt)];
});

function normalizeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null) return "—";
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

function formatPercent(value: number | null | undefined): string {
  return value == null ? "采样中" : `${value.toFixed(value >= 10 ? 1 : 2)}%`;
}

function formatTime(value: string | null | undefined): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(value));
}

function roleLabel(group: ProcessResourceGroup) {
  const value = processRole[group.role] ?? { label: group.role, detail: "应用关联进程" };
  if (group.role === "python" && group.processCount === 0) {
    return { ...value, detail: "当前未启动；首次 AI 请求时按需启动" };
  }
  if (group.role !== "python" || !props.recognizer) return value;
  return {
    ...value,
    detail: `${props.recognizer === "arcface" ? "ArcFace" : "SFace"} · ${value.detail}`,
  };
}

async function refreshProcesses() {
  if (processRequestActive || document.hidden) return;
  processRequestActive = true;
  try {
    processStatus.value = await captureApi.getProcessResourceStatus();
    processError.value = null;
  } catch (error) {
    processError.value = normalizeError(error);
  } finally {
    processLoading.value = false;
    processRequestActive = false;
  }
}

async function refreshStorage(showLoading = true) {
  if (showLoading) storageLoading.value = true;
  try {
    storageStatus.value = await captureApi.getStorageResourceStatus();
    storageError.value = null;
  } catch (error) {
    storageError.value = normalizeError(error);
  } finally {
    storageLoading.value = false;
  }
}

async function refreshLogSettings() {
  logSettingsLoading.value = true;
  try {
    logSettings.value = await captureApi.getLogSettings();
  } catch (error) {
    toast.error(`读取日志策略失败：${normalizeError(error)}`);
  } finally {
    logSettingsLoading.value = false;
  }
}

async function saveLogSettings() {
  if (logSettingsBusy.value) return;
  logSettingsBusy.value = true;
  try {
    logSettings.value = await captureApi.updateLogSettings(logSettings.value);
    toast.success("日志保留与清理策略已保存并立即生效");
    await refreshStorage(false);
  } catch (error) {
    toast.error(`保存日志策略失败：${normalizeError(error)}`);
  } finally {
    logSettingsBusy.value = false;
  }
}

function startProcessSampling() {
  stopProcessSampling();
  if (document.hidden) return;
  void refreshProcesses();
  processTimer = window.setInterval(() => void refreshProcesses(), PROCESS_SAMPLE_INTERVAL_MS);
}

function stopProcessSampling() {
  if (processTimer !== null) {
    window.clearInterval(processTimer);
    processTimer = null;
  }
}

function onVisibilityChange() {
  if (document.hidden) {
    stopProcessSampling();
  } else {
    startProcessSampling();
  }
}

async function openDirectory(entry: StorageResourceEntry) {
  if (!entry.path) return;
  try {
    if (entry.kind === "database") {
      await revealPath(entry.path);
    } else {
      await openPathExternal(entry.path);
    }
  } catch (error) {
    toast.error(`无法打开资源位置：${normalizeError(error)}`);
  }
}

async function cleanup(entry: StorageResourceEntry) {
  const kind = cleanupKind[entry.kind];
  if (!kind || cleanupBusy.value) return;
  if (!window.confirm(cleanupPrompt[entry.kind] ?? `确定清理“${entry.label}”吗？`)) return;
  cleanupBusy.value = entry.kind;
  try {
    const result = await captureApi.cleanupResource(kind);
    toast.success(`${result.message} 释放 ${formatBytes(result.reclaimedBytes)}。`);
    await refreshStorage(false);
  } catch (error) {
    toast.error(`清理失败：${normalizeError(error)}`);
  } finally {
    cleanupBusy.value = null;
  }
}

onMounted(() => {
  document.addEventListener("visibilitychange", onVisibilityChange);
  startProcessSampling();
  void refreshStorage();
  void refreshLogSettings();
});

onBeforeUnmount(() => {
  stopProcessSampling();
  document.removeEventListener("visibilitychange", onVisibilityChange);
});
</script>

<template>
  <div class="resource-panel">
    <section class="resource-section" aria-labelledby="process-resource-title">
      <div class="section-heading">
        <div>
          <span class="eyebrow">实时资源</span>
          <h2 id="process-resource-title">应用进程占用</h2>
          <p>仅停留在本页且窗口可见时每 2 秒采样；离开本页后不会继续监控。</p>
        </div>
        <button class="section-action" type="button" :disabled="processLoading" aria-label="刷新进程占用" @click="refreshProcesses">
          <LoaderCircle v-if="processLoading" class="animate-spin" :size="17" />
          <RefreshCw v-else :size="17" />
          刷新
        </button>
      </div>

      <div v-if="processStatus" class="summary-grid" aria-live="polite">
        <article>
          <Activity :size="18" />
          <span>总 CPU</span>
          <strong>{{ totalCpu }}</strong>
          <small>按 {{ processStatus.logicalProcessors }} 个逻辑处理器归一化</small>
        </article>
        <article>
          <HardDrive :size="18" />
          <span>工作集内存</span>
          <strong>{{ formatBytes(processStatus.totalWorkingSetBytes) }}</strong>
          <small>当前驻留物理内存，跨进程求和</small>
        </article>
        <article>
          <Database :size="18" />
          <span>专用内存</span>
          <strong>{{ formatBytes(processStatus.totalPrivateBytes) }}</strong>
          <small>不与其他进程共享的提交内存</small>
        </article>
      </div>

      <p v-if="processError" class="inline-error" role="alert">进程采样失败：{{ processError }}</p>
      <div v-if="processStatus" class="process-list">
        <article v-for="group in displayedProcessGroups" :key="group.role" class="process-row">
          <div class="process-identity">
            <strong>{{ roleLabel(group).label }}</strong>
            <span>
              {{ roleLabel(group).detail }} ·
              <template v-if="group.processCount">{{ group.processCount }} 个进程 · PID {{ group.pids.join(", ") }}</template>
              <template v-else>未运行</template>
            </span>
          </div>
          <dl>
            <div><dt>CPU</dt><dd>{{ group.processCount ? formatPercent(group.cpuPercent) : "—" }}</dd></div>
            <div><dt>工作集</dt><dd>{{ group.processCount ? formatBytes(group.workingSetBytes) : "—" }}</dd></div>
            <div><dt>峰值</dt><dd>{{ group.processCount ? formatBytes(group.peakWorkingSetBytes) : "—" }}</dd></div>
            <div><dt>专用</dt><dd>{{ group.processCount ? formatBytes(group.privateBytes) : "—" }}</dd></div>
          </dl>
        </article>
      </div>
      <div v-else-if="processLoading" class="loading-state"><LoaderCircle class="animate-spin" :size="20" />正在建立首次采样…</div>
      <footer v-if="processStatus" class="sample-note">
        最近采样 {{ formatTime(processStatus.capturedAt) }}。CPU 首次采样需等待下一周期；数值用于趋势诊断，不替代系统任务管理器。<template v-if="processStatus.approximate"> 部分子进程无法读取，当前合计为近似值。</template>
      </footer>
    </section>

    <section class="resource-section" aria-labelledby="storage-resource-title">
      <div class="section-heading">
        <div>
          <span class="eyebrow">磁盘空间</span>
          <h2 id="storage-resource-title">资源与缓存</h2>
          <p>进入页面时扫描一次；不会在后台遍历磁盘。清理后会自动刷新。</p>
        </div>
        <button class="section-action" type="button" :disabled="storageLoading || cleanupBusy !== null" @click="refreshStorage()">
          <LoaderCircle v-if="storageLoading" class="animate-spin" :size="17" />
          <RefreshCw v-else :size="17" />
          重新扫描
        </button>
      </div>

      <p v-if="storageError" class="inline-error" role="alert">存储扫描失败：{{ storageError }}</p>

      <form class="log-policy-card" @submit.prevent="saveLogSettings">
        <div class="log-policy-heading">
          <div>
            <strong>日志保留与自动清理</strong>
            <span>设置保存后立即生效；关闭自动清理后仍可使用下方“清理”按钮手动执行。</span>
          </div>
          <button type="submit" class="section-action" :disabled="logSettingsLoading || logSettingsBusy">
            <LoaderCircle v-if="logSettingsBusy" class="animate-spin" :size="16" />
            <Save v-else :size="16" />保存策略
          </button>
        </div>
        <div class="log-policy-fields" :aria-busy="logSettingsLoading">
          <label>
            保留天数
            <input v-model.number="logSettings.retentionDays" type="number" min="1" max="365" />
            <small>1–365 天</small>
          </label>
          <label>
            单文件上限
            <input v-model.number="logSettings.maxFileSizeMb" type="number" min="1" max="100" />
            <small>1–100 MiB</small>
          </label>
          <label>
            最多归档
            <input v-model.number="logSettings.maxArchivedFiles" type="number" min="1" max="200" />
            <small>1–200 个</small>
          </label>
          <label class="log-policy-toggle">
            <input v-model="logSettings.automaticCleanup" type="checkbox" />
            <span>启动及日志轮转时自动清理过期归档</span>
          </label>
        </div>
      </form>
      <div v-if="storageStatus" class="storage-summary">
        <span>已统计资源合计</span>
        <strong>{{ formatBytes(storageStatus.totalBytes) }}</strong>
        <small>{{ storageStatus.entries.reduce((sum, entry) => sum + entry.fileCount, 0).toLocaleString() }} 个文件</small>
      </div>
      <div v-if="storageStatus" class="storage-list">
        <article v-for="entry in storageStatus.entries" :key="entry.kind" class="storage-row">
          <div class="storage-main">
            <strong>{{ entry.label }}</strong>
            <span :title="entry.path ?? undefined">{{ entry.path || "尚未配置" }}</span>
            <small v-if="entry.cleanupDescription">{{ entry.cleanupDescription }}</small>
          </div>
          <div class="storage-amount">
            <strong>{{ formatBytes(entry.totalBytes) }}</strong>
            <span>{{ entry.fileCount.toLocaleString() }} 个文件</span>
          </div>
          <div class="storage-actions">
            <button v-if="entry.path" type="button" class="icon-only" :aria-label="`打开${entry.label}位置`" title="打开位置" @click="openDirectory(entry)">
              <FolderOpen :size="17" />
            </button>
            <button
              v-if="entry.cleanupAvailable"
              type="button"
              class="cleanup-action"
              :disabled="cleanupBusy !== null"
              @click="cleanup(entry)"
            >
              <LoaderCircle v-if="cleanupBusy === entry.kind" class="animate-spin" :size="16" />
              <Trash2 v-else :size="16" />
              清理
            </button>
          </div>
        </article>
      </div>
      <div v-else-if="storageLoading" class="loading-state"><LoaderCircle class="animate-spin" :size="20" />正在统计磁盘资源…</div>
    </section>
  </div>
</template>

<style scoped>
.resource-panel {
  display: grid;
  gap: 16px;
}

.resource-section {
  padding: 24px;
  border: 1px solid var(--border);
  border-radius: 16px;
  background: var(--card);
  box-shadow: var(--card-shadow), var(--inner-highlight);
}

.section-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 20px;
  margin-bottom: 20px;
}

.section-heading h2 {
  margin: 3px 0 5px;
  font-size: 20px;
}

.section-heading p,
.sample-note {
  margin: 0;
  color: var(--muted-foreground);
  font-size: 12px;
  line-height: 1.6;
}

.eyebrow {
  color: var(--accent);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.1em;
  text-transform: uppercase;
}

.section-action,
.cleanup-action,
.icon-only {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  min-height: 36px;
  border: 1px solid var(--border);
  border-radius: 9px;
  background: var(--background);
  color: var(--foreground);
  font-weight: 600;
  cursor: pointer;
}

.section-action { flex: none; padding: 0 12px; white-space: nowrap; }
.icon-only { width: 36px; padding: 0; }
.cleanup-action { padding: 0 11px; color: var(--destructive); }
.section-action:hover,
.icon-only:hover { border-color: color-mix(in srgb, var(--accent) 55%, var(--border)); }
.cleanup-action:hover { background: color-mix(in srgb, var(--destructive) 8%, transparent); }
.section-action:focus-visible,
.cleanup-action:focus-visible,
.icon-only:focus-visible { outline: 2px solid var(--ring); outline-offset: 2px; }
button:disabled { cursor: not-allowed; opacity: 0.55; }

.summary-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 10px;
  margin-bottom: 14px;
}

.summary-grid article {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 4px 8px;
  padding: 14px;
  border: 1px solid color-mix(in srgb, var(--border) 85%, transparent);
  border-radius: 12px;
  background: color-mix(in srgb, var(--background) 62%, transparent);
}

.summary-grid svg { grid-row: 1 / span 2; color: var(--accent); }
.summary-grid span { color: var(--muted-foreground); font-size: 11px; }
.summary-grid strong { font-size: 19px; font-variant-numeric: tabular-nums; }
.summary-grid small { grid-column: 1 / -1; margin-top: 5px; color: var(--muted-foreground); font-size: 10px; }

.process-list,
.storage-list {
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 12px;
}

.process-row,
.storage-row {
  display: grid;
  align-items: center;
  gap: 14px;
  padding: 13px 15px;
  border-bottom: 1px solid var(--border);
}

.process-row:last-child,
.storage-row:last-child { border-bottom: 0; }
.process-row { grid-template-columns: minmax(170px, 1fr) minmax(380px, 1.6fr); }
.process-identity,
.storage-main { display: flex; min-width: 0; flex-direction: column; gap: 3px; }
.process-identity span,
.storage-main span,
.storage-main small,
.storage-amount span { color: var(--muted-foreground); font-size: 11px; }

.process-row dl {
  display: grid;
  grid-template-columns: repeat(4, minmax(68px, 1fr));
  gap: 8px;
  margin: 0;
}

.process-row dl div { display: flex; flex-direction: column; gap: 2px; }
.process-row dt { color: var(--muted-foreground); font-size: 10px; }
.process-row dd { margin: 0; font-size: 13px; font-weight: 650; font-variant-numeric: tabular-nums; }
.sample-note { margin-top: 11px; }

.log-policy-card {
  display: grid;
  gap: 14px;
  margin-bottom: 14px;
  padding: 15px;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: color-mix(in srgb, var(--secondary) 45%, var(--card));
}
.log-policy-heading { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; }
.log-policy-heading > div { display: flex; flex-direction: column; gap: 4px; }
.log-policy-heading span { color: var(--muted-foreground); font-size: 11px; line-height: 1.5; }
.log-policy-fields { display: grid; grid-template-columns: repeat(3, minmax(120px, 1fr)); gap: 12px; }
.log-policy-fields label { display: grid; gap: 5px; color: var(--muted-foreground); font-size: 11px; font-weight: 650; }
.log-policy-fields input[type="number"] { min-width: 0; height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 8px; background: var(--background); color: var(--foreground); }
.log-policy-fields small { color: var(--muted-foreground); font-size: 10px; font-weight: 400; }
.log-policy-fields .log-policy-toggle { display: flex; grid-column: 1 / -1; align-items: center; gap: 8px; }
.log-policy-toggle input { width: 16px; height: 16px; accent-color: var(--accent); }

.storage-summary {
  display: flex;
  align-items: baseline;
  gap: 10px;
  margin-bottom: 12px;
  padding: 12px 15px;
  border-radius: 10px;
  background: color-mix(in srgb, var(--accent) 8%, transparent);
}

.storage-summary span,
.storage-summary small { color: var(--muted-foreground); font-size: 11px; }
.storage-summary strong { font-size: 18px; font-variant-numeric: tabular-nums; }
.storage-summary small { margin-left: auto; }
.storage-row { grid-template-columns: minmax(0, 1fr) 110px auto; }
.storage-main span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.storage-amount { display: flex; flex-direction: column; align-items: flex-end; gap: 3px; font-variant-numeric: tabular-nums; }
.storage-actions { display: flex; gap: 7px; }
.inline-error { padding: 10px 12px; border-radius: 9px; background: color-mix(in srgb, var(--destructive) 10%, transparent); color: var(--destructive); font-size: 12px; }
.loading-state { display: flex; align-items: center; justify-content: center; gap: 8px; min-height: 100px; color: var(--muted-foreground); font-size: 12px; }

@media (max-width: 900px) {
  .summary-grid { grid-template-columns: 1fr; }
  .process-row { grid-template-columns: 1fr; }
  .storage-row { grid-template-columns: minmax(0, 1fr) auto; }
  .storage-actions { grid-column: 1 / -1; justify-content: flex-end; }
  .log-policy-fields { grid-template-columns: 1fr; }
  .log-policy-heading { flex-direction: column; }
}
</style>
