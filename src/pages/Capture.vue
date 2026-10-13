<script setup lang="ts">
import { isOlderCapture } from "@/lib/capture-api";
import FaceRoiSelector from '@/components/capture/FaceRoiSelector.vue';
import CharacterAvatar from '@/components/character/CharacterAvatar.vue';
import { useCaptureFaceRoi } from '@/composables/useCaptureFaceRoi';
import { useCapturePreview } from '@/composables/useCapturePreview';
import { parseFaceBox } from '@/lib/face-box';
import {
  characterMatchesSpeaker,
  findCharacterBySpeaker,
  isLikelySpeakerName,
} from "@/lib/speaker-suggestion";
import { describeError } from '@/lib/vision-errors';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, watchEffect } from "vue";
import { useTauriEventListeners } from "@/composables/useTauriEventListeners";
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
  Plug,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  Sparkles,
  Trash2,
  Unplug,
  UserPlus,
  UserRound,
  Wifi,
} from "@lucide/vue";
import CaptureThumbnail from "@/components/capture/CaptureThumbnail.vue";
import CaptureProgress from "@/components/capture/CaptureProgress.vue";
import CaptureDeleteDialog from "@/components/capture/CaptureDeleteDialog.vue";
import RenpyScreenshotSettings from "@/components/capture/RenpyScreenshotSettings.vue";
import {
  captureFileIssues,
  captureVariantReadReason,
} from "@/components/capture/capture-file-state";
import ResponsiveDetailPanel from "@/components/layout/ResponsiveDetailPanel.vue";
import { ContextMenu } from "@/components/ui/context-menu";
import { useAdaptiveLayout } from "@/composables/useAdaptiveLayout";
import { useContextMenu, type ContextMenuItem } from "@/composables/useContextMenu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import ProjectRenameDialog from "@/components/project/ProjectRenameDialog.vue";
import ProjectSelect from "@/components/project/ProjectSelect.vue";
import { useProjectEngineBadges } from "@/composables/useProjectEngineBadges";
import {
  captureApi,
  engineApi,
  captureClassificationLabel,
  captureStatusLabel,
  pathFileName,
  pickDirectory,
  revealPath,
  type CaptureItem,
  type CaptureClassification,
  type CaptureEngineMetadata,
  type CaptureImageFacts,
  type CaptureRuntimeStatus,
  type CaptureSession,
  type Character,
  type Project,
  type ProjectSourceDirectory,
  type ProjectEngineIntegrationView,
  type ProjectGameLaunchOption,
  type CapturePolicyView,
  type RenpyScreenshotPolicyView,
  type UnimportedCapture,
  type VerificationResult,
} from "@/lib/capture-api";
import { toast } from "@/lib/toast";
import CaptureEngineChips from "@/components/capture/CaptureEngineChips.vue";
import { archiveFormatLabel, archiveSizeLabel } from "@/lib/image-facts";

const { isWideLayout } = useAdaptiveLayout();

const projects = ref<Project[]>([]);
const { engineVersions, refreshEngineBadges } = useProjectEngineBadges();
const characters = ref<Character[]>([]);
const sessions = ref<CaptureSession[]>([]);
const items = ref<CaptureItem[]>([]);
const runtime = ref<CaptureRuntimeStatus | null>(null);
const engineStatusLabel = computed(() => {
  switch (runtime.value?.engineStatus) {
    case "configured":
      return "AI 已配置";
    case "sidecar":
      return "内置引擎";
    default:
      return "AI 未配置";
  }
});
const engineOnline = computed(
  () =>
    runtime.value?.engineStatus === "configured" || runtime.value?.engineStatus === "sidecar",
);
const projectId = ref("");
const sourceDirs = ref<ProjectSourceDirectory[]>([]);
const sourceDirectoriesOpen = ref(false);
const projectDestination = ref("");
const selectedItemId = ref<string | null>(null);
const selectedCharacterId = ref<string | null>(null);
const pendingClassification = ref<CaptureClassification | null>(null);
const verificationWarning = ref<VerificationResult | null>(null);
const search = ref("");
const loading = ref(true);
const busy = ref(false);
const importCandidates = ref<UnimportedCapture[] | null>(null);
const deleteTarget = ref<CaptureItem | null>(null);
const itemMenu = useContextMenu();

function buildItemMenu(item: CaptureItem): ContextMenuItem[] {
  const items: ContextMenuItem[] = [];
  if (item.status === "awaiting_label") {
    items.push({
      id: "label",
      label: "标记角色/分类",
      icon: UserRound,
      disabled: busy.value || importBusy.value,
      action: () => {
        void submitClassification("person");
      },
    });
  }
  if (item.status === "failed") {
    items.push({
      id: "retry",
      label: "重新识别",
      icon: RefreshCw,
      disabled: busy.value || importBusy.value,
      action: () => retryItem(item),
    });
  }
  items.push({
    id: "reveal-source",
    label: "显示原图",
    icon: Image,
    disabled: captureVariantReadReason(item, "source") !== null,
    action: () => void reveal(item.sourcePath),
  });
  if (item.destinationPath) {
    items.push({
      id: "reveal-destination",
      label: "显示归档图",
      icon: Archive,
      disabled: captureVariantReadReason(item, "destination") !== null,
      action: () => void reveal(item.destinationPath),
    });
  }
  if (projectId.value && item.destinationPath && item.classification !== "private") {
    items.push({
      id: "set-cover",
      label: "设为项目封面",
      icon: FolderOpen,
      separatorBefore: true,
      disabled: busy.value || importBusy.value,
      action: () => setProjectCover(item),
    });
  }
  if (canDeleteItem(item)) {
    items.push({
      id: "delete-item",
      label: "从 Scene Vault 移除…",
      icon: Trash2,
      danger: true,
      separatorBefore: true,
      disabled: busy.value || importBusy.value,
      action: () => {
        deleteTarget.value = item;
      },
    });
  }
  return items;
}

function canDeleteItem(item: CaptureItem): boolean {
  return !["queued", "processing", "archive_pending"].includes(item.status);
}

async function reveal(path: string | null) {
  if (!path) return;
  try {
    await revealPath(path);
  } catch (error) {
    toast.error(normalizeError(error));
  }
}

function onItemContext(event: MouseEvent, item: CaptureItem) {
  if (!itemMenu.open(event, buildItemMenu(item))) return;
  selectedItemId.value = item.id;
}

async function setProjectCover(item: CaptureItem) {
  if (!projectId.value) return;
  try {
    await captureApi.setProjectCover(projectId.value, item.id);
    toast.success("已设为项目封面。");
  } catch (error) {
    toast.error(normalizeError(error));
  }
}
const importBusy = ref(false);
const deferredImportCount = ref(0);
const showProjectForm = ref(false);
const renameTarget = ref<Project | null>(null);
const labelPanelOpen = ref(false);
const sessionToolsOpen = ref(true);
const showCharacterForm = ref(false);
const newProjectName = ref("");
const newCharacterName = ref("");
const { listen, isActive } = useTauriEventListeners();
const stripRow = ref<HTMLElement | null>(null);
const stripBar = ref<HTMLElement | null>(null);
const stripThumb = ref<HTMLElement | null>(null);
const stripDragging = ref(false);
const stripThumbWidth = ref(0);
const stripThumbOffset = ref(0);
let stripObserver: ResizeObserver | null = null;

const activeSession = computed(() => sessions.value.find((session) => session.status === "active") ?? null);
const selectedItem = computed(() => stripItems.value.find((item) => item.id === selectedItemId.value) ?? stripItems.value[0] ?? null);
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
    .filter((item) => item.status !== 'completed')
    .sort((a, b) => {
      const pa = priority[a.status] ?? 3;
      const pb = priority[b.status] ?? 3;
      if (pa !== pb) return pa - pb;
      return new Date(a.capturedAt).getTime() - new Date(b.capturedAt).getTime();
    });
});
const { roi: faceRoi, busy: roiBusy, blocked: roiBlocked, loading: roiLoading, error: roiError, revision: roiRevision, save: saveFaceRoi } = useCaptureFaceRoi(selectedItem, upsertItem);
const { url: previewUrl, loading: previewLoading, unavailable: previewUnavailable, load: loadPreview } = useCapturePreview(
  selectedItem,
  () => selectedItem.value?.status === "completed" && selectedItem.value.destinationPath ? "destination" : "source",
  message => toast.error(message),
);
watch(roiRevision, () => { verificationWarning.value = null; });
// The last detected primary face: framing starts from it instead of an
// arbitrary centered box.
const detectedFaceBox = computed(() => parseFaceBox(selectedItem.value?.faceBoxJson));
const suggestedCharacter = computed(() => {
  const item = selectedItem.value;
  if (roiBusy.value || roiBlocked.value) return null;
  if (!item || item.reviewStatus !== "pending" || !item.suggestedCharacterId) return null;
  return characters.value.find((character) => character.id === item.suggestedCharacterId) ?? null;
});

// Ren'Py bridge metadata for the selected capture: speaker, dialogue,
// resolutions and extracted originals. Absent for every non-bridged capture.
const engineMetadata = ref<CaptureEngineMetadata | null>(null);
let engineMetadataRequest = 0;
let imageFactsRequest = 0;
const engineSuggestedCharacter = computed(() => {
  // Live match first: a character created or renamed after the capture still
  // matches the speaker name. The stored suggestion is only reused while that
  // character still answers to the speaker; a renamed-away character must not
  // be recommended for its old speaker.
  const live = findCharacterBySpeaker(characters.value, engineMetadata.value?.speakerRaw);
  if (live) return live;
  const suggestedId = engineMetadata.value?.suggestedCharacterId;
  if (suggestedId) {
    const stored = characters.value.find((character) => character.id === suggestedId);
    if (stored && characterMatchesSpeaker(stored, engineMetadata.value?.speakerRaw)) {
      return stored;
    }
  }
  return null;
});
const speakerName = computed(() => engineMetadata.value?.speakerRaw?.trim() ?? "");
const canCreateCharacterFromSpeaker = computed(
  () =>
    pendingClassification.value === "person" &&
    isLikelySpeakerName(speakerName.value) &&
    !engineSuggestedCharacter.value,
);

// A manual pick (or an explicit clear) always wins over the auto-preselect:
// the engine suggestion only fills an empty selection when the vision
// suggestion is absent, and never steals a choice back.
const characterTouchedFor = ref<string | null>(null);

function pickCharacter(characterId: string | null) {
  if (selectedItem.value) characterTouchedFor.value = selectedItem.value.id;
  // A manual pick (even the same character the auto-pick chose) is no longer
  // automatic: it must survive switching images.
  autoPickedCharacter.value = null;
  selectedCharacterId.value = characterId;
}

// The automatically picked suggestion of one capture, so a later, stronger
// result (vision over dialogue) can revise it — as long as the user has not
// picked anything themselves.
const autoPickedCharacter = ref<{ itemId: string; characterId: string } | null>(null);

watchEffect(() => {
  const item = selectedItem.value;
  if (!item || pendingClassification.value !== "person") return;
  if (characterTouchedFor.value === item.id) return;
  const preferred = suggestedCharacter.value?.id ?? engineSuggestedCharacter.value?.id ?? null;
  const auto =
    autoPickedCharacter.value && autoPickedCharacter.value.itemId === item.id
      ? autoPickedCharacter.value.characterId
      : null;
  if (auto) {
    if (selectedCharacterId.value === auto && preferred !== auto) {
      selectedCharacterId.value = preferred;
      autoPickedCharacter.value = preferred ? { itemId: item.id, characterId: preferred } : null;
    }
    return;
  }
  if (selectedCharacterId.value) return;
  // The dialogue match fills an empty selection automatically when the vision
  // suggestion is absent — or when both point at the same character (a double
  // match is the strongest signal and may pre-select). The vision suggestion
  // alone stays a human choice.
  const engine = engineSuggestedCharacter.value;
  const agrees =
    suggestedCharacter.value !== null &&
    engine !== null &&
    suggestedCharacter.value.id === engine.id;
  if (engine && (!suggestedCharacter.value || agrees)) {
    selectedCharacterId.value = engine.id;
    autoPickedCharacter.value = { itemId: item.id, characterId: engine.id };
  }
});

async function loadEngineMetadata() {
  const item = selectedItem.value;
  // Selection changes — including clearing the selection — invalidate older
  // responses, and the previous capture’s context is dropped immediately so
  // its speaker suggestion can never be clicked for the new image.
  const token = ++engineMetadataRequest;
  engineMetadata.value = null;
  if (!item) return;
  try {
    const metadata = await engineApi.getCaptureMetadata(item.id);
    if (token !== engineMetadataRequest || selectedItem.value?.id !== item.id) return;
    engineMetadata.value = metadata;
  } catch {
    if (token === engineMetadataRequest && selectedItem.value?.id === item.id) {
      engineMetadata.value = null;
    }
  }
}

async function loadImageFacts() {
  const item = selectedItem.value;
  const token = ++imageFactsRequest;
  imageFacts.value = null;
  if (!item) return;
  try {
    const facts = await captureApi.getCaptureImageFacts(item.id);
    if (token === imageFactsRequest && selectedItem.value?.id === item.id) {
      imageFacts.value = facts;
    }
  } catch {
    if (token === imageFactsRequest && selectedItem.value?.id === item.id) {
      imageFacts.value = null;
    }
  }
}

// Ren'Py plugin state for the session toolbar: detection runs whenever a
// source directory is added, and a clean Ren'Py game is installed silently.
const engineIntegrations = ref<ProjectEngineIntegrationView[]>([]);
const engineDetectedRoot = ref<string | null>(null);
const engineWorking = ref(false);
const screenshotSettingsOpen = ref(false);
const screenshotSettingsSaving = ref(false);
const screenshotPolicy = ref<RenpyScreenshotPolicyView | null>(null);
const screenshotSettingsLoading = ref(false);
const screenshotSettingsLoadError = ref<string | null>(null);
const screenshotSettingsWarning = ref<string | null>(null);
const engineError = ref<string | null>(null);
const engineConfirmUninstall = ref(false);
const engineConfirmUninstallId = ref<string | null>(null);
const engineUninstallOpen = ref(false);
const gameLaunchOptions = ref<ProjectGameLaunchOption[]>([]);
const gameLaunchOpen = ref(false);
const engineInstalled = computed(() => engineIntegrations.value.length > 0);
const engineStaleInstalls = computed(() =>
  engineIntegrations.value.filter(
    (view) => view.integration.bridgeVersion !== view.bundledBridgeVersion,
  ),
);
const archivePolicy = ref<CapturePolicyView | null>(null);
const archiveSizeChoice = ref("inherit");
const archiveFormatChoice = ref("inherit");
const archiveQualityInherit = ref(true);
const archiveQualityChoice = ref(90);
const archivePolicyOpen = ref(false);
const archivePolicySaving = ref(false);
const archivePolicyDisabled = computed(() =>
  !projectId.value || !archivePolicy.value || Boolean(activeSession.value) || archivePolicySaving.value,
);
const archiveUsesLossyQuality = computed(() => {
  const format = archiveFormatChoice.value === "inherit"
    ? archivePolicy.value?.globalFormat
    : archiveFormatChoice.value;
  return format === "jpeg" || format === "keep";
});
const archivePolicySummary = computed(() => {
  const policy = archivePolicy.value;
  if (!policy) return "归档规格暂不可用";
  const format = archiveFormatLabel(policy.resolvedFormat);
  const quality = policy.resolvedFormat === "jpeg"
    ? ` q${policy.resolvedQuality}` : "";
  return `${archiveSizeLabel(policy.resolvedSize)} · ${format}${quality}`;
});
const imageFacts = ref<CaptureImageFacts | null>(null);

const engineButtonTitle = computed(() => {
  if (engineError.value) return engineError.value;
  const stale = engineStaleInstalls.value[0];
  if (stale) {
    return `插件版本过旧（${stale.integration.bridgeVersion} → ${stale.bundledBridgeVersion}），点击重新安装`;
  }
  const missing = engineIntegrations.value.find(
    (view) => !view.bridgeFilePresent || !view.compiledFilePresent,
  );
  if (missing) {
    return `插件文件缺失（可能被游戏更新或校验移除）：${missing.integration.engineRoot}。点击重新安装`;
  }
  const warned = engineIntegrations.value.find((view) => view.warning);
  if (warned?.warning) return warned.warning;
  if (engineIntegrations.value.length) {
    const roots = engineIntegrations.value
      .map((view) => view.integration.engineRoot)
      .join("、");
    return `${engineIntegrations.value.length} 个 Ren'Py 插件已安装：${roots}。点击可安装检测到的其他游戏或重装`;
  }
  if (engineDetectedRoot.value) return "检测到 Ren'Py 游戏，可安装插件";
  return "未检测到 Ren'Py 游戏；点击可手动选择游戏目录";
});

let engineStateRequest = 0;
let engineIntegrationsRequest = 0;
let screenshotSettingsRequest = 0;
let screenshotSettingsLoadRequest = 0;
let screenshotSettingsDisposed = false;
let screenshotSettingsSavingProject: string | null = null;

async function loadScreenshotSettings(): Promise<boolean> {
  const current = projectId.value;
  if (!current || screenshotSettingsDisposed) return false;
  const token = ++screenshotSettingsLoadRequest;
  const integrationsToken = ++engineIntegrationsRequest;
  const isCurrent = () => !screenshotSettingsDisposed
    && token === screenshotSettingsLoadRequest && projectId.value === current;
  screenshotSettingsLoading.value = true;
  screenshotSettingsLoadError.value = null;
  try {
    const [policy, integrations] = await Promise.all([
      engineApi.getScreenshotPolicy(current),
      engineApi.getProjectIntegration(current),
    ]);
    if (!isCurrent()) return false;
    screenshotPolicy.value = policy;
    if (integrationsToken === engineIntegrationsRequest) engineIntegrations.value = integrations;
    screenshotSettingsWarning.value = null;
    return true;
  } catch (error) {
    if (isCurrent()) screenshotSettingsLoadError.value = normalizeError(error);
    return false;
  } finally {
    if (isCurrent()) screenshotSettingsLoading.value = false;
  }
}

async function saveScreenshotSettings(autoHideUi: boolean | null, integrationId: string | null = null) {
  const current = projectId.value;
  if (!current || screenshotSettingsDisposed || busy.value || engineWorking.value
    || screenshotSettingsSaving.value || screenshotSettingsLoading.value
    || screenshotSettingsLoadError.value || !screenshotPolicy.value) return;
  const installed = integrationId
    ? engineIntegrations.value.find((view) => view.integration.id === integrationId
      && view.integration.projectId === current)
    : null;
  if (integrationId && !installed) return;
  const token = ++screenshotSettingsRequest;
  const isCurrent = () => !screenshotSettingsDisposed
    && token === screenshotSettingsRequest && projectId.value === current;
  screenshotSettingsLoadRequest += 1;
  engineIntegrationsRequest += 1;
  engineWorking.value = true;
  screenshotSettingsSaving.value = true;
  screenshotSettingsSavingProject = current;
  screenshotSettingsWarning.value = null;
  try {
    if (integrationId) {
      const saved = await engineApi.setInstallationScreenshotPolicy(current, integrationId, autoHideUi);
      if (!isCurrent() || !engineIntegrations.value.some((view) => view.integration.id === integrationId)) return;
      if (saved.integration.id !== integrationId || saved.integration.projectId !== current
        || saved.integration.autoHideUiOverride !== autoHideUi) {
        screenshotSettingsWarning.value = saved.warning ?? "截图设置未保存，请重试。";
        return;
      }
      engineIntegrations.value = engineIntegrations.value.map((view) =>
        view.integration.id === integrationId ? saved : view,
      );
    } else {
      const saved = await engineApi.setProjectScreenshotPolicy(current, autoHideUi);
      if (!isCurrent()) return;
      if (saved.projectAutoHideUi !== autoHideUi) {
        screenshotSettingsWarning.value = "项目截图设置未保存，请重试。";
        return;
      }
      screenshotPolicy.value = saved;
    }
    // Refresh both levels while retaining the engine-operation lock. A policy
    // change may affect every inheriting installation without reinstalling it.
    const refreshed = await loadScreenshotSettings();
    if (!isCurrent()) return;
    if (!refreshed) {
      screenshotSettingsWarning.value = "设置已保存，但最新状态读取失败；请重试读取截图设置。";
      return;
    }
    toast.success(engineStaleInstalls.value.length
      ? "截图设置已保存；旧插件需要重新安装并重启游戏一次，更新后才会应用。"
      : "截图设置已保存；已更新并重启过的插件将在下一次截图时应用。");
  } catch (error) {
    if (isCurrent()) screenshotSettingsWarning.value = "截图设置保存失败：" + normalizeError(error) + "。请重试。";
  } finally {
    // Project switches invalidate the result, but must not release this lock
    // until the outstanding mutation ends.
    if (!screenshotSettingsDisposed) {
      screenshotSettingsSaving.value = false;
      screenshotSettingsSavingProject = null;
      engineWorking.value = false;
      if (!isCurrent() && projectId.value) void loadEngineState();
    }
  }
}

function retryScreenshotSettings() {
  if (busy.value || engineWorking.value || screenshotSettingsSaving.value || screenshotSettingsLoading.value) return;
  void loadScreenshotSettings();
}

function saveProjectScreenshotPolicy(autoHideUi: boolean | null) {
  void saveScreenshotSettings(autoHideUi);
}

function saveInstallationScreenshotPolicy(view: ProjectEngineIntegrationView, autoHideUi: boolean | null) {
  if (view.integration.projectId !== projectId.value) return;
  void saveScreenshotSettings(autoHideUi, view.integration.id);
}

async function loadEngineState() {
  if (screenshotSettingsDisposed) return;
  const current = projectId.value;
  // Responses carry a request generation and are only applied while the same
  // project is still selected: a late answer for project A must never
  // overwrite what project B is showing.
  const token = ++engineStateRequest;
  const canApplyIntegrations = screenshotSettingsSavingProject !== current;
  const integrationsToken = canApplyIntegrations ? ++engineIntegrationsRequest : engineIntegrationsRequest;
  if (!current) {
    engineIntegrations.value = [];
    archivePolicy.value = null;
    engineDetectedRoot.value = null;
    engineError.value = null;
    return;
  }
  try {
    const [integrations, policy] = await Promise.all([
      engineApi.getProjectIntegration(current),
      engineApi.getCapturePolicy(current).catch(() => null),
    ]);
    if (screenshotSettingsDisposed || token !== engineStateRequest || projectId.value !== current) return;
    if (canApplyIntegrations && integrationsToken === engineIntegrationsRequest) {
      engineIntegrations.value = integrations;
      void refreshEngineBadges();
    }
    engineConfirmUninstall.value = false;
    engineConfirmUninstallId.value = null;
    engineUninstallOpen.value = false;
    if (!archivePolicySaving.value) {
      archivePolicy.value = policy;
      if (policy) applyPolicyChoices(policy);
    }
  } catch (error) {
    if (!screenshotSettingsDisposed && token === engineStateRequest && projectId.value === current) {
      engineError.value = normalizeError(error);
    }
  }
}

let archiveSaveRequest = 0;

function applyPolicyChoices(policy: CapturePolicyView) {
  archiveSizeChoice.value = policy.projectSize;
  archiveFormatChoice.value = policy.projectFormat;
  archiveQualityInherit.value = policy.projectQuality == null;
  archiveQualityChoice.value = policy.projectQuality ?? policy.globalQuality;
}

async function saveArchivePolicy() {
  const current = projectId.value;
  if (archivePolicyDisabled.value) return;
  const token = ++archiveSaveRequest;
  archivePolicySaving.value = true;
  try {
    const policy = await engineApi.setProjectCapturePolicy({
      projectId: current,
      archiveOutputSize: archiveSizeChoice.value,
      archiveOutputFormat: archiveFormatChoice.value,
      archiveOutputQuality: archiveQualityInherit.value ? null : archiveQualityChoice.value,
    });
    if (token !== archiveSaveRequest || projectId.value !== current) return;
    // Invalidate any specification read that started before this save; its
    // stale value must not overwrite what was just persisted.
    engineStateRequest += 1;
    archivePolicy.value = policy;
    applyPolicyChoices(policy);
    toast.success("归档规格已保存。");
  } catch (error) {
    if (token === archiveSaveRequest && projectId.value === current) {
      toast.error(normalizeError(error));
      if (archivePolicy.value) applyPolicyChoices(archivePolicy.value);
    }
  } finally {
    if (token === archiveSaveRequest && projectId.value === current) {
      archivePolicySaving.value = false;
    }
  }
}

// The project is pinned by the caller for the whole operation: detecting and
// installing must never slide into whatever project happens to be selected
// when an awaited step finishes. Returns the install result (null on failure)
// so callers can surface its warning instead of dropping it.
async function installEnginePluginAt(
  project: string,
  root: string,
): Promise<ProjectEngineIntegrationView | null> {
  try {
    return await engineApi.installBridge(project, root);
  } catch (error) {
    if (projectId.value === project) engineError.value = normalizeError(error);
    return null;
  }
}

// Batch install/reinstall from the toolbar: every game already bridged (for
// version upgrades) plus every source directory that still detects as Ren'Py.
async function installEnginePlugins(interactive = true): Promise<boolean> {
  const current = projectId.value;
  if (!current || engineWorking.value) return false;
  engineError.value = null;
  const roots = new Set<string>();
  for (const view of engineIntegrations.value) roots.add(view.integration.engineRoot);
  for (const dir of sourceDirs.value) {
    try {
      const detection = await engineApi.detectEngine(dir.directory);
      roots.add(detection.engineRoot);
      if (projectId.value === current) engineDetectedRoot.value = detection.engineRoot;
    } catch {
      // Not a Ren'Py game directory.
    }
  }
  if (!roots.size) {
    if (!interactive) return false;
    const path = await pickDirectory();
    if (!path) return false;
    try {
      const detection = await engineApi.detectEngine(path);
      roots.add(detection.engineRoot);
      engineDetectedRoot.value = detection.engineRoot;
    } catch (error) {
      engineError.value = normalizeError(error);
      toast.error(engineError.value);
      return false;
    }
  }
  if (projectId.value !== current || engineWorking.value) return false;
  engineWorking.value = true;
  let installed = 0;
  let failed = 0;
  const warnings: string[] = [];
  try {
    for (const root of roots) {
      const view = await installEnginePluginAt(current, root);
      if (view) {
        installed += 1;
        if (view.warning) warnings.push(view.warning);
      } else {
        failed += 1;
      }
    }
    if (projectId.value === current) {
      await loadEngineState();
    }
    if (warnings.length) {
      toast.info(warnings.join("；"));
    }
    if (interactive) {
      if (failed) {
        toast.error(`已安装 ${installed} 个插件，${failed} 个失败：${engineError.value ?? ""}`);
      } else {
        toast.success(`已安装/重装 ${installed} 个 Ren'Py 插件。`);
      }
    }
    return installed > 0;
  } finally {
    engineWorking.value = false;
  }
}

async function uninstallEnginePlugin(integrationId: string | null = null) {
  const current = projectId.value;
  if (!current || engineWorking.value) return;
  const targetId = integrationId ?? engineIntegrations.value[0]?.integration.id ?? null;
  if (!targetId) return;
  if (integrationId) {
    if (engineConfirmUninstallId.value !== targetId) {
      engineConfirmUninstallId.value = targetId;
      return;
    }
  } else if (!engineConfirmUninstall.value) {
    engineConfirmUninstall.value = true;
    return;
  }
  engineConfirmUninstall.value = false;
  engineConfirmUninstallId.value = null;
  // Reads started before uninstall must never restore the removed game row.
  screenshotSettingsLoadRequest += 1;
  engineIntegrationsRequest += 1;
  screenshotSettingsLoading.value = false;
  engineWorking.value = true;
  try {
    const result = await engineApi.uninstallBridge(current, targetId);
    if (result.keptFiles.length) {
      toast.error(`部分文件无法证明归属，已保留未删除：${result.keptFiles.join("；")}`);
    } else {
      toast.success("插件已卸载。");
    }
    if (!screenshotSettingsDisposed && projectId.value === current) {
      await loadEngineState();
      if (screenshotSettingsOpen.value) await loadScreenshotSettings();
    }
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    engineWorking.value = false;
  }
}

// A new source directory that turns out to be a Ren'Py game is bridged
// silently; installing one game never disturbs the other installs.
async function detectAndMaybeInstallEngine(path: string) {
  const current = projectId.value;
  if (!current) return;
  try {
    const detection = await engineApi.detectEngine(path);
    if (projectId.value !== current) return;
    engineDetectedRoot.value = detection.engineRoot;
    const known = engineIntegrations.value.some(
      (view) => view.integration.gameDirectory === detection.gameDirectory,
    );
    if (known || engineWorking.value) return;
    engineWorking.value = true;
    let warning: string | null = null;
    try {
      const view = await installEnginePluginAt(current, detection.engineRoot);
      warning = view?.warning ?? null;
    } finally {
      engineWorking.value = false;
    }
    if (warning) toast.info(warning);
    if (projectId.value === current) await loadEngineState();
  } catch {
    if (projectId.value === current) engineDetectedRoot.value = null;
  }
}

async function launchProjectGame() {
  const current = projectId.value;
  if (!current || busy.value) return;
  try {
    const options = await engineApi.listProjectGames(current);
    if (projectId.value !== current) return;
    if (!options.length) {
      toast.error("没有找到可启动的游戏：请先安装插件，或确认游戏目录里有可执行文件。");
      return;
    }
    if (options.length === 1) {
      await startProjectGame(current, options[0]);
      return;
    }
    gameLaunchOptions.value = options;
    gameLaunchOpen.value = !gameLaunchOpen.value;
  } catch (error) {
    toast.error(normalizeError(error));
  }
}

async function startProjectGame(project: string, option: ProjectGameLaunchOption) {
  try {
    const launched = await engineApi.launchGame(
      project,
      option.integrationId,
      option.executable,
    );
    toast.success(`已启动游戏：${launched.gameName}`);
    gameLaunchOpen.value = false;
  } catch (error) {
    toast.error(normalizeError(error));
  }
}

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
    !busy.value && !roiBusy.value && !roiBlocked.value && !roiLoading.value,
);

async function initialize() {
  if (!isActive()) return;
  loading.value = true;
  void refreshEngineBadges();
  try {
    const nextProjects = await captureApi.listProjects();
    if (!isActive()) return;
    projects.value = nextProjects;
    const savedProject = localStorage.getItem("scene-vault.capture.project");
    projectId.value = projects.value.some((project) => project.id === savedProject)
      ? savedProject!
      : projects.value[0]?.id ?? "";
    if (localStorage.getItem("scene-vault.capture.create-project") === "1") {
      showProjectForm.value = true;
      localStorage.removeItem("scene-vault.capture.create-project");
    }
    const nextRuntime = await captureApi.runtimeStatus();
    if (!isActive()) return;
    runtime.value = nextRuntime;
    if (projectId.value) await loadProject();
  } catch (error) {
    if (isActive()) toast.error(normalizeError(error));
  } finally {
    if (isActive()) loading.value = false;
  }
}

let projectLoadRequest = 0;
let displayedProjectId = "";
async function loadProject() {
  if (!isActive()) return;
  const current = projectId.value;
  const request = ++projectLoadRequest;
  if (displayedProjectId !== current) {
    displayedProjectId = current;
    characters.value = [];
    sessions.value = [];
    sourceDirs.value = [];
    projectDestination.value = "";
    items.value = [];
    selectedItemId.value = null;
    selectedCharacterId.value = null;
    pendingClassification.value = null;
    deferredImportCount.value = 0;
  }
  if (!current) return;
  localStorage.setItem("scene-vault.capture.project", current);
  const [nextCharacters, nextSessions, nextSources] = await Promise.all([
    captureApi.listCharacters(current),
    captureApi.listSessions(current),
    captureApi.listSourceDirectories(current),
  ]);
  if (request !== projectLoadRequest || projectId.value !== current) return;
  characters.value = nextCharacters;
  sessions.value = nextSessions;
  sourceDirs.value = nextSources;
  const project = projects.value.find((value) => value.id === current);
  projectDestination.value = project?.destinationDirectory ?? "";
  await refreshRecentCaptures();
  if (request !== projectLoadRequest || projectId.value !== current) return;
  const deferred = activeSession.value
    ? await captureApi.deferredImportRecognitionCount(activeSession.value.id)
    : 0;
  if (request !== projectLoadRequest || projectId.value !== current) return;
  deferredImportCount.value = deferred;
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
  const current = projectId.value;
  const itemId = selectedItem.value?.id ?? null;
  if (!name || !current) return;
  await runBusy(async () => {
    const character = await captureApi.createCharacter(current, name);
    if (projectId.value !== current) return;
    characters.value.push(character);
    // Only select on the capture the form belonged to; switching images mid-
    // create must not mark a fresh item as manually chosen.
    if (itemId && selectedItem.value?.id === itemId) pickCharacter(character.id);
    newCharacterName.value = "";
    showCharacterForm.value = false;
  });
}

// One-click "create the character the dialogue is talking about": the name
// comes from the engine speaker, so a game whose cast was never entered into
// Scene Vault still gets a suggestion. After creation the live re-match makes
// the engine suggestion row appear as usual.
async function createCharacterFromSpeaker() {
  const name = engineMetadata.value?.speakerRaw?.trim();
  const current = projectId.value;
  const itemId = selectedItem.value?.id ?? null;
  if (!name || !current) return;
  await runBusy(async () => {
    const character = await captureApi.createCharacter(current, name);
    // The create is a server round-trip; the user may have switched projects
    // or images meanwhile. Never merge or select into a different context.
    if (projectId.value !== current) return;
    characters.value.push(character);
    if (itemId && selectedItem.value?.id === itemId) pickCharacter(character.id);
    toast.success(`已创建角色「${name}」。`);
  });
}

async function addSourceDir() {
  const current = projectId.value;
  if (!current) {
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
  const directory = path;
  await runBusy(async () => {
    try {
      await captureApi.addSourceDirectory(current, directory);
    } catch (error) {
      const directories = await captureApi.listSourceDirectories(current);
      if (projectId.value === current) sourceDirs.value = directories;
      throw error;
    }
    const directories = await captureApi.listSourceDirectories(current);
    if (projectId.value !== current) return;
    sourceDirs.value = directories;
    toast.success("已添加截图目录，下次开始会话时生效。");
    void detectAndMaybeInstallEngine(directory);
  });
}

async function removeSourceDir(dir: ProjectSourceDirectory) {
  await runBusy(async () => {
    await captureApi.removeSourceDirectory(dir.id);
    const directories = await captureApi.listSourceDirectories(dir.projectId);
    if (projectId.value === dir.projectId) sourceDirs.value = directories;
  });
}

async function toggleSourceDir(dir: ProjectSourceDirectory) {
  await runBusy(async () => {
    await captureApi.setSourceDirectoryEnabled(dir.id, !dir.enabled);
    const directories = await captureApi.listSourceDirectories(dir.projectId);
    if (projectId.value === dir.projectId) sourceDirs.value = directories;
  });
}

async function chooseDestination() {
  const current = projectId.value;
  if (!current) return;
  const path = await pickDirectory();
  if (!path) return;
  await runBusy(async () => {
    const project = await captureApi.setProjectDestination(current, path);
    const index = projects.value.findIndex((value) => value.id === project.id);
    if (index !== -1) projects.value[index] = project;
    if (projectId.value !== current) return;
    projectDestination.value = project.destinationDirectory ?? "";
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
    // The backend also emits capture:item-created for each imported row;
    // refreshing here keeps the list consistent even if an event is missed.
    await refreshRecentCaptures();
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
    // Without a configured engine the worker stays idle for awaiting-label
    // items, so live events never populate the list on their own.
    await refreshRecentCaptures();
    if (runtime.value?.engineStatus === "unconfigured") {
      toast.info(
        `已解除识别等待（${count} 张）。AI 未配置，截图保留为待分类；配置 AI 后会在空闲时自动逐张识别。`,
      );
    } else {
      toast.success(`已开始逐张识别 ${count} 张导入截图。`);
    }
  } catch (error) {
    toast.error(error instanceof Error ? error.message : String(error));
  } finally {
    importBusy.value = false;
  }
}

let recentRequest = 0;
async function refreshRecentCaptures() {
  if (!isActive()) return;
  const request = ++recentRequest;
  if (!projectId.value) return;
  const id = projectId.value;
  const [recent, pending] = await Promise.all([captureApi.listProjectRecentCaptures(id, 100), captureApi.listProjectPendingCaptures(id)]);
  if (id !== projectId.value || request !== recentRequest) return;
  items.value = [...new Map([...recent, ...pending].map(item => [item.id, item])).values()];
  selectBestItem();
}

function onItemDeleted() {
  deleteTarget.value = null;
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
  const revision = roiRevision.value;
  verificationWarning.value = null;
  await runBusy(async () => {
    const result = await captureApi.verifyCaptureIdentity({
      captureItemId: itemId,
      characterId,
    });
    if (selectedItem.value?.id !== itemId || revision !== roiRevision.value) return;
    if (result.level === "ok" || result.level === "unverified") {
      await doLabel(itemId, characterId);
    } else {
      verificationWarning.value = result;
    }
  });
}

async function confirmForcedLabel() {
  if (!selectedItem.value || !selectedCharacterId.value || !canSubmit.value) return;
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
// may have been enrolled earlier in the same batch. The local item may be a
// stale pre-feature snapshot, so the backend (which reads the real feature
// state) decides whether a comparison is possible; no front-end guard here.
async function refreshSuggestion() {
  const item = selectedItem.value;
  if (!item || roiBusy.value || roiBlocked.value || suggestionRequestedFor === item.id) return;
  const revision = roiRevision.value;
  suggestionRequestedFor = item.id;
  try {
    const updated = await captureApi.suggestForCapture(item.id);
    if (revision === roiRevision.value && selectedItem.value?.id === item.id) upsertItem(updated);
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
  if (item.projectId !== projectId.value) return;
  const index = items.value.findIndex((value) => value.id === item.id);
  if (index >= 0 && isOlderCapture(item, items.value[index])) return;
  if (index === -1) items.value.unshift(item);
  else items.value[index] = item;
  items.value = [...items.value].sort((left, right) => right.capturedAt.localeCompare(left.capturedAt));
  // Background feature/suggestion updates also arrive as item-updated while
  // the capture remains awaiting_label. Never let those events steal the
  // user's current selection in the middle of classification.
  if (!selectedItemId.value) selectedItemId.value = item.id;
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
  if (event.defaultPrevented || archivePolicyOpen.value || sourceDirectoriesOpen.value || screenshotSettingsOpen.value) return;
  if (event.target instanceof Element && event.target.closest('[role="dialog"]')) return;
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
    pickCharacter(null);
  } else if (/^[1-5]$/.test(event.key)) {
    const character = recentCharacters.value[Number(event.key) - 1];
    if (character) pickCharacter(character.id);
  }
}

function normalizeError(error: unknown): string {
  return describeError(error);
}

watch(projectId, () => void loadProject());
watch(
  projectId,
  () => {
    // Switching projects clears the previous project's toolbar state before
    // the new state loads.
    engineIntegrations.value = [];
    screenshotSettingsOpen.value = false;
    screenshotSettingsWarning.value = null;
    screenshotPolicy.value = null;
    screenshotSettingsLoadError.value = null;
    screenshotSettingsLoading.value = false;
    screenshotSettingsRequest += 1;
    screenshotSettingsLoadRequest += 1;
    engineIntegrationsRequest += 1;
    archivePolicy.value = null;
    archiveSaveRequest += 1;
    archivePolicySaving.value = false;
    archivePolicyOpen.value = false;
    sourceDirectoriesOpen.value = false;
    engineDetectedRoot.value = null;
    engineError.value = null;
    engineConfirmUninstall.value = false;
    engineConfirmUninstallId.value = null;
    engineUninstallOpen.value = false;
    gameLaunchOptions.value = [];
    gameLaunchOpen.value = false;
    void loadEngineState();
  },
  { immediate: true },
);
watch(screenshotSettingsOpen, (open) => {
  if (open && !screenshotSettingsSaving.value) void loadScreenshotSettings();
});
watch(() => selectedItem.value?.id, () => void loadEngineMetadata());
watch(() => selectedItem.value, () => void loadImageFacts());

// Leaving a capture invalidates the suggestion it auto-picked: the next
// image starts from its own suggestions instead of inheriting a stale
// automatic pick (a manual selection still carries over).
watch(
  () => selectedItem.value?.id,
  (_current, previous) => {
    if (!previous || autoPickedCharacter.value?.itemId !== previous) return;
    const auto = autoPickedCharacter.value.characterId;
    if (selectedCharacterId.value === auto) selectedCharacterId.value = null;
    autoPickedCharacter.value = null;
  },
);
watch(items, () => void nextTick(syncStripBar));
watch(
  () => [selectedItemId.value, pendingClassification.value] as const,
  () => {
    if (pendingClassification.value === "person") void refreshSuggestion();
  },
);

onMounted(async () => {
  window.addEventListener("keydown", onKeydown);
  // Register listeners before the first load so a late metadata event that
  // arrives while initialize() is still running is not lost.
  try {
    await listen<CaptureItem>("capture:item-created", (event) => upsertItem(event.payload));
    await listen<CaptureItem>("capture:item-updated", (event) => upsertItem(event.payload));
    await listen<{ projectId: string }>("capture:project-files-updated", (event) => {
      if (event.payload.projectId === projectId.value) void refreshRecentCaptures();
    });
    await listen<{ captureItemId: string }>("capture:engine-metadata-updated", (event) => {
      if (selectedItem.value?.id === event.payload.captureItemId) void loadEngineMetadata();
    });
    await listen<{ captureItemId: string }>("capture:item-purged", (event) => {
      const purgedId = event.payload.captureItemId;
      if (selectedItemId.value === purgedId) {
        selectedItemId.value = null;
        pendingClassification.value = null;
      }
      void refreshRecentCaptures();
    });
    await listen("capture:face-bank-rebuilt", () => void refreshRecentCaptures());
    await listen<CaptureRuntimeStatus>("capture:runtime-status", (event) => {
      runtime.value = event.payload;
    });
    await listen<string>("capture:runtime-error", (event) => {
      toast.error(event.payload);
    });
  } catch {
    // Browser previews do not expose the Tauri event bridge.
  }
  if (!isActive()) return;
  await initialize();
  if (!isActive()) return;
  await nextTick();
  if (!isActive()) return;
  syncStripBar();
  if (typeof ResizeObserver !== "undefined" && stripRow.value) {
    stripObserver = new ResizeObserver(() => syncStripBar());
    stripObserver.observe(stripRow.value);
  }
  await loadPreview();
});

onBeforeUnmount(() => {
  screenshotSettingsDisposed = true;
  screenshotSettingsRequest += 1;
  screenshotSettingsLoadRequest += 1;
  engineIntegrationsRequest += 1;
  engineStateRequest += 1;
  projectLoadRequest += 1;
  engineMetadataRequest++;
  imageFactsRequest++;
  recentRequest++;
  stripObserver?.disconnect();
  stripObserver = null;
  window.removeEventListener("keydown", onKeydown);
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
            <ProjectSelect v-model="projectId" :projects="projects" :engine-versions="engineVersions" @open="refreshEngineBadges" />
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
          <button
            type="button" class="source-directory-summary" :disabled="!projectId"
            :title="sourceDirs[0]?.directory" aria-haspopup="dialog" aria-label="查看截图来源目录"
            @click="sourceDirectoriesOpen = true"
          >
            <FolderOpen :size="16" aria-hidden="true" />
            <span class="source-dir-name">{{ sourceDirs[0]?.directory ?? "还没有来源目录" }}</span>
            <span v-if="sourceDirs.length > 1" class="source-directory-more" :title="`共 ${sourceDirs.length} 个来源目录`">…</span>
            <ChevronDown :size="14" aria-hidden="true" />
          </button>
        </section>

        <section class="session-stage archive-stage" aria-labelledby="session-archive-title">
          <div class="session-stage-heading">
            <span class="session-step" aria-hidden="true">3</span>
            <h2 id="session-archive-title">归档</h2>
            <Check v-if="projectDestination" class="session-stage-check" :size="16" aria-label="已配置归档位置" />
            <span class="archive-spec-summary" :title="archivePolicySummary">{{ archivePolicySummary }}</span>
            <button v-if="archivePolicy" type="button" class="archive-inline-action" :disabled="!projectId" aria-haspopup="dialog" @click="archivePolicyOpen = true">调整…</button>
            <button v-else type="button" class="archive-inline-action" :disabled="!projectId" @click="loadEngineState">重试</button>
          </div>
          <div class="archive-directory-row">
            <FolderOpen :size="16" aria-hidden="true" />
            <span class="archive-directory" :title="projectDestination">{{ projectDestination || "未选择归档目录" }}</span>
            <button type="button" class="archive-inline-action" :disabled="Boolean(activeSession) || !projectId" @click="chooseDestination">
              {{ projectDestination ? "更改" : "选择" }}
            </button>
          </div>
        </section>

        <section class="session-control" aria-label="会话控制">
          <div class="session-health">
            <span :class="{ online: activeSession }"><Wifi :size="15" />{{ activeSession ? "监听中" : "未监听" }}</span>
            <span :class="{ online: engineOnline }">
              <Sparkles :size="15" />{{ engineStatusLabel }}
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
          <button
            v-if="engineInstalled"
            type="button"
            :disabled="busy || !projectId"
            title="启动游戏（Ren'Py）"
            @click="launchProjectGame"
          >
            <Play :size="16" />启动游戏
          </button>
          <button
            type="button"
            :disabled="busy || engineWorking || !projectId"
            aria-haspopup="dialog"
            @click="screenshotSettingsOpen = true"
          >
            <Settings2 :size="16" />截图设置
          </button>
          <div v-if="gameLaunchOpen && gameLaunchOptions.length > 1" class="game-launch-list">
            <div
              v-for="option in gameLaunchOptions"
              :key="option.integrationId"
              class="game-launch-row"
            >
              <span :title="option.executable">
                {{ option.gameName }} · {{ pathFileName(option.executable) }}{{ option.recommended ? "（推荐）" : "" }}
              </span>
              <button type="button" :disabled="busy" @click="startProjectGame(projectId, option)">
                启动
              </button>
            </div>
          </div>
          <button
            type="button"
            :disabled="busy || engineWorking || !projectId"
            :title="engineButtonTitle"
            @click="installEnginePlugins(true)"
          >
            <Plug :size="16" />{{ engineInstalled ? "重新安装" : "插件安装" }}
          </button>
          <button
            v-if="engineInstalled"
            type="button"
            :disabled="busy || engineWorking || !projectId"
            title="卸载游戏目录中的 Scene Vault 插件文件"
            @click="
              engineIntegrations.length > 1
                ? (engineUninstallOpen = !engineUninstallOpen)
                : uninstallEnginePlugin(null)
            "
          >
            <Unplug :size="16" />{{ engineConfirmUninstall ? "确认卸载？" : "卸载" }}
          </button>
          <div
            v-if="engineUninstallOpen && engineIntegrations.length > 1"
            class="engine-uninstall-list"
          >
            <div
              v-for="view in engineIntegrations"
              :key="view.integration.id"
              class="engine-uninstall-row"
            >
              <span :title="view.integration.engineRoot">{{ view.integration.engineRoot }}</span>
              <button
                type="button"
                :disabled="engineWorking"
                @click="uninstallEnginePlugin(view.integration.id)"
              >
                {{ engineConfirmUninstallId === view.integration.id ? "确认？" : "卸载" }}
              </button>
            </div>
          </div>
          <template v-if="activeSession">
            <button type="button" :disabled="busy" @click="openImportDialog">
              <FolderInput :size="16" />导入截图
            </button>
            <button
              v-if="deferredImportCount"
              type="button"
              class="recognition-action"
              :disabled="importBusy || runtime?.engineStatus === 'unconfigured'"
              :title="
                runtime?.engineStatus === 'unconfigured'
                  ? 'AI 未配置：在设置中配置 Python 与模型，或安装带 AI 组件的版本，才能开始识别导入截图'
                  : '逐张识别已导入截图'
              "
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
          <div class="capture-heading-row">
            <div class="capture-heading-title">
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
              <span
                v-for="issue in selectedItem ? captureFileIssues(selectedItem) : []"
                :key="issue.kind"
                class="file-state-chip"
                :data-state="issue.state"
              >{{ issue.label }}</span>
            </div>
            <div class="stage-heading-actions">
              <div v-if="selectedItem" class="capture-heading-file">
                <span :title="pathFileName(selectedItem.sourcePath)">{{ pathFileName(selectedItem.sourcePath) }}</span>
                <span>{{ new Date(selectedItem.capturedAt).toLocaleString([], { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }) }}</span>
              </div>
              <CaptureEngineChips :engine-metadata="engineMetadata" :image-facts="imageFacts" />
              <button
                v-if="!isWideLayout"
                type="button"
                class="secondary-action label-panel-trigger"
                :aria-expanded="labelPanelOpen"
                @click="labelPanelOpen = true"
              >
                <UserRound :size="16" />角色与分类
              </button>
            </div>
          </div>
        </div>

        <CaptureProgress persistent :deferred-count="deferredImportCount" class="stage-progress" />

        <div class="preview-shell" :class="{ empty: !selectedItem }">
          <FaceRoiSelector inline v-if="selectedItem && pendingClassification === 'person' && selectedItem.status === 'awaiting_label' && selectedItem.classification === 'unclassified'" :item-id="selectedItem.id" :image-url="previewUrl" :image-error="previewUnavailable" :model-value="faceRoi" :face-box="detectedFaceBox" :mode="selectedItem.faceDetectionMode === 'normalized' ? 'normalized' : 'auto'" :busy="roiBusy" :loading="roiLoading" :disabled="busy || roiLoading || previewLoading" :error="roiError" @confirm="saveFaceRoi" />
          <img v-else-if="previewUrl" :src="previewUrl" :alt="selectedItem ? `${pathFileName(selectedItem.sourcePath)} 预览` : ''" />
          <div v-else-if="previewUnavailable" class="preview-unavailable">
            <ImageOff :size="44" />
            <strong>{{ previewUnavailable }}</strong>
            <span>此截图对应的文件已不可读取，可右键从 Scene Vault 移除该记录。</span>
          </div>
          <div v-else-if="selectedItem && previewLoading" class="preview-empty" role="status">
            <LoaderCircle class="animate-spin" :size="32" />
            <strong>正在加载图片…</strong>
          </div>
          <div v-else class="preview-empty">
            <ImageOff :size="44" />
            <strong>{{ activeSession ? "等待游戏产生新截图" : "开始会话后自动发现截图" }}</strong>
            <span>继续使用你习惯的截图方式，新截图会自动出现</span>
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
          <div class="capture-list-toolbar">
            <div class="capture-list-title"><h2 id="recent-title">近期捕获 <span>{{ stripItems.length }}</span></h2><span v-if="waitingCount" class="capture-list-hint">{{ waitingCount }} 张待分类</span></div>
          </div>
          <div v-if="stripItems.length" class="thumbnail-scroll">
          <div ref="stripRow" class="thumbnail-row" @wheel="onStripWheel" @scroll="onStripScroll">
            <div
              v-for="item in stripItems"
              :key="item.id"
              role="group"
              class="capture-card"
              :class="{ selected: selectedItemId === item.id }"
              :aria-label="`选择 ${pathFileName(item.sourcePath)}`"
              @click="selectedItemId = item.id"
              @contextmenu="onItemContext($event, item)"
            >
              <button class="capture-card-select" type="button" :aria-label="`查看 ${pathFileName(item.sourcePath)}`">
              <div class="thumb-image"><CaptureThumbnail :item="item" /></div>
              <div class="thumb-status" :data-status="item.status">
                <span class="status-dot" />{{ captureStatusLabel(item.status, item.failureStage) }}
              </div>
              <time>{{ new Date(item.capturedAt).toLocaleString([], { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }) }}</time>
              </button>
            </div>
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

        <button
          v-if="pendingClassification === 'person' && suggestedCharacter"
          type="button"
          class="suggestion-row"
          :class="{ selected: selectedCharacterId === suggestedCharacter.id }"
          @click="pickCharacter(suggestedCharacter.id)"
        >
          <Sparkles :size="16" />
          <span>
            <strong>推荐：{{ suggestedCharacter.name }}</strong>
            <small>相似度 {{ Math.round((selectedItem?.recognitionConfidence ?? 0) * 100) }}%{{ engineSuggestedCharacter && engineSuggestedCharacter.id === suggestedCharacter.id ? " · 说话人匹配" : "" }}</small>
          </span>
          <Check v-if="selectedCharacterId === suggestedCharacter.id" :size="16" />
        </button>

        <button
          v-if="pendingClassification === 'person' && engineSuggestedCharacter && engineSuggestedCharacter.id !== suggestedCharacter?.id"
          type="button"
          class="suggestion-row"
          :class="{ selected: selectedCharacterId === engineSuggestedCharacter.id }"
          @click="pickCharacter(engineSuggestedCharacter.id)"
        >
          <Sparkles :size="16" />
          <span>
            <strong>引擎建议：{{ engineSuggestedCharacter.name }}</strong>
            <small>{{ speakerName ? `当前说话人 ${speakerName}` : "来自 Ren'Py 引擎" }}</small>
          </span>
          <Check v-if="selectedCharacterId === engineSuggestedCharacter.id" :size="16" />
        </button>

        <button
          v-if="canCreateCharacterFromSpeaker"
          type="button"
          class="suggestion-row"
          :disabled="busy"
          @click="createCharacterFromSpeaker"
        >
          <UserPlus :size="16" />
          <span>
            <strong>创建角色「{{ speakerName }}」</strong>
            <small>来自对话说话人，创建后即可选中</small>
          </span>
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
              @click="pickCharacter(character.id)"
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
            @click="pickCharacter(character.id)"
          >
            <span class="avatar"><CharacterAvatar :character="character" /></span>
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

    <RenpyScreenshotSettings
      v-model:open="screenshotSettingsOpen"
      :integrations="engineIntegrations"
      :policy="screenshotPolicy"
      :loading="screenshotSettingsLoading"
      :load-error="screenshotSettingsLoadError"
      :working="engineWorking || busy"
      :saving="screenshotSettingsSaving"
      :warning="screenshotSettingsWarning"
      @project-change="saveProjectScreenshotPolicy"
      @installation-change="saveInstallationScreenshotPolicy"
      @retry="retryScreenshotSettings"
    />

    <Dialog v-model:open="sourceDirectoriesOpen">
      <DialogContent class="source-directories-dialog">
        <DialogTitle>截图来源目录</DialogTitle>
        <DialogDescription>当前项目的全部来源目录。目录调整在下次开始会话时生效。</DialogDescription>
        <ul class="source-dir-list">
          <li v-for="dir in sourceDirs" :key="dir.id" :class="{ muted: !dir.enabled }">
            <FolderOpen :size="15" class="source-dir-icon" />
            <span class="source-dir-name" :title="dir.directory">{{ dir.directory }}</span>
            <span class="source-dir-state" :class="{ online: activeSession && dir.enabled }">
              {{ !dir.enabled ? "已暂停" : activeSession ? "监听中" : "已启用" }}
            </span>
            <div v-if="!activeSession" class="source-dir-actions">
              <button type="button" :disabled="busy" @click="toggleSourceDir(dir)">{{ dir.enabled ? "暂停" : "启用" }}</button>
              <button type="button" class="danger" :disabled="busy" :aria-label="`移除 ${dir.directory}`" @click="removeSourceDir(dir)"><Trash2 :size="14" />移除</button>
            </div>
          </li>
          <li v-if="!sourceDirs.length" class="source-dir-empty">还没有来源目录</li>
        </ul>
        <button type="button" class="secondary-action" :disabled="busy" @click="addSourceDir"><FolderPlus :size="16" />添加截图目录</button>
      </DialogContent>
    </Dialog>

    <Dialog v-model:open="archivePolicyOpen">
      <DialogContent class="archive-policy-dialog">
        <DialogTitle>归档规格</DialogTitle>
        <DialogDescription>仅设置当前项目的新归档图片，已有归档不会改变。</DialogDescription>
        <p v-if="activeSession" class="archive-policy-note">会话运行中，请先停止监听再修改。</p>
        <label class="archive-policy-field">
          <span>尺寸</span>
          <select v-model="archiveSizeChoice" aria-label="归档尺寸" :disabled="archivePolicyDisabled" @change="saveArchivePolicy">
            <option value="inherit">继承全局（{{ archiveSizeLabel(archivePolicy?.globalSize) }}）</option>
            <option value="keep">保持原样</option>
            <option value="1080p">1080p</option>
            <option value="1440p">1440p</option>
            <option value="2160p">2160p</option>
            <option value="native">按游戏设定尺寸</option>
          </select>
        </label>
        <label class="archive-policy-field">
          <span>格式</span>
          <select v-model="archiveFormatChoice" aria-label="归档格式" :disabled="archivePolicyDisabled" @change="saveArchivePolicy">
            <option value="inherit">继承全局（{{ archiveFormatLabel(archivePolicy?.globalFormat) }}）</option>
            <option value="keep">保持原格式</option>
            <option value="png">PNG</option>
            <option value="jpeg">JPEG</option>
            <option value="webp">WebP（无损）</option>
          </select>
        </label>
        <div class="archive-policy-quality">
          <div class="archive-quality-heading">
            <span>JPEG 质量</span>
            <span v-if="!archiveUsesLossyQuality" class="archive-quality-inactive">不适用</span>
          </div>
          <template v-if="archiveUsesLossyQuality">
            <label class="archive-quality-toggle">
              <input v-model="archiveQualityInherit" type="checkbox" :disabled="archivePolicyDisabled" @change="saveArchivePolicy" />
              <span>跟随全局（{{ archivePolicy?.globalQuality }}）</span>
            </label>
            <div class="archive-quality-control">
              <input
                v-model.number="archiveQualityChoice"
                class="archive-quality-range"
                type="range"
                min="60"
                max="100"
                step="1"
                :disabled="archiveQualityInherit || archivePolicyDisabled"
                :aria-label="`JPEG 质量 ${archiveQualityChoice}`"
                @change="saveArchivePolicy"
              />
              <output class="archive-quality-value">{{ archiveQualityInherit ? archivePolicy?.globalQuality : archiveQualityChoice }}</output>
            </div>
            <p class="archive-policy-note">JPEG 使用此质量；保持原格式时，仅重新编码 JPEG 会应用。</p>
          </template>
          <p v-else class="archive-policy-note">PNG 与无损 WebP 不使用 JPEG 质量。</p>
        </div>
        <div class="archive-policy-footer">
          <span role="status" aria-live="polite">{{ archivePolicySaving ? "正在保存…" : "修改后自动保存" }}</span>
          <button type="button" class="archive-policy-done" :disabled="archivePolicySaving" @click="archivePolicyOpen = false">完成</button>
        </div>
      </DialogContent>
    </Dialog>

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
    <CaptureDeleteDialog
      v-if="deleteTarget"
      :capture-item="deleteTarget"
      @close="deleteTarget = null"
      @deleted="onItemDeleted"
    />
    <ContextMenu :menu="itemMenu" />
  </section>
</template>

<style scoped>
.archive-directory-row {
  display: flex;
  min-width: 0;
  min-height: 36px;
  align-items: center;
  gap: 8px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: color-mix(in srgb, var(--card) 75%, transparent);
}
.archive-directory-row > svg { flex: none; color: var(--muted-foreground); }
.archive-directory { min-width: 0; flex: 1; overflow: hidden; color: var(--foreground); font-size: 12px; text-overflow: ellipsis; white-space: nowrap; }
.archive-inline-action {
  flex: none;
  min-height: 30px;
  padding: 3px 6px;
  border: 0;
  border-radius: 5px;
  background: transparent;
  color: var(--accent);
  font-size: 12px;
  font-weight: 600;
  white-space: nowrap;
}
.archive-inline-action:hover:not(:disabled) { background: color-mix(in srgb, var(--accent) 12%, transparent); }
.archive-inline-action:disabled { color: var(--muted-foreground); cursor: not-allowed; opacity: .55; }
.archive-stage .session-stage-heading { gap: 6px; }
.archive-stage .session-stage-check { margin-left: 0; }
.archive-spec-summary { min-width: 0; flex: 1; overflow: hidden; color: var(--muted-foreground); font-size: 11px; line-height: 1.5; text-overflow: ellipsis; white-space: nowrap; }
.archive-stage .archive-inline-action { min-height: 22px; padding: 0 4px; font-size: 11px; }
.source-directory-summary { display: flex; width: 100%; min-width: 0; min-height: 36px; align-items: center; gap: 7px; padding: 0 9px; border: 1px solid var(--border); border-radius: var(--radius-control); background: color-mix(in srgb, var(--card) 75%, transparent); color: var(--foreground); text-align: left; }
.source-directory-summary > svg, .source-directory-more { flex: none; color: var(--muted-foreground); }
.source-directory-summary:not(:disabled):hover { border-color: var(--accent); }
.source-directories-dialog { max-width: min(620px, calc(100vw - 32px)); }
.source-directories-dialog .source-dir-list { max-height: min(360px, 50vh); overflow-y: auto; }
.source-directories-dialog .source-dir-list li { flex-wrap: wrap; border-color: var(--border); }
.source-directories-dialog .source-dir-name { flex-basis: 220px; white-space: normal; overflow-wrap: anywhere; }

.archive-policy-dialog { max-width: min(440px, calc(100vw - 32px)); }
.archive-policy-field { display: grid; grid-template-columns: 64px minmax(0, 1fr); align-items: center; gap: 12px; color: var(--foreground); font-size: 13px; }
.archive-policy-field > select {
  min-width: 0;
  width: 100%;
  height: 38px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  background: var(--card);
  color: var(--foreground);
  font-size: 12px;
}
.archive-policy-field > select:disabled { color: var(--muted-foreground); cursor: not-allowed; opacity: .65; }
.archive-policy-quality { display: grid; gap: 10px; padding-top: 14px; border-top: 1px solid var(--border); font-size: 13px; }
.archive-quality-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.archive-quality-inactive { color: var(--muted-foreground); font-size: 12px; }
.archive-quality-toggle { display: flex; align-items: center; gap: 8px; color: var(--muted-foreground); font-size: 12px; }
.archive-quality-toggle input { width: 16px; height: 16px; margin: 0; accent-color: var(--accent); }
.archive-quality-control { display: flex; align-items: center; gap: 14px; }
.archive-quality-range { min-width: 0; flex: 1; accent-color: var(--accent); }
.archive-quality-range:disabled { opacity: .5; }
.archive-quality-value { flex: none; min-width: 28px; color: var(--foreground); text-align: right; font-variant-numeric: tabular-nums; }
.archive-policy-note { margin: 0; color: var(--muted-foreground); font-size: 12px; line-height: 1.6; }
.archive-policy-footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding-top: 4px; }
.archive-policy-footer > span { color: var(--muted-foreground); font-size: 12px; }
.archive-policy-done { min-height: 36px; padding: 0 18px; border: 0; border-radius: var(--radius-control); background: var(--accent); color: var(--accent-foreground); font-size: 12px; font-weight: 600; }
.archive-policy-done:disabled { opacity: .55; }

.stage-heading { display: grid; grid-template-columns: minmax(0, 1fr); gap: 6px; padding: 10px 0; }
.capture-heading-row { display: flex; min-width: 0; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px 16px; }
.capture-heading-title { display: flex; min-width: 0; flex-wrap: wrap; align-items: center; gap: 8px; }
.stage-heading h1 { white-space: nowrap; }
.stage-heading-actions { display: flex; min-width: 0; max-width: 100%; flex: 1; flex-wrap: wrap; justify-content: flex-end; align-items: center; gap: 6px 14px; margin-left: auto; }
.capture-heading-file { display: flex; min-width: 0; align-items: center; gap: 8px; color: var(--muted-foreground); font-size: 11px; font-variant-numeric: tabular-nums; }
.capture-heading-file > span:first-child { max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.capture-heading-file > span:last-child { flex: none; white-space: nowrap; }

.stage-heading .file-state-chip {
  display: inline-flex;
  align-items: center;
  padding: 3px 8px;
  border: 1px solid color-mix(in srgb, var(--destructive) 35%, var(--border));
  border-radius: 99px;
  background: color-mix(in srgb, var(--destructive) 8%, transparent);
  color: var(--destructive);
  font-size: 10px;
  white-space: nowrap;
}

.stage-heading .file-state-chip[data-state="replaced"],
.stage-heading .file-state-chip[data-state="unavailable"] {
  border-color: color-mix(in srgb, var(--warn) 45%, var(--border));
  background: color-mix(in srgb, var(--warn) 8%, transparent);
  color: var(--warn);
}

.preview-unavailable {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-direction: column;
  gap: 7px;
  padding: 30px;
  color: var(--warn);
  text-align: center;
}

.preview-unavailable strong {
  color: var(--foreground);
  font-size: 13px;
}

.preview-unavailable span {
  color: var(--muted-foreground);
  font-size: 11px;
}


.capture-list-toolbar { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px 16px; margin-bottom: 10px; }
.capture-list-title { display: flex; align-items: center; gap: 8px; }
.capture-list-title h2 { margin: 0; font-size: 13px; font-weight: 600; white-space: nowrap; }
.capture-list-title h2 span { margin-left: 5px; color: var(--muted-foreground); font-size: 11px; font-weight: 400; }
.capture-list-hint { color: var(--muted-foreground); font-size: 11px; }
.capture-card { position: relative; }
.capture-card-select { display: block; width: 100%; padding: 0; border: 0; background: transparent; color: inherit; cursor: pointer; text-align: left; }
.capture-card-select:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }

</style>
