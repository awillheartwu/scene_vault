<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import {
  ArrowDown,
  ArrowUp,
  ArrowRight,
  Camera,
  CheckCircle2,
  CircleAlert,
  FolderKanban,
  Grid2X2,
  List,
  LoaderCircle,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  TriangleAlert,
} from "@lucide/vue";
import CaptureThumbnail from "@/components/capture/CaptureThumbnail.vue";
import PaginationControls from "@/components/common/PaginationControls.vue";
import PageHeader from "@/components/layout/PageHeader.vue";
import { ContextMenu } from "@/components/ui/context-menu";
import ProjectDeleteDialog from "@/components/project/ProjectDeleteDialog.vue";
import ProjectRenameDialog from "@/components/project/ProjectRenameDialog.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  captureApi,
  type CaptureItem,
  type CaptureRuntimeStatus,
  type ProjectOverviewSummary,
} from "@/lib/capture-api";

type ViewMode = "grid" | "list";
type SortMode = "recent" | "attention" | "name" | "captures" | "created";
type SortDirection = "asc" | "desc";

const VIEW_STORAGE_KEY = "scene-vault.home.project-view";
const SORT_STORAGE_KEY = "scene-vault.home.sort";
const CURRENT_PROJECT_KEY = "scene-vault.capture.project";

const router = useRouter();
const projects = ref<ProjectOverviewSummary[]>([]);
const runtime = ref<CaptureRuntimeStatus | null>(null);
const loading = ref(true);
const errorMessage = ref("");
const search = ref("");
const SORT_MODES: { value: SortMode; label: string }[] = [
  { value: "recent", label: "最近活跃" },
  { value: "attention", label: "待处理" },
  { value: "name", label: "名称" },
  { value: "captures", label: "图片数" },
  { value: "created", label: "创建时间" },
];
const SORT_DIRECTION_DEFAULTS: Record<SortMode, SortDirection> = {
  recent: "desc",
  attention: "desc",
  name: "asc",
  captures: "desc",
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
    } else if (sortMode.value === "captures") {
      primary = left.captureCount - right.captureCount;
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

async function initialize() {
  loading.value = true;
  errorMessage.value = "";
  try {
    [projects.value, runtime.value] = await Promise.all([
      captureApi.listProjectOverviews(),
      captureApi.runtimeStatus(),
    ]);
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error);
  } finally {
    loading.value = false;
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

function startCapture(projectId?: string) {
  if (projectId) selectProject(projectId);
  void router.push("/capture");
}

function createProject() {
  localStorage.setItem("scene-vault.capture.create-project", "1");
  void router.push("/capture");
}

function thumbnailItem(id: string): CaptureItem {
  return { id, sourcePath: "capture.jpg" } as CaptureItem;
}

function formatActivity(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "尚无活动";
  return date.toLocaleString("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function setupLabel(project: ProjectOverviewSummary): string {
  if (!project.sourceCount && !project.destinationConfigured) return "尚未配置目录";
  if (!project.sourceCount) return "缺少截图来源";
  if (!project.destinationConfigured) return "缺少归档目录";
  return `${project.sourceCount} 个截图来源`;
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
          <List :size="19" aria-hidden="true" />
        </button>
      </div>
      <button type="button" class="new-project-button" @click="createProject">
        <Plus :size="17" aria-hidden="true" />新建项目
      </button>
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
      <article
        v-for="project in pagedProjects"
        :key="project.projectId"
        class="project-card"
        @contextmenu="onProjectContext($event, project)"
      >
        <button
          type="button"
          class="project-card-main"
          data-context-allow
          @click="openProject(project.projectId)"
        >
          <div class="project-cover">
            <CaptureThumbnail
              v-if="project.coverCaptureItemId || project.latestCaptureItemId"
              :item="thumbnailItem((project.coverCaptureItemId ?? project.latestCaptureItemId)!)"
              variant="source"
              fallback-variant="destination"
              :alt="`${project.name} ${project.coverCaptureItemId ? '项目封面' : '最近截图'}`"
            />
            <div v-else class="project-cover-empty">
              <FolderKanban :size="32" aria-hidden="true" />
              <span>等待第一张截图</span>
            </div>
            <span v-if="project.activeSessionCount" class="active-session-badge">
              <span />正在捕获
            </span>
          </div>
          <div class="project-card-body">
            <div class="project-title-row">
              <div>
                <h2>{{ project.name }}</h2>
                <p>最近活动 · {{ formatActivity(project.lastActivityAt) }}</p>
              </div>
              <ArrowRight :size="18" aria-hidden="true" />
            </div>
            <p v-if="project.description" class="project-description">{{ project.description }}</p>
            <div class="project-statuses">
              <span v-if="project.awaitingCount" class="status pending">
                <span />待分类 {{ project.awaitingCount }}
              </span>
              <span v-if="project.processingCount" class="status processing">
                <span />处理中 {{ project.processingCount }}
              </span>
              <span v-if="project.failedCount" class="status failed">
                <span />失败 {{ project.failedCount }}
              </span>
              <span
                v-if="project.captureCount && !project.awaitingCount && !project.processingCount && !project.failedCount"
                class="status completed"
              >
                <CheckCircle2 :size="14" aria-hidden="true" />全部已归档
              </span>
              <span v-if="!project.captureCount" class="status muted">尚无截图</span>
            </div>
            <div class="project-config" :class="{ warning: !project.sourceCount || !project.destinationConfigured }">
              <CircleAlert v-if="!project.sourceCount || !project.destinationConfigured" :size="14" aria-hidden="true" />
              <CheckCircle2 v-else :size="14" aria-hidden="true" />
              {{ setupLabel(project) }}
            </div>
          </div>
        </button>
        <footer class="project-card-footer">
          <button
            type="button"
            class="capture-project-button"
            :aria-label="`继续捕获 ${project.name}`"
            title="继续捕获"
            @click="startCapture(project.projectId)"
          >
            <Camera :size="17" aria-hidden="true" />继续捕获
          </button>
          <DropdownMenu>
            <DropdownMenuTrigger as-child>
              <button
                type="button"
                class="card-action-button"
                :aria-label="`${project.name} 更多操作`"
                title="更多操作"
              >
                <MoreHorizontal :size="18" aria-hidden="true" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" class="project-action-menu">
              <DropdownMenuItem @select="renameTarget = project">
                <Pencil :size="15" aria-hidden="true" />重命名
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem class="project-menu-danger" @select="deleteTarget = project">
                <Trash2 :size="15" aria-hidden="true" />删除项目
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </footer>
      </article>
    </div>

    <div v-else class="project-list" aria-label="项目列表">
      <div class="project-list-head" aria-hidden="true">
        <span>项目与状态</span><span>最近活动</span>
      </div>
      <article
        v-for="project in pagedProjects"
        :key="project.projectId"
        class="project-row"
        @contextmenu="onProjectContext($event, project)"
      >
        <div class="row-project">
          <div class="row-cover">
            <CaptureThumbnail
              v-if="project.coverCaptureItemId || project.latestCaptureItemId"
              :item="thumbnailItem((project.coverCaptureItemId ?? project.latestCaptureItemId)!)"
              variant="source"
              fallback-variant="destination"
              :alt="`${project.name} ${project.coverCaptureItemId ? '项目封面' : '最近截图'}`"
            />
            <FolderKanban v-else :size="23" aria-hidden="true" />
          </div>
          <div class="row-project-copy">
            <strong>{{ project.name }}</strong>
            <span>{{ setupLabel(project) }}</span>
            <div class="row-counts">
              <span class="pending"><i />{{ project.awaitingCount }} 待分类</span>
              <span class="processing"><i />{{ project.processingCount }} 处理中</span>
              <span v-if="project.failedCount" class="failed"><i />{{ project.failedCount }} 失败</span>
              <span class="row-session" :class="{ active: project.activeSessionCount }">
                {{ project.activeSessionCount ? "捕获中" : `${project.sessionCount} 个会话` }}
              </span>
            </div>
          </div>
          <div class="row-actions">
            <button
              type="button"
              :aria-label="`继续捕获 ${project.name}`"
              title="继续捕获"
              @click="startCapture(project.projectId)"
            >
              <Camera :size="17" aria-hidden="true" />
            </button>
            <button
              type="button"
              class="row-action-button"
              :aria-label="`重命名 ${project.name}`"
              title="重命名"
              @click="renameTarget = project"
            >
              <Pencil :size="16" aria-hidden="true" />
            </button>
            <button
              type="button"
              class="row-action-button row-action-danger"
              :aria-label="`删除 ${project.name}`"
              title="删除项目"
              @click="deleteTarget = project"
            >
              <Trash2 :size="16" aria-hidden="true" />
            </button>
            <button type="button" class="open-row-button" @click="openProject(project.projectId)">
              打开<ArrowRight :size="16" aria-hidden="true" />
            </button>
          </div>
        </div>
        <time :datetime="project.lastActivityAt">{{ formatActivity(project.lastActivityAt) }}</time>
      </article>
    </div>

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
.secondary-action,
.enter-project-button,
.open-row-button {
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
.primary-action,
.enter-project-button {
  background: var(--accent);
  color: var(--accent-foreground);
  box-shadow: var(--inner-highlight);
}

.new-project-button:hover,
.primary-action:hover,
.enter-project-button:hover { box-shadow: var(--glow), var(--inner-highlight); }

.project-toolbar {
  display: flex;
  width: 100%;
  align-items: center;
  justify-content: flex-start;
  flex-wrap: wrap;
  gap: 10px;
}

.project-search {
  display: flex;
  min-width: 240px;
  max-width: 560px;
  flex: 1 1 360px;
  height: 44px;
  align-items: center;
  gap: 9px;
  padding: 0 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
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

.sort-select {
  display: inline-flex;
  height: 44px;
  align-items: center;
  gap: 8px;
  padding: 0 10px 0 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: color-mix(in srgb, var(--card) 58%, transparent);
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

.sort-direction-button {
  display: grid;
  width: 44px;
  height: 44px;
  place-items: center;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: color-mix(in srgb, var(--card) 58%, transparent);
  color: var(--muted-foreground);
  cursor: pointer;
  transition: border-color 0.18s ease, color 0.18s ease;
}

.sort-direction-button:hover {
  border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  color: var(--accent);
}

.view-switch {
  display: inline-flex;
  gap: 5px;
}

.view-switch button,
.capture-project-button,
.card-action-button,
.row-action-button,
.row-actions > button:first-child {
  display: grid;
  width: 44px;
  height: 44px;
  place-items: center;
  border: 1px solid var(--border);
  border-radius: 9px;
  background: color-mix(in srgb, var(--card) 58%, transparent);
  color: var(--muted-foreground);
  transition: border-color .18s ease, background-color .18s ease, color .18s ease, box-shadow .18s ease;
}

.view-switch button:hover,
.capture-project-button:hover,
.card-action-button:hover,
.row-action-button:hover,
.row-actions > button:first-child:hover {
  border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  color: var(--accent);
}

.card-action-danger:hover,
.row-action-danger:hover {
  border-color: color-mix(in srgb, var(--danger) 55%, var(--border));
  color: var(--danger);
}

.view-switch button.active {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, var(--card));
  color: var(--accent);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 15%, transparent), var(--inner-highlight);
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
  grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  gap: 18px;
}

.project-card {
  min-width: 0;
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--border) 78%, transparent);
  border-radius: 14px;
  background: var(--card);
  box-shadow: var(--card-shadow), var(--inner-highlight);
  transition: border-color .2s ease, box-shadow .2s ease;
}

.project-card:hover {
  border-color: color-mix(in srgb, var(--accent) 42%, var(--border));
  box-shadow: var(--card-shadow-hover), var(--inner-highlight);
}

.project-card-main {
  display: block;
  width: 100%;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  text-align: left;
}

.project-cover {
  position: relative;
  height: 158px;
  overflow: hidden;
  border-bottom: 1px solid var(--border);
  background: var(--background);
}

.project-cover :deep(img) { transition: transform .28s ease, filter .28s ease; }
.project-card:hover .project-cover :deep(img) { transform: scale(1.025); filter: brightness(1.04); }

.project-cover-empty {
  display: flex;
  height: 100%;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--muted-foreground);
  font-size: 11.5px;
}

.active-session-badge {
  position: absolute;
  z-index: 1;
  top: 12px;
  left: 12px;
  display: inline-flex;
  height: 26px;
  align-items: center;
  gap: 7px;
  padding: 0 10px;
  border-radius: 99px;
  background: color-mix(in srgb, var(--accent) 92%, black 8%);
  color: var(--accent-foreground);
  font-size: 10.5px;
  font-weight: 700;
  box-shadow: 0 4px 16px rgba(0, 0, 0, .28);
}

.active-session-badge span { width: 6px; height: 6px; border-radius: 50%; background: currentColor; }

.project-card-body { min-height: 136px; padding: 15px 16px 13px; }
.project-title-row { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }
.project-title-row svg { flex: none; margin-top: 4px; color: var(--muted-foreground); transition: color .18s ease, transform .18s ease; }
.project-card:hover .project-title-row svg { color: var(--accent); transform: translateX(2px); }
.project-title-row h2 { margin: 0; font-size: 17px; font-weight: 750; letter-spacing: -.015em; }
.project-title-row p { margin: 4px 0 0; color: var(--muted-foreground); font-size: 11.5px; }
.project-description { overflow: hidden; margin: 10px 0 0; color: var(--muted-foreground); font-size: 11.5px; text-overflow: ellipsis; white-space: nowrap; }

.project-statuses { display: flex; min-height: 25px; flex-wrap: wrap; align-items: center; gap: 11px; margin-top: 14px; }
.status { display: inline-flex; align-items: center; gap: 6px; color: var(--muted-foreground); font-size: 11.5px; }
.status > span, .row-counts i { width: 7px; height: 7px; border-radius: 50%; background: currentColor; box-shadow: 0 0 0 3px color-mix(in srgb, currentColor 14%, transparent); }
.status.pending, .row-counts .pending { color: var(--warn); }
.status.processing, .row-counts .processing { color: var(--info); }
.status.failed, .row-counts .failed { color: var(--destructive); }
.status.completed { color: var(--ok); }

.project-config {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 11px;
  color: var(--muted-foreground);
  font-size: 10.5px;
}

.project-config:not(.warning) { color: var(--ok); }
.project-config.warning { color: var(--warn); }

.project-card-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  padding: 12px 16px 15px;
  border-top: 1px solid color-mix(in srgb, var(--border) 72%, transparent);
}

.capture-project-button { display: inline-flex; width: auto; padding: 0 14px; gap: 7px; color: var(--foreground); font-weight: 650; }
.project-action-menu [role="menuitem"] { display: flex; align-items: center; gap: 8px; }
.project-menu-danger { color: var(--destructive); }

.project-list {
  width: 100%;
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--border) 78%, transparent);
  border-radius: 13px;
  background: color-mix(in srgb, var(--card) 72%, transparent);
}

.project-list-head,
.project-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 120px;
  align-items: center;
}

.project-list-head {
  min-height: 42px;
  padding: 0 14px;
  border-bottom: 1px solid var(--border);
  color: var(--muted-foreground);
  font-size: 10.5px;
  font-weight: 650;
  letter-spacing: .04em;
  text-transform: uppercase;
}

.project-row { min-height: 88px; border-bottom: 1px solid color-mix(in srgb, var(--border) 75%, transparent); }
.project-row:last-child { border-bottom: 0; }
.project-row:hover { background: color-mix(in srgb, var(--accent) 4%, var(--card)); }

.row-project { display: grid; min-width: 0; grid-template-columns: 90px minmax(0, 1fr) auto; align-items: center; gap: 13px; padding: 9px 0 9px 14px; }
.row-cover { display: grid; width: 90px; height: 62px; flex: none; overflow: hidden; place-items: center; border: 1px solid var(--border); border-radius: 8px; background: var(--background); color: var(--muted-foreground); }
.row-project-copy { min-width: 0; }
.row-project-copy > strong, .row-project-copy > span { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.row-project-copy > strong { font-size: 13.5px; }
.row-project-copy > span { margin-top: 5px; color: var(--muted-foreground); font-size: 10.5px; }
.project-row time { padding-left: 10px; color: var(--muted-foreground); font-size: 11px; font-variant-numeric: tabular-nums; }

.row-counts { display: flex; flex-wrap: wrap; align-items: center; gap: 5px 13px; margin-top: 7px; }
.row-counts span { display: inline-flex; align-items: center; gap: 6px; color: var(--muted-foreground); font-size: 10.5px; white-space: nowrap; }
.row-counts i { display: block; }

.row-session { display: inline-flex; width: fit-content; align-items: center; gap: 6px; color: var(--muted-foreground); font-size: 11px; }
.row-session.active { color: var(--accent); font-weight: 650; }
.row-session.active::before { width: 7px; height: 7px; border-radius: 50%; background: currentColor; content: ""; }

.row-actions { display: flex; align-items: center; justify-content: flex-end; gap: 8px; }
.open-row-button { border-color: var(--border); background: transparent; color: var(--accent); }
.open-row-button:hover { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 8%, transparent); }

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
  .project-list-head,
  .project-row { grid-template-columns: minmax(0, 1fr) 105px; }
}

@media (max-width: 1120px) {
  .project-grid { grid-template-columns: 1fr; }
  .project-toolbar { position: sticky; z-index: 10; top: -1px; padding: 8px; border: 1px solid var(--border); border-radius: 12px; background: color-mix(in srgb, var(--background) 92%, transparent); backdrop-filter: var(--panel-blur); }
  .project-search { max-width: none; flex-basis: calc(100% - 320px); }
  .project-cover { height: 132px; }
}

@media (prefers-reduced-motion: reduce) {
  .project-card,
  .project-cover :deep(img),
  .project-title-row svg,
  .view-switch button,
  .new-project-button { transition: none; }
}
</style>
