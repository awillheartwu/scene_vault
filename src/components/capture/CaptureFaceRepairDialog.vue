<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { LoaderCircle, Sparkles } from "@lucide/vue";
import { captureApi, type FaceRepairStatus } from "@/lib/capture-api";
import { describeError } from "@/lib/vision-errors";
import TaskDialog from "@/components/common/TaskDialog.vue";

const props = defineProps<{ projectId: string }>();
const emit = defineEmits<{
  (event: "close"): void;
  (event: "updated"): void;
}>();

const candidateCount = ref(0);
const missingCount = ref(0);
const retryCount = ref(0);
const loading = ref(true);
const starting = ref(false);
const status = ref<FaceRepairStatus | null>(null);
const blockedProject = ref<string | null>(null);
const error = ref("");
let timer: number | null = null;
let generation = 0;
let disposed = false;

const running = computed(() => status.value?.state === "running");
const summary = computed(() => {
  if (running.value) return `核对中… ${status.value!.processed}/${status.value!.total}`;
  if (status.value?.state === "failed") return "核对失败";
  if (status.value?.state === "partial" || status.value?.failed) return "核对结束，部分失败";
  return "核对完成";
});
const percent = computed(() => {
  const value = status.value;
  if (!value || value.total === 0) return 0;
  return Math.round((value.processed / value.total) * 100);
});

function stopPolling() {
  if (timer !== null) window.clearTimeout(timer);
  timer = null;
}
function current(token: number) { return !disposed && token === generation; }
function schedule(token: number) {
  if (current(token)) timer = window.setTimeout(() => void poll(token), 700);
}

async function initialize() {
  const token = ++generation;
  stopPolling();
  status.value = null;
  blockedProject.value = null;
  candidateCount.value = missingCount.value = retryCount.value = 0;
  loading.value = true;
  starting.value = false;
  error.value = "";
  const projectId = props.projectId;
  try {
    const existing = await captureApi.getFaceRepairStatus();
    if (!current(token)) return;
    if (existing.state === "running" && existing.projectId === projectId) {
      status.value = existing;
      schedule(token);
      return;
    }
    if (existing.state === "running") blockedProject.value = existing.projectId ?? "未知项目";
    const preview = await captureApi.previewFaceRepair(projectId);
    if (!current(token)) return;
    missingCount.value = preview.candidates.filter((item) => item.missingFaceBox).length;
    candidateCount.value = preview.candidates.length - missingCount.value;
    retryCount.value = preview.retries.length;
    if (blockedProject.value) schedule(token);
  } catch (caught) {
    if (current(token)) error.value = describeError(caught);
  } finally {
    if (current(token)) loading.value = false;
  }
}

watch(() => props.projectId, initialize, { immediate: true });
onBeforeUnmount(() => { disposed = true; ++generation; stopPolling(); });

async function start() {
  if (loading.value || starting.value || running.value || blockedProject.value) return;
  const token = ++generation;
  stopPolling();
  error.value = "";
  starting.value = true;
  try {
    const next = await captureApi.startFaceRepair(props.projectId);
    if (!current(token)) return;
    if (next.projectId !== props.projectId) {
      await initialize();
      return;
    }
    status.value = next;
    if (next.state === "running") schedule(token);
  } catch (caught) {
    if (!current(token)) return;
    // A different window may have reserved the global task after our preview.
    const message = describeError(caught);
    await initialize();
    if (!disposed && token + 1 === generation) error.value = message;
  } finally {
    if (current(token)) starting.value = false;
  }
}

async function poll(token: number) {
  try {
    const next = await captureApi.getFaceRepairStatus();
    if (!current(token)) return;
    if (blockedProject.value) {
      if (next.state !== "running" || next.projectId === props.projectId) await initialize();
      else { blockedProject.value = next.projectId ?? "未知项目"; schedule(token); }
      return;
    }
    if (next.projectId !== props.projectId || next.taskId !== status.value?.taskId) {
      await initialize();
      return;
    }
    status.value = next;
    if (next.state === "running") schedule(token);
    else emit("updated");
  } catch (caught) {
    if (current(token)) error.value = describeError(caught);
  }
}

function close() {
  disposed = true;
  ++generation;
  stopPolling();
  emit("close");
}
</script>

<template>
  <TaskDialog
    title="修复大脸框"
    description="核对历史遗留的碎片/半脸框和缺失脸框：确认受影响的截图会自动重新处理（重新生成标注、头像并替换归档），其余截图保持不动。"
    :icon="Sparkles"
    :width="520"
    @close="close"
  >
    <p v-if="loading" class="repair-line" role="status">
      <LoaderCircle :size="14" class="animate-spin" />正在读取待核对清单…
    </p>
    <template v-else-if="!status">
      <p class="repair-line">
        本项目有 <strong>{{ candidateCount }}</strong> 张低置信度脸框需要核对。
      </p>
      <p class="repair-line">
        另有 <strong>{{ missingCount }}</strong> 张已完成人物图缺少旧脸框，检测到有效人脸后才重新排队。
      </p>
      <p v-if="retryCount" class="repair-line">
        另有 <strong>{{ retryCount }}</strong> 张上次重新处理时归档没排上队，会一起重试。
      </p>
    </template>
    <template v-else>
      <p class="repair-line" role="status">
        {{ summary }}
      </p>
      <div
        class="repair-progress"
        role="progressbar"
        :aria-valuenow="status.processed"
        aria-valuemin="0"
        :aria-valuemax="status.total"
      >
        <span :style="{ width: `${percent}%` }" />
      </div>
      <p v-if="status.currentFile" class="repair-current">{{ status.currentFile }}</p>
      <p class="repair-line">
        已重新排队 <strong>{{ status.requeued }}</strong> 张 · 无需处理 {{ status.kept }} 张 · 失败
        {{ status.failed }} 张
      </p>
      <p class="repair-hint">
        {{ running ? "关闭这个窗口不会中断核对，后台会继续。" : "重新排队的截图会由后台逐张处理并刷新列表。" }}
      </p>
    </template>
    <p v-if="blockedProject" class="repair-error" role="status">
      其他项目（{{ blockedProject }}）正在核对，本项目暂时无法启动，请等待该任务结束。
    </p>
    <p class="repair-hint">已有检测失败项不包含在本次核对中，请在历史记录中查看错误并手动重试。</p>
    <p v-if="status?.message" class="repair-error" role="alert">{{ status.message }}</p>
    <p v-if="error" class="repair-error" role="alert">{{ error }}</p>
    <template #actions>
      <button
        type="button"
        class="primary-action"
        :disabled="loading || starting || running || !!blockedProject || !!status || candidateCount + missingCount + retryCount === 0"
        @click="start"
      >
        <LoaderCircle v-if="starting" :size="15" class="animate-spin" />
        <Sparkles v-else :size="15" />{{ running ? "后台进行中…" : "开始核对" }}
      </button>
    </template>
  </TaskDialog>
</template>

<style scoped>
.repair-line {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  color: var(--foreground);
  font-size: 11.5px;
}

.repair-hint,
.repair-current {
  margin: 0;
  overflow: hidden;
  color: var(--muted-foreground);
  font-size: 11px;
  text-overflow: ellipsis;
}

.repair-current {
  white-space: nowrap;
}

.repair-error {
  margin: 0;
  color: var(--warn);
  font-size: 11.5px;
}

.repair-progress {
  height: 6px;
  overflow: hidden;
  border-radius: 3px;
  background: var(--secondary);
}

.repair-progress span {
  display: block;
  height: 100%;
  border-radius: 3px;
  background: var(--accent);
  transition: width 0.2s ease;
}
</style>
