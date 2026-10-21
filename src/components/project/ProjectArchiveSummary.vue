<script setup lang="ts">
import { computed } from "vue";
import { CircleAlert, Images } from "@lucide/vue";
import type { ProjectArchiveUsage, ProjectOverviewSummary } from "@/lib/capture-api";
import { projectArchivePresentation } from "@/lib/project-archive-usage";

const props = defineProps<{
  project: ProjectOverviewSummary;
  archiveUsage?: ProjectArchiveUsage;
}>();

const presentation = computed(() => projectArchivePresentation(props.project, props.archiveUsage));
const sizeParts = computed(() => presentation.value.totalSize.split(" "));
</script>

<template>
  <span
    class="project-archive-summary"
    :class="`archive-${presentation.state}`"
    :title="presentation.tooltip"
    :aria-busy="presentation.state === 'loading'"
    :aria-label="presentation.tooltip"
    role="group"
  >
    <CircleAlert
      v-if="['unavailable', 'missing', 'partial', 'error'].includes(presentation.state)"
      :size="14"
      aria-hidden="true"
    />
    <Images v-else :size="14" aria-hidden="true" />
    <span class="archive-parts" aria-hidden="true">
      <span v-if="presentation.state === 'ready'" class="archive-main">
        归档 <strong>{{ presentation.mainImageCount }}</strong> 张
      </span>
      <span v-else class="archive-state">{{ presentation.label }}</span>
      <template v-if="presentation.state === 'ready'">
        <span class="separator">·</span>
        <span class="archive-size">
          <strong>{{ sizeParts[0] }}</strong>
          <span v-if="sizeParts[1]" class="archive-unit">{{ sizeParts[1] }}</span>
        </span>
      </template>
      <span class="separator">·</span>
      <span class="archive-source">{{ project.sourceCount }} 来源</span>
    </span>
  </span>
</template>

<style scoped>
.project-archive-summary {
  display: flex;
  min-width: 0;
  align-items: flex-start;
  gap: 6px;
  color: var(--home-foreground, var(--foreground));
  font-size: 12.5px;
  font-variant-numeric: tabular-nums;
  line-height: 1.5;
  overflow-wrap: anywhere;
}
.project-archive-summary > svg { flex: none; margin-top: 2px; }
.archive-parts { display: flex; flex-wrap: wrap; align-items: baseline; gap: 0 5px; }
.archive-main, .archive-size, .archive-source { white-space: nowrap; }
.archive-main > strong { color: var(--home-count, var(--accent)); font-weight: 650; }
.archive-size { display: inline-flex; align-items: baseline; gap: 3px; }
.archive-size > strong { color: var(--home-size, var(--info)); font-weight: 650; }
.archive-main, .archive-unit, .archive-source, .separator { color: var(--home-muted, var(--muted-foreground)); }
.archive-unavailable, .archive-missing, .archive-error { color: var(--home-danger, var(--destructive)); }
.archive-partial { color: var(--home-warn, var(--warn)); }
.archive-loading, .archive-unconfigured { color: var(--home-muted, var(--muted-foreground)); }
</style>
