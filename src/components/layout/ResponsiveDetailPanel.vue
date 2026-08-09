<script setup lang="ts">
import { X } from "@lucide/vue";
import { DialogContent, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
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
      <div class="responsive-detail-scrim" />
      <DialogContent
        :class="['responsive-detail-panel', 'is-drawer', panelClass]"
        :aria-describedby="description ? 'responsive-detail-description' : undefined"
      >
        <div class="responsive-detail-heading">
          <div>
            <DialogTitle>{{ title }}</DialogTitle>
            <p v-if="description" id="responsive-detail-description">{{ description }}</p>
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
