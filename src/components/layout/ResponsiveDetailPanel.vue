<script setup lang="ts">
import { nextTick, watch } from "vue";
import { X } from "@lucide/vue";
import {
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import { useAdaptiveLayout } from "@/composables/useAdaptiveLayout";

const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    description?: string;
    closeLabel?: string;
    panelClass?: string;
  }>(),
  {
    description: "",
    closeLabel: "关闭详情",
    panelClass: "",
  },
);

const emit = defineEmits<{
  (event: "update:open", value: boolean): void;
}>();

const { isWideLayout } = useAdaptiveLayout();
let restoreTarget: HTMLElement | null = null;

watch(
  () => props.open,
  (open, wasOpen) => {
    if (open && !wasOpen) {
      restoreTarget = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    } else if (!open && wasOpen && restoreTarget) {
      const target = restoreTarget;
      restoreTarget = null;
      void nextTick(() => target.isConnected && target.focus({ preventScroll: true }));
    }
  },
);
</script>

<template>
  <aside v-if="isWideLayout" :class="['responsive-detail-panel', 'is-static', panelClass]">
    <slot />
  </aside>

  <DialogRoot
    v-else
    :open="props.open"
    @update:open="emit('update:open', $event)"
  >
    <DialogPortal>
      <DialogOverlay class="responsive-detail-scrim" />
      <DialogContent
        :class="['responsive-detail-panel', 'is-drawer', panelClass]"
      >
        <div class="responsive-detail-heading">
          <div>
            <DialogTitle>{{ title }}</DialogTitle>
            <DialogDescription v-if="description">{{ description }}</DialogDescription>
          </div>
          <button
            type="button"
            class="icon-action"
            :aria-label="closeLabel"
            :title="closeLabel"
            @click="emit('update:open', false)"
          >
            <X :size="18" aria-hidden="true" />
          </button>
        </div>
        <div class="responsive-detail-body">
          <slot />
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
