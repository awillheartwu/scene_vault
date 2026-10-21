<script setup lang="ts">
import ProjectHomeRow from "@/components/project/ProjectHomeRow.vue";
import type { ProjectArchiveUsage, ProjectOverviewSummary } from "@/lib/capture-api";

defineProps<{
  projects: ProjectOverviewSummary[];
  archiveUsage: Record<string, ProjectArchiveUsage>;
}>();
const emit = defineEmits<{
  open: [project: ProjectOverviewSummary];
  capture: [project: ProjectOverviewSummary];
  launch: [project: ProjectOverviewSummary];
  rename: [project: ProjectOverviewSummary];
  delete: [project: ProjectOverviewSummary];
  contextmenu: [event: MouseEvent, project: ProjectOverviewSummary];
}>();
</script>

<template>
  <div class="project-home-list" role="list" aria-label="项目列表">
    <div class="list-header" aria-hidden="true">
      <span>项目</span>
      <span>状态</span>
      <span class="numeric-heading">归档主图</span>
      <span class="numeric-heading">总大小</span>
      <span class="numeric-heading">来源</span>
      <span class="numeric-heading">会话</span>
      <span>新图规格</span>
      <span>最近活动</span>
      <span class="actions-heading">操作</span>
    </div>
    <ProjectHomeRow
      v-for="project in projects"
      :key="project.projectId"
      :project="project"
      :archive-usage="archiveUsage[project.projectId]"
      @open="emit('open', project)"
      @capture="emit('capture', project)"
      @launch="emit('launch', project)"
      @rename="emit('rename', project)"
      @delete="emit('delete', project)"
      @contextmenu="emit('contextmenu', $event, project)"
    />
  </div>
</template>

<style scoped>
.project-home-list {
  --project-home-list-columns: minmax(228px, 1fr) 136px 96px 88px 56px 64px 152px 112px 114px;
  --project-home-list-gap: 10px;
  container: project-home-list / inline-size;
  width: 100%;
  min-width: 0;
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--border) 78%, transparent);
  border-radius: 13px;
  background: color-mix(in srgb, var(--card) 64%, transparent);
  color: var(--home-foreground, var(--foreground));
}
.list-header {
  display: none;
  align-items: center;
  gap: var(--project-home-list-gap);
  padding: 8px 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 78%, transparent);
  color: var(--home-muted, var(--muted-foreground));
  font-size: 12px;
  line-height: 1.5;
}
.numeric-heading, .actions-heading { text-align: right; }
@container project-home-list (min-width: 1160px) {
  .list-header { display: grid; grid-template-columns: var(--project-home-list-columns); }
}
</style>
