<script setup lang="ts">
import { computed } from "vue";
import type { CaptureEngineMetadata, CaptureImageFacts } from "@/lib/capture-api";
import { formatResolution } from "@/lib/image-facts";

const props = withDefaults(
  defineProps<{
    engineMetadata: CaptureEngineMetadata | null;
    imageFacts?: CaptureImageFacts | null;
  }>(),
  { imageFacts: null },
);

const speaker = computed(() => props.engineMetadata?.speakerRaw?.trim() ?? "");
const engineResolution = computed(() =>
  formatResolution(props.engineMetadata?.virtualWidth, props.engineMetadata?.virtualHeight),
);
const captureResolution = computed(() =>
  formatResolution(props.imageFacts?.source?.width, props.imageFacts?.source?.height),
);
</script>

<template>
  <div v-if="speaker || engineResolution || captureResolution" class="engine-chips">
    <span v-if="speaker">说话人 {{ speaker }}</span>
    <span v-if="engineResolution">引擎 {{ engineResolution }}</span>
    <span v-if="captureResolution">截图 {{ captureResolution }}</span>
  </div>
</template>

<style scoped>
.engine-chips {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 14px;
  color: var(--muted-foreground);
  font-size: 12px;
  line-height: 1.5;
}
.engine-chips > span {
  overflow-wrap: anywhere;
}
</style>
