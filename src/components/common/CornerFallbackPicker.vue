<script setup lang="ts">
import { computed } from "vue";
import { Star } from "@lucide/vue";

/**
 * Visual picker for where the name lands when no face was detected.
 * Four clickable stars sit at the corners of a small frame; the selected
 * corner is highlighted.
 */

const CORNERS = [
  { value: "top_left", label: "左上", top: "0%", left: "0%" },
  { value: "top_right", label: "右上", top: "0%", left: "100%" },
  { value: "bottom_left", label: "左下", top: "100%", left: "0%" },
  { value: "bottom_right", label: "右下", top: "100%", left: "100%" },
] as const;

const props = defineProps<{
  modelValue: string | null;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
}>();

const selected = computed<string>(() => props.modelValue || "top_left");

const selectedLabel = computed(
  () => CORNERS.find((c) => c.value === selected.value)?.label ?? "左上",
);

function pick(value: string) {
  emit("update:modelValue", value);
}
</script>

<template>
  <div class="corner-picker">
    <div class="corner-stage">
      <button
        v-for="corner in CORNERS"
        :key="corner.value"
        type="button"
        class="corner-btn"
        :class="{ selected: corner.value === selected }"
        :style="{
          '--top': corner.top,
          '--left': corner.left,
        }"
        :title="`${corner.label}角`"
        :aria-pressed="corner.value === selected"
        @click="pick(corner.value)"
      >
        <Star class="corner-icon" :size="14" :fill="corner.value === selected ? 'currentColor' : 'none'" />
      </button>
    </div>
    <p class="corner-hint">无脸时文字落在：<strong>{{ selectedLabel }}</strong></p>
  </div>
</template>

<style scoped>
.corner-picker {
  width: 100%;
  max-width: 120px;
}

.corner-stage {
  position: relative;
  width: 100%;
  height: 100px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--card) 60%, transparent);
}

.corner-btn {
  position: absolute;
  top: var(--top);
  left: var(--left);
  margin: -13px 0 0 -13px;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 9999px;
  border: 1px solid var(--border);
  background: var(--card);
  color: var(--muted-foreground);
  cursor: pointer;
  transition:
    scale 120ms ease,
    box-shadow 120ms ease,
    border-color 120ms ease,
    color 120ms ease;
}

.corner-btn:hover {
  transform: scale(1.15);
  border-color: var(--primary);
}

.corner-btn.selected {
  border-color: var(--primary);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 25%, transparent);
  color: var(--primary);
}

.corner-icon {
  pointer-events: none;
}

.corner-hint {
  margin-top: 8px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--muted-foreground);
}
</style>
