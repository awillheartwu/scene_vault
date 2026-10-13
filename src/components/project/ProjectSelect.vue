<script setup lang="ts">
import { computed } from "vue";
import { Check, ChevronDown } from "@lucide/vue";
import {
  SelectContent, SelectIcon, SelectItem, SelectItemIndicator, SelectItemText,
  SelectPortal, SelectRoot, SelectTrigger, SelectViewport,
} from "reka-ui";
import type { Project } from "@/lib/capture-api";
import RenpyBadge from "@/components/capture/RenpyBadge.vue";

const props = withDefaults(defineProps<{
  projects: Project[];
  engineVersions: Record<string, string[]>;
  label?: string;
  disabled?: boolean;
}>(), { label: "当前项目", disabled: false });
const model = defineModel<string>({ required: true });
const emit = defineEmits<{ open: [] }>();
const current = computed(() => props.projects.find((project) => project.id === model.value));
</script>

<template>
  <SelectRoot v-model="model" :disabled="disabled" @update:open="(open) => { if (open) emit('open'); }">
    <SelectTrigger class="project-select-trigger" :aria-label="label" :title="current?.name" @keydown.stop>
      <RenpyBadge v-if="engineVersions[model]" :versions="engineVersions[model]" />
      <span class="project-select-name">{{ current?.name ?? "选择项目" }}</span>
      <SelectIcon class="project-select-icon"><ChevronDown :size="15" /></SelectIcon>
    </SelectTrigger>
    <SelectPortal>
      <SelectContent class="project-select-menu" position="popper" :side-offset="5" align="start" @keydown.stop>
        <SelectViewport class="project-select-viewport">
          <SelectItem
            v-for="project in projects" :key="project.id" :value="project.id"
            :text-value="project.name" :title="project.name" class="project-select-option"
          >
            <SelectItemIndicator class="project-select-check"><Check :size="14" /></SelectItemIndicator>
            <RenpyBadge v-if="engineVersions[project.id]" :versions="engineVersions[project.id]" />
            <SelectItemText class="project-select-name">{{ project.name }}</SelectItemText>
          </SelectItem>
        </SelectViewport>
      </SelectContent>
    </SelectPortal>
  </SelectRoot>
</template>

<style scoped>
.project-select-trigger {
  display: flex;
  width: 100%;
  min-width: 0;
  min-height: 36px;
  align-items: center;
  gap: 8px;
  padding: 5px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control, 8px);
  background: var(--card);
  color: var(--foreground);
  text-align: left;
  font-size: 12px;
  cursor: pointer;
}
.project-select-trigger:focus-visible { outline: 2px solid var(--ring); outline-offset: 2px; }
.project-select-trigger:disabled { opacity: .6; cursor: not-allowed; }
.project-select-name { display: block; min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.project-select-icon { display: flex; flex: none; color: var(--muted-foreground); }
/* Reka's fragment roots do not inherit this component's scoped attribute. */
:global(.project-select-menu) {
  --project-select-menu-height: min(320px, var(--reka-select-content-available-height, 320px));
  z-index: 60;
  display: flex;
  flex-direction: column;
  box-sizing: border-box;
  width: var(--reka-select-trigger-width);
  max-width: calc(100vw - 24px);
  max-height: var(--project-select-menu-height);
  overflow: hidden;
  padding: 4px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control, 8px);
  background: var(--popover);
  color: var(--popover-foreground);
  box-shadow: var(--card-shadow);
}
:global(.project-select-viewport[data-reka-select-viewport]) {
  width: 100%;
  min-height: 0;
  max-height: calc(var(--project-select-menu-height) - 10px);
  overscroll-behavior: contain;
  scrollbar-width: thin;
}
:global(.project-select-viewport[data-reka-select-viewport]::-webkit-scrollbar) { display: block; width: 6px; }
:global(.project-select-viewport[data-reka-select-viewport]::-webkit-scrollbar-thumb) { border-radius: 999px; background: var(--border); }
.project-select-option { position: relative; display: flex; min-width: 0; align-items: center; gap: 8px; padding: 7px 9px 7px 28px; border-radius: 5px; font-size: 12px; cursor: pointer; }
.project-select-option[data-highlighted] { outline: none; background: var(--secondary); }
.project-select-check { position: absolute; left: 8px; color: var(--accent); }
</style>
