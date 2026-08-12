<script setup lang="ts">
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  watch,
  type Component,
} from "vue";
import { onBeforeRouteLeave } from "vue-router";
import {
  Cpu,
  FileSearch,
  FileText,
  FolderOpen,
  HardDrive,
  Keyboard,
  LoaderCircle,
  RotateCcw,
  Save,
  ScanFace,
  Settings2,
  Sparkles,
  Wand2,
} from "@lucide/vue";
import {
  captureApi,
  pickDirectory,
  pickFile,
  type AppSettings,
  type ArchiveNamingSettings,
  type BundledFont,
  type AnnotationSettings,
  type CropSettings,
  type DetectionSettings,
  type ModelRecognitionProfile,
  type ProcessingSettings,
  type RecognitionSettings,
  type ResolvedRecognitionProfile,
  type ThumbnailCacheStatus,
  type VisionHealth,
  type VisionSettings,
} from "@/lib/capture-api";
import { openPathExternal } from "@/lib/capture-api";
import ColorField from "@/components/common/ColorField.vue";
import CornerFallbackPicker from "@/components/common/CornerFallbackPicker.vue";
import FaceTextPositionPicker from "@/components/common/FaceTextPositionPicker.vue";
import PageHeader from "@/components/layout/PageHeader.vue";
import ResourceStoragePanel from "@/components/settings/ResourceStoragePanel.vue";
import { toast } from "@/lib/toast";

const NAMING_PLACEHOLDERS: [string, string][] = [
  ["{source}", "源截图文件名（不含扩展名）"],
  ["{character}", "角色名（无角色时省略）"],
  ["{date}", "截图日期 yyyyMMdd"],
  ["{time}", "截图时间 HHmmss"],
  ["{datetime}", "截图日期与时间 yyyyMMdd-HHmmss"],
  ["{id}", "截图短标识（唯一）"],
  ["{classification}", "分类：person / scene / private"],
  ["{seq}", "序号，目标名被占用时自动递增"],
];

const DEFAULT_NAMING_TEMPLATE = "{source} - {character} - {id}";

type SettingsCategoryKey = "general" | "naming" | "recognition" | "processing" | "vision" | "resources";

const CATEGORIES: { key: SettingsCategoryKey; label: string; icon: Component }[] = [
  { key: "general", label: "通用设置", icon: Settings2 },
  { key: "naming", label: "归档命名规则", icon: FileText },
  { key: "recognition", label: "自动角色建议", icon: ScanFace },
  { key: "processing", label: "视觉处理参数", icon: Wand2 },
  { key: "vision", label: "视觉引擎", icon: Cpu },
  { key: "resources", label: "资源与存储", icon: HardDrive },
];

const activeCategory = ref<SettingsCategoryKey>("general");

const settings = ref<VisionSettings>({
  pythonExecutablePath: null,
  pythonModuleRoot: null,
  yunetModelPath: null,
  sfaceModelPath: null,
  recognizer: null,
  arcfaceModelPath: null,
  fontPath: null,
});
const health = ref<VisionHealth | null>(null);
const runtimeEngineStatus = ref<string | null>(null);
const appSettings = ref<AppSettings>({
  classifyShortcut: "Ctrl+Shift+S",
  noteShortcut: "Ctrl+Shift+N",
  showPrivateByDefault: false,
  autoSaveNotes: true,
  splitPopupWindows: false,
  autoCloseEmptyPopup: false,
  thumbnailCacheSizeMb: 256,
  thumbnailGenerationConcurrency: 1,
});
const cacheStatus = ref<ThumbnailCacheStatus | null>(null);
const busy = ref(false);
const appBusy = ref(false);
const namingSettings = ref<ArchiveNamingSettings>({
  template: DEFAULT_NAMING_TEMPLATE,
  separator: " - ",
});
const recognitionSettings = ref<RecognitionSettings>({
  extractAtRegistration: true,
  verificationEnabled: true,
  profiles: {},
  minSampleSharpness: 3,
  minFaceAreaRatio: 0.005,
  minFaceConfidence: 0.6,
});
const minAreaRatioPercent = computed({
  get: () =>
    recognitionSettings.value.minFaceAreaRatio === null
      ? null
      : Math.round(recognitionSettings.value.minFaceAreaRatio * 1000) / 10,
  set: (value: number | null) => {
    recognitionSettings.value.minFaceAreaRatio =
      value === null ? null : value / 100;
  },
});
const activeModelId = computed(() =>
  settings.value.recognizer === "arcface" ? "arcface-r50" : "opencv-sface",
);
const activeModelLabel = computed(() =>
  settings.value.recognizer === "arcface"
    ? "arcface-r50 · 512D"
    : "opencv-sface · 128D",
);
const profileInput = ref<ModelRecognitionProfile>({
  confidenceThreshold: null,
  margin: null,
  verificationHighThreshold: null,
  verificationLowThreshold: null,
  crossCheckDelta: null,
});
// Benchmark-calibrated defaults, mirroring the Rust `known_defaults` values.
// Used as a local fallback so the restore button works even when the backend
// command is not yet available (stale dev binary).
const BENCHMARK_DEFAULTS: Record<string, ResolvedRecognitionProfile> = {
  "opencv-sface": {
    confidenceThreshold: 0.5,
    margin: 0.05,
    verificationHighThreshold: 0.65,
    verificationLowThreshold: 0.4,
    crossCheckDelta: 0.1,
  },
  "arcface-r50": {
    confidenceThreshold: 0.5,
    margin: 0.1,
    verificationHighThreshold: 0.55,
    verificationLowThreshold: 0.35,
    crossCheckDelta: 0.1,
  },
};

async function getDefaultsFor(
  modelId: string,
): Promise<ResolvedRecognitionProfile> {
  try {
    return await captureApi.getRecognitionDefaults(modelId);
  } catch {
    return BENCHMARK_DEFAULTS[modelId] ?? BENCHMARK_DEFAULTS["opencv-sface"];
  }
}
const processingUi = ref<{
  detection: DetectionSettings;
  annotation: AnnotationSettings;
  crop: CropSettings;
}>({
  detection: emptyDetection(),
  annotation: emptyAnnotation(),
  crop: emptyCrop(),
});
const textColorHex = ref("#50dcff");
const strokeColorHex = ref("#000000");
const namingBusy = ref(false);
const recognitionBusy = ref(false);
const processingBusy = ref(false);
const bundledFonts = ref<BundledFont[]>([]);

interface SettingsBaseline {
  general: AppSettings;
  naming: ArchiveNamingSettings;
  recognition: RecognitionSettings;
  processing: {
    detection: DetectionSettings;
    annotation: AnnotationSettings;
    crop: CropSettings;
    textColorHex: string;
    strokeColorHex: string;
  };
  vision: VisionSettings;
}

/** Deep snapshot of the loaded server state per category. */
const baseline = ref<SettingsBaseline | null>(null);
/** Server truth for the profile currently shown per model id. */
const profileBaselines = ref<Record<string, ModelRecognitionProfile>>({});
/** Unsaved profile drafts kept per model id while switching recognizers. */
const profileDrafts = ref<Record<string, ModelRecognitionProfile>>({});

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const keysA = Object.keys(a as Record<string, unknown>);
  const keysB = Object.keys(b as Record<string, unknown>);
  if (keysA.length !== keysB.length) return false;
  return keysA.every((key) =>
    deepEqual((a as Record<string, unknown>)[key], (b as Record<string, unknown>)[key]),
  );
}

const generalDirty = computed(() =>
  baseline.value ? !deepEqual(appSettings.value, baseline.value.general) : false,
);
const namingDirty = computed(() =>
  baseline.value ? !deepEqual(namingSettings.value, baseline.value.naming) : false,
);
const recognitionDirty = computed(() => {
  if (!baseline.value) return false;
  if (!deepEqual(recognitionSettings.value, baseline.value.recognition)) return true;
  const server = profileBaselines.value[activeModelId.value];
  return server ? !deepEqual(profileInput.value, server) : false;
});
const processingDirty = computed(() => {
  if (!baseline.value) return false;
  const saved = baseline.value.processing;
  return (
    !deepEqual(processingUi.value.detection, saved.detection) ||
    !deepEqual(processingUi.value.annotation, saved.annotation) ||
    !deepEqual(processingUi.value.crop, saved.crop) ||
    textColorHex.value !== saved.textColorHex ||
    strokeColorHex.value !== saved.strokeColorHex
  );
});
const visionDirty = computed(() =>
  baseline.value ? !deepEqual(settings.value, baseline.value.vision) : false,
);
const dirtyByCategory = computed<Record<SettingsCategoryKey, boolean>>(() => ({
  general: generalDirty.value,
  naming: namingDirty.value,
  recognition: recognitionDirty.value,
  processing: processingDirty.value,
  vision: visionDirty.value,
  resources: false,
}));
const hasDirtySettings = computed(() => Object.values(dirtyByCategory.value).some(Boolean));

async function initialize() {
  try {
    settings.value = await captureApi.getVisionSettings();
    captureApi
      .runtimeStatus()
      .then((status) => {
        runtimeEngineStatus.value = status.engineStatus;
      })
      .catch(() => {
        runtimeEngineStatus.value = null;
      });
    appSettings.value = await captureApi.getAppSettings();
    cacheStatus.value = await captureApi.getThumbnailCacheStatus();
    namingSettings.value = await captureApi.getArchiveNamingSettings();
    recognitionSettings.value = await captureApi.getRecognitionSettings();
    await loadRecognitionProfile();
    const processing = await captureApi.getProcessingSettings();
    processingUi.value = {
      detection: processing.detection ?? emptyDetection(),
      annotation: processing.annotation ?? emptyAnnotation(),
      crop: processing.crop ?? emptyCrop(),
    };
    textColorHex.value = rgbToHex(processingUi.value.annotation.textColor);
    strokeColorHex.value = rgbToHex(processingUi.value.annotation.strokeColor);
    bundledFonts.value = await captureApi.listBundledFonts().catch(() => []);
    baseline.value = {
      general: clone(appSettings.value),
      naming: clone(namingSettings.value),
      recognition: clone(recognitionSettings.value),
      processing: {
        detection: clone(processingUi.value.detection),
        annotation: clone(processingUi.value.annotation),
        crop: clone(processingUi.value.crop),
        textColorHex: textColorHex.value,
        strokeColorHex: strokeColorHex.value,
      },
      vision: clone(settings.value),
    };
  } catch (error) {
    toast.error(normalizeError(error));
  }
}

async function loadRecognitionProfile(modelId = activeModelId.value) {
  const profile = recognitionSettings.value.profiles[modelId] ?? {};
  const hasValues =
    profile.confidenceThreshold != null ||
    profile.margin != null ||
    profile.verificationHighThreshold != null ||
    profile.verificationLowThreshold != null ||
    profile.crossCheckDelta != null;
  const shown: ModelRecognitionProfile = hasValues
    ? {
        confidenceThreshold: profile.confidenceThreshold ?? null,
        margin: profile.margin ?? null,
        verificationHighThreshold: profile.verificationHighThreshold ?? null,
        verificationLowThreshold: profile.verificationLowThreshold ?? null,
        crossCheckDelta: profile.crossCheckDelta ?? null,
      }
    : await getDefaultsFor(modelId).then((defaults) => ({
        confidenceThreshold: defaults.confidenceThreshold,
        margin: defaults.margin,
        verificationHighThreshold: defaults.verificationHighThreshold,
        verificationLowThreshold: defaults.verificationLowThreshold,
        crossCheckDelta: defaults.crossCheckDelta,
      }));
  // Server truth for this model; drafts are kept separately so switching
  // recognizers does not silently drop unsaved threshold edits.
  profileBaselines.value[modelId] = clone(shown);
  profileInput.value = clone(profileDrafts.value[modelId] ?? shown);
}

watch(activeModelId, (newModel, oldModel) => {
  if (oldModel) profileDrafts.value[oldModel] = clone(profileInput.value);
  void loadRecognitionProfile(newModel);
});

async function refreshCacheStatus() {
  cacheStatus.value = await captureApi.getThumbnailCacheStatus().catch(() => null);
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(0)} MB`;
  return `${Math.round(bytes / 1024)} KB`;
}

async function openCacheDirectory(directory: string) {
  try {
    await openPathExternal(directory);
  } catch (error) {
    toast.error(`无法打开缓存目录：${normalizeError(error)}`);
  }
}

async function choosePython() {
  settings.value.pythonExecutablePath = await pickFile([
    { name: "Python", extensions: ["exe"] },
  ]);
}

async function chooseModuleRoot() {
  settings.value.pythonModuleRoot = await pickDirectory();
}

async function chooseModel() {
  settings.value.yunetModelPath = await pickFile([
    { name: "ONNX model", extensions: ["onnx"] },
  ]);
}

async function chooseSFace() {
  settings.value.sfaceModelPath = await pickFile([
    { name: "ONNX model", extensions: ["onnx"] },
  ]);
}

async function chooseArcFace() {
  settings.value.arcfaceModelPath = await pickFile([
    { name: "ONNX model", extensions: ["onnx"] },
  ]);
}

async function chooseFont() {
  settings.value.fontPath = await pickFile([
    { name: "Font", extensions: ["ttf", "ttc", "otf"] },
  ]);
}

async function save() {
  busy.value = true;
  health.value = null;
  try {
    const previousRecognizer = baseline.value?.vision?.recognizer;
    settings.value = await captureApi.updateVisionSettings(settings.value);
    if (previousRecognizer && previousRecognizer !== settings.value.recognizer) {
      toast.info("识别器已切换：请在各项目「重建人脸样本库」，否则新截图不会产生建议。");
    }
    if (baseline.value) baseline.value.vision = clone(settings.value);
    toast.success("视觉引擎配置已保存。");
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    busy.value = false;
  }
}

async function saveAppSettings() {
  appBusy.value = true;
  try {
    appSettings.value = await captureApi.updateAppSettings(appSettings.value);
    if (baseline.value) baseline.value.general = clone(appSettings.value);
    await refreshCacheStatus();
    toast.success("通用设置已保存。");
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    appBusy.value = false;
  }
}

async function saveNaming() {
  namingBusy.value = true;
  try {
    namingSettings.value = await captureApi.updateArchiveNamingSettings(
      namingSettings.value,
    );
    if (baseline.value) baseline.value.naming = clone(namingSettings.value);
    toast.success("归档命名规则已保存，只影响新的归档。");
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    namingBusy.value = false;
  }
}

async function saveRecognition() {
  recognitionBusy.value = true;
  try {
    recognitionSettings.value.profiles[activeModelId.value] = {
      ...profileInput.value,
    };
    recognitionSettings.value = await captureApi.updateRecognitionSettings(
      recognitionSettings.value,
    );
    profileDrafts.value[activeModelId.value] = clone(profileInput.value);
    profileBaselines.value[activeModelId.value] = clone(profileInput.value);
    if (baseline.value) baseline.value.recognition = clone(recognitionSettings.value);
    toast.success("识别建议设置已保存。");
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    recognitionBusy.value = false;
  }
}

async function restoreRecognitionDefaults() {
  if (recognitionBusy.value) return;
  recognitionBusy.value = true;
  try {
    const defaults = await getDefaultsFor(activeModelId.value);
    profileInput.value = {
      confidenceThreshold: defaults.confidenceThreshold,
      margin: defaults.margin,
      verificationHighThreshold: defaults.verificationHighThreshold,
      verificationLowThreshold: defaults.verificationLowThreshold,
      crossCheckDelta: defaults.crossCheckDelta,
    };
    toast.success(
      `已填入 ${activeModelId.value} 默认值：阈值 ${defaults.confidenceThreshold} / ` +
        `margin ${defaults.margin} / 校验 ${defaults.verificationHighThreshold}-` +
        `${defaults.verificationLowThreshold}-${defaults.crossCheckDelta}，` +
        `点击"保存建议设置"生效。`,
    );
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    recognitionBusy.value = false;
  }
}

function emptyDetection(): DetectionSettings {
  return {
    scoreThreshold: null,
    nmsThreshold: null,
    topK: null,
    areaWeight: null,
    confidenceWeight: null,
    centerWeight: null,
    sharpnessWeight: null,
    edgePenaltyWeight: null,
    edgeMarginRatio: null,
    minSharpness: null,
    blurPenaltyWeight: null,
  };
}

function emptyAnnotation(): AnnotationSettings {
  return {
    textColor: null,
    strokeColor: null,
    strokeWidth: null,
    padding: null,
    faceBoxExpansion: null,
    faceTextPosition: null,
    fallbackPosition: null,
    textOffsetX: null,
    textOffsetY: null,
    fontSize: null,
  };
}

function emptyCrop(): CropSettings {
  return {
    aspectRatio: null,
    scaleX: null,
    scaleTop: null,
    scaleBottom: null,
    minSize: null,
  };
}

function rgbToHex(color: [number, number, number] | null): string {
  if (!color) return "#50dcff";
  return (
    "#" +
    color
      .map((value) => Math.max(0, Math.min(255, value)).toString(16).padStart(2, "0"))
      .join("")
  );
}

const detectionFields: {
  key: keyof DetectionSettings;
  label: string;
  hint: string;
  placeholder: number;
  min: number;
  max: number;
  step: number;
}[] = [
  { key: "scoreThreshold", label: "检测置信度阈值", hint: "人脸检测置信度下限，低于此分数不认为是脸", placeholder: 0.6, min: 0, max: 1, step: 0.05 },
  { key: "nmsThreshold", label: "NMS 阈值", hint: "重叠人脸去重阈值，越大越容易保留重叠框", placeholder: 0.3, min: 0, max: 1, step: 0.05 },
  { key: "topK", label: "最大候选数", hint: "单张图最多保留的候选人脸数量", placeholder: 5000, min: 1, max: 100000, step: 1 },
  { key: "areaWeight", label: "面积权重", hint: "主脸评分中“脸面积大”的权重", placeholder: 1.5, min: 0, max: 100, step: 0.1 },
  { key: "confidenceWeight", label: "置信度权重", hint: "主脸评分中检测置信度的权重", placeholder: 1, min: 0, max: 100, step: 0.1 },
  { key: "centerWeight", label: "居中权重", hint: "主脸评分中“靠近画面中心”的权重", placeholder: 3, min: 0, max: 100, step: 0.1 },
  { key: "sharpnessWeight", label: "清晰度权重", hint: "主脸评分中清晰度（拉普拉斯方差）的权重", placeholder: 1.2, min: 0, max: 100, step: 0.1 },
  { key: "edgePenaltyWeight", label: "边缘惩罚权重", hint: "主脸评分中“贴住画面边缘”的惩罚力度", placeholder: 2, min: 0, max: 100, step: 0.1 },
  { key: "edgeMarginRatio", label: "边缘留白比例", hint: "判定“贴边”的留白比例阈值（相对画面尺寸）", placeholder: 0.18, min: 0, max: 0.49, step: 0.01 },
  { key: "minSharpness", label: "最小清晰度", hint: "清晰度评分的基准值，越高越要求人脸锐利", placeholder: 120, min: 0.01, max: 1000000, step: 1 },
  { key: "blurPenaltyWeight", label: "模糊惩罚权重", hint: "主脸评分中模糊（低清晰度）的惩罚力度", placeholder: 1.5, min: 0, max: 100, step: 0.1 },
];

const cropFields: {
  key: keyof CropSettings;
  label: string;
  hint: string;
  placeholder: number;
  step: number;
}[] = [
  { key: "scaleX", label: "横向留白倍数", hint: "头像左右留白 = 脸宽 × 此倍数", placeholder: 1.8, step: 0.1 },
  { key: "scaleTop", label: "顶部留白倍数", hint: "头像上方留白 = 脸高 × 此倍数", placeholder: 1.3, step: 0.1 },
  { key: "scaleBottom", label: "底部留白倍数", hint: "头像下方留白 = 脸高 × 此倍数", placeholder: 1.8, step: 0.1 },
  { key: "minSize", label: "头像最小尺寸", hint: "头像最短边小于此像素则跳过裁剪", placeholder: 224, step: 1 },
];

function hexToRgb(hex: string): [number, number, number] {
  const value = hex.replace("#", "");
  const parsed = [0, 2, 4].map((index) =>
    parseInt(value.slice(index, index + 2), 16),
  );
  return [parsed[0] ?? 0, parsed[1] ?? 0, parsed[2] ?? 0];
}

function hasValue<T extends Record<string, unknown>>(group: T): boolean {
  return Object.values(group).some((value) => value !== null && value !== "");
}

function buildProcessingPayload(): ProcessingSettings {
  const annotation = processingUi.value.annotation;
  annotation.textColor = textColorHex.value ? hexToRgb(textColorHex.value) : null;
  annotation.strokeColor = strokeColorHex.value
    ? hexToRgb(strokeColorHex.value)
    : null;
  return {
    detection: hasValue(processingUi.value.detection)
      ? processingUi.value.detection
      : null,
    annotation: hasValue(annotation) ? annotation : null,
    crop: hasValue(processingUi.value.crop) ? processingUi.value.crop : null,
  };
}

function resetProcessingGroup(
  group: "detection" | "annotation" | "crop",
) {
  if (group === "detection") processingUi.value.detection = emptyDetection();
  if (group === "annotation") {
    processingUi.value.annotation = emptyAnnotation();
    textColorHex.value = "#50dcff";
    strokeColorHex.value = "#000000";
  }
  if (group === "crop") processingUi.value.crop = emptyCrop();
}

async function saveProcessing() {
  processingBusy.value = true;
  try {
    await captureApi.updateProcessingSettings(buildProcessingPayload());
    if (baseline.value) {
      baseline.value.processing = {
        detection: clone(processingUi.value.detection),
        annotation: clone(processingUi.value.annotation),
        crop: clone(processingUi.value.crop),
        textColorHex: textColorHex.value,
        strokeColorHex: strokeColorHex.value,
      };
    }
    toast.success("视觉处理参数已保存，下次处理生效。");
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    processingBusy.value = false;
  }
}

function resetNaming() {
  namingSettings.value = {
    template: DEFAULT_NAMING_TEMPLATE,
    separator: " - ",
  };
}

function sanitizeWindowsComponent(value: string): string {
  let sanitized = value
    .replace(/[\x00-\x1f<>:"/\\|?*]/g, "_")
    .slice(0, 80)
    .trim()
    .replace(/[. ]+$/, "");
  if (!sanitized) return "capture";
  const upper = sanitized.toUpperCase();
  const reserved =
    ["CON", "PRN", "AUX", "NUL"].includes(upper) ||
    /^COM[1-9]$/.test(upper) ||
    /^LPT[1-9]$/.test(upper);
  return reserved ? `_${sanitized}` : sanitized;
}

function shortIdentifier(value: string): string {
  const short = [...value]
    .filter((character) => /[A-Za-z0-9]/.test(character))
    .join("")
    .slice(0, 8);
  return short || "capture";
}

function datePart(iso: string): string {
  return iso.slice(0, 10).replace(/\D/g, "");
}

function timePart(iso: string): string {
  return iso.length >= 19 ? iso.slice(11, 19).replace(/\D/g, "") : "";
}

const namingPreview = computed(() => {
  const template = namingSettings.value.template || DEFAULT_NAMING_TEMPLATE;
  const separator = namingSettings.value.separator;
  const now = new Date().toISOString();
  const shortId = shortIdentifier("a1b2c3d4e5f6");
  const values: Record<string, string> = {
    "{source}": sanitizeWindowsComponent("shot-001"),
    "{character}": sanitizeWindowsComponent("Ava"),
    "{date}": datePart(now),
    "{time}": timePart(now),
    "{datetime}": `${datePart(now)}-${timePart(now)}`,
    "{id}": shortId,
    "{classification}": "person",
    "{seq}": "1",
  };
  const unknown = template.match(/\{[^}]*\}/g)?.filter(
    (token) => !(token in values),
  );
  let raw = template;
  for (const [token, value] of Object.entries(values)) {
    raw = raw.split(token).join(value);
  }
  const hasId = template.includes("{id}");
  const hasSeq = template.includes("{seq}");
  if (!hasId && !hasSeq) raw += separator + shortId;
  raw += "-avatar";
  if (separator) {
    const repeated = separator + separator;
    while (raw.includes(repeated)) raw = raw.split(repeated).join(separator);
  }
  return {
    name: sanitizeWindowsComponent(raw) + ".png",
    unknown,
  };
});

async function checkHealth() {
  busy.value = true;
  try {
    health.value = await captureApi.checkVision();
  } catch (error) {
    toast.error(normalizeError(error));
  } finally {
    busy.value = false;
  }
}

function clearOptionalFont() {
  settings.value.fontPath = null;
}

function fontOptions(): { label: string; value: string }[] {
  const FONT_LABELS: Record<string, string> = {
    "SmileySans-Oblique": "得意黑 Smiley Sans（斜体展示）",
    "LXGWWenKai-Regular": "霞鹜文楷 LXGW WenKai（手写楷体）",
    "NotoSansSC-Regular": "Noto Sans SC（思源黑体）",
    "ZCOOLKuaiLe-Regular": "站酷快乐体（圆润可爱）",
    "ZCOOLQingKeHuangYou-Regular": "站酷高端黑（硬朗标题）",
    "ZCOOLXiaoWei-Regular": "站酷小薇（文艺纤细）",
  };
  const options = bundledFonts.value.map((font) => ({
    label: FONT_LABELS[font.name] ?? font.name,
    value: font.path,
  }));
  const current = settings.value.fontPath;
  if (current && !options.some((option) => option.value === current)) {
    options.unshift({ label: `自定义：${current}`, value: current });
  }
  return options;
}

function normalizeError(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function onBeforeUnload(event: BeforeUnloadEvent) {
  if (hasDirtySettings.value) {
    event.preventDefault();
    event.returnValue = "";
  }
}

onBeforeRouteLeave(() => {
  if (!hasDirtySettings.value) return true;
  return window.confirm("有未保存的设置更改，离开将丢失这些更改。确定要离开吗？");
});

function setActiveCategory(key: SettingsCategoryKey) {
  activeCategory.value = key;
}

function onNavKeydown(event: KeyboardEvent) {
  const index = CATEGORIES.findIndex((category) => category.key === activeCategory.value);
  if (index < 0) return;
  let next = -1;
  if (event.key === "ArrowDown" || event.key === "ArrowRight") {
    next = (index + 1) % CATEGORIES.length;
  } else if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
    next = (index - 1 + CATEGORIES.length) % CATEGORIES.length;
  } else if (event.key === "Home") {
    next = 0;
  } else if (event.key === "End") {
    next = CATEGORIES.length - 1;
  } else {
    return;
  }
  event.preventDefault();
  const target = CATEGORIES[next];
  activeCategory.value = target.key;
  void nextTick(() => document.getElementById(`settings-tab-${target.key}`)?.focus());
}

onMounted(() => {
  window.addEventListener("beforeunload", onBeforeUnload);
  void initialize();
});

onBeforeUnmount(() => {
  window.removeEventListener("beforeunload", onBeforeUnload);
});
</script>

<template>
  <section class="settings-page">
    <PageHeader
      eyebrow="Scene Vault"
      title="设置"
      description="通用交互、可选 AI 引擎与本地资源诊断；未配置 AI 时核心素材管理仍然可用。"
    >
      <template #actions>
        <Keyboard :size="30" />
      </template>
    </PageHeader>

    <div class="settings-layout">
      <nav class="settings-nav" role="tablist" aria-label="设置分类" @keydown="onNavKeydown">
        <button
          v-for="category in CATEGORIES"
          :id="`settings-tab-${category.key}`"
          :key="category.key"
          type="button"
          role="tab"
          :aria-selected="activeCategory === category.key"
          :aria-controls="`settings-pane-${category.key}`"
          :tabindex="activeCategory === category.key ? 0 : -1"
          :class="{ active: activeCategory === category.key }"
          @click="setActiveCategory(category.key)"
        >
          <component :is="category.icon" :size="16" />
          <span>{{ category.label }}</span>
          <span v-if="dirtyByCategory[category.key]" class="settings-nav-dirty">未保存</span>
        </button>
      </nav>

      <div class="settings-panels">
        <div
          id="settings-pane-general"
          class="settings-pane"
          role="tabpanel"
          aria-labelledby="settings-tab-general"
          v-show="activeCategory === 'general'"
        >
          <form class="settings-card" @submit.prevent="saveAppSettings">
      <div class="settings-card-title">
        <span class="eyebrow">通用</span>
        <h2>通用设置</h2>
      </div>
      <div class="settings-field">
        <label for="classify-shortcut">截图分类快捷键</label>
        <p>游戏中按下时呼出置顶分类小窗。格式示例：<code>Ctrl+Shift+S</code>。</p>
        <div><input id="classify-shortcut" v-model="appSettings.classifyShortcut" spellcheck="false" /><button type="button" class="shortcut-reset" @click="appSettings.classifyShortcut = 'Ctrl+Shift+S'">恢复默认</button></div>
      </div>
      <div class="settings-field">
        <label for="note-shortcut">快速笔记快捷键</label>
        <p>游戏中按下时呼出置顶 Markdown 笔记小窗。格式示例：<code>Ctrl+Shift+N</code>。</p>
        <div><input id="note-shortcut" v-model="appSettings.noteShortcut" spellcheck="false" /><button type="button" class="shortcut-reset" @click="appSettings.noteShortcut = 'Ctrl+Shift+N'">恢复默认</button></div>
      </div>
      <div class="settings-field toggle-row">
        <div class="toggle-text">
          <label for="show-private">默认显示收藏图</label>
          <p>历史与首页默认隐藏收藏图；打开后默认显示，仍可在历史页单独切换。</p>
        </div>
        <label class="toggle">
          <input id="show-private" v-model="appSettings.showPrivateByDefault" type="checkbox" />
          <span class="toggle-track" />
        </label>
      </div>
      <div class="settings-field toggle-row">
        <div class="toggle-text">
          <label for="auto-save-notes">笔记失焦自动保存</label>
          <p>笔记编辑框或整个弹窗失去焦点（切回游戏）时自动保存，避免丢失。</p>
        </div>
        <label class="toggle">
          <input id="auto-save-notes" v-model="appSettings.autoSaveNotes" type="checkbox" />
          <span class="toggle-track" />
        </label>
      </div>
      <div class="settings-field toggle-row">
        <div class="toggle-text">
          <label for="split-popup-windows">分类与笔记拆分窗口</label>
          <p>关闭时两个快捷键共用同一个工作台弹窗；开启后各快捷键呼出独立小窗。</p>
        </div>
        <label class="toggle">
          <input id="split-popup-windows" v-model="appSettings.splitPopupWindows" type="checkbox" />
          <span class="toggle-track" />
        </label>
      </div>
      <div class="settings-field toggle-row">
        <div class="toggle-text">
          <label for="auto-close-empty-popup">无待分类时自动关闭弹窗</label>
          <p>没有待分类截图时 3 秒后自动隐藏弹窗。默认关闭，避免误关正在编辑的笔记。</p>
        </div>
        <label class="toggle">
          <input id="auto-close-empty-popup" v-model="appSettings.autoCloseEmptyPopup" type="checkbox" />
          <span class="toggle-track" />
        </label>
      </div>
      <div class="settings-field">
        <label for="thumbnail-cache-size">缩略图缓存上限（MB）</label>
        <p>
          列表缩略图（320px JPEG）缓存在此目录：<code>{{ cacheStatus?.dir || "…" }}</code>。
          当前占用 {{ cacheStatus ? formatBytes(cacheStatus.totalBytes) : "…" }}，超过上限自动清理最旧文件。
          原始截图与归档文件不受影响。
        </p>
        <div>
          <input
            id="thumbnail-cache-size"
            v-model.number="appSettings.thumbnailCacheSizeMb"
            type="number"
            min="32"
            max="4096"
            step="32"
            class="threshold-input"
          />
          <button
            v-if="cacheStatus?.dir"
            type="button"
            class="clear-field"
            @click="openCacheDirectory(cacheStatus!.dir)"
          >
            打开缓存目录
          </button>
        </div>
      </div>
      <div class="settings-field">
        <label for="thumbnail-concurrency">缩略图生成并发数（CPU 核）</label>
        <p>
          为避免批量导入时占满处理器，缩略图只在进入可视区域后请求，并固定一次生成一张。
          Windows 开发入口仍会将整个进程限制在 CPU 0–3。
        </p>
        <div>
          <input
            id="thumbnail-concurrency"
            v-model.number="appSettings.thumbnailGenerationConcurrency"
            type="number"
            min="1"
            max="1"
            step="1"
            disabled
            class="threshold-input"
          />
        </div>
      </div>
      <div class="settings-actions">
        <button type="submit" class="primary-action" :disabled="appBusy">
          <LoaderCircle v-if="appBusy" class="animate-spin" :size="17" /><Save :size="17" />保存通用设置
        </button>
      </div>
          </form>
        </div>

        <div
          id="settings-pane-naming"
          class="settings-pane"
          role="tabpanel"
          aria-labelledby="settings-tab-naming"
          v-show="activeCategory === 'naming'"
        >
          <form class="settings-card" @submit.prevent="saveNaming">
      <div class="settings-card-title">
        <span class="eyebrow">捕获与归档</span>
        <h2>归档命名规则</h2>
      </div>
      <div class="settings-field">
        <label for="naming-template">命名模板</label>
        <p>按占位符组合归档文件名；头像图会自动在末尾追加 <code>-avatar</code>。模板不含 <code>{id}</code>/<code>{seq}</code> 时自动追加短标识保证唯一。只影响新归档。</p>
        <div><input id="naming-template" v-model="namingSettings.template" spellcheck="false" /><button type="button" @click="resetNaming">恢复默认</button></div>
      </div>
      <div class="settings-field">
        <label for="naming-separator">追加分隔符</label>
        <p>模板缺少唯一标识时，用此分隔符把短标识追加到名称末尾。</p>
        <div><input id="naming-separator" v-model="namingSettings.separator" class="separator-input" spellcheck="false" /></div>
      </div>
      <div class="settings-field">
        <label>占位符</label>
        <ul class="placeholder-list">
          <li v-for="[token, help] in NAMING_PLACEHOLDERS" :key="token">
            <code>{{ token }}</code><span>{{ help }}</span>
          </li>
        </ul>
      </div>
      <div class="settings-field preview-field">
        <label>示例预览</label>
        <p>以 <code>shot-001.png</code>（角色 Ava，人物分类）为例：</p>
        <code class="preview-name">{{ namingPreview.name }}</code>
        <p v-if="namingPreview.unknown" class="preview-warning">未知占位符：{{ namingPreview.unknown.join(" ") }}</p>
      </div>
      <div class="settings-actions">
        <button type="submit" class="primary-action" :disabled="namingBusy">
          <LoaderCircle v-if="namingBusy" class="animate-spin" :size="17" /><Save :size="17" />保存命名规则
        </button>
      </div>
          </form>
        </div>

        <div
          id="settings-pane-recognition"
          class="settings-pane"
          role="tabpanel"
          aria-labelledby="settings-tab-recognition"
          v-show="activeCategory === 'recognition'"
        >
          <form class="settings-card" @submit.prevent="saveRecognition">
      <div class="settings-card-title">
        <span class="eyebrow">人物识别 · 可选</span>
        <h2>自动角色建议</h2>
      </div>
      <div class="settings-field toggle-row">
        <div class="toggle-text">
          <label for="extract-at-registration">登记即提取特征</label>
          <p>新截图一出现就提取人脸特征并匹配角色样本库，分类弹窗显示推荐按钮；关闭后特征在标记后的处理阶段提取，建议只出现在工作台。</p>
        </div>
        <label class="toggle">
          <input id="extract-at-registration" v-model="recognitionSettings.extractAtRegistration" type="checkbox" />
          <span class="toggle-track" />
        </label>
      </div>
      <div class="settings-field">
        <label for="recognition-model">当前识别器：{{ activeModelLabel }}</label>
        <p>以下阈值仅作用于该识别器，与另一识别器互不影响（不同模型的特征分数分布不同）。留空使用 benchmark 校准的默认值。</p>
        <div>
          <button
            type="button"
            class="secondary-action compact-action"
            :disabled="recognitionBusy"
            @click="restoreRecognitionDefaults"
          >
            <RotateCcw :size="14" />恢复 benchmark 默认值
          </button>
        </div>
      </div>
      <div class="settings-field">
        <label for="recognition-threshold">建议置信度阈值</label>
        <p>相似度达到该值才给出建议；值越低建议越多、误报越多。</p>
        <div><input id="recognition-threshold" v-model.number="profileInput.confidenceThreshold" type="number" min="0" max="1" step="0.05" class="threshold-input" /></div>
      </div>
      <div class="settings-field">
        <label for="recognition-margin">建议 margin（分差）</label>
        <p>第一名必须比第二名高出该分差才给出建议；拿不准时宁可沉默。默认 SFace 0.05、ArcFace 0.10（2026-08-08 benchmark 校准）。</p>
        <div><input id="recognition-margin" v-model.number="profileInput.margin" type="number" min="0" max="1" step="0.05" class="threshold-input" /></div>
      </div>
      <div class="settings-field toggle-row">
        <div class="toggle-text">
          <label for="verification-enabled">标记时校验角色</label>
          <p>选角色时用该角色的样本校验相似度；低置信弹框提示并允许二次确认（强制确认的样本默认不参与匹配）。</p>
        </div>
        <label class="toggle">
          <input id="verification-enabled" v-model="recognitionSettings.verificationEnabled" type="checkbox" />
          <span class="toggle-track" />
        </label>
      </div>
      <div class="settings-field">
        <label for="verification-high">校验高分阈值</label>
        <p>相似度达到该值静默通过；低于它显示低置信警告。SFace 默认 0.65，ArcFace 暂定 0.55（待实测重校准）。</p>
        <div><input id="verification-high" v-model.number="profileInput.verificationHighThreshold" type="number" min="0" max="1" step="0.05" class="threshold-input" /></div>
      </div>
      <div class="settings-field">
        <label for="verification-low">校验低分阈值</label>
        <p>低于该值显示强警告（"很可能标错了"）。SFace 默认 0.40，ArcFace 暂定 0.35（待实测重校准）。</p>
        <div><input id="verification-low" v-model.number="profileInput.verificationLowThreshold" type="number" min="0" max="1" step="0.05" class="threshold-input" /></div>
      </div>
      <div class="settings-field">
        <label for="cross-check-delta">交叉校验分差</label>
        <p>当"更像其他角色"的分差超过该值且其分数达到高分阈值时，提示"更像别人"。默认 0.10。</p>
        <div><input id="cross-check-delta" v-model.number="profileInput.crossCheckDelta" type="number" min="0" max="1" step="0.05" class="threshold-input" /></div>
      </div>
      <div class="settings-field">
        <label for="min-sample-sharpness">样本最小清晰度</label>
        <p>人脸清晰度（Laplacian 方差）低于该值时不登记进样本库——桌面卡片/海报/相框里的印刷脸清晰度通常低于 2，正常游戏脸一般 20+。留空 = 不限制。默认 3。</p>
        <div><input id="min-sample-sharpness" v-model.number="recognitionSettings.minSampleSharpness" type="number" min="0" step="1" class="threshold-input" placeholder="3" /><button v-if="recognitionSettings.minSampleSharpness !== null" type="button" class="clear-field" @click="recognitionSettings.minSampleSharpness = null">不限制</button></div>
      </div>
      <div class="settings-field">
        <label for="min-face-area-ratio">最小人脸面积占比（%）</label>
        <p>主脸占整张图的比例低于该值时不登记样本（拦掉背景里的路人小脸）。正常主脸中位约 2.8%，默认 0.5%。留空 = 不限制。</p>
        <div><input id="min-face-area-ratio" v-model.number="minAreaRatioPercent" type="number" min="0" step="0.1" class="threshold-input" placeholder="0.5" /><button v-if="recognitionSettings.minFaceAreaRatio !== null" type="button" class="clear-field" @click="recognitionSettings.minFaceAreaRatio = null">不限制</button></div>
      </div>
      <div class="settings-field">
        <label for="min-face-confidence">最小检测置信度 <span>后续启用</span></label>
        <p>低于该检测置信度的脸不登记样本。当前版本暂不生效，后续迭代接入。</p>
        <div><input id="min-face-confidence" v-model.number="recognitionSettings.minFaceConfidence" type="number" min="0" max="1" step="0.05" class="threshold-input" disabled title="后续版本启用" /></div>
      </div>
      <div class="settings-actions">
        <button type="submit" class="primary-action" :disabled="recognitionBusy">
          <LoaderCircle v-if="recognitionBusy" class="animate-spin" :size="17" /><Save :size="17" />保存建议设置
        </button>
      </div>
          </form>
        </div>

        <div
          id="settings-pane-processing"
          class="settings-pane"
          role="tabpanel"
          aria-labelledby="settings-tab-processing"
          v-show="activeCategory === 'processing'"
        >
          <form class="settings-card" @submit.prevent="saveProcessing">
      <div class="settings-card-title">
        <span class="eyebrow">图像处理 · 可选</span>
        <h2>视觉处理参数</h2>
      </div>
      <section class="processing-group" aria-labelledby="processing-detection-title">
        <div class="processing-group-title">
          <h3 id="processing-detection-title"><span aria-hidden="true">01</span>人脸检测</h3>
          <button type="button" class="clear-field" @click="resetProcessingGroup('detection')">恢复默认</button>
        </div>
        <div class="processing-grid">
          <label v-for="field in detectionFields" :key="field.key" class="processing-field">
            <span>{{ field.label }}</span>
            <input
              v-model.number="processingUi.detection[field.key]"
              type="number"
              :min="field.min"
              :max="field.max"
              :step="field.step"
              :placeholder="String(field.placeholder)"
            />
            <small>{{ field.hint }}</small>
          </label>
        </div>
      </section>

      <section class="processing-group" aria-labelledby="processing-annotation-title">
        <div class="processing-group-title">
          <h3 id="processing-annotation-title"><span aria-hidden="true">02</span>文字标注</h3>
          <button type="button" class="clear-field" @click="resetProcessingGroup('annotation')">恢复默认</button>
        </div>
        <div class="processing-grid">
          <label class="processing-field">
            <span>文字颜色</span>
            <ColorField v-model="textColorHex" title="文字颜色" />
            <small>标注在图上的人物名字颜色</small>
          </label>
          <label class="processing-field">
            <span>描边颜色</span>
            <ColorField v-model="strokeColorHex" title="描边颜色" />
            <small>文字描边颜色，深色描边提升浅色背景可读性</small>
          </label>
          <label class="processing-field">
            <span>描边宽度</span>
            <input v-model.number="processingUi.annotation.strokeWidth" type="number" min="0" max="64" step="1" placeholder="2" />
            <small>文字描边粗细（像素），0 为无描边</small>
          </label>
          <label class="processing-field">
            <span>字号</span>
            <input v-model.number="processingUi.annotation.fontSize" type="number" min="1" max="512" step="1" placeholder="48" />
            <small>角色名字的字体大小（像素）</small>
          </label>
        </div>

        <section class="positioning-config" aria-labelledby="positioning-title">
          <div class="positioning-header">
            <div>
              <h4 id="positioning-title">文字定位</h4>
              <p>分别控制人脸参考框、画面安全区和文字落点；外扩量会叠加在基础安全边距之上。</p>
            </div>
          </div>
          <div class="positioning-controls">
            <label class="processing-field">
              <span>人脸框外扩</span>
              <input
                id="face-box-expansion"
                v-model.number="processingUi.annotation.faceBoxExpansion"
                type="number"
                min="0"
                max="4096"
                step="1"
                placeholder="0"
              />
              <small>在检测框四周额外增加像素；0 保持原有结果，数值越大，文字离脸越远，自定义定位范围也随之扩大。</small>
            </label>
            <label class="processing-field">
              <span>画面安全边距</span>
              <input id="canvas-padding" v-model.number="processingUi.annotation.padding" type="number" min="0" max="4096" step="1" placeholder="32" />
              <small>文字离图片边缘至少保留的像素，同时也是人脸参考框与文字之间的基础间距。</small>
            </label>
          </div>
          <div class="processing-position-row">
            <div class="processing-position-block">
              <div class="processing-position-heading">
                <strong>检测到人脸</strong>
                <span>先选择首选方向，也可以直接拖动圆点。</span>
              </div>
            <FaceTextPositionPicker v-model="processingUi.annotation" />
            </div>
            <div class="processing-position-block is-fallback">
              <div class="processing-position-heading">
                <strong>未检测到人脸</strong>
                <span>选择文字在完整画面中的备用落点。</span>
              </div>
            <CornerFallbackPicker v-model="processingUi.annotation.fallbackPosition" />
            </div>
          </div>
        </section>
      </section>

      <section class="processing-group" aria-labelledby="processing-crop-title">
        <div class="processing-group-title">
          <h3 id="processing-crop-title"><span aria-hidden="true">03</span>头像裁剪</h3>
          <button type="button" class="clear-field" @click="resetProcessingGroup('crop')">恢复默认</button>
        </div>
        <div class="processing-grid">
          <label class="processing-field">
            <span>纵横比</span>
            <input v-model="processingUi.crop.aspectRatio" placeholder="1:1" spellcheck="false" />
            <small>头像裁切比例，如 1:1 正方形、3:4 竖版</small>
          </label>
          <label v-for="field in cropFields" :key="field.key" class="processing-field">
            <span>{{ field.label }}</span>
            <input
              v-model.number="processingUi.crop[field.key]"
              type="number"
              min="0.1"
              :step="field.step"
              :placeholder="String(field.placeholder)"
            />
            <small>{{ field.hint }}</small>
          </label>
        </div>
      </section>

      <div class="settings-actions">
        <button type="submit" class="primary-action" :disabled="processingBusy">
          <LoaderCircle v-if="processingBusy" class="animate-spin" :size="17" /><Save :size="17" />保存处理参数
        </button>
      </div>
          </form>
        </div>

        <div
          id="settings-pane-vision"
          class="settings-pane"
          role="tabpanel"
          aria-labelledby="settings-tab-vision"
          v-show="activeCategory === 'vision'"
        >
          <form class="settings-card" @submit.prevent="save">
      <div class="settings-card-title">
        <span class="eyebrow">本地视觉 · 可选</span>
        <h2>视觉引擎</h2>
      </div>
      <div
        v-if="runtimeEngineStatus === 'sidecar'"
        class="engine-banner engine-banner-sidecar"
        role="status"
      >
        内置引擎（bundled sidecar）已就绪：AI 组件已随安装包提供，无需配置 Python。
      </div>
      <div
        v-else-if="runtimeEngineStatus === 'configured'"
        class="engine-banner engine-banner-configured"
        role="status"
      >
        外部 Python 引擎已配置。
      </div>
      <div class="settings-field">
        <label for="python-path">Python 可执行文件</label>
        <p>开发阶段使用本地 Python 3.11+，不会随应用自动安装。</p>
        <div><input id="python-path" v-model="settings.pythonExecutablePath" placeholder="C:\Python311\python.exe" /><button type="button" @click="choosePython"><FileSearch :size="17" />选择</button></div>
      </div>
      <div class="settings-field">
        <label for="module-root">Python 模块目录</label>
        <p>选择包含 <code>scene_vault_ai</code> 包的目录，例如仓库中的 <code>python\src</code>。</p>
        <div><input id="module-root" v-model="settings.pythonModuleRoot" placeholder="C:\Projects\scene_vault\python\src" /><button type="button" @click="chooseModuleRoot"><FolderOpen :size="17" />选择</button></div>
      </div>
      <div class="settings-field">
        <label for="model-path">YuNet 模型</label>
        <p>模型必须位于本地磁盘，Python 不直接读取 NAS 输出位置。</p>
        <div><input id="model-path" v-model="settings.yunetModelPath" placeholder="C:\Models\face_detection_yunet.onnx" /><button type="button" @click="chooseModel"><FileSearch :size="17" />选择</button></div>
      </div>
      <div class="settings-field">
        <label for="recognizer">识别器</label>
        <p>OpenCV SFace 内置、可随包分发；ArcFace R50（w600k）效果更好但模型为 non-commercial 许可，需自行提供 ONNX 文件，不会随应用分发。切换后需对项目执行"重建人脸样本库"。</p>
        <div>
          <select id="recognizer" v-model="settings.recognizer">
            <option value="sface">OpenCV SFace</option>
            <option value="arcface">ArcFace ONNX</option>
          </select>
        </div>
      </div>
      <div v-if="settings.recognizer === 'sface'" class="settings-field">
        <label for="sface-path">SFace 模型 <span>可选</span></label>
        <p>角色人脸特征模型，用于自动角色建议；未配置时建议功能自动关闭，不影响截图发现、标记与归档。</p>
        <div><input id="sface-path" v-model="settings.sfaceModelPath" placeholder="C:\Models\face_recognition_sface_2021dec.onnx" /><button type="button" @click="chooseSFace"><FileSearch :size="17" />选择</button><button v-if="settings.sfaceModelPath" type="button" class="clear-field" @click="settings.sfaceModelPath = null">清除</button></div>
      </div>
      <div v-else class="settings-field">
        <label for="arcface-path">ArcFace 模型</label>
        <p>InsightFace w600k_r50.onnx（512 维）。模型由你自己提供（如 <code>buffalo_l</code> 包中的 <code>w600k_r50.onnx</code>），本项目不分发该权重。</p>
        <div><input id="arcface-path" v-model="settings.arcfaceModelPath" placeholder="C:\Models\w600k_r50.onnx" /><button type="button" @click="chooseArcFace"><FileSearch :size="17" />选择</button><button v-if="settings.arcfaceModelPath" type="button" class="clear-field" @click="settings.arcfaceModelPath = null">清除</button></div>
      </div>
      <div class="settings-field">
        <label for="font-path">标注字体 <span>可选</span></label>
        <p>选择内置字体或本机字体文件；留空时按系统字体顺序选择微软雅黑、黑体或 Arial。也可以把 <code>.ttf</code> 文件放进 <code>%LOCALAPPDATA%\com.wuhaoli.scene_vault\fonts</code> 后刷新本页。</p>
        <div>
          <select
            id="font-path"
            :value="settings.fontPath ?? ''"
            @change="settings.fontPath = ($event.target as HTMLSelectElement).value || null"
          >
            <option value="">默认（系统选择）</option>
            <option v-for="option in fontOptions()" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
          <button type="button" @click="chooseFont"><FileSearch :size="17" />选择文件…</button>
          <button v-if="settings.fontPath" type="button" class="clear-field" @click="clearOptionalFont">清除</button>
        </div>
      </div>

      <div class="settings-actions">
        <button type="button" class="secondary-action" :disabled="busy" @click="checkHealth">
          <LoaderCircle v-if="busy" class="animate-spin" :size="17" /><Sparkles v-else :size="17" />检查引擎
        </button>
        <button type="submit" class="primary-action" :disabled="busy">
          <Save :size="17" />保存配置
        </button>
      </div>
          </form>

          <section
            v-if="health"
            class="health-card"
            :class="health.processScreenshotAvailable ? 'healthy' : 'degraded'"
            aria-live="polite"
          >
            <div><strong>{{ health.processScreenshotAvailable ? "视觉引擎可用" : "视觉引擎不可用" }}</strong><span>{{ health.status }}</span></div>
            <dl><div><dt>引擎版本</dt><dd>{{ health.engineVersion || "—" }}</dd></div><div><dt>Python</dt><dd>{{ health.pythonVersion || "—" }}</dd></div><div><dt>单图处理</dt><dd>{{ health.processScreenshotAvailable ? "支持" : "不支持" }}</dd></div></dl>
            <p v-if="health.errorMessage">{{ health.errorMessage }}</p>
          </section>
        </div>

        <div
          id="settings-pane-resources"
          class="settings-pane"
          role="tabpanel"
          aria-labelledby="settings-tab-resources"
          v-show="activeCategory === 'resources'"
        >
          <ResourceStoragePanel
            v-if="activeCategory === 'resources'"
            :recognizer="settings.recognizer"
          />
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.settings-page > :deep(.page-header) {
  width: 100%;
  max-width: 1080px;
}

.settings-layout {
  display: flex;
  align-items: flex-start;
  gap: 20px;
  width: 100%;
  max-width: 1080px;
}

.settings-nav {
  position: sticky;
  top: 0;
  display: flex;
  flex: none;
  flex-direction: column;
  gap: 4px;
  width: 220px;
  padding: 6px;
  box-sizing: border-box;
  border: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
  border-radius: 14px;
  background: var(--card);
  box-shadow: var(--card-shadow), var(--inner-highlight);
}

.settings-nav button {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 10px 12px;
  box-sizing: border-box;
  border: 0;
  border-radius: 9px;
  background: transparent;
  color: var(--muted-foreground);
  font-size: 13px;
  font-weight: 600;
  line-height: 1.3;
  text-align: left;
  cursor: pointer;
}

.settings-nav button:hover {
  background: color-mix(in srgb, var(--foreground) 6%, transparent);
  color: var(--foreground);
}

.settings-nav button:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 1px;
}

.settings-nav button.active {
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--foreground);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 45%, transparent);
}

.settings-nav-dirty {
  margin-left: auto;
  flex: none;
  padding: 1px 7px;
  border: 1px solid color-mix(in srgb, var(--destructive) 55%, transparent);
  border-radius: 999px;
  color: var(--destructive);
  font-size: 10px;
  font-weight: 600;
}

.settings-panels {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}

.settings-pane .settings-card,
.settings-pane .health-card {
  width: 100%;
  max-width: none;
}

.settings-pane .health-card {
  margin-top: 14px;
}

.settings-pane .engine-banner {
  margin-top: 14px;
  padding: 10px 14px;
  border-radius: 10px;
  font-size: 13px;
  line-height: 1.5;
}

.settings-pane .engine-banner-sidecar {
  background: color-mix(in srgb, var(--primary) 12%, transparent);
  color: var(--primary);
  border: 1px solid color-mix(in srgb, var(--primary) 35%, transparent);
}

.settings-pane .engine-banner-configured {
  background: color-mix(in srgb, var(--chart-2) 10%, transparent);
  color: var(--foreground);
  border: 1px solid color-mix(in srgb, var(--chart-2) 30%, transparent);
}

@media (max-width: 860px) {
  .settings-layout {
    flex-direction: column;
  }

  .settings-nav {
    flex-direction: row;
    flex-wrap: wrap;
    width: 100%;
  }

  .settings-nav button {
    flex: 1 1 150px;
    width: auto;
  }
}
</style>
