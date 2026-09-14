<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { DialogRoot, DialogPortal, DialogOverlay, DialogContent, DialogTitle, DialogDescription } from 'reka-ui';
import { CheckCircle2, RotateCcw, X } from '@lucide/vue';
import { captureApi, pathFileName, type CaptureResetInput, type ResetJob } from '@/lib/capture-api';
const props = defineProps<{ input?: CaptureResetInput }>();
const emit = defineEmits<{ close: []; updated: []; started: [] }>();
const job = ref<ResetJob | null>(null);
const error = ref('');
const executing = ref(false);
const loading = ref(false);
const deleteDestinationFiles = ref(true);
const networkConfirmed = ref(false);
const started = ref(false);
let disposed = false;
let timer: ReturnType<typeof setTimeout> | undefined;
let version = 0;
const running = computed(() => executing.value || job.value?.executing === true);
const retryable = computed(() => job.value?.items.some(item => ['ready', 'running', 'failed'].includes(item.status)));
const progress = computed(() => job.value?.items.filter(item => ['succeeded', 'skipped', 'failed'].includes(item.status)).length ?? 0);
const percent = computed(() => job.value?.items.length ? Math.round((progress.value / job.value.items.length) * 100) : 0);
const failedCount = computed(() => job.value?.items.filter(item => item.status === 'failed').length ?? 0);
const succeededCount = computed(() => job.value?.items.filter(item => item.status === 'succeeded').length ?? 0);
const skippedCount = computed(() => job.value?.items.filter(item => item.status === 'skipped').length ?? 0);
const statusLabel = computed(() => {
  if (running.value) return '重置进行中';
  if (!job.value || job.value.status === 'preview') return '等待确认';
  if (failedCount.value) return `已完成 ${succeededCount.value} 项，${failedCount.value} 项失败`;
  if (!succeededCount.value && skippedCount.value) return `所选图片均不可撤销（${skippedCount.value} 项已跳过）`;
  return '已全部完成';
});
const doneLabel = computed(() => succeededCount.value ? '任务完成' : '任务已结束');
const retryLabel = computed(() => {
  if (job.value?.status === 'preview') return '确认撤销分类';
  return failedCount.value ? `重试失败项（${failedCount.value}）` : '重试未完成项';
});
// Unfinished work first, so failures stay visible in a long list.
const orderedItems = computed(() => {
  const rank: Record<string, number> = { failed: 0, running: 1, ready: 2, succeeded: 3, skipped: 4 };
  return [...(job.value?.items ?? [])].sort((left, right) => (rank[left.status] ?? 9) - (rank[right.status] ?? 9));
});
const labels: Record<string, string> = { ready: '待重置', skipped: '已跳过', running: '处理中', succeeded: '已返回待分类', failed: '失败' };
/// Failure classes the service reports, translated into what the user can do.
const reasonLabels: Record<string, string> = {
  source_gone: '原图已不存在或被替换',
  network: 'NAS/网络路径不可达',
  locked: '目标文件被占用或无法删除',
  denied: '目标位置没有权限或不允许删除',
  changed: '目标文件在预览后被改动',
  busy: '图片正被其他操作占用',
  unknown: '其他错误',
};
const reasonAdvice: Record<string, string> = {
  source_gone: '原图已不在，这些图片保持原分类，可重新导入或忽略',
  network: '确认 NAS/共享在线、盘符映射正常',
  locked: '关闭可能占用该文件的程序（看图工具、同步盘、杀软扫描）',
  denied: '检查目标目录权限和只读属性，或该位置是否支持回收站删除',
  changed: '先核对目录内容，确认文件是否被替换',
  busy: '等这些图片处理结束',
  unknown: '详情见「调试日志」中的 reset_item_failed',
};
function groupReasons(status: 'failed' | 'skipped') {
  const counts = new Map<string, number>();
  for (const item of job.value?.items ?? []) {
    if (item.status !== status) continue;
    const reason = item.reason ?? 'unknown';
    counts.set(reason, (counts.get(reason) ?? 0) + 1);
  }
  return [...counts].map(([reason, count]) => ({
    reason,
    count,
    label: reasonLabels[reason] ?? reasonLabels.unknown,
    advice: reasonAdvice[reason] ?? reasonAdvice.unknown,
  }));
}
const failures = computed(() => groupReasons('failed'));
const skippedGroups = computed(() => groupReasons('skipped'));
const skippedSummary = computed(() => skippedGroups.value.map(entry => `${entry.label}（${entry.count}）`).join('、'));
const failedNames = computed(() => (job.value?.items ?? []).filter(item => item.status === 'failed').map(item => pathFileName(item.sourcePath)).join('、'));
function accept(value: ResetJob) {
  if (disposed) {
    // The dialog was closed while a slow preview was still loading: the answer
    // arrives after unmount, so the preview is dropped here instead.
    if (value.status === "preview") {
      void captureApi.discardCaptureReset(value.id).catch(() => undefined);
    }
    return;
  }
  const previous = job.value;
  job.value = value;
  started.value = value.deleteDestinationFiles != null;
  if (started.value) {
    if (!previous?.deleteDestinationFiles && previous?.deleteDestinationFiles !== false) emit('started');
    deleteDestinationFiles.value = value.deleteDestinationFiles!;
    networkConfirmed.value = value.allowPermanentNetworkDelete === true;
  }
  if (previous && JSON.stringify(previous.items.map(item => [item.captureItemId, item.status])) !== JSON.stringify(value.items.map(item => [item.captureItemId, item.status]))) emit('updated');
}
function schedulePoll() {
  clearTimeout(timer);
  if (!disposed && (executing.value || job.value?.executing)) timer = setTimeout(() => void poll(), 2000);
}
async function poll() {
  const id = job.value?.id;
  const request = ++version;
  if (!id) return;
  try {
    const value = await captureApi.getCaptureReset(id);
    if (request === version) { accept(value); error.value = ''; }
  } catch (caught) {
    if (!disposed && request === version) error.value = `读取进度失败，可继续刷新：${String(caught)}`;
  } finally { schedulePoll(); }
}
async function load() {
  loading.value = true;
  error.value = '';
  try {
    if (!props.input?.projectId) throw new Error('请选择当前项目');
    accept(await captureApi.previewCaptureReset(props.input));
    schedulePoll();
  } catch (caught) { error.value = String(caught); }
  finally { loading.value = false; }
}
async function execute() {
  if (!job.value || running.value || !retryable.value) return;
  if (deleteDestinationFiles.value && job.value.networkDestinationFileCount > 0 && !networkConfirmed.value) return;
  executing.value = true;
  error.value = '';
  version++;
  schedulePoll();
  try {
    const result = await captureApi.executeCaptureReset({ jobId: job.value.id, deleteDestinationFiles: deleteDestinationFiles.value, allowPermanentNetworkDelete: deleteDestinationFiles.value && networkConfirmed.value });
    version++;
    accept(result);
  } catch (caught) {
    await poll();
    if (!disposed) error.value = String(caught);
  } finally {
    executing.value = false;
    schedulePoll();
  }
}
onMounted(load);
// A preview that was never confirmed is dropped right away; a job that already
// started keeps its report (the backend refuses to discard it anyway).
onBeforeUnmount(() => {
  const pending = job.value;
  if (pending && pending.status === "preview" && !executing.value) {
    void captureApi.discardCaptureReset(pending.id).catch(() => undefined);
  }
  disposed = true;
  clearTimeout(timer);
  version++;
});
</script>
<template>
  <DialogRoot :open="true" @update:open="!$event && emit('close')">
    <DialogPortal><DialogOverlay class="import-overlay" /><DialogContent class="import-dialog delete-dialog">
      <div class="delete-title">
        <DialogTitle>
          <span class="delete-icon reset-icon"><RotateCcw :size="15" /></span>
          撤销分类并返回待分类
        </DialogTitle>
        <button
          type="button"
          class="dialog-close"
          :title="running ? '关闭窗口，撤销任务继续运行' : '关闭'"
          :aria-label="running ? '关闭窗口，撤销任务继续运行' : '关闭撤销分类'"
          @click="emit('close')"
        >
          <X :size="16" />
        </button>
      </div>
      <DialogDescription>清除所选截图的分类、角色、人脸样本和处理结果，保留原图及截图记录。预览范围已固定，筛选变化不会扩大本次任务。</DialogDescription>
      <p v-if="error" role="alert" class="delete-error">{{ error }}</p>
      <p v-if="loading" role="status">正在读取固定预览…</p>
      <template v-if="job">
        <p>{{ job.items.length }} 张截图；{{ job.destinationFileCount }} 个归档目标，其中 {{ job.networkDestinationFileCount }} 个 NAS/网络目标。</p>
        <label class="delete-target-option"><input v-model="deleteDestinationFiles" type="checkbox" :disabled="running || started" @change="networkConfirmed = false" />同时删除归档图和头像（本地目标移入回收站）</label>
        <label v-if="deleteDestinationFiles && job.networkDestinationFileCount > 0" class="delete-target-option delete-danger">
          <input v-model="networkConfirmed" type="checkbox" :disabled="running || started" />我确认永久删除 {{ job.networkDestinationFileCount }} 个 NAS/网络目标，无法恢复；原图保留。
        </label>
        <p v-if="!deleteDestinationFiles" role="note">归档图和头像将保留在原位置，但 Scene Vault 会清除其关联；后续重新分类可能生成新的归档文件。</p>
        <p role="status" aria-live="polite">{{ statusLabel }} · {{ progress }} / {{ job.items.length }}（{{ percent }}%）</p>
        <div
          class="reset-progress"
          :class="{ running }"
          :data-state="failedCount ? 'warn' : 'ok'"
          role="progressbar"
          aria-label="重置进度"
          aria-valuemin="0"
          :aria-valuemax="job.items.length || 1"
          :aria-valuenow="progress"
          :aria-valuetext="`${progress} / ${job.items.length}`"
        >
          <div class="reset-progress-bar" :style="{ width: `${percent}%` }" />
        </div>
        <div v-if="failedCount" class="partial-warning" role="alert">
          <strong>{{ failedCount }} 项未完成（{{ failedNames }}），这些图片保持原分类，原图没有被删除。</strong>
          <ul class="partial-failures">
            <li v-for="entry in failures" :key="entry.reason">{{ entry.count }} 项 · {{ entry.label }} — {{ entry.advice }}</li>
          </ul>
          <span>处理后点「{{ retryLabel }}」重试；失败详情也在「调试日志」里。</span>
        </div>
        <p v-else-if="skippedCount" role="note">{{ skippedCount }} 张不可处理：{{ skippedSummary }}。这些图片不会被撤销。</p>
        <ul class="reset-items"><li v-for="item in orderedItems" :key="item.captureItemId"><strong :title="item.sourcePath">{{ pathFileName(item.sourcePath) }}</strong> — {{ labels[item.status] || item.status }}<span v-if="item.error" class="delete-error">：{{ item.error }}</span></li></ul>
        <div class="delete-actions">
          <button v-if="running" type="button" class="secondary-action" @click="poll">刷新进度</button>
          <button v-if="job.status === 'completed' && !failedCount" type="button" class="reset-complete" title="关闭窗口" @click="emit('close')"><CheckCircle2 :size="15" />{{ doneLabel }}</button>
          <button v-if="retryable" type="button" class="danger-action" :disabled="running || (deleteDestinationFiles && job.networkDestinationFileCount > 0 && !networkConfirmed)" @click="execute">{{ retryLabel }}</button>
        </div>
      </template>
      <button v-else-if="!loading" type="button" class="secondary-action" @click="load">重试读取预览</button>
    </DialogContent></DialogPortal>
  </DialogRoot>
</template>
<style scoped src="./delete-dialog.css"></style>
<style scoped>
.reset-icon { background: color-mix(in srgb, var(--accent) 12%, transparent); color: var(--accent); }
.reset-items { max-height: 35vh; overflow: auto; overflow-wrap: anywhere; padding-left: 20px; }
.reset-progress { position: relative; height: 8px; overflow: hidden; border-radius: 99px; background: var(--muted); }
.reset-progress-bar { height: 100%; border-radius: 99px; background: var(--accent-gradient, var(--accent)); transition: width .35s ease; }
.reset-progress[data-state="warn"] .reset-progress-bar { background: var(--warn); }
.reset-complete {
  display: inline-flex;
  height: 38px;
  align-items: center;
  gap: 6px;
  padding: 0 14px;
  border: 1px solid color-mix(in srgb, var(--ok) 42%, var(--border));
  border-radius: 9px;
  background: color-mix(in srgb, var(--ok) 14%, var(--card));
  color: var(--ok);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: filter .15s ease;
}
.reset-complete:hover { filter: brightness(1.06); }
.reset-complete:focus-visible { outline: 2px solid var(--ok); outline-offset: 2px; }
.reset-progress.running::after {
  position: absolute;
  inset: 0;
  content: "";
  background: linear-gradient(90deg, transparent, color-mix(in srgb, #fff 38%, transparent), transparent);
  animation: reset-progress-sheen 1.4s linear infinite;
}
@keyframes reset-progress-sheen {
  from { transform: translateX(-100%); }
  to { transform: translateX(100%); }
}
@media (prefers-reduced-motion: reduce) {
  .reset-progress-bar { transition: none; }
  .reset-progress.running::after { animation: none; }
}
</style>
