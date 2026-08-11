<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  AlertTriangle,
  Copy,
  Database,
  FolderOpen,
  HardDrive,
  LoaderCircle,
  RotateCcw,
  ShieldAlert,
} from "@lucide/vue";
import {
  captureApi,
  openPathExternal,
  pathDirectory,
  pickFile,
  type DatabaseStartupStatus,
  type RestoreRequest,
} from "@/lib/capture-api";

const RESTORE_CONFIRM_TEXT =
  "将校验并暂存所选备份，替换会在应用重启后执行；当前数据库会先保留一份回滚快照。" +
  "如果恢复失败会自动回滚。确定继续吗？";

const status = ref<DatabaseStartupStatus | null>(null);
const loadError = ref<string | null>(null);
const stagedRestore = ref<RestoreRequest | null>(null);
const stagingBusy = ref(false);
const actionError = ref<string | null>(null);
const copied = ref(false);

const dataDirectory = computed(() =>
  status.value ? pathDirectory(status.value.databasePath) : "",
);
const canRestart = computed(
  () => stagedRestore.value !== null || status.value?.pendingRestore === true,
);

function normalizeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function formatTime(value: string | null | undefined): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

async function loadStatus() {
  loadError.value = null;
  try {
    status.value = await captureApi.getDatabaseStartupStatus();
  } catch (error) {
    loadError.value = normalizeError(error);
  }
}

async function chooseBackup() {
  if (stagingBusy.value) return;
  let backupPath: string | null;
  try {
    backupPath = await pickFile([
      { name: "SQLite 数据库备份", extensions: ["sqlite", "db"] },
    ]);
  } catch (error) {
    actionError.value = `选择备份文件失败：${normalizeError(error)}`;
    return;
  }
  if (!backupPath) return;
  if (!window.confirm(RESTORE_CONFIRM_TEXT)) return;
  stagingBusy.value = true;
  actionError.value = null;
  try {
    stagedRestore.value = await captureApi.stageDatabaseRestore(backupPath);
  } catch (error) {
    stagedRestore.value = null;
    actionError.value = normalizeError(error);
  } finally {
    stagingBusy.value = false;
  }
}

async function restartApp() {
  if (!canRestart.value || stagingBusy.value) return;
  actionError.value = null;
  try {
    await captureApi.restartAfterDatabaseRestore();
  } catch (error) {
    actionError.value = `无法重启应用：${normalizeError(error)}`;
  }
}

async function openDataDirectory() {
  if (!dataDirectory.value) return;
  try {
    await openPathExternal(dataDirectory.value);
  } catch (error) {
    actionError.value = `无法打开数据目录：${normalizeError(error)}`;
  }
}

async function copyErrorSummary() {
  if (!status.value) return;
  const summary = [
    "Scene Vault 数据库恢复模式",
    `错误：${status.value.errorMessage ?? "未知数据库错误"}`,
    `数据库：${status.value.databasePath}`,
    `备份目录：${status.value.backupDirectory}`,
    `恢复目录：${status.value.recoveryDirectory}`,
  ].join("\n");
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(summary);
      copied.value = true;
      return;
    } catch {
      // Fall through to the legacy copy path below.
    }
  }
  const textarea = document.createElement("textarea");
  textarea.value = summary;
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.appendChild(textarea);
  textarea.select();
  const ok = document.execCommand("copy");
  document.body.removeChild(textarea);
  if (!ok) throw new Error("clipboard is unavailable");
  copied.value = true;
}

onMounted(() => {
  void loadStatus();
});
</script>

<template>
  <main class="recovery-page">
    <div class="recovery-shell">
      <header class="recovery-header">
        <span class="recovery-badge"><ShieldAlert :size="18" />恢复模式</span>
        <h1>数据库恢复</h1>
        <p>数据库无法正常打开，应用已进入最小恢复模式。此时不会启动截图监听、笔记同步或 AI 处理。</p>
      </header>

      <p v-if="loadError" class="recovery-error" role="alert">
        无法读取启动状态：{{ loadError }}
      </p>

      <div v-if="status" class="recovery-card" aria-live="polite">
        <section class="recovery-block" aria-labelledby="startup-error-title">
          <h2 id="startup-error-title"><AlertTriangle :size="17" />启动错误</h2>
          <p class="startup-error">{{ status.errorMessage ?? "未知数据库错误" }}</p>
          <div class="recovery-inline-actions">
            <button class="recovery-action" type="button" @click="copyErrorSummary">
              <Copy :size="15" />{{ copied ? "已复制" : "复制错误摘要" }}
            </button>
            <button class="recovery-action" type="button" @click="openDataDirectory">
              <FolderOpen :size="15" />打开数据目录
            </button>
          </div>
        </section>

        <section class="recovery-block" aria-labelledby="data-locations-title">
          <h2 id="data-locations-title"><Database :size="17" />数据位置</h2>
          <dl class="recovery-paths">
            <div><dt>数据库文件</dt><dd>{{ status.databasePath }}</dd></div>
            <div><dt>备份目录</dt><dd>{{ status.backupDirectory }}</dd></div>
            <div><dt>恢复暂存目录</dt><dd>{{ status.recoveryDirectory }}</dd></div>
          </dl>
        </section>

        <section class="recovery-block" aria-labelledby="restore-flow-title">
          <h2 id="restore-flow-title"><RotateCcw :size="17" />从备份恢复</h2>
          <template v-if="stagedRestore">
            <p class="restore-note">备份已校验并暂存，重启后生效；当前数据库会保留回滚快照。</p>
            <dl class="recovery-paths">
              <div><dt>来源备份</dt><dd>{{ stagedRestore.sourceBackupPath }}</dd></div>
              <div><dt>应用版本</dt><dd>{{ stagedRestore.appVersion }}</dd></div>
              <div><dt>Schema 版本</dt><dd>{{ stagedRestore.schemaVersion ?? "—" }}</dd></div>
              <div><dt>校验值</dt><dd>{{ stagedRestore.sha256.slice(0, 16) }}…</dd></div>
              <div><dt>暂存时间</dt><dd>{{ formatTime(stagedRestore.requestedAtUtc) }}</dd></div>
            </dl>
          </template>
          <template v-else-if="status.pendingRestore">
            <p class="restore-note">检测到已暂存的恢复，直接重启即可应用；如本次启动已应用并失败，将自动回滚并允许重新选择备份。</p>
          </template>
          <template v-else>
            <p class="restore-note">请选择一份应用创建的 SQLite 备份。校验与暂存不会修改当前数据库，替换将在重启后执行。</p>
            <div class="recovery-inline-actions">
              <button class="recovery-action primary" type="button" :disabled="stagingBusy" @click="chooseBackup">
                <LoaderCircle v-if="stagingBusy" class="animate-spin" :size="15" />
                <HardDrive v-else :size="15" />
                {{ stagingBusy ? "正在校验备份…" : "选择备份文件" }}
              </button>
            </div>
          </template>

          <p v-if="actionError" class="recovery-error" role="alert">{{ actionError }}</p>

          <div v-if="canRestart" class="recovery-restart">
            <button class="recovery-action restart-action" type="button" :disabled="stagingBusy" @click="restartApp">
              <RotateCcw :size="16" />立即重启应用
            </button>
            <small>重启后若恢复失败，应用会自动回滚到之前的数据库。</small>
          </div>
        </section>
      </div>

      <div v-else-if="!loadError" class="recovery-loading">
        <LoaderCircle class="animate-spin" :size="20" />正在读取启动状态…
      </div>
    </div>
  </main>
</template>

<style scoped>
.recovery-page {
  display: flex;
  min-height: 100vh;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: var(--background);
  color: var(--foreground);
}

.recovery-shell {
  width: min(680px, 100%);
  display: grid;
  gap: 16px;
}

.recovery-header {
  display: grid;
  gap: 8px;
}

.recovery-badge {
  display: inline-flex;
  align-items: center;
  justify-self: start;
  gap: 7px;
  padding: 5px 10px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--destructive) 12%, transparent);
  color: var(--destructive);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.06em;
}

.recovery-header h1 {
  margin: 0;
  font-size: 28px;
}

.recovery-header p {
  margin: 0;
  color: var(--muted-foreground);
  font-size: 12px;
  line-height: 1.6;
}

.recovery-card {
  display: grid;
  gap: 16px;
}

.recovery-block {
  display: grid;
  gap: 10px;
  padding: 20px;
  border: 1px solid var(--border);
  border-radius: 14px;
  background: var(--card);
  box-shadow: var(--card-shadow), var(--inner-highlight);
}

.recovery-block h2 {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0;
  font-size: 16px;
}

.startup-error {
  margin: 0;
  padding: 11px 13px;
  border-radius: 9px;
  background: color-mix(in srgb, var(--destructive) 9%, transparent);
  color: var(--destructive);
  font-size: 12px;
  line-height: 1.6;
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}

.recovery-paths {
  display: grid;
  gap: 7px;
  margin: 0;
}

.recovery-paths div {
  display: grid;
  grid-template-columns: 96px minmax(0, 1fr);
  gap: 10px;
}

.recovery-paths dt {
  color: var(--muted-foreground);
  font-size: 11px;
}

.recovery-paths dd {
  margin: 0;
  font-size: 12px;
  overflow-wrap: anywhere;
  font-variant-numeric: tabular-nums;
}

.restore-note {
  margin: 0;
  color: var(--muted-foreground);
  font-size: 12px;
  line-height: 1.6;
}

.recovery-inline-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.recovery-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  min-height: 36px;
  padding: 0 12px;
  border: 1px solid var(--border);
  border-radius: 9px;
  background: var(--background);
  color: var(--foreground);
  font-weight: 600;
  font-size: 12px;
  cursor: pointer;
}

.recovery-action.primary { border-color: color-mix(in srgb, var(--accent) 55%, var(--border)); }
.recovery-action.restart-action { color: var(--accent); border-color: color-mix(in srgb, var(--accent) 55%, var(--border)); }
.recovery-action:hover { border-color: color-mix(in srgb, var(--accent) 70%, var(--border)); }
.recovery-action:focus-visible { outline: 2px solid var(--ring); outline-offset: 2px; }
.recovery-action:disabled { cursor: not-allowed; opacity: 0.55; }

.recovery-restart {
  display: grid;
  gap: 6px;
  justify-items: start;
  padding-top: 4px;
}

.recovery-restart small {
  color: var(--muted-foreground);
  font-size: 11px;
}

.recovery-error {
  margin: 0;
  padding: 10px 12px;
  border-radius: 9px;
  background: color-mix(in srgb, var(--destructive) 10%, transparent);
  color: var(--destructive);
  font-size: 12px;
  line-height: 1.6;
  overflow-wrap: anywhere;
}

.recovery-loading {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 120px;
  color: var(--muted-foreground);
  font-size: 12px;
}
</style>
