<script setup lang="ts">
import { computed, onMounted } from "vue";
import { Activity, AlertCircle, LoaderCircle } from "@lucide/vue";
import { stageLabel, useCaptureProgress } from "@/lib/capture-progress";

withDefaults(defineProps<{ persistent?: boolean; deferredCount?: number }>(), {
  persistent: false,
  deferredCount: 0,
});

const {
  start,
  activeItemId,
  activeSourcePath,
  stage,
  percent,
  queuedCount,
  prelabelPendingCount,
  engineStatus,
} = useCaptureProgress();
const activeFileName = computed(() => activeSourcePath.value?.split(/[\\/]/).pop() || "正在读取任务…");
const engineUnconfigured = computed(() => engineStatus.value === "unconfigured");
onMounted(() => void start());
</script>

<template>
  <div v-if="activeItemId || persistent" class="capture-progress" :class="{ idle: !activeItemId }" role="status" aria-live="polite">
    <div class="capture-progress-text">
      <template v-if="activeItemId">
        <span class="capture-progress-task">
          <LoaderCircle class="animate-spin" :size="13" />
          <span>{{ stageLabel(stage) }}</span>
          <strong :title="activeSourcePath || activeFileName">{{ activeFileName }}</strong>
        </span>
        <span class="capture-progress-value">
          <strong>{{ Math.round(percent) }}%</strong>
          <template v-if="queuedCount > 0"> · 后续 {{ queuedCount }} 张</template>
          <template v-if="prelabelPendingCount > 0"> · 剩余 {{ prelabelPendingCount }} 张</template>
        </span>
      </template>
      <template v-else>
        <span><Activity :size="13" />图片处理状态</span>
        <span v-if="engineUnconfigured" class="capture-progress-hint" data-state="engine-unconfigured">
          <AlertCircle :size="13" />AI 未配置：导入截图不会自动识别，可人工分类，或到设置中配置后逐张处理
        </span>
        <span v-else-if="deferredCount > 0" class="capture-progress-hint" data-state="deferred-import">
          已登记 {{ deferredCount }} 张截图，点击「开始识别导入截图」后逐张处理
        </span>
        <span v-else-if="prelabelPendingCount > 0" class="capture-progress-batch-text" data-state="batch-remaining">
          本批识别 剩余 {{ prelabelPendingCount }} 张
        </span>
        <span v-else>{{ queuedCount > 0 ? `等待队列 ${queuedCount}` : "空闲" }}</span>
      </template>
    </div>
    <div class="capture-progress-track">
      <div class="capture-progress-bar" :style="{ width: `${Math.min(100, Math.max(0, percent))}%` }" />
    </div>
  </div>
</template>
