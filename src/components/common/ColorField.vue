<script setup lang="ts">
import { ref } from "vue";
import { onClickOutside, onKeyStroke } from "@vueuse/core";
import { Sketch } from "@ckpack/vue-color";
import { Check } from "@lucide/vue";

/**
 * Color swatch that opens a Sketch-style picker popover.
 * Works with a plain hex string (e.g. "#50dcff").
 */

defineProps<{
  modelValue: string;
  title?: string;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
}>();

const open = ref(false);
const root = ref<HTMLElement | null>(null);

const presetColors = [
  "#ffffff",
  "#f2e7d3",
  "#ffe9a8",
  "#f6c453",
  "#f08c00",
  "#b3541e",
  "#b3392e",
  "#d94848",
  "#c94f8d",
  "#9b59b6",
  "#7d5bb0",
  "#3b5bdb",
  "#2f7fb8",
  "#50dcff",
  "#20c997",
  "#4d7c4f",
  "#9aa03b",
  "#a37a3c",
  "#7a6a4f",
  "#5a5f73",
  "#3b3123",
  "#000000",
] as string[];

onClickOutside(root, () => {
  open.value = false;
});

onKeyStroke("Escape", () => {
  open.value = false;
});

function toggle() {
  open.value = !open.value;
}

function applyColor(value: unknown) {
  if (typeof value === "string") {
    emit("update:modelValue", value);
  }
}
</script>

<template>
  <div ref="root" class="color-field">
    <button
      type="button"
      class="swatch"
      :class="{ open }"
      :style="{ background: modelValue }"
      :title="title"
      :aria-label="`选择颜色（当前 ${modelValue}）`"
      :aria-expanded="open"
      @click="toggle"
    >
      <span class="swatch-check" :style="{ color: modelValue }">
        <Check :size="13" />
      </span>
    </button>
    <div v-if="open" class="picker-popover" role="dialog" aria-label="颜色选择器">
      <Sketch
        :model-value="modelValue"
        :preset-colors="presetColors"
        disable-alpha
        @update:model-value="applyColor"
      />
    </div>
  </div>
</template>

<style scoped>
.color-field {
  position: relative;
  display: inline-block;
}

.swatch {
  width: 34px;
  height: 34px;
  border-radius: var(--radius-md);
  border: 1px solid var(--border);
  cursor: pointer;
  box-shadow: inset 0 0 0 1px rgb(0 0 0 / 0.08);
  transition:
    box-shadow 120ms ease,
    border-color 120ms ease;
}

.swatch:hover,
.swatch.open {
  border-color: var(--primary);
  box-shadow:
    inset 0 0 0 1px rgb(0 0 0 / 0.08),
    0 0 0 3px color-mix(in srgb, var(--primary) 20%, transparent);
}

.swatch-check {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  mix-blend-mode: difference;
}

.picker-popover {
  position: absolute;
  top: calc(100% + 8px);
  left: 0;
  z-index: 50;
  border-radius: var(--radius-lg);
  box-shadow: 0 10px 32px rgb(0 0 0 / 0.25);
}

.picker-popover :deep(.vc-sketch) {
  border-radius: var(--radius-lg);
}
</style>
