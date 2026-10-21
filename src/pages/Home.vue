<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import {
  ArrowDown,
  ArrowUp,
  Camera,
  FolderKanban,
  Grid2X2,
  List,
  LoaderCircle,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  TriangleAlert,
} from "@lucide/vue";
import PaginationControls from "@/components/common/PaginationControls.vue";
import PageHeader from "@/components/layout/PageHeader.vue";
import { ContextMenu } from "@/components/ui/context-menu";
import ProjectDeleteDialog from "@/components/project/ProjectDeleteDialog.vue";
import ProjectRenameDialog from "@/components/project/ProjectRenameDialog.vue";
import ProjectHomeCard from "@/components/project/ProjectHomeCard.vue";
import ProjectHomeList from "@/components/project/ProjectHomeList.vue";
import { toast } from "@/lib/toast";
import { useContextMenu } from "@/composables/useContextMenu";
import {
  captureApi,
  engineApi,
  type CaptureRuntimeStatus,
  type ProjectArchiveUsage,
  type ProjectOverviewSummary,
} from "@/lib/capture-api";

type ViewMode = "grid" | "list";
type SortMode = "recent" | "attention" | "name" | "captures" | "created" | "size";
type SortDirection = "asc" | "desc";

const VIEW_STORAGE_KEY = "scene-vault.home.project-view";
const SORT_STORAGE_KEY = "scene-vault.home.sort";
const CURRENT_PROJECT_KEY = "scene-vault.capture.project";

const router = useRouter();
const projects = ref<ProjectOverviewSummary[]>([]);
const archiveUsage = ref<Record<string, ProjectArchiveUsage>>({});
let loadGeneration = 0;
let activeArchiveRequestId: string | null = null;
const runtime = ref<CaptureRuntimeStatus | null>(null);
const loading = ref(true);
const errorMessage = ref("");
const search = ref("");
const SORT_MODES: { value: SortMode; label: string }[] = [
  { value: "recent", label: "最近活跃" },
  { value: "attention", label: "待处理" },
  { value: "name", label: "名称" },
  { value: "captures", label: "归档主图数" },
  { value: "size", label: "归档图片大小" },
  { value: "created", label: "创建时间" },
];
const SORT_DIRECTION_DEFAULTS: Record<SortMode, SortDirection> = {
  recent: "desc",
  attention: "desc",
  name: "asc",
  captures: "desc",
  size: "desc",
  created: "desc",
};
const savedSort = localStorage.getItem(SORT_STORAGE_KEY);
const sortMode = ref<SortMode>(
  SORT_MODES.some((mode) => mode.value === savedSort) ? (savedSort as SortMode) : "recent",
);
const sortDirection = ref<SortDirection>(
  localStorage.getItem(`${SORT_STORAGE_KEY}-direction`) === "asc" ? "asc" : "desc",
);
const viewMode = ref<ViewMode>(
  localStorage.getItem(VIEW_STORAGE_KEY) === "list" ? "list" : "grid",
);
const renameTarget = ref<ProjectOverviewSummary | null>(null);
const deleteTarget = ref<ProjectOverviewSummary | null>(null);
const page = ref(1);
const pageSize = ref(50);
const projectMenu = useContextMenu();

function onProjectContext(event: MouseEvent, project: ProjectOverviewSummary) {
  projectMenu.open(event, [
    {
      id: "open",
      label: "打开项目",
      icon: FolderKanban,
      action: () => openProject(project.projectId),
    },
    {
      id: "capture",
      label: "继续捕获",
      icon: Camera,
      action: () => startCapture(project.projectId),
    },
    {
      id: "rename",
      label: "重命名…",
      icon: Pencil,
      separatorBefore: true,
      action: () => {
        renameTarget.value = project;
      },
    },
    {
      id: "delete",
      label: "删除项目…",
      icon: Trash2,
      danger: true,
      action: () => {
        deleteTarget.value = project;
      },
    },
  ]);
}

const pagedProjects = computed(() =>
  visibleProjects.value.slice((page.value - 1) * pageSize.value, page.value * pageSize.value),
);

const visibleProjects = computed(() => {
  const query = search.value.trim().toLocaleLowerCase();
  const filtered = query
    ? projects.value.filter((project) =>
        `${project.name} ${project.description ?? ""}`.toLocaleLowerCase().includes(query),
      )
    : projects.value;

  return [...filtered].sort((left, right) => {
    const direction = sortDirection.value === "asc" ? 1 : -1;
    let primary = 0;
    if (sortMode.value === "name") {
      primary = left.name.localeCompare(right.name, "zh-CN", { sensitivity: "base" });
    } else if (sortMode.value === "captures" || sortMode.value === "size") {
      const leftUsage = archiveUsage.value[left.projectId];
      const rightUsage = archiveUsage.value[right.projectId];
      const leftReady = leftUsage?.state === "ready";
      const rightReady = rightUsage?.state === "ready";
      // Unknown and offline directories always follow complete results, even
      // when sorting ascending; they do not represent empty archives.
      if (leftReady !== rightReady) return leftReady ? -1 : 1;
      if (leftReady && rightReady) {
        const field = sortMode.value === "captures" ? "mainImageCount" : "totalBytes";
        primary = (leftUsage[field] ?? 0) - (rightUsage[field] ?? 0);
      }
    } else if (sortMode.value === "created") {
      primary =
        new Date(left.createdAt).getTime() - new Date(right.createdAt).getTime();
    } else if (sortMode.value === "attention") {
      const leftAttention = left.awaitingCount + left.processingCount + left.failedCount * 2;
      const rightAttention = right.awaitingCount + right.processingCount + right.failedCount * 2;
      primary = leftAttention - rightAttention;
    } else {
      primary =
        new Date(left.lastActivityAt).getTime() - new Date(right.lastActivityAt).getTime();
    }
    if (primary !== 0) return direction * primary;
    return left.name.localeCompare(right.name, "zh-CN", { sensitivity: "base" });
  });
});

const globalPendingCount = computed(() =>
  projects.value.reduce(
    (total, project) => total + project.awaitingCount + project.processingCount + project.failedCount,
    0,
  ),
);

onMounted(() => {
  void initialize();
});

onUnmounted(() => {
  loadGeneration += 1;
  cancelArchiveUsage();
});

async function initialize() {
  cancelArchiveUsage();
  const generation = ++loadGeneration;
  loading.value = true;
  errorMessage.value = "";
  archiveUsage.value = {};
  try {
    const [overviews, status] = await Promise.all([
      captureApi.listProjectOverviews(),
      captureApi.runtimeStatus(),
    ]);
    if (generation !== loadGeneration) return;
    projects.value = overviews;
    runtime.value = status;
    if (overviews.length) void loadArchiveUsage(generation);
  } catch (error) {
    if (generation === loadGeneration) {
      errorMessage.value = error instanceof Error ? error.message : String(error);
    }
  } finally {
    if (generation === loadGeneration) loading.value = false;
  }
}

function cancelArchiveUsage() {
  const requestId = activeArchiveRequestId;
  activeArchiveRequestId = null;
  if (requestId) {
    void captureApi.cancelProjectArchiveUsage(requestId).catch((error) => {
      console.warn("取消归档目录统计失败", error);
    });
  }
}

async function loadArchiveUsage(generation: number) {
  const requestId = crypto.randomUUID();
  activeArchiveRequestId = requestId;
  const apply = (usage: ProjectArchiveUsage) => {
    if (generation === loadGeneration) archiveUsage.value[usage.projectId] = usage;
  };
  try {
    const results = await captureApi.listProjectArchiveUsage(requestId, apply);
    results.forEach(apply);
  } catch (error) {
    if (generation !== loadGeneration) return;
    const message = error instanceof Error ? error.message : String(error);
    for (const project of projects.value) {
      if (!archiveUsage.value[project.projectId]) {
        apply({
          projectId: project.projectId,
          state: "error",
          mainImageCount: null,
          avatarImageCount: null,
          totalBytes: null,
          scannedAt: new Date().toISOString(),
          message,
        });
      }
    }
  } finally {
    if (activeArchiveRequestId === requestId) activeArchiveRequestId = null;
  }
}

function onProjectRenamed() {
  renameTarget.value = null;
  void initialize();
}

function onProjectDeleted() {
  const deletedId = deleteTarget.value?.projectId;
  deleteTarget.value = null;
  if (deletedId && localStorage.getItem(CURRENT_PROJECT_KEY) === deletedId) {
    localStorage.removeItem(CURRENT_PROJECT_KEY);
  }
  void initialize();
}

function setViewMode(value: ViewMode) {
  viewMode.value = value;
  localStorage.setItem(VIEW_STORAGE_KEY, value);
}

function setSortMode(mode: SortMode) {
  sortMode.value = mode;
  sortDirection.value = SORT_DIRECTION_DEFAULTS[mode];
  localStorage.setItem(SORT_STORAGE_KEY, mode);
  localStorage.setItem(`${SORT_STORAGE_KEY}-direction`, sortDirection.value);
}

function toggleSortDirection() {
  sortDirection.value = sortDirection.value === "asc" ? "desc" : "asc";
  localStorage.setItem(`${SORT_STORAGE_KEY}-direction`, sortDirection.value);
}

function onPageChange(next: number) {
  page.value = next;
}

function onPageSizeChange(size: number) {
  pageSize.value = size;
  page.value = 1;
}

watch([search, sortMode, sortDirection], () => {
  page.value = 1;
});

watch(
  () => visibleProjects.value.length,
  (length) => {
    const maxPage = Math.max(1, Math.ceil(length / pageSize.value));
    if (page.value > maxPage) page.value = maxPage;
  },
);

function selectProject(projectId: string) {
  localStorage.setItem(CURRENT_PROJECT_KEY, projectId);
}

function openProject(projectId: string) {
  selectProject(projectId);
  void router.push("/workbench");
}

async function launchGame(project: ProjectOverviewSummary) {
  try {
    const launched = await engineApi.launchGame(project.projectId);
    toast.success(`已启动游戏：${launched.gameName}`);
  } catch (error) {
    toast.error(error instanceof Error ? error.message : String(error));
  }
}

function startCapture(projectId?: string) {
  if (projectId) selectProject(projectId);
  void router.push("/capture");
}

function createProject() {
  localStorage.setItem("scene-vault.capture.create-project", "1");
  void router.push("/capture");
}

</script>

<template>
  <section class="project-home">
    <PageHeader
      eyebrow="项目库"
      title="项目中心"
      description="浏览所有项目，进入人物工作台或继续截图采集。"
    >
      <template #actions>
        <div class="runtime-pill" :class="{ idle: runtime?.workerStatus === 'idle' }">
          <span class="runtime-dot" />
          捕获服务 {{ runtime?.workerStatus === "idle" ? "待命" : "运行中" }}
        </div>
        <div class="runtime-pill" :class="{ idle: runtime?.engineStatus !== 'configured' }">
          <span class="runtime-dot engine" />
          {{ runtime?.engineStatus === "configured" ? "AI 引擎已就绪" : "AI 未配置" }}
        </div>
      </template>
    </PageHeader>

    <div v-if="errorMessage" class="home-alert" role="alert">
      <TriangleAlert :size="17" aria-hidden="true" />
      <span>{{ errorMessage }}</span>
      <button type="button" aria-label="重试加载项目" @click="initialize">
        <RefreshCw :size="16" aria-hidden="true" />
      </button>
    </div>

    <div v-if="projects.length" class="project-toolbar" aria-label="项目浏览工具">
      <label class="project-search">
        <Search :size="17" aria-hidden="true" />
        <span class="sr-only">搜索项目</span>
        <input v-model="search" type="search" placeholder="搜索项目名称…" />
      </label>
      <div class="project-toolbar-controls">
        <div class="project-sort" role="group" aria-label="项目排序">
          <label class="sort-select">
            <span>排序</span>
            <select :value="sortMode" @change="setSortMode(($event.target as HTMLSelectElement).value as SortMode)">
              <option v-for="mode in SORT_MODES" :key="mode.value" :value="mode.value">
                {{ mode.label }}
              </option>
            </select>
          </label>
          <button
            type="button"
            class="sort-direction-button"
            :aria-label="sortDirection === 'asc' ? '当前正序，点击切换为倒序' : '当前倒序，点击切换为正序'"
            :title="sortDirection === 'asc' ? '正序' : '倒序'"
            @click="toggleSortDirection"
          >
            <ArrowUp v-if="sortDirection === 'asc'" :size="16" aria-hidden="true" />
            <ArrowDown v-else :size="16" aria-hidden="true" />
          </button>
        </div>
        <div class="view-switch" role="group" aria-label="项目显示方式">
          <button
            type="button"
            aria-label="网格视图"
            :aria-pressed="viewMode === 'grid'"
            :class="{ active: viewMode === 'grid' }"
            title="网格视图"
            @click="setViewMode('grid')"
          >
            <Grid2X2 :size="18" aria-hidden="true" />
          </button>
          <button
            type="button"
            aria-label="列表视图"
            :aria-pressed="viewMode === 'list'"
            :class="{ active: viewMode === 'list' }"
            title="列表视图"
            @click="setViewMode('list')"
          >
            <List :size="18" aria-hidden="true" />
          </button>
        </div>
        <button type="button" class="new-project-button" @click="createProject">
          <Plus :size="17" aria-hidden="true" />新建项目
        </button>
      </div>
    </div>

    <div v-if="loading" class="home-state" aria-live="polite">
      <LoaderCircle class="animate-spin" :size="24" aria-hidden="true" />
      正在整理项目…
    </div>

    <div v-else-if="!projects.length" class="home-state home-empty">
      <FolderKanban :size="38" aria-hidden="true" />
      <strong>还没有项目</strong>
      <p>创建第一个项目，配置截图来源和归档目录后即可开始。</p>
      <button type="button" class="primary-action" @click="createProject">
        <Plus :size="17" aria-hidden="true" />创建项目
      </button>
    </div>

    <div v-else-if="!visibleProjects.length" class="home-state filtered-empty">
      <Search :size="32" aria-hidden="true" />
      <strong>没有匹配的项目</strong>
      <p>尝试缩短关键词或清空搜索。</p>
      <button type="button" class="secondary-action" @click="search = ''">清空搜索</button>
    </div>

    <div v-else-if="viewMode === 'grid'" class="project-grid" aria-label="项目网格">
      <ProjectHomeCard
        v-for="project in pagedProjects"
        :key="project.projectId"
        :project="project"
        :archive-usage="archiveUsage[project.projectId]"
        @open="openProject(project.projectId)"
        @capture="startCapture(project.projectId)"
        @launch="launchGame(project)"
        @rename="renameTarget = project"
        @delete="deleteTarget = project"
        @contextmenu="onProjectContext($event, project)"
      />
    </div>

    <ProjectHomeList
      v-else
      :projects="pagedProjects"
      :archive-usage="archiveUsage"
      @open="openProject($event.projectId)"
      @capture="startCapture($event.projectId)"
      @launch="launchGame"
      @rename="renameTarget = $event"
      @delete="deleteTarget = $event"
      @contextmenu="onProjectContext"
    />

    <PaginationControls
      v-if="visibleProjects.length > 0"
      :page="page"
      :page-size="pageSize"
      :total="visibleProjects.length"
      @update:page="onPageChange"
      @update:page-size="onPageSizeChange"
    />

    <footer v-if="projects.length && !loading" class="project-footer">
      <span>{{ projects.length }} 个项目</span>
      <span>全局待处理 {{ globalPendingCount }}</span>
      <span>{{ runtime?.queuedCount ?? 0 }} 排队 · {{ runtime?.archivePendingCount ?? 0 }} 待归档</span>
    </footer>

    <ProjectRenameDialog
      v-if="renameTarget"
      :project-id="renameTarget.projectId"
      :project-name="renameTarget.name"
      @close="renameTarget = null"
      @saved="onProjectRenamed"
    />
    <ProjectDeleteDialog
      v-if="deleteTarget"
      :project-id="deleteTarget.projectId"
      :project-name="deleteTarget.name"
      @close="deleteTarget = null"
      @deleted="onProjectDeleted"
    />
    <ContextMenu :menu="projectMenu" />
  </section>
</template>

<style scoped>
.project-home {
  --home-foreground: #151816;
  --home-muted: var(--muted-foreground);
  --home-count: #7d350e;
  --home-size: #24547e;
  --home-spec: #5e456f;
  --home-warn: #925b08;
  --home-info: #315fa8;
  --home-danger: #b02c29;
  --home-accent: var(--accent);
  display: flex;
  width: 100%;
  max-width: none;
  min-width: 0;
  min-height: 100%;
  box-sizing: border-box;
  flex-direction: column;
  gap: 20px;
  margin: 0;
}

:global(.dark .project-home) {
  --home-foreground: #f7fafc;
  --home-count: #67e0d0;
  --home-size: #8ed0f5;
  --home-spec: #d0bae4;
  --home-warn: #ffd073;
  --home-info: #8ed0f5;
  --home-danger: #ff8b8b;
}

.heading-actions {
  display: none;
  align-items: center;
  gap: 9px;
  margin-left: auto;
}

.runtime-pill {
  display: inline-flex;
  height: 36px;
  align-items: center;
  gap: 7px;
  padding: 0 11px;
  border: 1px solid var(--border);
  border-radius: 9px;
  background: color-mix(in srgb, var(--card) 70%, transparent);
  color: var(--foreground);
  font-size: 11.5px;
  white-space: nowrap;
}

.runtime-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--ok);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ok) 15%, transparent);
}

.runtime-pill.idle .runtime-dot { background: var(--muted-foreground); box-shadow: none; }
.runtime-pill:not(.idle) .runtime-dot.engine { background: var(--info); }

.new-project-button,
.primary-action,
.secondary-action {
  display: inline-flex;
  height: 44px;
  align-items: center;
  justify-content: center;
  gap: 7px;
  padding: 0 16px;
  border: 1px solid transparent;
  border-radius: 9px;
  font-size: 12.5px;
  font-weight: 700;
  transition: border-color .18s ease, background-color .18s ease, color .18s ease, box-shadow .18s ease;
}

.new-project-button,
.primary-action {
  background: var(--accent);
  color: var(--accent-foreground);
  box-shadow: var(--inner-highlight);
}

.new-project-button:hover,
.primary-action:hover { box-shadow: var(--glow), var(--inner-highlight); }

.project-toolbar {
  display: grid;
  width: 100%;
  grid-template-columns: minmax(240px, 1fr) auto;
  align-items: center;
  gap: var(--space-3);
}

.project-toolbar-controls {
  display: flex;
  min-width: 0;
  align-items: center;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: var(--space-3);
}

.project-search {
  display: flex;
  min-width: 0;
  height: 44px;
  align-items: center;
  gap: 9px;
  padding: 0 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: color-mix(in srgb, var(--background) 74%, transparent);
  color: var(--muted-foreground);
  transition: border-color .18s ease, box-shadow .18s ease;
}

.project-search:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 16%, transparent);
}

.project-search input {
  min-width: 0;
  flex: 1;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--foreground);
  font-size: 13px;
}

.project-sort {
  display: flex;
  flex: none;
  height: 44px;
  align-items: center;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: color-mix(in srgb, var(--card) 58%, transparent);
  box-shadow: var(--inner-highlight);
}

.sort-select {
  display: inline-flex;
  height: 100%;
  align-items: center;
  gap: var(--space-2);
  padding: 0 10px 0 12px;
  border-radius: var(--radius-control) 0 0 var(--radius-control);
  color: var(--muted-foreground);
  font-size: 12px;
  font-weight: 650;
}

.sort-select select {
  height: 34px;
  padding: 0 28px 0 8px;
  border: 0;
  background: transparent;
  color: var(--foreground);
  font-size: 12px;
  font-weight: 600;
}

.sort-select:focus-within {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.sort-select select:focus-visible { outline: none; }

.sort-direction-button {
  display: grid;
  width: 44px;
  height: 100%;
  flex: none;
  place-items: center;
  border: 0;
  border-left: 1px solid var(--border);
  border-radius: 0 var(--radius-control) var(--radius-control) 0;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
  transition: background-color .18s ease, color .18s ease;
}

.sort-direction-button:hover {
  background: color-mix(in srgb, var(--accent) 8%, transparent);
  color: var(--accent);
}

.view-switch {
  display: inline-flex;
  height: 44px;
  flex: none;
  align-items: center;
  gap: 2px;
  padding: 3px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: color-mix(in srgb, var(--card) 58%, transparent);
  box-shadow: var(--inner-highlight);
}

.view-switch button {
  display: grid;
  width: 36px;
  height: 36px;
  place-items: center;
  border: 1px solid transparent;
  border-radius: calc(var(--radius-control) - 3px);
  background: transparent;
  color: var(--muted-foreground);
  transition: border-color .18s ease, background-color .18s ease, color .18s ease;
}

.view-switch button:hover {
  border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  color: var(--accent);
}

.view-switch button.active {
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  background: color-mix(in srgb, var(--accent) 12%, var(--card));
  color: var(--accent);
  box-shadow: var(--inner-highlight);
}

.home-alert {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 11px 14px;
  border: 1px solid color-mix(in srgb, var(--destructive) 42%, var(--border));
  border-radius: 10px;
  background: color-mix(in srgb, var(--destructive) 10%, var(--card));
  color: var(--destructive);
  font-size: 13px;
}

.home-alert span { flex: 1; }
.home-alert button { display: grid; width: 36px; height: 36px; place-items: center; border: 0; background: transparent; color: inherit; }

.home-state {
  display: flex;
  min-height: 360px;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 11px;
  border: 1px dashed color-mix(in srgb, var(--border) 80%, transparent);
  border-radius: 14px;
  background: color-mix(in srgb, var(--card) 40%, transparent);
  color: var(--muted-foreground);
  text-align: center;
}

.home-state strong { color: var(--foreground); font-size: 16px; }
.home-state p { margin: 0 0 6px; font-size: 12.5px; }
.secondary-action { border-color: var(--border); background: var(--card); color: var(--foreground); }

.project-grid {
  display: grid;
  width: 100%;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 280px), 1fr));
  align-items: stretch;
  gap: 14px;
}

.project-footer {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 22px;
  padding-top: 3px;
  color: var(--muted-foreground);
  font-size: 10.5px;
}

.project-footer span + span { padding-left: 22px; border-left: 1px solid var(--border); }

@media (max-width: 1220px) {
  .runtime-pill { display: none; }
}

@media (max-width: 1120px) {
  .project-toolbar { position: sticky; z-index: 10; top: -1px; grid-template-columns: minmax(0, 1fr); padding: 8px; border: 1px solid var(--border); border-radius: 12px; background: color-mix(in srgb, var(--background) 92%, transparent); backdrop-filter: var(--panel-blur); }
  .project-sort { margin-right: auto; }
}

@media (prefers-reduced-motion: reduce) {
  .view-switch button,
  .new-project-button { transition: none; }
}
</style>
