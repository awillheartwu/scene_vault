<script setup lang="ts">
import { computed } from "vue";
import { renpyBadgeLabel, renpyBadgeTitle } from "@/lib/image-facts";

const props = withDefaults(
  defineProps<{ versions?: Array<string | null | undefined> }>(),
  { versions: () => [] },
);
const label = computed(() => renpyBadgeLabel(props.versions));
const title = computed(() => renpyBadgeTitle(props.versions));
</script>

<template>
  <span class="renpy-badge" :title="title ?? undefined">{{ label }}</span>
</template>

<style scoped>
.renpy-badge {
  display: inline-flex;
  flex: none;
  align-items: center;
  min-height: 24px;
  padding: 2px 8px;
  border: 1px solid color-mix(in srgb, var(--info) 32%, var(--border));
  border-radius: 999px;
  background: color-mix(in srgb, var(--info) 12%, var(--card));
  color: var(--info);
  font-size: 11px;
  font-weight: 600;
  line-height: 1.3;
  white-space: nowrap;
}
</style>
