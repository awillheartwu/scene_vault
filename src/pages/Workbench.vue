<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  Archive,
  Ban,
  Check,
  ChevronDown,
  ChevronsLeft,
  ChevronsRight,
  ExternalLink,
  Image,
  Pencil,
  RefreshCw,
  RotateCcw,
  Sparkles,
  UserPlus,
  UserRound,
  Users,
  X,
} from "@lucide/vue";
import CaptureThumbnail from "@/components/capture/CaptureThumbnail.vue";
import CaptureProgress from "@/components/capture/CaptureProgress.vue";
import CharacterMergeDialog from "@/components/character/CharacterMergeDialog.vue";
import {
  captureApi,
  captureStatusLabel,
  pathFileName,
  revealPath,
  type CaptureItem,
  type Character,
  type CharacterSummary,
  type FaceBankModelStatus,
  type FaceSample,
  type Project,
} from "@/lib/capture-api";
import { toast } from "@/lib/toast";

const projects = ref<Project[]>([]);
const summaries = ref<CharacterSummary[]>([]);
const characters = ref<Character[]>([]);
const projectId = ref("");
const rebuildProgress = ref<{ processed: number; total: number } | null>(null);
const modelStatus = ref<FaceBankModelStatus | null>(null);
const selectedCharacterId = ref<string | null>(null);
const items = ref<CaptureItem[]>([]);
const samples = ref<FaceSample[]>([]);
const selectedItemId = ref<string | null>(null);
const previewVariant = ref<"source" | "annotated" | "avatar" | "destination">("source");
const loading = ref(false);
const busy = ref(false);
const renameOpen = ref(false);
const renameName = ref("");
const renameBusy = ref(false);
const mergeOpen = ref(false);
const mergeButton = ref<HTMLButtonElement | null>(null);
let unlisteners: UnlistenFn[] = [];

type WorkbenchView = "characters" | "unclassified" | "scene" | "private";
const view = ref<WorkbenchView>("characters");
const panelCollapsed = ref(false);
const showPrivate = ref(false);
const sampleStripOpen = ref(false);

const selectedCharacter = computed(
  () => summaries.value.find((summary) => summary.id === selectedCharacterId.value) ?? null,
);
const projectDegradedCount = computed(() =>
  summaries.value.reduce((sum, summary) => sum + summary.degradedCount, 0),
);
const viewLabel = computed(() => {
  if (view.value === "unclassified") return "未分类";
  if (view.value === "scene") return "游戏截图";
  if (view.value === "private") return "收藏图";
  return selectedCharacter.value?.name ?? "未选择角色";
});
const selectedItem = computed(
  () => items.value.find((item) => item.id === selectedItemId.value) ?? null,
);
const canSetSelectedAsAvatar = computed(() => {
  const item = selectedItem.value;
  return Boolean(
    view.value === "characters" &&
      selectedCharacter.value &&
      item?.characterId === selectedCharacterId.value &&
      item.assetId &&
      item.avatarPath,
  );
});
const selectedItemIsRepresentativeAvatar = computed(() =>
  Boolean(
    selectedCharacter.value?.avatarAssetId &&
      selectedCharacter.value.avatarAssetId === selectedItem.value?.assetId,
  ),
);

function characterName(id: string | null | undefined): string {
  if (!id) return "";
  return characters.value.find((character) => character.id === id)?.name ?? "";
}

function classificationLabel(value: string): string {
  if (value === "person") return "人物";
  if (value === "scene") return "游戏截图";
  if (value === "private") return "收藏图";
  return "未分类";
}

function reviewStatusLabel(value: string): string {
  if (value === "pending") return "建议待确认";
  if (value === "accepted") return "建议已确认";
  if (value === "rejected") return "建议已拒绝";
  return "无建议";
}

function recognitionSourceLabel(value: string | null): string {
  if (value === "face_bank") return "Face Bank";
  if (value === "vision") return "视觉引擎";
  if (value === "manual") return "手动";
  return "—";
}

function normalizeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function thumbnailItem(id: string): CaptureItem {
  return { id, sourcePath: "capture.png" } as CaptureItem;
}

function summaryThumbnailItemId(summary: CharacterSummary): string {
  return summary.avatarCaptureItemId ?? summary.latestCaptureItemId ?? "";
}

function canReprocess(item: CaptureItem): boolean {
  // Degraded fallback: completed person capture archived raw because the
  // Python engine was not configured.
  return item.status === "completed" && item.classification === "person" && !item.annotatedPath;
}

async function initialize() {
  projects.value = await captureApi.listProjects();
  const savedProject = localStorage.getItem("scene-vault.capture.project");
  projectId.value = projects.value.some((project) => project.id === savedProject)
    ? savedProject!
    : projects.value[0]?.id ?? "";
  if (projectId.value) {
    localStorage.setItem("scene-vault.capture.project", projectId.value);
  }
  try {
    const settings = await captureApi.getAppSettings();
    showPrivate.value = settings.showPrivateByDefault;
  } catch {
    // Keep the default hidden state when the backend is unavailable.
  }
  await loadCharacterData();
}

async function loadCharacterData() {
  if (!projectId.value) {
    summaries.value = [];
    characters.value = [];
    items.value = [];
    modelStatus.value = null;
    selectedItemId.value = null;
    return;
  }
  loading.value = true;
  try {
    [summaries.value, characters.value, modelStatus.value] = await Promise.all([
      captureApi.listProjectCharacterSummaries(projectId.value),
      captureApi.listCharacters(projectId.value),
      (captureApi.getFaceBankModelStatus?.(projectId.value) ??
        Promise.resolve(null)).catch(() => null),
    ]);
    if (
      !selectedCharacterId.value ||
      !summaries.value.some((summary) => summary.id === selectedCharacterId.value)
    ) {
      selectedCharacterId.value = summaries.value[0]?.id ?? null;
    }
    await loadItems();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    loading.value = false;
  }
}

async function loadItems() {
  if (!projectId.value) {
    items.value = [];
    samples.value = [];
    selectedItemId.value = null;
    return;
  }
  const projectIdValue = projectId.value;
  loading.value = true;
  try {
    if (view.value === "characters" && !selectedCharacterId.value) {
      items.value = [];
      samples.value = [];
      selectedItemId.value = null;
    } else if (view.value === "characters") {
      // The first branch already returned for character views without a
      // selection, so this is logically non-null.
      const characterIdValue = selectedCharacterId.value!;
      [items.value, samples.value] = await Promise.all([
        captureApi.listCharacterCaptureItems({
          projectId: projectIdValue,
          characterId: characterIdValue,
        }),
        captureApi.listCharacterFaceSamples(characterIdValue),
      ]);
    } else {
      const category = view.value as "unclassified" | "scene" | "private";
      items.value = await captureApi.listCategoryItems({
        projectId: projectIdValue,
        category,
      });
      samples.value = [];
    }
    if (
      items.value.length &&
      !items.value.some((item) => item.id === selectedItemId.value)
    ) {
      selectedItemId.value = items.value[0]?.id ?? null;
    }
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    loading.value = false;
  }
}

async function runMutation(task: () => Promise<unknown>) {
  busy.value = true;
  try {
    await task();
    await loadCharacterData();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    busy.value = false;
  }
}

function acceptSuggestion(item: CaptureItem) {
  return runMutation(async () => {
    await captureApi.acceptRecognitionSuggestion(item.id);
    toast.success("已接受建议、完成角色改判，并重新排队处理与归档。");
  });
}

function rejectSuggestion(item: CaptureItem) {
  return runMutation(() =>
    captureApi.reviewRecognitionSuggestion({ captureItemId: item.id, decision: "rejected" }),
  );
}

function rejectAndEnroll(item: CaptureItem) {
  return runMutation(async () => {
    await captureApi.rejectSuggestionAndEnroll(item.id);
    toast.success(`已拒绝建议，并把这张脸登记到「${characterName(item.characterId)}」的样本库。`);
  });
}

async function batchRejectAndEnroll() {
  if (!projectId.value || !selectedCharacterId.value || busy.value) return;
  busy.value = true;
  try {
    const count = await captureApi.batchRejectAndEnroll(
      projectId.value,
      selectedCharacterId.value,
    );
    if (count > 0) {
      toast.success(
        `已批量拒绝并登记 ${count} 条建议到「${characterName(selectedCharacterId.value)}」的样本库。`,
      );
    } else {
      toast.info("当前角色没有待确认的建议。");
    }
    await loadCharacterData();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    busy.value = false;
  }
}

function reprocessDegraded(item: CaptureItem) {
  return runMutation(() => captureApi.retry(item.id));
}

async function onRelabelCharacter(event: Event) {
  const item = selectedItem.value;
  if (!item) return;
  const characterId = (event.target as HTMLSelectElement).value || null;
  // A character only exists on person captures; choosing one upgrades the
  // classification, clearing it moves a person capture back to scene.
  const classification = characterId ? "person" : item.classification === "person" ? "scene" : item.classification;
  if (characterId === (item.characterId ?? null) && classification === item.classification) return;
  await runMutation(() =>
    captureApi.relabel(item.id, characterId, classification as CaptureItem["classification"]),
  );
}

async function onRelabelClassification(event: Event) {
  const item = selectedItem.value;
  if (!item) return;
  const classification = (event.target as HTMLSelectElement).value;
  if (classification === "person" && !item.characterId) {
    toast.error("人物分类需要先选择角色。");
    return;
  }
  if (classification === item.classification) return;
  await runMutation(() =>
    captureApi.relabel(item.id, item.characterId, classification as CaptureItem["classification"]),
  );
}

async function reveal(path: string | null) {
  if (!path) return;
  try {
    await revealPath(path);
  } catch (error) {
    toast.error(normalizeError(error));
  }
}

function openRename() {
  const character = selectedCharacter.value;
  if (!character) return;
  renameName.value = character.name;
  renameOpen.value = true;
}

async function submitRename() {
  const characterId = selectedCharacterId.value;
  const name = renameName.value.trim();
  if (!characterId || !name || renameBusy.value) return;
  renameBusy.value = true;
  try {
    await captureApi.renameCharacter({ characterId, name });
    renameOpen.value = false;
    await loadCharacterData();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    renameBusy.value = false;
  }
}

function closeMerge() {
  mergeOpen.value = false;
  void nextTick(() => mergeButton.value?.focus());
}

async function onCharacterMerged(character: Character) {
  mergeOpen.value = false;
  selectedCharacterId.value = character.id;
  await loadCharacterData();
  void nextTick(() => mergeButton.value?.focus());
}

async function setRepresentativeAvatar(avatarAssetId: string | null) {
  const character = selectedCharacter.value;
  if (!character || busy.value) return;
  busy.value = true;
  try {
    await captureApi.setCharacterAvatar({
      characterId: character.id,
      avatarAssetId,
    });
    toast.success(
      avatarAssetId
        ? `已将当前截图设为「${character.name}」的代表头像。`
        : `已清除「${character.name}」的代表头像，将自动显示最近头像。`,
    );
    await loadCharacterData();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    busy.value = false;
  }
}

function setSelectedAsRepresentativeAvatar() {
  if (!canSetSelectedAsAvatar.value || !selectedItem.value?.assetId) return;
  return setRepresentativeAvatar(selectedItem.value.assetId);
}

async function toggleSample(sample: FaceSample) {
  if (busy.value) return;
  busy.value = true;
  try {
    const updated = await captureApi.setFaceSampleStatus(
      sample.id,
      sample.status === "active" ? "revoked" : "active",
    );
    const index = samples.value.findIndex((entry) => entry.id === sample.id);
    if (index >= 0) samples.value[index] = updated;
    await loadCharacterData();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    busy.value = false;
  }
}

async function clearSampleFlag(sample: FaceSample) {
  if (busy.value) return;
  busy.value = true;
  try {
    const updated = await captureApi.setFaceSampleFlagged(sample.id, false);
    const index = samples.value.findIndex((entry) => entry.id === sample.id);
    if (index >= 0) samples.value[index] = updated;
    await loadCharacterData();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    busy.value = false;
  }
}

function selectCapture(captureItemId: string) {
  if (items.value.some((item) => item.id === captureItemId)) {
    selectedItemId.value = captureItemId;
  }
}

async function batchReprocess(characterId: string | null) {
  if (!projectId.value || busy.value) return;
  busy.value = true;
  try {
    const requeued = await captureApi.retryDegradedCaptures(
      projectId.value,
      characterId,
    );
    if (requeued > 0) {
      toast.success(`已重新排队 ${requeued} 张降级图，正在后台重新识别…`);
    } else {
      toast.info("没有需要重新识别的降级图。");
    }
    await loadCharacterData();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    busy.value = false;
  }
}

async function rebuildFaceBank() {
  if (!projectId.value || busy.value) return;
  busy.value = true;
  rebuildProgress.value = null;
  try {
    const summary = await captureApi.rebuildFaceBank(projectId.value);
    toast.success(
      `人脸样本库重建完成：${summary.rebuilt} 张已提取，${summary.noFace} 张无脸，` +
        `${summary.notEnrolled} 张未通过样本门槛，${summary.skippedMissingSource} 张源文件缺失，` +
        `${summary.failed} 张失败，其中 ${summary.stalePreserved} 张保留旧样本（共 ${summary.total} 张）。`,
    );
    await loadCharacterData();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    busy.value = false;
    rebuildProgress.value = null;
  }
}

watch(projectId, async () => {
  if (projectId.value) {
    localStorage.setItem("scene-vault.capture.project", projectId.value);
  }
  selectedCharacterId.value = null;
  await loadCharacterData();
});
watch(view, async () => {
  selectedItemId.value = null;
  await loadItems();
});
watch(selectedCharacterId, loadItems);
watch(selectedItemId, () => {
  previewVariant.value = "source";
});
onMounted(async () => {
  try {
    // Items change state while the worker processes them (reprocess, archive,
    // face-bank suggestions); the workbench is pull-based otherwise, so it
    // refreshes whenever any capture item changes.
    unlisteners.push(
      await listen("capture:item-updated", () => {
        void loadCharacterData();
      }),
    );
    unlisteners.push(
      await listen<{ processed: number; total: number }>(
        "capture:face-bank-rebuild-progress",
        (event) => {
          rebuildProgress.value = event.payload;
        },
      ),
    );
  } catch {
    // Browser previews do not expose the Tauri event bridge.
  }
  await initialize();
});

onBeforeUnmount(() => {
  unlisteners.forEach((unlisten) => unlisten());
});
</script>

<template>
  <section class="workbench-page">
    <header class="workbench-header">
      <div>
        <span class="eyebrow">Character Workbench</span>
        <h1>人物工作台</h1>
        <p>按角色复查截图与 AI 建议；改判后自动重新处理并替换归档。</p>
      </div>
      <label class="workbench-project">
        项目
        <select v-model="projectId">
          <option v-for="project in projects" :key="project.id" :value="project.id">
            {{ project.name }}
          </option>
        </select>
      </label>
      <button type="button" class="secondary-action" :disabled="loading" @click="loadCharacterData">
        <RefreshCw :size="17" :class="{ 'animate-spin': loading }" />刷新
      </button>
      <button
        type="button"
        class="secondary-action"
        :disabled="busy || !projectId"
        title="用当前配置的识别模型重新提取全部人物截图特征并重建样本库"
        @click="rebuildFaceBank"
      >
        <Sparkles :size="17" />重建人脸样本库
        <span v-if="rebuildProgress" class="rebuild-progress">
          {{ rebuildProgress.processed }}/{{ rebuildProgress.total }}
        </span>
      </button>
    </header>
    <div v-if="modelStatus && !modelStatus.compatible" class="model-mismatch-banner">
      <span>
        样本库共有 {{ modelStatus.sampleCount }} 条，其中
        {{ modelStatus.incompatibleSampleCount }} 条与当前识别器
        {{ modelStatus.activeModelId }}（{{ modelStatus.activeModelVersion }}）不兼容；
        这些旧样本不会参与 AI 建议。重新提取后即可清理。
      </span>
      <button
        type="button"
        class="secondary-action compact-action"
        :disabled="busy"
        @click="rebuildFaceBank"
      >
        <Sparkles :size="14" />重建人脸样本库
      </button>
    </div>

    <div class="workbench-workspace" :class="{ collapsed: panelCollapsed }">
      <aside v-show="!panelCollapsed" class="workbench-characters" aria-label="角色总览">
        <div class="workbench-tabs" role="tablist" aria-label="素材分类">
          <button
            type="button"
            role="tab"
            :aria-selected="view === 'characters'"
            :class="{ active: view === 'characters' }"
            @click="view = 'characters'"
          >
            人物
          </button>
          <button
            type="button"
            role="tab"
            :aria-selected="view === 'unclassified'"
            :class="{ active: view === 'unclassified' }"
            @click="view = 'unclassified'"
          >
            未分类
          </button>
          <button
            type="button"
            role="tab"
            :aria-selected="view === 'scene'"
            :class="{ active: view === 'scene' }"
            @click="view = 'scene'"
          >
            游戏截图
          </button>
          <button
            v-if="showPrivate"
            type="button"
            role="tab"
            :aria-selected="view === 'private'"
            :class="{ active: view === 'private' }"
            @click="view = 'private'"
          >
            收藏图
          </button>
        </div>
        <template v-if="view === 'characters'">
        <div class="workbench-characters-title">
          <Users :size="16" /><span>角色</span><span class="count">{{ summaries.length }}</span>
        </div>
        <button
          v-for="summary in summaries"
          :key="summary.id"
          type="button"
          class="character-card"
          :class="{ selected: selectedCharacterId === summary.id }"
          @click="selectedCharacterId = summary.id"
        >
          <span class="character-avatar">
            <CaptureThumbnail
              v-if="summary.avatarCaptureItemId || summary.latestCaptureItemId"
              :item="thumbnailItem(summaryThumbnailItemId(summary))"
              :variant="summary.avatarCaptureItemId || summary.latestAvatarPath ? 'avatar' : 'source'"
              :fallback-variant="summary.avatarCaptureItemId || summary.latestAvatarPath ? 'source' : undefined"
              :alt="`${summary.name} 的角色头像`"
            />
            <UserRound v-else :size="18" />
          </span>
          <span class="character-main">
            <strong>{{ summary.name }}</strong>
            <span>
              {{ summary.captureCount }} 张
              <template v-if="summary.sampleCount"> · 样本 {{ summary.sampleCount }}</template>
              <template v-if="summary.degradedCount"> · 未识别 {{ summary.degradedCount }}</template>
              <template v-if="summary.lastCapturedAt">
                · 最近 {{ new Date(summary.lastCapturedAt).toLocaleDateString() }}
              </template>
            </span>
          </span>
          <span
            v-if="summary.pendingReviewCount > 0"
            class="pending-badge"
            :title="`${summary.pendingReviewCount} 条建议待确认`"
          >
            {{ summary.pendingReviewCount }}
          </span>
        </button>
        <div v-if="!summaries.length && !loading" class="workbench-empty">
          还没有角色，先在捕获流程创建角色并标记截图。
        </div>
        </template>
        <div v-else class="workbench-category-note">
          <strong>{{ viewLabel }}</strong>
          <span>{{ items.length }} 张截图</span>
          <p v-if="view === 'unclassified'">等待分类的截图会出现在这里，可在右侧直接标记角色或分类。</p>
          <p v-else-if="view === 'scene'">标记为游戏截图的原图直存归档。</p>
          <p v-else>标记为收藏的截图默认隐藏，可在设置中开启本标签页。</p>
        </div>
      </aside>

      <div class="workbench-grid-column">
        <div class="workbench-grid-header">
          <h2>
            <button
              type="button"
              class="panel-toggle"
              :title="panelCollapsed ? '展开左侧栏' : '折叠左侧栏'"
              aria-label="折叠或展开左侧栏"
              @click="panelCollapsed = !panelCollapsed"
            >
              <ChevronsRight v-if="panelCollapsed" :size="15" />
              <ChevronsLeft v-else :size="15" />
            </button>
            {{ viewLabel }}
            <button
              v-if="view === 'characters' && selectedCharacter"
              type="button"
              class="rename-button"
              :title="`重命名 ${selectedCharacter.name}`"
              aria-label="重命名角色"
              @click="openRename"
            >
              <Pencil :size="14" />
            </button>
          </h2>
          <span class="grid-actions">
            <button
              v-if="view === 'characters' && selectedCharacter && summaries.length > 1"
              ref="mergeButton"
              type="button"
              class="secondary-action"
              :disabled="busy"
              :title="`将 ${selectedCharacter.name} 合并到另一个角色`"
              @click="mergeOpen = true"
            >
              <Users :size="14" />合并角色
            </button>
            <button
              v-if="view === 'characters' && selectedCharacter && selectedCharacter.pendingReviewCount > 0"
              type="button"
              class="secondary-action compact-action"
              :disabled="busy"
              title="否决当前角色全部待确认建议，并把对应人脸登记进该角色样本库"
              @click="batchRejectAndEnroll"
            >
              <Ban :size="14" />批量拒绝并登记 ({{ selectedCharacter.pendingReviewCount }})
            </button>
            <button
              v-if="view === 'characters' && selectedCharacter && selectedCharacter.degradedCount > 0"
              type="button"
              class="secondary-action compact-action"
              :disabled="busy"
              @click="batchReprocess(selectedCharacter.id)"
            >
              <Sparkles :size="14" />当前角色重新识别 ({{ selectedCharacter.degradedCount }})
            </button>
            <button
              v-if="view === 'characters' && projectDegradedCount > 0"
              type="button"
              class="secondary-action compact-action"
              :disabled="busy"
              @click="batchReprocess(null)"
            >
              <Sparkles :size="14" />全部重新识别 ({{ projectDegradedCount }})
            </button>
            <span>{{ items.length }} 张截图</span>
          </span>
        </div>
        <CaptureProgress />
        <div class="workbench-grid" role="list" aria-label="角色截图网格">
          <button
            v-for="item in items"
            :key="item.id"
            type="button"
            role="listitem"
            class="workbench-cell"
            :class="{ selected: selectedItemId === item.id }"
            @click="selectedItemId = item.id"
          >
            <span class="workbench-cell-thumb">
              <CaptureThumbnail :item="item" :variant="item.destinationPath ? 'destination' : 'source'" />
            </span>
            <span class="workbench-cell-meta">
              <span class="workbench-cell-name" :title="item.sourcePath">{{ pathFileName(item.sourcePath) }}</span>
              <span class="status-pill" :data-status="item.status">
                {{ captureStatusLabel(item.status, item.failureStage) }}
              </span>
              <span class="classification-chip" :data-classification="item.classification">
                {{ classificationLabel(item.classification) }}
              </span>
              <span v-if="item.faceCount && item.faceCount > 1" class="multi-face-chip" :title="`YuNet 检测到 ${item.faceCount} 张脸，AI 建议基于主脸`">
                多脸 ×{{ item.faceCount }}
              </span>
              <span v-if="item.reviewStatus === 'pending'" class="review-chip">建议待确认</span>
            </span>
          </button>
          <div v-if="!items.length && !loading" class="workbench-empty">该角色名下还没有截图。</div>
        </div>
      </div>

      <aside v-if="selectedItem" class="workbench-detail" aria-label="截图详情">
        <section v-if="view === 'characters'" class="sample-strip" aria-label="Face Bank 样本库">
          <div class="sample-strip-header">
            <button type="button" class="sample-strip-toggle" :aria-expanded="sampleStripOpen" @click="sampleStripOpen = !sampleStripOpen">
              <span class="eyebrow">Face Bank</span>
              <span class="sample-strip-count">{{ samples.length }} 条样本</span>
              <ChevronDown :size="14" :class="{ rotated: sampleStripOpen }" />
            </button>
          </div>
          <div v-if="sampleStripOpen && samples.length" class="sample-list">
            <button
              v-for="sample in samples"
              :key="sample.id"
              type="button"
              class="sample-item"
              :class="{ revoked: sample.status === 'revoked', flagged: sample.flagged === 1 }"
              :title="`查看来源截图（${sample.status === 'active' ? '使用中' : '已撤销'}）`"
              @click="selectCapture(sample.captureItemId)"
            >
              <span class="sample-thumb">
                <CaptureThumbnail
                  :item="thumbnailItem(sample.captureItemId)"
                  variant="avatar"
                  fallback-variant="source"
                />
              </span>
              <span class="sample-meta">
                <strong>{{ sample.confidence != null ? `${Math.round(sample.confidence * 100)}%` : "—" }}</strong>
                <span>
                  {{ sample.flagged === 1 ? "待复查" : sample.status === "active" ? "使用中" : "已撤销" }}
                </span>
              </span>
              <span
                class="sample-toggle"
                :title="sample.flagged === 1 ? '信任该样本（恢复参与匹配）' : sample.status === 'active' ? '撤销样本（不再参与匹配）' : '恢复样本'"
                @click.stop="sample.flagged === 1 ? clearSampleFlag(sample) : toggleSample(sample)"
              >
                <Ban v-if="sample.flagged === 0 && sample.status === 'active'" :size="14" />
                <RotateCcw v-else :size="14" />
              </span>
            </button>
          </div>
          <p v-if="!samples.length" class="sample-empty">还没有样本：对人物图执行"重新识别"后自动登记。</p>
        </section>

        <div class="workbench-detail-top">
          <div class="detail-preview">
            <div class="preview-frame">
              <CaptureThumbnail
                :item="selectedItem"
                :variant="previewVariant"
                size="full"
              />
            </div>
            <div class="variant-tabs" role="tablist" aria-label="预览图切换">
              <button
                type="button"
                :class="{ active: previewVariant === 'source' }"
                @click="previewVariant = 'source'"
              >
                原图
              </button>
              <button
                v-if="selectedItem.annotatedPath"
                type="button"
                :class="{ active: previewVariant === 'annotated' }"
                @click="previewVariant = 'annotated'"
              >
                标注图
              </button>
              <button
                v-if="selectedItem.avatarPath"
                type="button"
                :class="{ active: previewVariant === 'avatar' }"
                @click="previewVariant = 'avatar'"
              >
                头像
              </button>
              <button
                v-if="selectedItem.destinationPath"
                type="button"
                :class="{ active: previewVariant === 'destination' }"
                @click="previewVariant = 'destination'"
              >
                归档图
              </button>
            </div>
          </div>
          <dl class="detail-facts">
            <div><dt>截图</dt><dd>{{ pathFileName(selectedItem.sourcePath) }}</dd></div>
            <div><dt>状态</dt><dd>{{ captureStatusLabel(selectedItem.status, selectedItem.failureStage) }}</dd></div>
            <div><dt>当前角色</dt><dd>{{ characterName(selectedItem.characterId) || "未标记" }}</dd></div>
            <div><dt>分类</dt><dd>{{ classificationLabel(selectedItem.classification) }}</dd></div>
            <div><dt>捕获时间</dt><dd>{{ new Date(selectedItem.capturedAt).toLocaleString() }}</dd></div>
            <div v-if="selectedItem.processingWarningsJson !== '[]'">
              <dt>处理提示</dt><dd>{{ selectedItem.processingWarningsJson }}</dd>
            </div>
            <div v-if="selectedItem.errorMessage" class="detail-error">
              <dt>错误</dt><dd>{{ selectedItem.errorMessage }}</dd>
            </div>
          </dl>
        </div>

        <div class="detail-actions">
          <section v-if="view === 'characters' || view === 'unclassified'" class="action-card">
            <span class="action-card-title">AI 建议</span>
            <div class="action-card-body">
              <span v-if="selectedItem.suggestedCharacterId" class="suggestion-status">
                <strong>{{ characterName(selectedItem.suggestedCharacterId) }}</strong>
                <span>
                  置信度 {{ Math.round((selectedItem.recognitionConfidence ?? 0) * 100) }}%
                  · {{ recognitionSourceLabel(selectedItem.recognitionSource) }}
                </span>
                <span class="status-pill" :data-review="selectedItem.reviewStatus">
                  {{ reviewStatusLabel(selectedItem.reviewStatus) }}
                </span>
              </span>
              <span v-else class="suggestion-status empty">暂无建议</span>
              <span v-if="selectedItem.reviewStatus === 'pending'" class="action-buttons">
                <button type="button" class="primary-action" :disabled="busy" @click="acceptSuggestion(selectedItem)">
                  <Check :size="16" />确认建议
                </button>
                <button type="button" class="secondary-action" :disabled="busy" @click="rejectSuggestion(selectedItem)">
                  <X :size="16" />拒绝建议
                </button>
                <button
                  type="button"
                  class="secondary-action"
                  :disabled="busy"
                  title="拒绝建议，并把这张脸加入当前角色的样本库（同脸模角色建议用这个）"
                  @click="rejectAndEnroll(selectedItem)"
                >
                  <UserPlus :size="16" />拒绝并登记
                </button>
              </span>
              <span v-else-if="selectedItem.reviewStatus === 'accepted'" class="action-buttons">
                <button
                  type="button"
                  class="secondary-action"
                  :disabled="busy"
                  title="清除这条 AI 建议（标注保持不变）"
                  @click="rejectSuggestion(selectedItem)"
                >
                  <RotateCcw :size="16" />撤销
                </button>
              </span>
            </div>
          </section>

          <section class="action-card">
            <span class="action-card-title">改判</span>
            <div class="action-card-body">
              <label>
                改判角色
                <select
                  :value="selectedItem.characterId ?? ''"
                  :disabled="busy"
                  @change="onRelabelCharacter"
                >
                  <option value="">未标记</option>
                  <option v-for="character in characters" :key="character.id" :value="character.id">
                    {{ character.name }}
                  </option>
                </select>
              </label>
              <label>
                改分类
                <select
                  :value="selectedItem.classification"
                  :disabled="busy"
                  @change="onRelabelClassification"
                >
                  <option value="person">人物</option>
                  <option value="scene">游戏截图</option>
                  <option value="private">收藏图</option>
                </select>
              </label>
            </div>
          </section>

          <section v-if="view === 'characters' && selectedCharacter" class="action-card">
            <span class="action-card-title">代表头像</span>
            <div class="action-card-body avatar-actions">
              <button
                type="button"
                class="primary-action"
                :disabled="busy || !canSetSelectedAsAvatar || selectedItemIsRepresentativeAvatar"
                @click="setSelectedAsRepresentativeAvatar"
              >
                <UserRound :size="16" />
                {{ selectedItemIsRepresentativeAvatar ? "当前代表头像" : "设为代表头像" }}
              </button>
              <button
                v-if="selectedCharacter.avatarAssetId"
                type="button"
                class="secondary-action"
                :disabled="busy"
                @click="setRepresentativeAvatar(null)"
              >
                <RotateCcw :size="16" />清除代表头像
              </button>
              <span class="action-hint">
                <template v-if="canSetSelectedAsAvatar">
                  使用当前截图的人脸裁剪；只影响角色卡片展示，不修改截图或归档文件。
                </template>
                <template v-else>
                  当前截图需要完成人脸处理和归档后，才能设为代表头像。
                </template>
              </span>
            </div>
          </section>

          <section v-if="canReprocess(selectedItem)" class="action-card">
            <span class="action-card-title">处理</span>
            <div class="action-card-body">
              <button type="button" class="primary-action" :disabled="busy" @click="reprocessDegraded(selectedItem)">
                <Sparkles :size="16" />重新识别
              </button>
              <span class="action-hint">重新运行视觉引擎，为这张降级归档的人物图补全标注与特征。</span>
            </div>
          </section>

          <section class="action-card">
            <span class="action-card-title">定位文件</span>
            <div class="action-card-body">
              <button type="button" class="path-button" @click="reveal(selectedItem.sourcePath)">
                <Image :size="15" />原图<ExternalLink :size="12" />
              </button>
              <button v-if="selectedItem.destinationPath" type="button" class="path-button" @click="reveal(selectedItem.destinationPath)">
                <Archive :size="15" />归档图<ExternalLink :size="12" />
              </button>
              <button
                v-if="selectedItem.destinationAvatarPath"
                type="button"
                class="path-button"
                @click="reveal(selectedItem.destinationAvatarPath)"
              >
                <UserRound :size="15" />头像归档<ExternalLink :size="12" />
              </button>
            </div>
          </section>
        </div>
      </aside>
      <aside v-else class="workbench-detail empty">选择一张截图查看详情。</aside>
    </div>

    <CharacterMergeDialog
      v-if="mergeOpen && selectedCharacter"
      :source="selectedCharacter"
      :characters="summaries"
      @close="closeMerge"
      @merged="onCharacterMerged"
    />

    <div v-if="renameOpen && selectedCharacter" class="rename-overlay" @click.self="renameOpen = false">
      <form class="rename-dialog" @submit.prevent="submitRename" @keydown.esc="renameOpen = false">
        <span class="eyebrow">Character</span>
        <h3>重命名角色</h3>
        <p>只影响之后归档的截图文件名；已归档文件与历史记录保持原名。角色与截图始终按 ID 关联，不受改名影响。</p>
        <input v-model="renameName" :disabled="renameBusy" placeholder="角色名称" autofocus />
        <div class="rename-actions">
          <button type="button" class="secondary-action" :disabled="renameBusy" @click="renameOpen = false">取消</button>
          <button type="submit" class="primary-action" :disabled="renameBusy || !renameName.trim()">保存</button>
        </div>
      </form>
    </div>
  </section>
</template>
