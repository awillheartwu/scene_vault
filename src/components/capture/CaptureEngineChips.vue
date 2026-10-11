<script setup lang="ts">
import { computed } from "vue";
import type { CaptureEngineMetadata, CaptureImageFacts } from "@/lib/capture-api";
import { formatResolution, renpyBadgeLabel, renpyBadgeTitle } from "@/lib/image-facts";

const props = withDefaults(
  defineProps<{
    engineMetadata: CaptureEngineMetadata | null;
    imageFacts?: CaptureImageFacts | null;
  }>(),
  { imageFacts: null },
);

const badge = computed(() =>
  props.engineMetadata ? renpyBadgeLabel([props.engineMetadata.engineVersion]) : null,
);
const badgeTitle = computed(() => renpyBadgeTitle([props.engineMetadata?.engineVersion]));
const speaker = computed(() => props.engineMetadata?.speakerRaw?.trim() ?? "");
const engineResolution = computed(() =>
  formatResolution(props.engineMetadata?.virtualWidth, props.engineMetadata?.virtualHeight),
);
const captureResolution = computed(() =>
  formatResolution(props.imageFacts?.source?.width, props.imageFacts?.source?.height),
);
</script>

<template>
  <div v-if="engineMetadata" class="file-meta engine-chips">
    <span class="engine-chip" :title="badgeTitle ?? undefined">{{ badge }}</span>
    <span v-if="speaker">说话人 {{ speaker }}</span>
    <span v-if="engineResolution">引擎 {{ engineResolution }}</span>
    <span v-if="captureResolution">截图 {{ captureResolution }}</span>
  </div>
</template>

<style scoped>
.engine-chip {
  padding: 1px 7px;
  border: 1px solid color-mix(in srgb, currentColor 35%, transparent);
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
}
</style>
