<script setup lang="ts">
import { computed, onMounted } from "vue";
import { Activity, LoaderCircle } from "@lucide/vue";
import { stageLabel, useCaptureProgress } from "@/lib/capture-progress";

withDefaults(defineProps<{ persistent?: boolean }>(), { persistent: false });

const { start, activeItemId, activeSourcePath, stage, percent, queuedCount } = useCaptureProgress();
const activeFileName = computed(() => activeSourcePath.value?.split(/[\\/]/).pop() || "正在读取任务…");
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
        </span>
      </template>
      <template v-else>
        <span><Activity :size="13" />图片处理状态</span>
        <span>{{ queuedCount > 0 ? `等待队列 ${queuedCount}` : "空闲" }}</span>
      </template>
    </div>
    <div class="capture-progress-track">
      <div class="capture-progress-bar" :style="{ width: `${Math.min(100, Math.max(0, percent))}%` }" />
    </div>
  </div>
</template>
