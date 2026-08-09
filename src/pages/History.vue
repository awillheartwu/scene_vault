<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { Archive, ExternalLink, Eye, EyeOff, Filter, Image, RefreshCw, Sparkles, UserRound } from "@lucide/vue";
import CaptureThumbnail from "@/components/capture/CaptureThumbnail.vue";
import PaginationControls from "@/components/common/PaginationControls.vue";
import {
  captureApi,
  captureStatusLabel,
  pathFileName,
  revealPath,
  type CaptureSession,
  type Character,
  type CaptureHistoryEntry,
  type Project,
} from "@/lib/capture-api";

const projects = ref<Project[]>([]);
const sessions = ref<CaptureSession[]>([]);
const characters = ref<Character[]>([]);
const projectId = ref("");
const sessionId = ref("");
const status = ref("");
const characterId = ref("");
const entries = ref<CaptureHistoryEntry[]>([]);
const page = ref(1);
const pageSize = ref(100);
const total = ref(0);
const selectedId = ref<string | null>(null);
const loading = ref(false);
const errorMessage = ref("");
const showPrivate = ref(false);

const selected = computed(() => entries.value.find((entry) => entry.id === selectedId.value) ?? entries.value[0] ?? null);
const filtered = computed(() => entries.value);

async function initialize() {
  projects.value = await captureApi.listProjects();
  projectId.value = projects.value[0]?.id ?? "";
  await loadProjectFilters();
}

async function loadProjectFilters() {
  sessionId.value = "";
  characterId.value = "";
  if (!projectId.value) {
    sessions.value = [];
    characters.value = [];
    entries.value = [];
    total.value = 0;
    return;
  }
  [sessions.value, characters.value] = await Promise.all([
    captureApi.listSessions(projectId.value),
    captureApi.listCharacters(projectId.value),
  ]);
  page.value = 1;
  await load();
}

async function load() {
  if (!projectId.value) return;
  loading.value = true;
  errorMessage.value = "";
  try {
    const result = await captureApi.listHistory({
      projectId: projectId.value,
      status: status.value || null,
      sessionId: sessionId.value || null,
      characterId: characterId.value || null,
      limit: pageSize.value,
      offset: (page.value - 1) * pageSize.value,
      includePrivate: showPrivate.value,
    });
    entries.value = result.entries;
    total.value = result.total;
    selectedId.value = entries.value[0]?.id ?? null;
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error);
  } finally {
    loading.value = false;
  }
}

async function togglePrivate() {
  showPrivate.value = !showPrivate.value;
  page.value = 1;
  await load();
}

function onPageChange(next: number) {
  page.value = next;
  void load();
}

function onPageSizeChange(size: number) {
  pageSize.value = size;
  page.value = 1;
  void load();
}

async function initializeSettings() {
  try {
    const settings = await captureApi.getAppSettings();
    showPrivate.value = settings.showPrivateByDefault;
  } catch {
    // Keep the default hidden state when the backend is unavailable.
  }
}

function canReprocess(entry: CaptureHistoryEntry): boolean {
  // Degraded fallback: person capture archived raw because the Python engine
  // was not configured; a later retry re-queues it for full recognition.
  return entry.status === "completed" && entry.classification === "person" && !entry.annotatedPath;
}

async function reprocess(entry: CaptureHistoryEntry) {
  loading.value = true;
  errorMessage.value = "";
  try {
    await captureApi.retry(entry.id);
    await load();
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error);
  } finally {
    loading.value = false;
  }
}

async function reveal(path: string | null) {
  if (!path) return;
  try {
    await revealPath(path);
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error);
  }
}

watch(projectId, loadProjectFilters);
watch([sessionId, characterId, status], () => {
  page.value = 1;
  void load();
});
onMounted(async () => {
  await initializeSettings();
  await initialize();
});
</script>

<template>
  <section class="history-page">
    <header class="history-header">
      <div>
        <span class="eyebrow">Capture Archive</span>
        <h1>截图历史</h1>
        <p>按项目、角色和处理状态查看原图与归档结果。</p>
      </div>
      <button type="button" class="secondary-action" :disabled="loading" @click="load">
        <RefreshCw :size="17" :class="{ 'animate-spin': loading }" />刷新
      </button>
    </header>

    <div v-if="errorMessage" class="capture-alert" role="alert">{{ errorMessage }}</div>

    <div class="history-filters" aria-label="历史筛选">
      <Filter :size="17" />
      <label>项目<select v-model="projectId"><option v-for="project in projects" :key="project.id" :value="project.id">{{ project.name }}</option></select></label>
      <label>会话<select v-model="sessionId"><option value="">全部会话</option><option v-for="session in sessions" :key="session.id" :value="session.id">{{ new Date(session.startedAt).toLocaleString() }}</option></select></label>
      <label>状态<select v-model="status"><option value="">全部状态</option><option value="awaiting_label">等待标记</option><option value="queued">排队中</option><option value="processing">处理中</option><option value="archive_pending">等待归档</option><option value="completed">已完成</option><option value="failed">失败</option></select></label>
      <label>角色<select v-model="characterId"><option value="">全部角色</option><option v-for="character in characters" :key="character.id" :value="character.id">{{ character.name }}</option></select></label>
      <button type="button" class="private-toggle" :class="{ on: showPrivate }" @click="togglePrivate">
        <EyeOff v-if="!showPrivate" :size="15" /><Eye v-else :size="15" />
        {{ showPrivate ? "收藏图可见" : "收藏图已隐藏" }}
      </button>
      <span>{{ total }} 条记录</span>
    </div>

    <PaginationControls
      v-if="total > 0"
      :page="page"
      :page-size="pageSize"
      :total="total"
      @update:page="onPageChange"
      @update:page-size="onPageSizeChange"
    />

    <div class="history-workspace">
      <div class="history-list" role="list" aria-label="截图历史记录">
        <button
          v-for="entry in filtered"
          :key="entry.id"
          type="button"
          role="listitem"
          class="history-row"
          :class="{ selected: selected?.id === entry.id }"
          @click="selectedId = entry.id"
        >
          <div class="history-thumb"><CaptureThumbnail :item="entry" /></div>
          <div class="history-main">
            <strong>{{ pathFileName(entry.sourcePath) }}</strong>
            <span>{{ new Date(entry.capturedAt).toLocaleString() }} · {{ entry.projectName }}</span>
          </div>
          <div class="history-character"><UserRound :size="15" />{{ entry.characterName || "未标记" }}</div>
          <span class="status-pill" :data-status="entry.status">{{ captureStatusLabel(entry.status, entry.failureStage) }}</span>
        </button>
        <div v-if="!filtered.length && !loading" class="history-empty">没有符合筛选条件的截图。</div>
      </div>

      <aside v-if="selected" class="history-detail">
        <span class="eyebrow">记录详情</span>
        <h2>{{ pathFileName(selected.sourcePath) }}</h2>
        <div class="detail-preview">
          <CaptureThumbnail
            :item="selected"
            :variant="selected.destinationPath ? 'destination' : 'source'"
            size="full"
          />
        </div>

        <dl>
          <div><dt>状态</dt><dd>{{ captureStatusLabel(selected.status, selected.failureStage) }}</dd></div>
          <div><dt>角色</dt><dd>{{ selected.characterName || "未标记" }}</dd></div>
          <div><dt>会话</dt><dd>{{ selected.sessionStatus }}</dd></div>
          <div><dt>捕获时间</dt><dd>{{ new Date(selected.capturedAt).toLocaleString() }}</dd></div>
          <div v-if="selected.processingWarningsJson !== '[]'"><dt>处理提示</dt><dd>{{ selected.processingWarningsJson }}</dd></div>
          <div v-if="selected.errorMessage" class="detail-error"><dt>错误</dt><dd>{{ selected.errorMessage }}</dd></div>
        </dl>

        <div class="detail-paths">
          <button type="button" @click="reveal(selected.sourcePath)"><Image :size="16" />显示原图<ExternalLink :size="13" /></button>
          <button v-if="selected.destinationPath" type="button" @click="reveal(selected.destinationPath)"><Archive :size="16" />显示归档图<ExternalLink :size="13" /></button>
          <button v-if="selected.destinationAvatarPath" type="button" @click="reveal(selected.destinationAvatarPath)"><UserRound :size="16" />显示头像<ExternalLink :size="13" /></button>
          <button v-if="canReprocess(selected)" type="button" :disabled="loading" @click="reprocess(selected)"><Sparkles :size="16" />重新识别<ExternalLink :size="13" /></button>
        </div>
      </aside>
      <aside v-else class="history-detail empty">选择一条历史记录查看详情。</aside>
    </div>
  </section>
</template>
