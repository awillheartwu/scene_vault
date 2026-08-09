<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  AlertCircle,
  Archive,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  CircleStop,
  FolderOpen,
  FolderPlus,
  FolderInput,
  Image,
  ImageOff,
  Lock,
  LoaderCircle,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Search,
  Sparkles,
  Trash2,
  UserRound,
  Wifi,
} from "@lucide/vue";
import CaptureThumbnail from "@/components/capture/CaptureThumbnail.vue";
import CaptureProgress from "@/components/capture/CaptureProgress.vue";
import ResponsiveDetailPanel from "@/components/layout/ResponsiveDetailPanel.vue";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import ProjectRenameDialog from "@/components/project/ProjectRenameDialog.vue";
import {
  captureApi,
  captureClassificationLabel,
  captureStatusLabel,
  pathFileName,
  pathMimeType,
  pickDirectory,
  type CaptureItem,
  type CaptureClassification,
  type CaptureRuntimeStatus,
  type CaptureSession,
  type Character,
  type Project,
  type ProjectSourceDirectory,
  type UnimportedCapture,
  type VerificationResult,
} from "@/lib/capture-api";
import { toast } from "@/lib/toast";

const projects = ref<Project[]>([]);
const characters = ref<Character[]>([]);
const sessions = ref<CaptureSession[]>([]);
const items = ref<CaptureItem[]>([]);
const runtime = ref<CaptureRuntimeStatus | null>(null);
const projectId = ref("");
const sourceDirs = ref<ProjectSourceDirectory[]>([]);
const projectDestination = ref("");
const selectedItemId = ref<string | null>(null);
const selectedCharacterId = ref<string | null>(null);
const pendingClassification = ref<CaptureClassification | null>(null);
const verificationWarning = ref<VerificationResult | null>(null);
const search = ref("");
const previewUrl = ref<string | null>(null);
const loading = ref(true);
const busy = ref(false);
const importCandidates = ref<UnimportedCapture[] | null>(null);
const importBusy = ref(false);
const deferredImportCount = ref(0);
const showProjectForm = ref(false);
const renameTarget = ref<Project | null>(null);
const labelPanelOpen = ref(false);
const sessionToolsOpen = ref(true);
const showCharacterForm = ref(false);
const newProjectName = ref("");
const newCharacterName = ref("");
const unlisteners: UnlistenFn[] = [];
const stripRow = ref<HTMLElement | null>(null);
const stripBar = ref<HTMLElement | null>(null);
const stripThumb = ref<HTMLElement | null>(null);
const stripDragging = ref(false);
const stripThumbWidth = ref(0);
const stripThumbOffset = ref(0);
let stripObserver: ResizeObserver | null = null;

const activeSession = computed(() => sessions.value.find((session) => session.status === "active") ?? null);
const selectedItem = computed(() => items.value.find((item) => item.id === selectedItemId.value) ?? items.value[0] ?? null);
const selectedCharacter = computed(() => characters.value.find((character) => character.id === selectedCharacterId.value) ?? null);
// The recent strip is a working queue: awaiting-label first (oldest first),
// then in-flight, then failed. Completed captures leave the strip entirely;
// they live in the History page.
const stripItems = computed(() => {
  const priority: Record<string, number> = {
    awaiting_label: 0,
    queued: 1,
    processing: 1,
    archive_pending: 1,
    failed: 2,
  };
  return items.value
    .filter((item) => item.status !== "completed")
    .sort((a, b) => {
      const pa = priority[a.status] ?? 3;
      const pb = priority[b.status] ?? 3;
      if (pa !== pb) return pa - pb;
      return new Date(a.capturedAt).getTime() - new Date(b.capturedAt).getTime();
    });
});
const suggestedCharacter = computed(() => {
  const item = selectedItem.value;
  if (!item || item.reviewStatus !== "pending" || !item.suggestedCharacterId) return null;
  return characters.value.find((character) => character.id === item.suggestedCharacterId) ?? null;
});

function characterName(id: string | null): string {
  if (!id) return "其他角色";
  return characters.value.find((character) => character.id === id)?.name ?? "其他角色";
}
const waitingCount = computed(() => items.value.filter((item) => item.status === "awaiting_label").length);
const filteredCharacters = computed(() => {
  const query = search.value.trim().toLocaleLowerCase();
  return query
    ? characters.value.filter((character) => character.name.toLocaleLowerCase().includes(query))
    : characters.value;
});
const recentCharacters = computed(() => {
  const ids: string[] = [];
  for (const item of items.value) {
    if (item.characterId && !ids.includes(item.characterId)) ids.push(item.characterId);
  }
  const recent = ids
    .map((id) => characters.value.find((character) => character.id === id))
    .filter((character): character is Character => Boolean(character));
  for (const character of characters.value) {
    if (recent.length >= 5) break;
    if (!recent.some((value) => value.id === character.id)) recent.push(character);
  }
  return recent.slice(0, 5);
});
// Long names shrink so the compact "recent" rows stay one line without
// wrapping; short names keep the readable size.
function recentNameFont(name: string): string {
  const length = name.length;
  if (length >= 10) return "10px";
  if (length >= 6) return "11px";
  return "12px";
}
const canSubmit = computed(
  () =>
    selectedItem.value?.status === "awaiting_label" &&
    pendingClassification.value === "person" &&
    Boolean(selectedCharacterId.value) &&
    !busy.value,
);

async function initialize() {
  loading.value = true;
  try {
    projects.value = await captureApi.listProjects();
    const savedProject = localStorage.getItem("scene-vault.capture.project");
    projectId.value = projects.value.some((project) => project.id === savedProject)
      ? savedProject!
      : projects.value[0]?.id ?? "";
    if (localStorage.getItem("scene-vault.capture.create-project") === "1") {
      showProjectForm.value = true;
      localStorage.removeItem("scene-vault.capture.create-project");
    }
    runtime.value = await captureApi.runtimeStatus();
    if (projectId.value) await loadProject();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    loading.value = false;
  }
}

async function loadProject() {
  if (!projectId.value) return;
  localStorage.setItem("scene-vault.capture.project", projectId.value);
  [characters.value, sessions.value, sourceDirs.value] = await Promise.all([
    captureApi.listCharacters(projectId.value),
    captureApi.listSessions(projectId.value),
    captureApi.listSourceDirectories(projectId.value),
  ]);
  const project = projects.value.find((value) => value.id === projectId.value);
  projectDestination.value = project?.destinationDirectory ?? "";
  items.value = await captureApi.listProjectRecentCaptures(projectId.value, 100);
  deferredImportCount.value = activeSession.value
    ? await captureApi.deferredImportRecognitionCount(activeSession.value.id)
    : 0;
  selectBestItem();
}

function selectBestItem() {
  if (selectedItemId.value && items.value.some((item) => item.id === selectedItemId.value)) return;
  selectedItemId.value =
    items.value.find((item) => item.status === "awaiting_label")?.id ?? items.value[0]?.id ?? null;
  pendingClassification.value = null;
}

async function createProject() {
  const name = newProjectName.value.trim();
  if (!name) return;
  await runBusy(async () => {
    const project = await captureApi.createProject(name);
    projects.value.unshift(project);
    projectId.value = project.id;
    newProjectName.value = "";
    showProjectForm.value = false;
    await loadProject();
  });
}

async function createCharacter() {
  const name = newCharacterName.value.trim();
  if (!name || !projectId.value) return;
  await runBusy(async () => {
    const character = await captureApi.createCharacter(projectId.value, name);
    characters.value.push(character);
    selectedCharacterId.value = character.id;
    newCharacterName.value = "";
    showCharacterForm.value = false;
  });
}

async function addSourceDir() {
  if (!projectId.value) {
    toast.error("请先选择项目。");
    return;
  }
  let path: string | null = null;
  try {
    path = await pickDirectory();
  } catch (error) {
    toast.error(`无法打开目录选择器：${normalizeError(error)}`);
    return;
  }
  if (!path) return;
  await runBusy(async () => {
    sourceDirs.value = await captureApi.listSourceDirectories(projectId.value);
    try {
      await captureApi.addSourceDirectory(projectId.value, path);
    } catch (error) {
      sourceDirs.value = await captureApi.listSourceDirectories(projectId.value);
      throw error;
    }
    sourceDirs.value = await captureApi.listSourceDirectories(projectId.value);
    toast.success("已添加截图目录，下次开始会话时生效。");
  });
}

async function removeSourceDir(dir: ProjectSourceDirectory) {
  await runBusy(async () => {
    await captureApi.removeSourceDirectory(dir.id);
    sourceDirs.value = await captureApi.listSourceDirectories(projectId.value);
  });
}

async function toggleSourceDir(dir: ProjectSourceDirectory) {
  await runBusy(async () => {
    await captureApi.setSourceDirectoryEnabled(dir.id, !dir.enabled);
    sourceDirs.value = await captureApi.listSourceDirectories(projectId.value);
  });
}

async function chooseDestination() {
  if (!projectId.value) return;
  const path = await pickDirectory();
  if (!path) return;
  await runBusy(async () => {
    const project = await captureApi.setProjectDestination(projectId.value, path);
    projectDestination.value = project.destinationDirectory ?? "";
    const index = projects.value.findIndex((value) => value.id === project.id);
    if (index !== -1) projects.value[index] = project;
    toast.success("归档目录已设置。");
  });
}

async function startSession() {
  if (!projectId.value) {
    toast.error("请先选择项目。");
    return;
  }
  if (!sourceDirs.value.some((dir) => dir.enabled)) {
    toast.error("请先添加至少一个截图来源目录。");
    return;
  }
  if (!projectDestination.value) {
    toast.error("请先配置归档目录。");
    return;
  }
  await runBusy(async () => {
    const result = await captureApi.startSession(projectId.value);
    sessions.value.unshift(result.session);
    items.value = [];
    deferredImportCount.value = 0;
    if (result.stoppedProjects.length > 0) {
      toast.info(`已开始监听，并自动停止了 ${result.stoppedProjects.join("、")} 的监听`);
    } else {
      toast.success("已开始监听");
    }
  });
}

async function openImportDialog() {
  const session = activeSession.value;
  if (!session) return;
  importBusy.value = true;
  try {
    importCandidates.value = await captureApi.listUnimportedCaptures(session.id);
  } catch (error) {
    toast.error(error instanceof Error ? error.message : String(error));
  } finally {
    importBusy.value = false;
  }
}

function onProjectRenamed(name: string) {
  if (renameTarget.value) {
    projects.value = projects.value.map((project) =>
      project.id === renameTarget.value!.id ? { ...project, name } : project,
    );
  }
  renameTarget.value = null;
}

async function confirmImport() {
  const session = activeSession.value;
  const candidates = importCandidates.value;
  if (!session || !candidates) return;
  importBusy.value = true;
  try {
    const count = await captureApi.importDirectoryCaptures(
      session.id,
      candidates.map((candidate) => candidate.path),
    );
    importCandidates.value = null;
    deferredImportCount.value = await captureApi.deferredImportRecognitionCount(session.id);
    toast.success(`已登记 ${count} 张截图；尚未生成缩略图或开始识别。`);
  } catch (error) {
    toast.error(error instanceof Error ? error.message : String(error));
  } finally {
    importBusy.value = false;
  }
}

async function startImportedRecognition() {
  const session = activeSession.value;
  if (!session || importBusy.value || deferredImportCount.value === 0) return;
  importBusy.value = true;
  try {
    const count = await captureApi.startImportedRecognition(session.id);
    deferredImportCount.value = 0;
    toast.success(`已开始逐张识别 ${count} 张导入截图。`);
  } catch (error) {
    toast.error(error instanceof Error ? error.message : String(error));
  } finally {
    importBusy.value = false;
  }
}

async function stopSession() {
  if (!activeSession.value) return;
  await runBusy(async () => {
    const ended = await captureApi.endSession(activeSession.value!.id);
    sessions.value = sessions.value.map((session) => (session.id === ended.id ? ended : session));
    toast.success("已停止监听");
  });
}

async function submitLabel() {
  if (!selectedItem.value || !selectedCharacterId.value || !canSubmit.value) return;
  const itemId = selectedItem.value.id;
  const characterId = selectedCharacterId.value;
  verificationWarning.value = null;
  await runBusy(async () => {
    const result = await captureApi.verifyCaptureIdentity({
      captureItemId: itemId,
      characterId,
    });
    if (result.level === "ok" || result.level === "unverified") {
      await doLabel(itemId, characterId);
    } else {
      verificationWarning.value = result;
    }
  });
}

async function confirmForcedLabel() {
  if (!selectedItem.value || !selectedCharacterId.value) return;
  const itemId = selectedItem.value.id;
  const characterId = selectedCharacterId.value;
  verificationWarning.value = null;
  await runBusy(async () => doLabel(itemId, characterId));
}

async function doLabel(itemId: string, characterId: string) {
  const updated = await captureApi.label(itemId, characterId, "person");
  upsertItem(updated);
  selectedCharacterId.value = null;
  pendingClassification.value = null;
  labelPanelOpen.value = false;
  selectNextWaiting(itemId);
}

async function submitClassification(classification: CaptureClassification) {
  if (!selectedItem.value || classification === "person") {
    pendingClassification.value = "person";
    labelPanelOpen.value = true;
    void refreshSuggestion();
    return;
  }
  const itemId = selectedItem.value.id;
  await runBusy(async () => {
    const updated = await captureApi.label(itemId, null, classification);
    upsertItem(updated);
    pendingClassification.value = null;
    selectNextWaiting(itemId);
  });
}

let suggestionRequestedFor: string | null = null;

// Re-runs the face-bank comparison with the freshest samples whenever the
// user is about to label a person capture, so a suggestion uses samples that
// may have been enrolled earlier in the same batch.
async function refreshSuggestion() {
  const item = selectedItem.value;
  if (!item || !item.faceCount || suggestionRequestedFor === item.id) return;
  suggestionRequestedFor = item.id;
  try {
    upsertItem(await captureApi.suggestForCapture(item.id));
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    if (suggestionRequestedFor === item.id) suggestionRequestedFor = null;
  }
}

async function retryItem(item: CaptureItem) {
  await runBusy(async () => upsertItem(await captureApi.retry(item.id)));
}

async function runBusy(action: () => Promise<void>) {
  busy.value = true;
  try {
    await action();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    busy.value = false;
  }
}

function upsertItem(item: CaptureItem) {
  const index = items.value.findIndex((value) => value.id === item.id);
  if (index === -1) items.value.unshift(item);
  else items.value[index] = item;
  items.value = [...items.value].sort((left, right) => right.capturedAt.localeCompare(left.capturedAt));
  if (!selectedItemId.value || item.status === "awaiting_label") selectedItemId.value = item.id;
}

function selectNextWaiting(afterId?: string) {
  const waiting = items.value.filter((item) => item.status === "awaiting_label");
  if (!waiting.length) return;
  const index = afterId ? waiting.findIndex((item) => item.id === afterId) : -1;
  selectedItemId.value = waiting[(index + 1 + waiting.length) % waiting.length].id;
  pendingClassification.value = null;
}

// Vertical wheel over the thumbnail strip scrolls it horizontally; the
// strip rarely has a visible overflow indicator while playing a game.
function onStripWheel(event: WheelEvent) {
  const target = event.currentTarget as HTMLElement;
  if (Math.abs(event.deltaY) > Math.abs(event.deltaX)) {
    target.scrollLeft += event.deltaY;
  }
}

// The native overlay scrollbar is effectively invisible in WebView2, so the
// strip gets a visible custom bar whose thumb mirrors scrollLeft. The row
// keeps native overflow for keyboard/wheel scrolling.
function syncStripBar() {
  const row = stripRow.value;
  const bar = stripBar.value;
  if (!row) return;
  const max = Math.max(0, row.scrollWidth - row.clientWidth);
  const track = Math.max(1, bar?.clientWidth || row.clientWidth);
  // The bar is always rendered; when there is no overflow the thumb simply
  // fills the whole track so the strip still looks like a (disabled) track.
  if (max <= 4) {
    stripThumbWidth.value = track;
    stripThumbOffset.value = 0;
    return;
  }
  stripThumbWidth.value = Math.min(track, Math.max(48, (track * row.clientWidth) / row.scrollWidth));
  stripThumbOffset.value = (row.scrollLeft / max) * (track - stripThumbWidth.value);
}


function onStripScroll() {
  syncStripBar();
}

function onBarPointerDown(event: PointerEvent) {
  if (event.target === stripThumb.value) return;
  const row = stripRow.value;
  const bar = stripBar.value;
  if (!row || !bar) return;
  const max = row.scrollWidth - row.clientWidth;
  if (max <= 0) return;
  const rect = bar.getBoundingClientRect();
  const ratio =
    (event.clientX - rect.left - stripThumbWidth.value / 2) /
    Math.max(1, rect.width - stripThumbWidth.value);
  row.scrollLeft = Math.min(max, Math.max(0, ratio * max));
}

function onThumbPointerDown(event: PointerEvent) {
  const row = stripRow.value;
  if (!row) return;
  event.preventDefault();
  event.stopPropagation();
  stripDragging.value = true;
  const thumb = event.currentTarget as HTMLElement;
  thumb.setPointerCapture(event.pointerId);
  thumb.dataset.dragX = String(event.clientX);
  thumb.dataset.dragLeft = String(row.scrollLeft);
}

function onThumbPointerMove(event: PointerEvent) {
  if (!stripDragging.value) return;
  const row = stripRow.value;
  const thumb = event.currentTarget as HTMLElement;
  if (!row || !thumb) return;
  const startX = Number(thumb.dataset.dragX ?? event.clientX);
  const startLeft = Number(thumb.dataset.dragLeft ?? row.scrollLeft);
  const max = row.scrollWidth - row.clientWidth;
  const movable = (stripBar.value?.clientWidth ?? 0) - stripThumbWidth.value;
  if (max <= 0 || movable <= 0) return;
  const ratio = (event.clientX - startX) / movable;
  row.scrollLeft = Math.min(max, Math.max(0, startLeft + ratio * max));
}

function onThumbPointerUp(event: PointerEvent) {
  stripDragging.value = false;
  const thumb = event.currentTarget as HTMLElement;
  if (thumb?.hasPointerCapture(event.pointerId)) thumb.releasePointerCapture(event.pointerId);
}

function moveSelection(direction: number) {
  if (!items.value.length) return;
  const current = items.value.findIndex((item) => item.id === selectedItemId.value);
  selectedItemId.value = items.value[(current + direction + items.value.length) % items.value.length].id;
  pendingClassification.value = null;
}

function onKeydown(event: KeyboardEvent) {
  if (event.target instanceof Element && event.target.matches("input, textarea, select")) return;
  if (event.key === "Enter") {
    event.preventDefault();
    void submitLabel();
  } else if (event.key === "ArrowLeft") {
    event.preventDefault();
    moveSelection(-1);
  } else if (event.key === "ArrowRight") {
    event.preventDefault();
    moveSelection(1);
  } else if (event.key === "Escape") {
    selectedCharacterId.value = null;
  } else if (/^[1-5]$/.test(event.key)) {
    const character = recentCharacters.value[Number(event.key) - 1];
    if (character) selectedCharacterId.value = character.id;
  }
}

async function loadPreview() {
  if (previewUrl.value) URL.revokeObjectURL(previewUrl.value);
  previewUrl.value = null;
  if (!selectedItem.value) return;
  try {
    const item = selectedItem.value;
    const variant = item.status === "completed" && item.destinationPath ? "destination" : "source";
    const bytes = await captureApi.readImage(item.id, variant);
    previewUrl.value = URL.createObjectURL(new Blob([bytes], { type: pathMimeType(item.sourcePath) }));
  } catch (error) {
    toast.error(normalizeError(error));
  }
}

function normalizeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

watch(projectId, () => void loadProject());
watch(() => selectedItem.value?.id, () => void loadPreview());
watch(items, () => void nextTick(syncStripBar));
watch(
  () => [selectedItemId.value, pendingClassification.value] as const,
  () => {
    if (pendingClassification.value === "person") void refreshSuggestion();
  },
);

onMounted(async () => {
  window.addEventListener("keydown", onKeydown);
  await initialize();
  try {
    unlisteners.push(
      await listen<CaptureItem>("capture:item-created", (event) => upsertItem(event.payload)),
      await listen<CaptureItem>("capture:item-updated", (event) => upsertItem(event.payload)),
      await listen<CaptureRuntimeStatus>("capture:runtime-status", (event) => {
        runtime.value = event.payload;
      }),
      await listen<string>("capture:runtime-error", (event) => {
        toast.error(event.payload);
      }),
    );
  } catch {
    // Browser previews do not expose the Tauri event bridge.
  }
  await nextTick();
  syncStripBar();
  if (typeof ResizeObserver !== "undefined" && stripRow.value) {
    stripObserver = new ResizeObserver(() => syncStripBar());
    stripObserver.observe(stripRow.value);
  }
  await loadPreview();
});

onBeforeUnmount(() => {
  stripObserver?.disconnect();
  stripObserver = null;
  window.removeEventListener("keydown", onKeydown);
  unlisteners.forEach((unlisten) => unlisten());
  if (previewUrl.value) URL.revokeObjectURL(previewUrl.value);
});
</script>

<template>
  <section class="capture-page" aria-labelledby="capture-title">
    <header class="capture-toolbar">
      <div class="session-rail">
        <section class="session-stage project-stage" aria-labelledby="session-project-title">
          <div class="session-stage-heading">
            <span class="session-step" aria-hidden="true">1</span>
            <h2 id="session-project-title">选择项目</h2>
            <Check v-if="projectId" class="session-stage-check" :size="16" aria-label="已选择项目" />
          </div>
          <div class="project-controls">
            <select v-model="projectId" aria-label="当前项目" class="capture-select">
              <option value="" disabled>选择项目</option>
              <option v-for="project in projects" :key="project.id" :value="project.id">{{ project.name }}</option>
            </select>
            <button class="icon-button" type="button" aria-label="新建项目" @click="showProjectForm = !showProjectForm">
              <Plus :size="18" />
            </button>
            <button
              class="icon-button"
              type="button"
              :disabled="!projectId"
              aria-label="重命名项目"
              title="重命名项目"
              @click="renameTarget = projects.find((project) => project.id === projectId) ?? null"
            >
              <Pencil :size="18" />
            </button>
          </div>
          <form v-if="showProjectForm" class="inline-create" @submit.prevent="createProject">
            <input v-model="newProjectName" autofocus aria-label="项目名称" placeholder="项目名称" />
            <button type="submit">创建</button>
          </form>
        </section>

        <section class="session-stage sources-stage" aria-labelledby="session-sources-title">
          <div class="session-stage-heading">
            <span class="session-step" aria-hidden="true">2</span>
            <h2 id="session-sources-title">截图来源</h2>
            <Check v-if="sourceDirs.some((dir) => dir.enabled)" class="session-stage-check" :size="16" aria-label="已配置截图来源" />
          </div>
          <ul class="source-dir-list session-source-list">
            <li v-for="dir in sourceDirs" :key="dir.id" :class="{ muted: !dir.enabled }">
              <FolderOpen :size="15" class="source-dir-icon" />
              <span class="source-dir-name" :title="dir.directory">{{ dir.directory }}</span>
              <span class="source-dir-state" :class="{ online: activeSession && dir.enabled }">
                {{ !dir.enabled ? "已暂停" : activeSession ? "监听中" : "已启用" }}
              </span>
              <div v-if="!activeSession" class="source-dir-actions">
                <button type="button" :disabled="busy" @click="toggleSourceDir(dir)">
                  {{ dir.enabled ? "暂停" : "启用" }}
                </button>
                <button type="button" class="danger" :disabled="busy" :aria-label="`移除 ${dir.directory}`" @click="removeSourceDir(dir)">
                  <Trash2 :size="14" />移除
                </button>
              </div>
            </li>
            <li v-if="!sourceDirs.length" class="source-dir-empty">还没有来源目录</li>
          </ul>
        </section>

        <section class="session-stage archive-stage" aria-labelledby="session-archive-title">
          <div class="session-stage-heading">
            <span class="session-step" aria-hidden="true">3</span>
            <h2 id="session-archive-title">归档位置</h2>
            <Check v-if="projectDestination" class="session-stage-check" :size="16" aria-label="已配置归档位置" />
          </div>
          <button type="button" class="archive-path-button" :disabled="Boolean(activeSession)" @click="chooseDestination">
            <Archive :size="16" />
            <span :title="projectDestination">{{ projectDestination || "选择归档目录" }}</span>
            <ChevronRight :size="15" />
          </button>
        </section>

        <section class="session-control" aria-label="会话控制">
          <div class="session-health">
            <span :class="{ online: activeSession }"><Wifi :size="15" />{{ activeSession ? "监听中" : "未监听" }}</span>
            <span :class="{ online: runtime?.engineStatus === 'configured' }">
              <Sparkles :size="15" />{{ runtime?.engineStatus === "configured" ? "AI 已配置" : "AI 未配置" }}
            </span>
          </div>
          <button v-if="activeSession" type="button" class="stop-button" :disabled="busy" @click="stopSession">
            <CircleStop :size="17" />停止会话
          </button>
          <button v-else type="button" class="start-button" :disabled="busy || !projectId" @click="startSession">
            <Play :size="17" />开始会话
          </button>
        </section>
      </div>

      <div class="session-tools-shelf" :class="{ collapsed: !sessionToolsOpen }">
        <div v-show="sessionToolsOpen" class="session-tools-actions">
          <button type="button" :disabled="busy" @click="addSourceDir">
            <FolderPlus :size="16" />添加截图目录
          </button>
          <template v-if="activeSession">
            <button type="button" :disabled="busy" @click="openImportDialog">
              <FolderInput :size="16" />导入截图
            </button>
            <button
              v-if="deferredImportCount"
              type="button"
              class="recognition-action"
              :disabled="importBusy"
              @click="startImportedRecognition"
            >
              <Sparkles :size="16" />开始识别导入截图（{{ deferredImportCount }}）
            </button>
          </template>
          <span class="session-tools-hint">目录调整在下次开始会话时生效</span>
        </div>
        <button
          type="button"
          class="session-tools-toggle"
          :aria-expanded="sessionToolsOpen"
          @click="sessionToolsOpen = !sessionToolsOpen"
        >
          {{ sessionToolsOpen ? "收起会话工具" : "展开会话工具" }}
          <ChevronUp v-if="sessionToolsOpen" :size="15" />
          <ChevronDown v-else :size="15" />
        </button>
      </div>
    </header>

    <div v-if="loading" class="capture-loading" aria-live="polite">
      <LoaderCircle class="animate-spin" :size="24" />正在加载捕获会话…
    </div>

    <div v-else class="capture-workspace">
      <main class="capture-stage">
        <div class="stage-heading">
          <div class="flex items-center gap-3">
            <h1 id="capture-title">最新捕获</h1>
            <span
              v-if="selectedItem && selectedItem.classification !== 'unclassified'"
              class="classification-pill"
              :data-classification="selectedItem.classification"
            >
              {{ captureClassificationLabel(selectedItem.classification) }}
            </span>
            <span class="status-pill" :data-status="selectedItem?.status ?? 'idle'">
              {{ selectedItem ? captureStatusLabel(selectedItem.status, selectedItem.failureStage) : "等待截图" }}
            </span>
          </div>
          <div class="stage-heading-actions">
            <div v-if="selectedItem" class="file-meta">
              <span>{{ pathFileName(selectedItem.sourcePath) }}</span>
              <span>{{ new Date(selectedItem.capturedAt).toLocaleString([], { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }) }}</span>
            </div>
            <button
              type="button"
              class="secondary-action label-panel-trigger"
              :aria-expanded="labelPanelOpen"
              @click="labelPanelOpen = true"
            >
              <UserRound :size="16" />角色与分类
            </button>
          </div>
        </div>

        <div class="preview-shell" :class="{ empty: !selectedItem }">
          <img v-if="previewUrl" :src="previewUrl" :alt="selectedItem ? `${pathFileName(selectedItem.sourcePath)} 预览` : ''" />
          <div v-else class="preview-empty">
            <ImageOff :size="44" />
            <strong>{{ activeSession ? "等待游戏产生新截图" : "开始会话后自动发现截图" }}</strong>
            <span>继续在游戏中使用 H 隐藏界面、S 截图</span>
          </div>
          <div v-if="selectedItem?.status === 'failed'" class="failure-overlay">
            <AlertCircle :size="20" />
            <div>
              <strong>{{ selectedItem.failureStage === "archive" ? "归档失败" : "处理失败" }}</strong>
              <p>{{ selectedItem.errorMessage }}</p>
            </div>
            <button type="button" :disabled="busy" @click="retryItem(selectedItem)">
              <RefreshCw :size="16" />重试
            </button>
          </div>
          <div v-if="selectedItem?.status === 'awaiting_label'" class="classify-bar" aria-label="截图分类">
            <span class="classify-hint">这张截图属于：</span>
            <button
              type="button"
              class="classify-option"
              :class="{ active: pendingClassification === 'person' }"
              @click="submitClassification('person')"
            >
              <UserRound :size="16" />人物
            </button>
            <button type="button" class="classify-option" :disabled="busy" @click="submitClassification('scene')">
              <Image :size="16" />游戏截图
            </button>
            <button type="button" class="classify-option" :disabled="busy" @click="submitClassification('private')">
              <Lock :size="16" />收藏
            </button>
          </div>
        </div>

        <section class="recent-strip" aria-labelledby="recent-title">
          <div class="strip-title">
            <h2 id="recent-title">近期捕获 <span>({{ stripItems.length }})</span></h2>
            <span>{{ waitingCount }} 张等待标记</span>
          </div>
          <div v-if="stripItems.length" class="thumbnail-scroll">
          <div ref="stripRow" class="thumbnail-row" @wheel="onStripWheel" @scroll="onStripScroll">
            <button
              v-for="item in stripItems"
              :key="item.id"
              type="button"
              class="capture-card"
              :class="{ selected: selectedItemId === item.id }"
              :aria-label="`选择 ${pathFileName(item.sourcePath)}`"
              @click="selectedItemId = item.id"
            >
              <div class="thumb-image"><CaptureThumbnail :item="item" /></div>
              <div class="thumb-status" :data-status="item.status">
                <span class="status-dot" />{{ captureStatusLabel(item.status, item.failureStage) }}
              </div>
              <time>{{ new Date(item.capturedAt).toLocaleString([], { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }) }}</time>
            </button>
          </div>
          <div ref="stripBar" class="strip-scrollbar" :class="{ dragging: stripDragging }" @pointerdown="onBarPointerDown">
            <div
              ref="stripThumb"
              class="strip-scrollbar-thumb"
              :style="{ width: `${stripThumbWidth}px`, transform: `translateX(${stripThumbOffset}px)` }"
              @pointerdown="onThumbPointerDown"
              @pointermove="onThumbPointerMove"
              @pointerup="onThumbPointerUp"
              @pointercancel="onThumbPointerUp"
            />
          </div>
          </div>
          <div v-else class="strip-empty">
            {{ items.length ? "全部截图已处理完成，可到历史页查看。" : "本次会话还没有截图" }}
          </div>
        </section>
      </main>

      <ResponsiveDetailPanel
        v-model:open="labelPanelOpen"
        title="角色与分类"
        description="选择分类，并在人物截图中指定主角色。"
        panel-class="label-panel"
      >
        <div class="label-panel-heading">
          <div>
            <span class="eyebrow">快速分类</span>
            <h2 id="label-title">{{ pendingClassification === "person" ? "选择主角色" : "等待分类" }}</h2>
          </div>
          <span v-if="pendingClassification === 'person'" class="shortcut-hint">1–5</span>
        </div>

        <CaptureProgress class="aside-progress" />

        <button
          v-if="pendingClassification === 'person' && suggestedCharacter"
          type="button"
          class="suggestion-row"
          :class="{ selected: selectedCharacterId === suggestedCharacter.id }"
          @click="selectedCharacterId = suggestedCharacter.id"
        >
          <Sparkles :size="16" />
          <span>
            <strong>推荐：{{ suggestedCharacter.name }}</strong>
            <small>相似度 {{ Math.round((selectedItem?.recognitionConfidence ?? 0) * 100) }}%</small>
          </span>
          <Check v-if="selectedCharacterId === suggestedCharacter.id" :size="16" />
        </button>

        <div v-if="pendingClassification !== 'person'" class="classify-guide">
          <p>在预览区选择这张截图的类型：</p>
          <ul>
            <li><UserRound :size="14" />人物 — 选择角色后生成标注图与头像</li>
            <li><Image :size="14" />游戏截图 — 原图直存，无需输入</li>
            <li><Lock :size="14" />收藏 — 原图直存，默认隐藏</li>
          </ul>
        </div>

        <label v-else class="character-search">
          <Search :size="17" />
          <span class="sr-only">搜索角色名称</span>
          <input v-model="search" placeholder="搜索角色名称" />
        </label>

        <div v-if="pendingClassification === 'person' && recentCharacters.length" class="character-section">
          <span class="section-label">最近使用</span>
          <div class="recent-chips">
            <button
              v-for="(character, index) in recentCharacters"
              :key="character.id"
              type="button"
              class="recent-chip"
              :class="{ selected: selectedCharacterId === character.id }"
              :style="{ fontSize: recentNameFont(character.name) }"
              :title="character.name"
              @click="selectedCharacterId = character.id"
            >
              {{ character.name }}<kbd>{{ index + 1 }}</kbd>
            </button>
          </div>
        </div>

        <div v-if="pendingClassification === 'person'" class="character-section all-characters">
          <div class="section-heading">
            <span class="section-label">全部角色</span>
            <button type="button" @click="showCharacterForm = !showCharacterForm"><Plus :size="15" />新建</button>
          </div>
          <form v-if="showCharacterForm" class="inline-create character-create" @submit.prevent="createCharacter">
            <input v-model="newCharacterName" autofocus aria-label="角色名称" placeholder="角色名称" />
            <button type="submit">添加</button>
          </form>
          <button
            v-for="character in filteredCharacters"
            :key="character.id"
            type="button"
            class="character-row"
            :class="{ selected: selectedCharacterId === character.id }"
            @click="selectedCharacterId = character.id"
          >
            <span class="avatar"><UserRound :size="18" /></span>
            <span><strong>{{ character.name }}</strong><small>项目角色</small></span>
            <Check v-if="selectedCharacterId === character.id" :size="17" />
          </button>
          <p v-if="!filteredCharacters.length" class="character-empty">还没有角色，先创建一个角色。</p>
        </div>

        <div v-if="pendingClassification === 'person'" class="label-action">
          <div v-if="verificationWarning" class="verification-warning" role="alert">
            <p>
              <template v-if="verificationWarning.level === 'strong'">
                相似度很低（{{ verificationWarning.score != null ? Math.round(verificationWarning.score * 100) + "%" : "—" }}）
                <template v-if="verificationWarning.bestOtherCharacterId">
                  ，更像 {{ characterName(verificationWarning.bestOtherCharacterId) }}
                  （{{ verificationWarning.bestOtherScore != null ? Math.round(verificationWarning.bestOtherScore * 100) + "%" : "—" }}）
                </template>
                ，很可能标错了角色！
              </template>
              <template v-else>
                相似度偏低（{{ verificationWarning.score != null ? Math.round(verificationWarning.score * 100) + "%" : "—" }}），确认这张脸是「{{ selectedCharacter?.name }}」吗？
              </template>
            </p>
            <div class="verification-warning-actions">
              <button type="button" class="process-button confirm" :disabled="busy" @click="confirmForcedLabel">仍确认</button>
              <button type="button" class="secondary-action" :disabled="busy" @click="verificationWarning = null">取消</button>
            </div>
          </div>
          <div v-if="selectedCharacter" class="selection-summary">
            将 <strong>{{ selectedCharacter.name }}</strong> 标记到当前截图
          </div>
          <button type="button" class="process-button" :disabled="!canSubmit" @click="submitLabel">
            <LoaderCircle v-if="busy" class="animate-spin" :size="18" />
            <Sparkles v-else :size="18" />
            分配并处理
            <kbd>Enter</kbd>
          </button>
          <p>选择角色后开始本地处理和可靠归档</p>
        </div>
      </ResponsiveDetailPanel>
    </div>

    <footer class="capture-footer">
      <div><span class="status-dot active" />{{ activeSession ? "会话运行中" : "会话未开始" }}</div>
      <div class="footer-nav">
        <button type="button" :disabled="!items.length" aria-label="上一张截图" @click="moveSelection(-1)"><ChevronLeft :size="17" />上一张</button>
        <button type="button" :disabled="!items.length" aria-label="下一张截图" @click="moveSelection(1)">下一张<ChevronRight :size="17" /></button>
      </div>
      <div>{{ runtime?.queuedCount ?? 0 }} 排队 · {{ runtime?.archivePendingCount ?? 0 }} 待归档</div>
    </footer>

    <Dialog
      v-if="importCandidates"
      :open="true"
      @update:open="!$event && !importBusy && (importCandidates = null)"
    >
      <DialogContent class="import-dialog" :show-close-button="false" aria-label="导入已有截图">
        <span class="eyebrow">导入已有截图</span>
        <DialogTitle>发现 {{ importCandidates.length }} 张未入库图片</DialogTitle>
        <DialogDescription>
          来源目录中已存在、但还没有登记记录的图片；导入只登记记录，不会立即识别或批量加载缩略图。
        </DialogDescription>
        <ul class="import-list">
          <li v-for="candidate in importCandidates.slice(0, 10)" :key="candidate.path">
            {{ pathFileName(candidate.path) }}
          </li>
          <li v-if="importCandidates.length > 10">… 还有 {{ importCandidates.length - 10 }} 张</li>
        </ul>
        <p v-if="importCandidates.length === 0" class="import-empty">来源目录里没有未登记的图片。</p>
        <div class="import-actions">
          <button type="button" class="secondary-action" :disabled="importBusy" @click="importCandidates = null">取消</button>
          <button type="button" class="primary-action" :disabled="importBusy || importCandidates.length === 0" @click="confirmImport">
            <LoaderCircle v-if="importBusy" class="animate-spin" :size="17" />全部导入（{{ importCandidates.length }}）
          </button>
        </div>
      </DialogContent>
    </Dialog>

    <ProjectRenameDialog
      v-if="renameTarget"
      :project-id="renameTarget.id"
      :project-name="renameTarget.name"
      @close="renameTarget = null"
      @saved="onProjectRenamed"
    />
  </section>
</template>
