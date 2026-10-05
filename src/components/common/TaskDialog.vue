<script setup lang="ts">
import type { Component } from "vue";
import { X } from "@lucide/vue";
import {
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";

/**
 * Shared shell for the app's single-task dialogs: identical card, title,
 * description, scroll behaviour, positioning and footer. Task content goes
 * into the default slot, the task's own buttons into the `actions` slot.
 */
const props = defineProps<{
  title: string;
  description: string;
  icon?: Component;
  /** Preferred card width in pixels, clamped to the viewport. */
  width?: number;
}>();

const emit = defineEmits<{ (event: "close"): void }>();
</script>

<template>
  <DialogRoot :open="true" @update:open="!$event && emit('close')">
    <DialogPortal>
      <DialogOverlay class="import-overlay" />
      <DialogContent
        class="import-dialog task-dialog"
        :style="{ width: `min(${props.width ?? 480}px, calc(100vw - 48px))` }"
      >
        <DialogTitle>
          <span v-if="props.icon" class="task-dialog-icon">
            <component :is="props.icon" :size="15" />
          </span>
          {{ props.title }}
        </DialogTitle>
        <DialogDescription>{{ props.description }}</DialogDescription>
        <slot />
        <div class="task-dialog-actions">
          <slot name="actions" />
          <button type="button" class="secondary-action" @click="emit('close')">
            <X :size="15" />关闭
          </button>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped>
.task-dialog {
  /* The shared .import-dialog only draws the card; the shell positions it. */
  position: fixed;
  z-index: calc(var(--layer-overlay) + 1);
  top: 50%;
  left: 50%;
  max-height: min(78vh, 760px);
  overflow-y: auto;
  transform: translate(-50%, -50%);
  gap: 12px;
}

.task-dialog-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  margin-right: 6px;
  border-radius: 7px;
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--accent);
  vertical-align: -5px;
}

.task-dialog-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
}
</style>
