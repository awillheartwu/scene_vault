<script setup lang="ts">
import { computed } from "vue";
import {
  Camera, ChevronRight, CircleAlert, FolderKanban, MoreHorizontal, Pencil, Play, Trash2,
} from "@lucide/vue";
import CaptureThumbnail from "@/components/capture/CaptureThumbnail.vue";
import RenpyBadge from "@/components/capture/RenpyBadge.vue";
import ProjectArchiveSummary from "@/components/project/ProjectArchiveSummary.vue";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { CaptureItem, ProjectArchiveUsage, ProjectOverviewSummary } from "@/lib/capture-api";
import { archiveSpecSummary, archiveSpecTooltip, engineVersionList } from "@/lib/image-facts";

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
  <article class="project-home-card" @contextmenu="emit('contextmenu', $event)">
    <!-- Keep the cover sharp; the same image continues behind the glass below. -->
    <div class="card-backdrop" aria-hidden="true">
      <CaptureThumbnail
        v-if="coverItem"
        :item="coverItem"
        variant="source"
        fallback-variant="destination"
        size="full"
        alt=""
      />
    </div>
    <button
      type="button"
      class="card-cover"
      data-context-allow
      tabindex="-1"
      :aria-label="`打开项目：${project.name}`"
      @click="emit('open')"
    >
      <span v-if="!coverItem" class="card-cover-empty">
        <FolderKanban :size="30" aria-hidden="true" />
        <span>暂无可用封面</span>
      </span>
      <span class="card-cover-badges">
        <RenpyBadge
          v-if="project.engine === 'renpy'"
          :versions="engineVersionList(project.engineVersions)"
        />
        <span v-if="project.activeSessionCount" class="capturing-badge">
          <span aria-hidden="true" />正在捕获
        </span>
      </span>
    </button>
    <div class="card-information">
      <div class="card-glass" aria-hidden="true">
        <span class="glass-soft" />
        <span class="glass-medium" />
        <span class="glass-deep" />
      </div>
      <button
        type="button"
        class="card-details"
        data-context-allow
        @click="emit('open')"
      >
        <span class="card-title-row">
          <span class="sr-only">打开项目：</span>
          <strong :title="project.name">{{ project.name }}</strong>
          <ChevronRight :size="18" aria-hidden="true" />
        </span>
        <span v-if="project.description" class="card-description" :title="project.description">
          {{ project.description }}
        </span>
        <ProjectArchiveSummary :project="project" :archive-usage="archiveUsage" />
        <span class="card-spec" :title="archiveSpecTitle" :aria-label="archiveSpecTitle">
          <span class="spec-label">新图：</span>{{ archiveSpec }}
        </span>
        <span class="card-statuses">
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
        </span>
      </button>
      <footer class="card-footer">
        <time :datetime="project.lastActivityAt" :title="`最近活动：${project.lastActivityAt}`">
          最近活动 · {{ activity }}
        </time>
        <div class="card-actions">
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
      </footer>
    </div>
  </article>
</template>

<style scoped>
.project-home-card {
  --card-surface: color-mix(in srgb, var(--card) 94%, var(--accent));
  --glass-smoke: #646d66;
  --glass-text-bed: #b7bab1;
  --glass-contrast: .86;
  --glass-brightness: 1.12;
  --glass-top-opacity: 34%;
  --glass-middle-opacity: 44%;
  --glass-bottom-opacity: 54%;
  --glass-text-shadow: rgb(235 237 229 / .28);
  display: flex;
  flex-direction: column;
  position: relative;
  min-width: 0;
  overflow: hidden;
  isolation: isolate;
  border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
  border-radius: 12px;
  background: var(--card-surface);
  color: var(--home-foreground, var(--foreground));
  box-shadow: 0 2px 8px -3px color-mix(in srgb, var(--foreground) 12%, transparent),
    0 12px 26px -15px color-mix(in srgb, var(--foreground) 20%, transparent);
  transition: border-color .18s ease, box-shadow .18s ease;
}
:global(.dark .project-home-card) {
  --glass-smoke: #081418;
  --glass-text-bed: #0c171c;
  --glass-contrast: 1;
  --glass-brightness: 1;
  --glass-top-opacity: 62%;
  --glass-middle-opacity: 74%;
  --glass-bottom-opacity: 82%;
  --glass-text-shadow: rgb(0 0 0 / .35);
}
.project-home-card:hover, .project-home-card:focus-within {
  border-color: color-mix(in srgb, var(--accent) 38%, transparent);
  box-shadow: var(--card-shadow-hover);
}
/* Only the outer ring is softened; the text and image centre stay crisp. */
.project-home-card::after {
  position: absolute;
  z-index: 2;
  inset: 0;
  padding: 3px;
  border-radius: inherit;
  -webkit-backdrop-filter: blur(1.5px);
  backdrop-filter: blur(1.5px);
  -webkit-mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
  -webkit-mask-composite: xor;
  mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0);
  mask-composite: exclude;
  pointer-events: none;
  content: "";
}
.card-backdrop {
  position: absolute;
  z-index: -1;
  inset: 0;
  -webkit-mask-image: linear-gradient(90deg, transparent, #000 2px, #000 calc(100% - 2px), transparent),
    linear-gradient(180deg, transparent, #000 2px, #000 calc(100% - 2px), transparent);
  -webkit-mask-composite: source-in;
  mask-image: linear-gradient(90deg, transparent, #000 2px, #000 calc(100% - 2px), transparent),
    linear-gradient(180deg, transparent, #000 2px, #000 calc(100% - 2px), transparent);
  mask-composite: intersect;
  pointer-events: none;
}
.card-cover, .card-details { padding: 0; border: 0; background: transparent; color: inherit; text-align: left; }
.card-cover:focus-visible, .card-details:focus-visible { outline: 2px solid var(--ring); outline-offset: -3px; border-radius: 11px; }
.card-cover { position: relative; display: block; width: 100%; flex: none; aspect-ratio: 16 / 9; }
.card-cover-empty { display: flex; height: 100%; flex-direction: column; align-items: center; justify-content: center; gap: 8px; padding-bottom: 38px; color: var(--home-muted, var(--muted-foreground)); font-size: 12px; }
.card-cover-badges { position: absolute; top: 10px; right: 10px; left: 10px; display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
.capturing-badge { display: inline-flex; min-height: 24px; align-items: center; gap: 6px; padding: 2px 8px; border-radius: 999px; background: var(--accent); color: var(--accent-foreground); font-size: 12px; font-weight: 650; line-height: 1.3; }
.capturing-badge > span { width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
.card-information {
  position: relative;
  display: flex;
  min-width: 0;
  flex: 1;
  flex-direction: column;
  margin-top: -44px;
}
.card-glass {
  position: absolute;
  inset: -42px 0 0;
  z-index: 0;
  pointer-events: none;
}
.card-glass > span, .card-glass::after { position: absolute; inset: 0; }
/* Overlapping masked layers let both blur and tint build up without a seam. */
.card-glass > span {
  -webkit-mask-image: var(--glass-mask);
  mask-image: var(--glass-mask);
  -webkit-backdrop-filter: blur(var(--glass-blur)) saturate(1.04);
  backdrop-filter: blur(var(--glass-blur)) saturate(1.04);
}
.glass-soft { --glass-blur: 2px; --glass-mask: linear-gradient(180deg, transparent, #000 35%); }
.card-glass > .glass-medium {
  --glass-blur: 6px;
  --glass-mask: linear-gradient(180deg, transparent, #000 38%);
  -webkit-backdrop-filter: blur(6px) saturate(1.04) contrast(var(--glass-contrast)) brightness(var(--glass-brightness));
  backdrop-filter: blur(6px) saturate(1.04) contrast(var(--glass-contrast)) brightness(var(--glass-brightness));
}
.glass-deep { --glass-blur: 12px; --glass-mask: linear-gradient(180deg, transparent 28%, #000 88%); }
.card-glass::after {
  background: linear-gradient(180deg,
    transparent,
    color-mix(in srgb, var(--glass-smoke) 10%, transparent) 10%,
    color-mix(in srgb, var(--glass-text-bed) var(--glass-top-opacity), transparent) 25%,
    color-mix(in srgb, var(--glass-text-bed) var(--glass-middle-opacity), transparent) 52%,
    color-mix(in srgb, var(--glass-text-bed) var(--glass-bottom-opacity), transparent));
  content: "";
}
.card-details {
  position: relative;
  z-index: 1;
  display: grid;
  width: 100%;
  min-width: 0;
  flex: 1;
  align-content: start;
  gap: 5px;
  padding: 11px 12px 7px;
  text-shadow: 0 1px 1px var(--glass-text-shadow);
}
.card-title-row { display: flex; min-width: 0; align-items: center; gap: 8px; }
.card-title-row > strong { flex: 1; min-width: 0; overflow: hidden; font-size: 16px; font-weight: 700; line-height: 1.4; text-overflow: ellipsis; white-space: nowrap; }
.card-title-row > svg { flex: none; color: var(--accent); }
.card-description, .card-spec { display: block; overflow: hidden; font-size: 12px; line-height: 1.5; text-overflow: ellipsis; white-space: nowrap; }
.card-description, .spec-label { color: var(--home-muted, var(--muted-foreground)); }
.card-spec { color: var(--home-spec, var(--muted-foreground)); font-weight: 550; }
.spec-label { font-weight: 400; }
.card-statuses { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; margin-top: 1px; font-size: 12px; line-height: 1.5; }
.task-status { display: inline-flex; align-items: center; gap: 5px; }
.task-status strong { font-weight: 650; }
.task-status > svg, .task-status > i { flex: none; }
.task-status > i { width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
.pending, .config-warning { color: var(--home-warn, var(--warn)); }
.processing { color: var(--home-info, var(--info)); }
.failed { color: var(--home-danger, var(--destructive)); }
.idle-status { color: var(--home-muted, var(--muted-foreground)); }
.card-footer {
  position: relative;
  z-index: 1;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 6px 10px;
  padding: 3px 12px 10px;
}
.card-footer > time { color: var(--home-muted, var(--muted-foreground)); font-size: 11.5px; font-variant-numeric: tabular-nums; line-height: 1.5; }
.card-actions { display: flex; flex: none; align-items: center; gap: 6px; margin-left: auto; }
.icon-button { display: grid; width: 34px; height: 34px; flex: none; place-items: center; padding: 0; border: 1px solid color-mix(in srgb, var(--home-foreground, var(--foreground)) 20%, transparent); border-radius: 8px; background: color-mix(in srgb, var(--glass-text-bed) 38%, transparent); color: var(--home-foreground, var(--foreground)); transition: color .15s ease, border-color .15s ease, background-color .15s ease; }
.launch-button { border-color: color-mix(in srgb, var(--accent) 40%, transparent); color: var(--accent); }
.icon-button:disabled { opacity: .4; }
.icon-button:not(:disabled):hover, .icon-button[data-state="open"] { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 12%, var(--card)); color: var(--accent); }
.icon-button:not(:disabled):active { background: color-mix(in srgb, var(--accent) 22%, var(--card)); }
.icon-button:focus-visible { outline: 2px solid var(--ring); outline-offset: 2px; }
@media (prefers-reduced-motion: reduce) {
  .project-home-card, .icon-button { transition: none; }
}
</style>
