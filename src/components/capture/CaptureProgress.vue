<script setup lang="ts">
import { onMounted } from "vue";
import { LoaderCircle } from "@lucide/vue";
import { stageLabel, useCaptureProgress } from "@/lib/capture-progress";

const { start, activeItemId, stage, percent, queuedCount } = useCaptureProgress();
onMounted(() => void start());
</script>

<template>
  <div v-if="activeItemId" class="capture-progress" role="status" aria-live="polite">
    <div class="capture-progress-text">
      <span><LoaderCircle class="animate-spin" :size="13" />{{ stageLabel(stage) }}</span>
      <span>
        {{ Math.round(percent) }}%
        <template v-if="queuedCount > 0"> · 队列 {{ queuedCount }}</template>
      </span>
    </div>
    <div class="capture-progress-track">
      <div class="capture-progress-bar" :style="{ width: `${Math.min(100, Math.max(0, percent))}%` }" />
    </div>
  </div>
</template>
