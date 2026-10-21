<script setup lang="ts">
import { computed } from "vue";
import { Camera, CircleAlert, FolderKanban, MoreHorizontal, Pencil, Play, Trash2 } from "@lucide/vue";
import CaptureThumbnail from "@/components/capture/CaptureThumbnail.vue";
import RenpyBadge from "@/components/capture/RenpyBadge.vue";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { CaptureItem, ProjectArchiveUsage, ProjectOverviewSummary } from "@/lib/capture-api";
import { archiveSpecSummary, archiveSpecTooltip, engineVersionList } from "@/lib/image-facts";
import { projectArchivePresentation } from "@/lib/project-archive-usage";

const props = defineProps<{
  project: ProjectOverviewSummary;
  archiveUsage?: ProjectArchiveUsage;
}>();
const emit = defineEmits<{
  open: [];
  capture: [];
  launch: [];
  rename: [];
  delete: [];
  contextmenu: [event: MouseEvent];
}>();

const coverItem = computed(() => {
  const id = props.project.coverCaptureItemId ?? props.project.latestCaptureItemId;
  return id ? { id, sourcePath: "capture.jpg" } as CaptureItem : null;
});
const archive = computed(() => projectArchivePresentation(props.project, props.archiveUsage));
const archiveSize = computed(() => archive.value.totalSize.split(" "));
const archiveSpec = computed(() => archiveSpecSummary(
  props.project.archiveOutputSize,
  props.project.archiveOutputFormat,
  props.project.archiveOutputQuality,
));
const archiveSpecTitle = computed(() => archiveSpecTooltip(
  props.project.archiveOutputSize,
  props.project.archiveOutputFormat,
  props.project.archiveOutputQuality,
));
const configWarning = computed(() => {
  const project = props.project;
  if (!project.sourceCount && !project.destinationConfigured) return "尚未配置目录";
  if (!project.sourceCount) return "缺少截图来源";
  if (!project.destinationConfigured) return "缺少归档目录";
  return "";
});
const idleStatus = computed(() => {
  const project = props.project;
  if (!project.captureCount) return "暂无待处理任务";
  return project.completedCount === project.captureCount
    ? "全部已归档"
    : `已归档 ${project.completedCount} / ${project.captureCount}`;
});
const activity = computed(() => {
  const date = new Date(props.project.lastActivityAt);
  return Number.isNaN(date.getTime()) ? "尚无活动" : date.toLocaleString("zh-CN", {
    month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
});
</script>

<template>
  <article class="project-home-row" role="listitem" @contextmenu="emit('contextmenu', $event)">
    <div class="row-layout">
      <div class="row-project">
        <div class="row-identity">
          <button
            type="button"
            class="row-cover"
            data-context-allow
            :aria-label="`打开项目：${project.name}`"
            @click="emit('open')"
          >
            <CaptureThumbnail
              v-if="coverItem"
              :item="coverItem"
              variant="source"
              fallback-variant="destination"
              alt=""
            />
            <FolderKanban v-else :size="23" aria-hidden="true" />
          </button>
          <div class="row-name">
            <button type="button" class="row-title" data-context-allow @click="emit('open')">
              <span class="sr-only">打开项目：</span>
              <strong :title="project.name">{{ project.name }}</strong>
              <RenpyBadge
                v-if="project.engine === 'renpy'"
                :versions="engineVersionList(project.engineVersions)"
              />
            </button>
            <span v-if="project.description" class="row-description" :title="project.description">
              {{ project.description }}
            </span>
          </div>
        </div>
        <div class="row-summary">
          <span
            class="row-count"
            role="group"
            :class="`archive-${archive.state}`"
            :title="archive.tooltip"
            :aria-label="`${archive.label}，主图 ${archive.mainImageCount}${archive.mainImageCount === '—' ? '' : ' 张'}`"
            :aria-busy="archive.state === 'loading'"
          >
            <span class="archive-label">{{ archive.label }}</span>
            <strong class="row-value">{{ archive.mainImageCount }}</strong>
            <span v-if="archive.mainImageCount !== '—'" class="row-unit">张</span>
          </span>
          <span
            class="row-size"
            :class="{ 'size-unavailable': archive.state !== 'ready' }"
            role="group"
            :title="archive.tooltip"
            :aria-label="`总大小：${archive.totalSize}`"
          >
            <strong class="row-value">{{ archiveSize[0] }}</strong>
            <span v-if="archiveSize[1]" class="row-unit">{{ archiveSize[1] }}</span>
          </span>
          <span
            class="row-sources row-secondary"
            role="group"
            :title="archive.tooltip"
            :aria-label="`${project.sourceCount} 个截图来源`"
          >
            <strong class="row-value">{{ project.sourceCount.toLocaleString("zh-CN") }}</strong>
            <span class="row-inline-label">来源</span>
          </span>
        </div>
        <div class="row-meta">
          <span class="row-spec" role="group" :title="archiveSpecTitle" :aria-label="archiveSpecTitle">
            <span class="row-spec-prefix">新图：</span><strong>{{ archiveSpec }}</strong>
          </span>
          <span
            class="row-sessions row-secondary"
            role="group"
            :aria-label="`${project.sessionCount} 个会话`"
          >
            <strong class="row-value">{{ project.sessionCount.toLocaleString("zh-CN") }}</strong>
            <span class="row-inline-label">个会话</span>
          </span>
        </div>
      </div>
      <div class="row-statuses" aria-label="项目状态">
        <span v-if="project.activeSessionCount" class="task-status capturing">
          <i aria-hidden="true" />正在捕获
        </span>
        <span v-if="project.awaitingCount" class="task-status pending">
          <i aria-hidden="true" /><span>待分类 <strong>{{ project.awaitingCount }}</strong></span>
        </span>
        <span v-if="project.processingCount" class="task-status processing">
          <i aria-hidden="true" /><span>处理中 <strong>{{ project.processingCount }}</strong></span>
        </span>
        <span v-if="project.failedCount" class="task-status failed">
          <CircleAlert :size="13" aria-hidden="true" /><span>失败 <strong>{{ project.failedCount }}</strong></span>
        </span>
        <span v-if="configWarning" class="task-status config-warning">
          <CircleAlert :size="13" aria-hidden="true" />{{ configWarning }}
        </span>
        <span
          v-if="!project.awaitingCount && !project.processingCount && !project.failedCount"
          class="idle-status"
        >{{ idleStatus }}</span>
      </div>
      <time class="row-activity" :datetime="project.lastActivityAt" :title="`最近活动：${project.lastActivityAt}`">
        <span class="activity-label">最近活动</span>{{ activity }}
      </time>
      <div class="row-actions">
        <button
          type="button"
          class="icon-button launch-button"
          :disabled="project.engine !== 'renpy'"
          :aria-label="`启动游戏 ${project.name}`"
          :title="project.engine === 'renpy' ? '启动游戏' : '此项目暂不支持直接启动游戏'"
          @click="emit('launch')"
        >
          <Play :size="17" aria-hidden="true" />
        </button>
        <button
          type="button"
          class="icon-button"
          :aria-label="`继续捕获 ${project.name}`"
          title="继续捕获"
          @click="emit('capture')"
        >
          <Camera :size="17" aria-hidden="true" />
        </button>
        <DropdownMenu>
          <DropdownMenuTrigger as-child>
            <button
              type="button"
              class="icon-button"
              :aria-label="`${project.name} 更多操作`"
              title="更多操作"
            >
              <MoreHorizontal :size="18" aria-hidden="true" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem @select="emit('rename')">
              <Pencil :size="15" aria-hidden="true" />重命名
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" @select="emit('delete')">
              <Trash2 :size="15" aria-hidden="true" />删除项目
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  </article>
</template>

<style scoped>
.project-home-row {
  min-width: 0;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 78%, transparent);
  background: color-mix(in srgb, var(--card) 80%, transparent);
  color: var(--home-foreground, var(--foreground));
  transition: background-color .18s ease;
}
.project-home-row:last-child { border-bottom: 0; }
.project-home-row:hover, .project-home-row:focus-within { background: color-mix(in srgb, var(--home-accent, var(--accent)) 5%, var(--card)); }
.row-layout { display: grid; min-width: 0; min-height: 79px; grid-template-columns: minmax(0, 1fr) minmax(136px, 200px) 108px 114px; align-items: center; gap: 10px 12px; padding: 8px 12px; }
.row-project { display: grid; min-width: 0; grid-template-columns: 96px minmax(0, 1fr); align-items: center; gap: 2px 12px; }
.row-identity { display: contents; }
.row-cover { display: grid; grid-column: 1; grid-row: 1 / 4; width: 96px; aspect-ratio: 16 / 9; overflow: hidden; place-items: center; padding: 0; border: 1px solid var(--border); border-radius: 7px; background: var(--background); color: var(--home-muted, var(--muted-foreground)); }
.row-name { grid-column: 2; grid-row: 1; min-width: 0; }
.row-title { display: flex; width: 100%; min-width: 0; align-items: center; gap: 8px; padding: 0; border: 0; border-radius: 4px; background: transparent; color: inherit; text-align: left; }
.row-cover:focus-visible, .row-title:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }
.row-title > strong { min-width: 0; overflow: hidden; font-size: 14px; font-weight: 650; line-height: 1.5; text-overflow: ellipsis; white-space: nowrap; }
.row-title :deep(.renpy-badge) { min-height: 20px; padding: 1px 7px; font-size: 11.5px; }
.row-description { display: block; overflow: hidden; color: var(--home-muted, var(--muted-foreground)); font-size: 12px; line-height: 1.5; text-overflow: ellipsis; white-space: nowrap; }
.row-summary, .row-meta { display: flex; grid-column: 2; min-width: 0; flex-wrap: wrap; align-items: baseline; gap: 2px 8px; font-size: 12px; font-variant-numeric: tabular-nums; line-height: 1.5; }
.row-summary { grid-row: 2; }
.row-meta { grid-row: 3; }
.row-count, .row-size, .row-secondary { display: inline-flex; min-width: 0; align-items: baseline; gap: 4px; }
.row-count { flex-wrap: wrap; color: var(--home-count, var(--accent)); }
.row-size { color: var(--home-size, var(--info)); white-space: nowrap; }
.row-value { font-weight: 650; }
.archive-label, .row-unit, .row-secondary, .row-spec-prefix { color: var(--home-muted, var(--muted-foreground)); }
.archive-loading, .archive-unconfigured { color: var(--home-muted, var(--muted-foreground)); }
.archive-unavailable, .archive-missing, .archive-error { color: var(--home-danger, var(--destructive)); }
.archive-partial { color: var(--home-warn, var(--warn)); }
.row-count:not(.archive-ready) .archive-label { color: inherit; }
.row-count:not(.archive-ready) .row-value, .size-unavailable { display: none; }
.row-spec { min-width: 0; max-width: 100%; overflow: hidden; color: var(--home-spec, var(--muted-foreground)); text-overflow: ellipsis; white-space: nowrap; }
.row-spec > strong { font-weight: 600; }
.row-sessions { flex: none; white-space: nowrap; }
.row-statuses { display: flex; min-width: 0; flex-wrap: wrap; align-items: center; gap: 4px 10px; font-size: 12px; line-height: 1.5; }
.task-status { display: inline-flex; align-items: center; gap: 5px; }
.task-status strong { font-weight: 650; }
.task-status > svg, .task-status > i { flex: none; }
.task-status > i { width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
.capturing { color: var(--home-accent, var(--accent)); font-weight: 650; }
.pending, .config-warning { color: var(--home-warn, var(--warn)); }
.processing { color: var(--home-info, var(--info)); }
.failed { color: var(--home-danger, var(--destructive)); }
.idle-status { color: var(--home-muted, var(--muted-foreground)); }
.row-activity { color: var(--home-muted, var(--muted-foreground)); font-size: 12px; font-variant-numeric: tabular-nums; line-height: 1.5; }
.activity-label { display: block; font-size: 11.5px; }
.row-actions { display: flex; align-items: center; justify-content: flex-end; gap: 6px; }
.icon-button { display: grid; width: 34px; height: 34px; flex: none; place-items: center; padding: 0; border: 1px solid var(--border); border-radius: 8px; background: transparent; color: var(--home-foreground, var(--foreground)); transition: color .15s ease, border-color .15s ease, background-color .15s ease; }
.launch-button { color: var(--home-accent, var(--accent)); }
.icon-button:disabled { opacity: .4; }
.icon-button:not(:disabled):hover, .icon-button[data-state="open"] { border-color: var(--home-accent, var(--accent)); background: color-mix(in srgb, var(--home-accent, var(--accent)) 10%, var(--card)); color: var(--home-accent, var(--accent)); }
.icon-button:not(:disabled):active { background: color-mix(in srgb, var(--home-accent, var(--accent)) 20%, var(--card)); }
.icon-button:focus-visible { outline: 2px solid var(--ring); outline-offset: 2px; }
@container project-home-list (min-width: 1160px) {
  .row-layout { grid-template-columns: var(--project-home-list-columns); gap: var(--project-home-list-gap); }
  .row-project, .row-summary, .row-meta { display: contents; }
  .row-identity { display: grid; grid-column: 1; grid-row: 1; min-width: 0; grid-template-columns: 96px minmax(0, 1fr); align-items: center; gap: 12px; }
  .row-cover { grid-row: 1; }
  .row-statuses { grid-column: 2; grid-row: 1; }
  .row-count { grid-column: 3; grid-row: 1; justify-content: flex-end; font-size: 13px; }
  .archive-ready .archive-label { display: none; }
  .row-count:not(.archive-ready) .archive-label { flex-basis: 100%; text-align: right; font-size: 12px; }
  .row-size { grid-column: 4; grid-row: 1; justify-content: flex-end; font-size: 13px; }
  .size-unavailable { display: inline-flex; color: var(--home-muted, var(--muted-foreground)); }
  .row-sources { grid-column: 5; grid-row: 1; justify-content: flex-end; }
  .row-sessions { grid-column: 6; grid-row: 1; justify-content: flex-end; }
  .row-inline-label, .row-spec-prefix, .activity-label { display: none; }
  .row-spec { grid-column: 7; grid-row: 1; font-size: 12px; }
  .row-activity { grid-column: 8; grid-row: 1; }
  .row-actions { grid-column: 9; grid-row: 1; }
}
@container project-home-list (max-width: 900px) {
  .row-layout { grid-template-columns: minmax(0, 1fr) 170px 114px; gap: 8px 12px; }
  .row-project { grid-column: 1; grid-row: 1 / 3; }
  .row-statuses { grid-column: 2; grid-row: 1; align-self: end; }
  .row-activity { grid-column: 2; grid-row: 2; align-self: start; font-size: 11.5px; }
  .activity-label { display: inline; margin-right: 6px; }
  .row-actions { grid-column: 3; grid-row: 1 / 3; }
}
@container project-home-list (max-width: 620px) {
  .row-layout { grid-template-columns: minmax(0, 1fr) 114px; }
  .row-project { grid-column: 1; grid-row: 1; grid-template-columns: 80px minmax(0, 1fr); column-gap: 10px; }
  .row-cover { width: 80px; }
  .row-statuses { grid-column: 1 / -1; grid-row: 2; padding-left: 90px; align-self: auto; }
  .row-activity { grid-column: 1 / -1; grid-row: 3; padding-left: 90px; align-self: auto; }
  .row-actions { grid-column: 2; grid-row: 1; }
}
@container project-home-list (max-width: 420px) {
  .row-layout { gap: 8px; }
  .row-project { grid-column: 1 / -1; }
  .row-statuses { grid-column: 1 / -1; grid-row: 2; padding-left: 0; }
  .row-activity { grid-column: 1; grid-row: 3; padding-left: 0; }
  .row-actions { grid-column: 2; grid-row: 3; }
}
@media (prefers-reduced-motion: reduce) {
  .project-home-row, .icon-button { transition: none; }
}
</style>
