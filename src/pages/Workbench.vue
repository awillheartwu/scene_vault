<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  Archive,
  Ban,
  Check,
  ChevronsLeft,
  ChevronsRight,
  Eye,
  ExternalLink,
  Flag,
  Image,
  MoreHorizontal,
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
import PaginationControls from "@/components/common/PaginationControls.vue";
import PageHeader from "@/components/layout/PageHeader.vue";
import ResponsiveDetailPanel from "@/components/layout/ResponsiveDetailPanel.vue";
import { ContextMenu } from "@/components/ui/context-menu";
import { useContextMenu, type ContextMenuItem } from "@/composables/useContextMenu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
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
import {
  readWorkbenchSnapshot,
  writeWorkbenchSnapshot,
} from "@/lib/workbench-cache";

const persistedProjectId = localStorage.getItem("scene-vault.capture.project");
const cachedWorkbench = readWorkbenchSnapshot(persistedProjectId);
const projects = ref<Project[]>(cachedWorkbench?.projects ?? []);
const summaries = ref<CharacterSummary[]>(cachedWorkbench?.summaries ?? []);
const characters = ref<Character[]>(cachedWorkbench?.characters ?? []);
const projectId = ref(cachedWorkbench?.projectId ?? persistedProjectId ?? "");
const rebuildProgress = ref<{ processed: number; total: number } | null>(null);
const modelStatus = ref<FaceBankModelStatus | null>(cachedWorkbench?.modelStatus ?? null);
const selectedCharacterId = ref<string | null>(
  cachedWorkbench?.selectedCharacterId ??
    (projectId.value
      ? localStorage.getItem(`scene-vault.workbench.character.${projectId.value}`)
      : null),
);
const items = ref<CaptureItem[]>(cachedWorkbench?.items ?? []);
const samples = ref<FaceSample[]>(cachedWorkbench?.samples ?? []);
const selectedItemId = ref<string | null>(cachedWorkbench?.selectedItemId ?? null);
const previewVariant = ref<"source" | "annotated" | "avatar" | "destination">("source");
const loading = ref(!cachedWorkbench);
const busy = ref(false);
const renameOpen = ref(false);
const renameName = ref("");
const renameBusy = ref(false);
const mergeOpen = ref(false);
const detailOpen = ref(false);
const mergeButton = ref<HTMLButtonElement | null>(null);
let unlisteners: UnlistenFn[] = [];
let initialized = false;
let componentActive = true;

type WorkbenchView = "characters" | "unclassified" | "scene" | "private";
const view = ref<WorkbenchView>("characters");
const panelCollapsed = ref(false);
const showPrivate = ref(cachedWorkbench?.showPrivate ?? false);
const itemPage = ref(cachedWorkbench?.itemPage ?? 1);
const itemPageSize = ref(cachedWorkbench?.itemPageSize ?? 100);
const itemTotal = ref(cachedWorkbench?.itemTotal ?? 0);
const characterMenu = useContextMenu();
const itemMenu = useContextMenu();

function onCharacterContext(event: MouseEvent, summary: CharacterSummary) {
  if (!characterMenu.open(event, buildCharacterItems(summary))) return;
  void selectCharacter(summary.id);
}

function buildCharacterItems(summary: CharacterSummary): ContextMenuItem[] {
  return [
    {
      id: "rename",
      label: "重命名…",
      icon: Pencil,
      disabled: busy.value || renameBusy.value,
      action: () => {
        selectedCharacterId.value = summary.id;
        openRename();
      },
    },
    {
      id: "merge",
      label: "合并到其他角色…",
      icon: Users,
      disabled: busy.value || summaries.value.length <= 1,
      action: () => {
        selectedCharacterId.value = summary.id;
        mergeOpen.value = true;
      },
    },
    {
      id: "batch-reject",
      label: "批量拒绝并登记",
      icon: Ban,
      separatorBefore: true,
      disabled: busy.value || summary.pendingReviewCount === 0,
      action: () => {
        selectedCharacterId.value = summary.id;
        void batchRejectAndEnroll();
      },
    },
    {
      id: "reprocess",
      label: "当前角色重新识别",
      icon: Sparkles,
      disabled: busy.value || summary.degradedCount === 0,
      action: () => {
        selectedCharacterId.value = summary.id;
        void batchReprocess(summary.id);
      },
    },
  ];
}

function onItemContext(event: MouseEvent, item: CaptureItem) {
  if (!itemMenu.open(event, buildItemItems(item))) return;
  selectedItemId.value = item.id;
}

function buildItemItems(item: CaptureItem): ContextMenuItem[] {
  const items: ContextMenuItem[] = [
    { id: "detail", label: "查看详情", icon: Eye, action: () => openItemDetail(item.id) },
  ];
  if ((view.value === "characters" || view.value === "unclassified") && item.reviewStatus === "pending") {
    items.push(
      {
        id: "accept",
        label: "确认建议",
        icon: Check,
        separatorBefore: true,
        disabled: busy.value,
        action: () => void acceptSuggestion(item),
      },
      {
        id: "reject",
        label: "拒绝建议",
        icon: X,
        disabled: busy.value,
        action: () => void rejectSuggestion(item),
      },
      {
        id: "reject-enroll",
        label: "拒绝并登记",
        icon: UserPlus,
        disabled: busy.value,
        action: () => void rejectAndEnroll(item),
      },
    );
  }
  if (canRefreshFaceFeature(item)) {
    items.push({
      id: "refresh-feature",
      label: "重新提取人脸特征",
      icon: RefreshCw,
      separatorBefore: items.length > 1,
      disabled: busy.value,
      action: () => void refreshFaceFeature(item),
    });
  }
  if (canReprocess(item)) {
    items.push({
      id: "reprocess",
      label: "重新识别",
      icon: Sparkles,
      disabled: busy.value,
      action: () => void reprocessDegraded(item),
    });
  }
  if (
    view.value === "characters" &&
    item.characterId === selectedCharacterId.value &&
    item.assetId &&
    item.avatarPath &&
    selectedCharacter.value?.avatarAssetId !== item.assetId
  ) {
    items.push({
      id: "avatar",
      label: "设为代表头像",
      icon: UserRound,
      separatorBefore: true,
      disabled: busy.value,
      action: () => setRepresentativeAvatar(item.assetId!),
    });
  }
  const entrySample =
    view.value === "characters"
      ? (samples.value.find(
          (entry) => entry.captureItemId === item.id && entry.status === "active",
        ) ?? null)
      : null;
  if (entrySample) {
    items.push({
      id: "flag-sample",
      label: entrySample.flagged ? "恢复参与匹配" : "标记可疑",
      icon: Flag,
      disabled: busy.value,
      action: () => void toggleSampleFlagged(entrySample),
    });
  }
  if (item.classification !== "private") {
    items.push({
      id: "cover",
      label: currentProject.value?.coverCaptureItemId === item.id ? "取消项目封面" : "设为项目封面",
      icon: Image,
      separatorBefore: true,
      disabled: busy.value,
      action: () => void toggleProjectCover(),
    });
  }
  items.push({
    id: "reveal-source",
    label: "显示原图",
    icon: Image,
    action: () => reveal(item.sourcePath),
  });
  if (item.destinationPath) {
    items.push({
      id: "reveal-destination",
      label: "显示归档图",
      icon: Archive,
      action: () => reveal(item.destinationPath),
    });
  }
  return items;
}

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
const activeSampleCaptureIds = computed(
  () => new Set(samples.value.filter((sample) => sample.status === "active").map((sample) => sample.captureItemId)),
);
// The enrolled sample of the selected capture, if any. Flagged samples are
// excluded from matching until restored from the correction card.
const selectedItemSample = computed<FaceSample | null>(() => {
  const item = selectedItem.value;
  if (!item) return null;
  return (
    samples.value.find(
      (sample) => sample.captureItemId === item.id && sample.status === "active",
    ) ?? null
  );
});
const currentProject = computed(
  () => projects.value.find((project) => project.id === projectId.value) ?? null,
);
const selectedIsProjectCover = computed(
  () =>
    selectedItem.value != null &&
    currentProject.value?.coverCaptureItemId === selectedItem.value.id,
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
  if (value === "face_bank") return "人脸样本库";
  if (value === "vision") return "视觉引擎";
  if (value === "manual") return "手动";
  return "—";
}

function processingWarnings(item: CaptureItem): string[] {
  try {
    const value = JSON.parse(item.processingWarningsJson || "[]");
    return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string") : [];
  } catch {
    return [];
  }
}

function sampleIssueLabel(item: CaptureItem): string | null {
  if (item.faceCount === 0) return "未检测到脸";
  if (
    view.value !== "characters" ||
    item.classification !== "person" ||
    item.characterId !== selectedCharacterId.value ||
    activeSampleCaptureIds.value.has(item.id)
  ) {
    return null;
  }
  const warning = processingWarnings(item).find((entry) => entry.startsWith("sample_not_enrolled"));
  if (warning?.includes("sharpness")) return "清晰度不足";
  if (warning?.includes("area ratio")) return "人脸过小";
  return "未入样本";
}

function suggestionEmptyReason(item: CaptureItem): string {
  if (item.faceCount === 0) {
    return "视觉引擎未在这张图片中检测到可用人脸，因此无法生成匹配建议，也不会加入样本库。";
  }
  if (item.faceCount == null) {
    return item.status === "queued" || item.status === "processing"
      ? "人脸识别尚未完成，处理结束后会自动更新建议。"
      : "这张图片还没有可用于匹配的人脸特征。";
  }
  const warning = processingWarnings(item).find((entry) => entry.startsWith("sample_not_enrolled"));
  if (warning?.includes("sharpness")) {
    const values = warning.match(/sharpness ([\d.]+) below ([\d.]+)/);
    return values
      ? `检测到了人脸，但清晰度 ${values[1]} 低于样本门槛 ${values[2]}，因此不会加入样本库。`
      : "检测到了人脸，但清晰度未达到样本门槛，因此不会加入样本库。";
  }
  if (warning?.includes("area ratio")) {
    return "检测到了人脸，但人脸在画面中占比过小，因此不会加入样本库。";
  }
  if (item.classification === "person" && item.characterId) {
    return `已检测到 ${item.faceCount} 张脸。当前截图已归为「${characterName(item.characterId)}」，匹配时会排除该角色自身样本，只提示可能的错分；其他候选未达到建议门槛。`;
  }
  return `已检测到 ${item.faceCount} 张脸，但最佳候选没有同时达到相似度和区分度门槛。`;
}

function normalizeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function thumbnailItem(id: string): CaptureItem {
  return { id, sourcePath: "capture.png" } as CaptureItem;
}

function openItemDetail(id: string) {
  selectedItemId.value = id;
  detailOpen.value = true;
}

async function selectCharacter(characterId: string) {
  if (selectedCharacterId.value === characterId) return;
  selectedCharacterId.value = characterId;
  itemPage.value = 1;
  if (projectId.value) {
    localStorage.setItem(`scene-vault.workbench.character.${projectId.value}`, characterId);
  }
  selectedItemId.value = null;
  await loadItems();
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
  const initialProjectId = projectId.value;
  if (!cachedWorkbench) loading.value = true;
  try {
    const projectsLoad = captureApi.listProjects();
    const settingsLoad = captureApi.getAppSettings()
      .then((settings) => {
        showPrivate.value = settings.showPrivateByDefault;
      })
      .catch(() => {
        // Keep the default hidden state when settings are unavailable.
      });
    const dataLoad = initialProjectId
      ? loadCharacterData({ manageLoading: false })
      : Promise.resolve();
    const [nextProjects] = await Promise.all([projectsLoad, settingsLoad, dataLoad]);
    projects.value = nextProjects;
    const resolvedProjectId = nextProjects.some((project) => project.id === initialProjectId)
      ? initialProjectId
      : nextProjects[0]?.id ?? "";
    if (resolvedProjectId !== projectId.value) {
      projectId.value = resolvedProjectId;
      selectedCharacterId.value = resolvedProjectId
        ? localStorage.getItem(`scene-vault.workbench.character.${resolvedProjectId}`)
        : null;
      await loadCharacterData({ manageLoading: false });
    }
    if (projectId.value) localStorage.setItem("scene-vault.capture.project", projectId.value);
    cacheCurrentWorkbench();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    initialized = true;
    loading.value = false;
  }
}

interface LoadedWorkbenchItems {
  items: CaptureItem[];
  samples: FaceSample[];
  total: number;
}

async function fetchWorkbenchItems(
  projectIdValue: string,
  viewValue: WorkbenchView,
  characterIdValue: string | null,
  pageValue: number,
  pageSizeValue: number,
): Promise<LoadedWorkbenchItems> {
  if (viewValue === "characters") {
    if (!characterIdValue) return { items: [], samples: [], total: 0 };
    const [nextItems, nextSamples] = await Promise.all([
      captureApi.listCharacterCaptureItemsPage({
        projectId: projectIdValue,
        characterId: characterIdValue,
        page: pageValue,
        pageSize: pageSizeValue,
      }),
      captureApi.listCharacterFaceSamples(characterIdValue),
    ]);
    return { items: nextItems.items, samples: nextSamples, total: nextItems.total };
  }
  const nextItems = await captureApi.listCategoryItemsPage({
    projectId: projectIdValue,
    category: viewValue,
    page: pageValue,
    pageSize: pageSizeValue,
  });
  return { items: nextItems.items, samples: [], total: nextItems.total };
}

async function applyClampedPage(
  loaded: LoadedWorkbenchItems,
  projectIdValue: string,
  viewValue: WorkbenchView,
  characterIdValue: string | null,
): Promise<LoadedWorkbenchItems> {
  const pageValue = itemPage.value;
  const pageSizeValue = itemPageSize.value;
  const maxPage = Math.max(1, Math.ceil(loaded.total / pageSizeValue));
  if (pageValue > maxPage) {
    itemPage.value = maxPage;
    loaded = await fetchWorkbenchItems(
      projectIdValue,
      viewValue,
      characterIdValue,
      maxPage,
      pageSizeValue,
    );
  }
  itemTotal.value = loaded.total;
  return loaded;
}

async function fetchAndApplyItems(
  projectIdValue: string,
  viewValue: WorkbenchView,
  characterIdValue: string | null,
): Promise<LoadedWorkbenchItems> {
  const loaded = await fetchWorkbenchItems(
    projectIdValue,
    viewValue,
    characterIdValue,
    itemPage.value,
    itemPageSize.value,
  );
  return applyClampedPage(loaded, projectIdValue, viewValue, characterIdValue);
}

function cacheCurrentWorkbench() {
  if (!projectId.value || view.value !== "characters") return;
  writeWorkbenchSnapshot({
    projectId: projectId.value,
    projects: projects.value,
    summaries: summaries.value,
    characters: characters.value,
    modelStatus: modelStatus.value,
    selectedCharacterId: selectedCharacterId.value,
    items: items.value,
    samples: samples.value,
    selectedItemId: selectedItemId.value,
    showPrivate: showPrivate.value,
    itemPage: itemPage.value,
    itemPageSize: itemPageSize.value,
    itemTotal: itemTotal.value,
  });
}

async function loadCharacterData(options: { manageLoading?: boolean } = {}) {
  if (!projectId.value) {
    summaries.value = [];
    characters.value = [];
    items.value = [];
    modelStatus.value = null;
    selectedItemId.value = null;
    return;
  }
  const manageLoading = options.manageLoading ?? true;
  const projectIdValue = projectId.value;
  const viewValue = view.value;
  const requestedCharacterId = selectedCharacterId.value;
  if (manageLoading) loading.value = true;
  try {
    const [nextSummaries, nextCharacters, nextModelStatus, requestedItems] = await Promise.all([
      captureApi.listProjectCharacterSummaries(projectIdValue),
      captureApi.listCharacters(projectIdValue),
      (captureApi.getFaceBankModelStatus?.(projectIdValue) ??
        Promise.resolve(null)).catch(() => null),
      fetchWorkbenchItems(
        projectIdValue,
        viewValue,
        requestedCharacterId,
        itemPage.value,
        itemPageSize.value,
      ),
    ]);
    if (projectId.value !== projectIdValue || view.value !== viewValue) return;
    const resolvedCharacterId =
      requestedCharacterId && nextSummaries.some((summary) => summary.id === requestedCharacterId)
        ? requestedCharacterId
        : nextSummaries[0]?.id ?? null;
    let loadedItems = requestedItems;
    if (viewValue === "characters" && resolvedCharacterId !== requestedCharacterId) {
      itemPage.value = 1;
      loadedItems = await fetchAndApplyItems(projectIdValue, viewValue, resolvedCharacterId);
    } else {
      loadedItems = await applyClampedPage(
        requestedItems,
        projectIdValue,
        viewValue,
        resolvedCharacterId,
      );
    }
    if (projectId.value !== projectIdValue || view.value !== viewValue) return;

    summaries.value = nextSummaries;
    characters.value = nextCharacters;
    modelStatus.value = nextModelStatus;
    selectedCharacterId.value = resolvedCharacterId;
    items.value = loadedItems.items;
    samples.value = loadedItems.samples;
    if (resolvedCharacterId) {
      localStorage.setItem(
        `scene-vault.workbench.character.${projectIdValue}`,
        resolvedCharacterId,
      );
    }
    if (!items.value.some((item) => item.id === selectedItemId.value)) {
      selectedItemId.value = items.value[0]?.id ?? null;
    }
    cacheCurrentWorkbench();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    if (manageLoading) loading.value = false;
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
  const viewValue = view.value;
  const characterIdValue = selectedCharacterId.value;
  loading.value = true;
  try {
    const loaded = await fetchAndApplyItems(projectIdValue, viewValue, characterIdValue);
    if (
      projectId.value !== projectIdValue ||
      view.value !== viewValue ||
      selectedCharacterId.value !== characterIdValue
    ) return;
    items.value = loaded.items;
    samples.value = loaded.samples;
    if (
      items.value.length &&
      !items.value.some((item) => item.id === selectedItemId.value)
    ) {
      selectedItemId.value = items.value[0]?.id ?? null;
    }
    cacheCurrentWorkbench();
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

function canRefreshFaceFeature(item: CaptureItem): boolean {
  return (
    (item.classification === "person" || item.classification === "unclassified") &&
    item.status !== "queued" &&
    item.status !== "processing" &&
    item.status !== "archive_pending"
  );
}

function refreshFaceFeature(item: CaptureItem) {
  return runMutation(() => captureApi.refreshCaptureFaceFeature(item.id));
}

function toggleSampleFlagged(sample: FaceSample) {
  return runMutation(async () => {
    const updated = await captureApi.setFaceSampleFlagged(sample.id, !sample.flagged);
    toast.success(
      updated.flagged
        ? "已标记为可疑，这张样本不再参与匹配（可在修正卡恢复）。"
        : "已恢复参与匹配，这张样本重新参与相似度计算与建议。",
    );
  });
}

function toggleProjectCover() {
  const item = selectedItem.value;
  if (!item || !projectId.value) return;
  return runMutation(async () => {
    const clear = selectedIsProjectCover.value;
    const updated = await captureApi.setProjectCover(projectId.value, clear ? null : item.id);
    projects.value = projects.value.map((project) => (project.id === updated.id ? updated : project));
    toast.success(clear ? "已取消项目封面，首页恢复为最近截图。" : "已设为项目封面。");
  });
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
        `${summary.failed} 张失败，其中 ${summary.stalePreserved} 张保留旧样本；` +
        `已按当前匹配参数刷新 ${summary.suggestionsRefreshed} 张待分类图片的建议（共 ${summary.total} 张人物图）。`,
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
  if (!initialized) return;
  itemPage.value = 1;
  if (projectId.value) {
    localStorage.setItem("scene-vault.capture.project", projectId.value);
  }
  summaries.value = [];
  characters.value = [];
  items.value = [];
  samples.value = [];
  selectedItemId.value = null;
  selectedCharacterId.value = projectId.value
    ? localStorage.getItem(`scene-vault.workbench.character.${projectId.value}`)
    : null;
  await loadCharacterData();
});
watch(view, async () => {
  selectedItemId.value = null;
  itemPage.value = 1;
  await loadItems();
});
watch(selectedItemId, () => {
  previewVariant.value = "source";
});

function onItemPageChange(page: number) {
  itemPage.value = page;
  void loadItems();
}

function onItemPageSizeChange(size: number) {
  itemPageSize.value = size;
  itemPage.value = 1;
  void loadItems();
}
async function registerWorkbenchListeners() {
  // Listener registration crosses the Tauri bridge. It must never delay the
  // first data request, and all three registrations can run concurrently.
  const registrations = await Promise.allSettled([
    listen("capture:item-updated", () => {
      if (initialized) void loadCharacterData();
    }),
    listen("capture:face-bank-rebuilt", () => {
      // A rebuild refreshed features and suggestions for many items; the
      // completion event is the only signal, so reload the whole view.
      if (initialized) void loadCharacterData();
    }),
    listen<{ processed: number; total: number }>(
      "capture:face-bank-rebuild-progress",
      (event) => {
        rebuildProgress.value = event.payload;
      },
    ),
  ]);
  const listeners = registrations.flatMap((result) =>
    result.status === "fulfilled" ? [result.value] : [],
  );
  if (componentActive) {
    unlisteners.push(...listeners);
  } else {
    listeners.forEach((unlisten) => unlisten());
  }
}

onMounted(() => {
  componentActive = true;
  void registerWorkbenchListeners();
  void initialize();
});

onBeforeUnmount(() => {
  componentActive = false;
  unlisteners.forEach((unlisten) => unlisten());
  unlisteners = [];
});
</script>

<template>
  <section class="workbench-page">
    <PageHeader
      eyebrow="人物与识别"
      title="人物工作台"
      description="按角色复查截图与识别建议；改判后自动重新处理并替换归档。"
    >
      <template #actions>
        <label class="workbench-project">
          项目
          <select v-model="projectId" :disabled="loading">
            <option v-for="project in projects" :key="project.id" :value="project.id">
              {{ project.name }}
            </option>
          </select>
        </label>
        <button type="button" class="secondary-action" :disabled="loading" @click="loadCharacterData()">
          <RefreshCw :size="17" :class="{ 'animate-spin': loading }" />刷新
        </button>
        <DropdownMenu>
          <DropdownMenuTrigger as-child>
            <button type="button" class="icon-action" aria-label="人物维护" title="人物维护">
              <MoreHorizontal :size="18" aria-hidden="true" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" class="workbench-maintenance-menu">
            <DropdownMenuItem
              title="重新提取特征，保留截图当前角色绑定，并按当前匹配参数刷新待分类建议"
              :disabled="busy || !projectId"
              @select="rebuildFaceBank"
            >
              <Sparkles :size="15" aria-hidden="true" />
              重建人脸样本库
              <span v-if="rebuildProgress">{{ rebuildProgress.processed }}/{{ rebuildProgress.total }}</span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </template>
    </PageHeader>
    <div v-if="modelStatus && !modelStatus.compatible" class="model-mismatch-banner">
      <span>
        样本库共有 {{ modelStatus.sampleCount }} 条，其中
        {{ modelStatus.incompatibleSampleCount }} 条与当前识别器
        {{ modelStatus.activeModelId }}（{{ modelStatus.activeModelVersion }}）不兼容；
        这些旧样本不会参与 AI 建议，切换识别器后新截图在重建前也不会产生建议。
        重新提取后即可清理。
      </span>
      <button
        type="button"
        class="secondary-action compact-action"
        title="重新提取特征，保留截图当前角色绑定，并按当前匹配参数刷新待分类建议"
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
          <Users :size="16" /><span>角色</span><span class="count">{{ loading && !summaries.length ? "…" : summaries.length }}</span>
        </div>
        <div
          v-if="loading && !summaries.length"
          class="workbench-character-skeletons"
          aria-label="正在加载角色"
          aria-live="polite"
        >
          <div v-for="index in 6" :key="index" class="character-card-skeleton" aria-hidden="true">
            <span class="workbench-skeleton character-skeleton-avatar" />
            <span class="character-skeleton-copy">
              <span class="workbench-skeleton character-skeleton-name" />
              <span class="workbench-skeleton character-skeleton-meta" />
            </span>
          </div>
        </div>
        <template v-else>
        <button
          v-for="summary in summaries"
          :key="summary.id"
          type="button"
          class="character-card"
          :class="{ selected: selectedCharacterId === summary.id }"
          @click="selectCharacter(summary.id)"
          @contextmenu="onCharacterContext($event, summary)"
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
        </template>
        <div v-else class="workbench-category-note">
          <strong>{{ viewLabel }}</strong>
          <span>{{ itemTotal }} 张截图</span>
          <p v-if="view === 'unclassified'">等待分类的截图会出现在这里，可在右侧直接标记角色或分类。</p>
          <p v-else-if="view === 'scene'">标记为游戏截图的原图直存归档。</p>
          <p v-else>标记为收藏的截图默认隐藏，可在设置中开启本标签页。</p>
        </div>
      </aside>

      <div class="workbench-grid-column">
        <div class="workbench-grid-header">
          <div class="workbench-grid-header-title">
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
              {{ loading && !selectedCharacter ? "正在加载人物…" : viewLabel }}
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
            <span class="workbench-screenshot-count">{{ loading && !itemTotal ? "—" : itemTotal }} 张截图</span>
          </div>
          <div v-if="view === 'characters' && selectedCharacter" class="workbench-grid-actions">
            <button
              ref="mergeButton"
              type="button"
              class="secondary-action compact-action"
              :disabled="busy || summaries.length <= 1"
              :title="summaries.length > 1 ? `将 ${selectedCharacter.name} 合并到另一个角色` : '当前项目只有一个角色，无法合并'"
              @click="mergeOpen = true"
            >
              <Users :size="14" />合并角色
            </button>
            <button
              v-if="selectedCharacter.pendingReviewCount > 0"
              type="button"
              class="secondary-action compact-action"
              :disabled="busy"
              title="否决当前角色全部待确认建议，并把对应人脸登记进该角色样本库"
              @click="batchRejectAndEnroll"
            >
              <Ban :size="14" />批量拒绝并登记 ({{ selectedCharacter.pendingReviewCount }})
            </button>
            <button
              v-if="selectedCharacter.degradedCount > 0"
              type="button"
              class="secondary-action compact-action"
              :disabled="busy"
              @click="batchReprocess(selectedCharacter.id)"
            >
              <Sparkles :size="14" />当前角色重新识别 ({{ selectedCharacter.degradedCount }})
            </button>
            <button
              v-if="projectDegradedCount > 0"
              type="button"
              class="secondary-action compact-action"
              :disabled="busy"
              @click="batchReprocess(null)"
            >
              <Sparkles :size="14" />全部重新识别 ({{ projectDegradedCount }})
            </button>
          </div>
        </div>
        <CaptureProgress />
        <div
          class="workbench-grid"
          role="list"
          aria-label="角色截图网格"
          :aria-busy="loading && !items.length"
        >
          <template v-if="loading && !items.length">
            <div
              v-for="index in 6"
              :key="`loading-${index}`"
              class="workbench-grid-skeleton-card"
              aria-hidden="true"
            >
              <span class="workbench-skeleton workbench-grid-skeleton-image" />
              <span class="workbench-grid-skeleton-copy">
                <span class="workbench-skeleton workbench-grid-skeleton-name" />
                <span class="workbench-skeleton workbench-grid-skeleton-chip" />
              </span>
            </div>
          </template>
          <template v-else>
          <button
            v-for="item in items"
            :key="item.id"
            type="button"
            role="listitem"
            class="workbench-cell"
            :class="{ selected: selectedItemId === item.id }"
            @click="openItemDetail(item.id)"
            @contextmenu="onItemContext($event, item)"
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
              <span
                v-if="sampleIssueLabel(item)"
                class="face-status-chip"
                :data-status="item.faceCount === 0 ? 'no-face' : 'not-enrolled'"
              >
                {{ sampleIssueLabel(item) }}
              </span>
              <span v-if="item.reviewStatus === 'pending'" class="review-chip">建议待确认</span>
            </span>
          </button>
          <div v-if="!items.length && !loading" class="workbench-empty">该角色名下还没有截图。</div>
          </template>
        </div>

        <PaginationControls
          v-if="itemTotal > 0"
          :page="itemPage"
          :page-size="itemPageSize"
          :total="itemTotal"
          @update:page="onItemPageChange"
          @update:page-size="onItemPageSizeChange"
        />
      </div>

      <ResponsiveDetailPanel
        v-if="selectedItem"
        v-model:open="detailOpen"
        title="截图详情"
        :description="pathFileName(selectedItem.sourcePath)"
        panel-class="workbench-detail"
      >
        <section
          v-if="view === 'characters' && selectedCharacter"
          class="sample-summary-bar"
          aria-label="人脸样本库与代表头像"
        >
          <div class="sample-summary-copy">
            <span class="eyebrow">人脸样本库</span>
            <strong>{{ selectedCharacter.sampleCount }} 条样本</strong>
            <span v-if="sampleIssueLabel(selectedItem)" class="sample-current-status">
              当前图：{{ sampleIssueLabel(selectedItem) }}
            </span>
          </div>
          <div class="sample-summary-actions">
            <button
              type="button"
              class="secondary-action compact-action"
              :disabled="busy || !canSetSelectedAsAvatar || selectedItemIsRepresentativeAvatar"
              :title="canSetSelectedAsAvatar ? '使用当前截图的人脸裁剪作为角色头像' : '当前截图需要完成人脸处理和归档'"
              @click="setSelectedAsRepresentativeAvatar"
            >
              <UserRound :size="15" />{{ selectedItemIsRepresentativeAvatar ? "当前代表头像" : "设为代表头像" }}
            </button>
            <button
              v-if="selectedCharacter.avatarAssetId"
              type="button"
              class="secondary-action compact-action avatar-restore-action"
              title="清除手工代表头像，改由最近一张完成归档的人物图自动担任"
              :disabled="busy"
              @click="setRepresentativeAvatar(null)"
            >
              <RotateCcw :size="15" aria-hidden="true" />恢复自动头像
            </button>
          </div>
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
              <span v-else class="suggestion-status empty">
                <strong>暂无建议</strong>
                <span>{{ suggestionEmptyReason(selectedItem) }}</span>
              </span>
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
            <div class="action-card-body correction-card-body">
              <div class="action-card-fields">
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
              <div class="action-card-actions">
                <button
                  type="button"
                  class="secondary-action face-feature-action"
                  :disabled="busy || !canRefreshFaceFeature(selectedItem)"
                  title="只重新提取这张截图的人脸特征：保留当前角色与分类，不生成标注/头像，也不重新归档；完成后按当前匹配参数刷新建议"
                  @click="refreshFaceFeature(selectedItem)"
                >
                  <RefreshCw :size="16" />重新提取人脸特征
                </button>
                <button
                  v-if="selectedItemSample"
                  type="button"
                  class="secondary-action"
                  :disabled="busy"
                  :title="
                    selectedItemSample.flagged
                      ? '取消可疑标记，让这张样本重新参与匹配与相似度计算'
                      : '把这张样本标记为可疑，不再参与匹配；确认无误后可在修正卡恢复'
                  "
                  @click="toggleSampleFlagged(selectedItemSample)"
                >
                  <Flag :size="16" />{{ selectedItemSample.flagged ? "恢复参与匹配" : "标记可疑" }}
                </button>
                <button
                  v-if="selectedItem.classification !== 'private'"
                  type="button"
                  class="secondary-action"
                  :disabled="busy"
                  :title="
                    selectedIsProjectCover
                      ? '清除自定义封面，首页恢复为最近截图'
                      : '把当前截图设为项目封面（首页网格与列表都会使用这张图；收藏图不会作为封面）'
                  "
                  @click="toggleProjectCover"
                >
                  <Image :size="16" />{{ selectedIsProjectCover ? "取消项目封面" : "设为项目封面" }}
                </button>
              </div>
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
      </ResponsiveDetailPanel>
      <aside
        v-else-if="loading"
        class="workbench-detail empty loading"
        aria-label="正在加载截图详情"
        aria-live="polite"
      >
        <div class="workbench-detail-skeleton" aria-hidden="true">
          <span class="workbench-skeleton detail-skeleton-toolbar" />
          <span class="workbench-skeleton detail-skeleton-preview" />
          <span v-for="index in 5" :key="index" class="workbench-skeleton detail-skeleton-line" />
        </div>
        <span class="sr-only">正在加载人物资料</span>
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

    <Dialog
      v-if="selectedCharacter"
      :open="renameOpen"
      @update:open="!$event && !renameBusy && (renameOpen = false)"
    >
      <DialogContent class="rename-dialog" :show-close-button="false" aria-label="重命名角色">
        <span class="eyebrow">角色管理</span>
        <DialogTitle>重命名角色</DialogTitle>
        <DialogDescription>
          只影响之后归档的截图文件名；已归档文件与历史记录保持原名。角色与截图始终按 ID 关联，不受改名影响。
        </DialogDescription>
        <form @submit.prevent="submitRename">
          <input v-model="renameName" :disabled="renameBusy" placeholder="角色名称" autofocus />
          <div class="rename-actions">
            <button type="button" class="secondary-action" :disabled="renameBusy" @click="renameOpen = false">取消</button>
            <button type="submit" class="primary-action" :disabled="renameBusy || !renameName.trim()">保存</button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
    <ContextMenu :menu="characterMenu" />
    <ContextMenu :menu="itemMenu" />
  </section>
</template>
