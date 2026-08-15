import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "@/lib/toast";

const { api, pickFile, pickDirectory, openDirectoryExternal, revealPath, leaveGuard, recordClientEvent } = vi.hoisted(() => ({
  api: {
    getVisionSettings: vi.fn(),
    getAppSettings: vi.fn(),
    getThumbnailCacheStatus: vi.fn(),
    getArchiveNamingSettings: vi.fn(),
    getRecognitionSettings: vi.fn(),
    getRecognitionDefaults: vi.fn(),
    getProcessingSettings: vi.fn(),
    listBundledFonts: vi.fn(),
    updateVisionSettings: vi.fn(),
    updateAppSettings: vi.fn(),
    updateArchiveNamingSettings: vi.fn(),
    updateRecognitionSettings: vi.fn(),
    updateProcessingSettings: vi.fn(),
    checkVision: vi.fn(),
    runtimeStatus: vi.fn(),
    getProcessResourceStatus: vi.fn(),
    getStorageResourceStatus: vi.fn(),
    cleanupResource: vi.fn(),
  },
  pickFile: vi.fn(),
  pickDirectory: vi.fn(),
  openDirectoryExternal: vi.fn(),
  revealPath: vi.fn(),
  leaveGuard: { current: null as null | ((...args: unknown[]) => unknown) },
  recordClientEvent: vi.fn(),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  pickFile,
  pickDirectory,
  openDirectoryExternal,
  revealPath,
}));

vi.mock("vue-router", () => ({
  onBeforeRouteLeave: (guard: (...args: unknown[]) => unknown) => {
    leaveGuard.current = guard;
  },
}));

vi.mock("@/lib/toast", () => ({
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

vi.mock("@/lib/client-log", () => ({ recordClientEvent }));

import Settings from "./Settings.vue";

const visionSettings = {
  pythonExecutablePath: "C:\\Python311\\python.exe",
  pythonModuleRoot: "C:\\sv\\python\\src",
  yunetModelPath: "C:\\models\\yunet.onnx",
  sfaceModelPath: "C:\\models\\sface.onnx",
  recognizer: "sface" as const,
  arcfaceModelPath: null,
  fontPath: null,
};

const appSettings = {
  classifyShortcut: "Ctrl+Shift+S",
  noteShortcut: "Ctrl+Shift+N",
  showPrivateByDefault: false,
  autoSaveNotes: true,
  splitPopupWindows: false,
  autoCloseEmptyPopup: false,
  thumbnailCacheSizeMb: 256,
  imageProcessingCoreLimit: 4,
};

const namingSettings = {
  template: "{source} - {character} - {id}",
  separator: " - ",
};

const recognitionSettings = {
  extractAtRegistration: true,
  verificationEnabled: true,
  profiles: {
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
  },
  minSampleSharpness: 3,
  minFaceAreaRatio: 0.005,
  minFaceConfidence: 0.6,
};

const processingSettings = {
  detection: {
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
  },
  annotation: {
    textColor: [80, 220, 255],
    strokeColor: [0, 0, 0],
    strokeWidth: null,
    padding: null,
    faceBoxExpansion: null,
    faceTextPosition: null,
    fallbackPosition: null,
    textOffsetX: null,
    textOffsetY: null,
    fontSize: null,
  },
  crop: {
    aspectRatio: null,
    scaleX: null,
    scaleTop: null,
    scaleBottom: null,
    minSize: null,
  },
  annotatePerson: true,
};

const cacheStatus = {
  dir: "C:\\cache",
  totalBytes: 1024 * 1024,
  limitBytes: 256 * 1024 * 1024,
};

let wrapper: ReturnType<typeof mount> | null = null;

function mountSettings() {
  wrapper = mount(Settings, {
    attachTo: document.body,
    global: {
      stubs: {
        ColorField: true,
        CornerFallbackPicker: true,
        FaceTextPositionPicker: true,
      },
    },
  });
  return wrapper;
}

beforeEach(() => {
  api.runtimeStatus.mockResolvedValue({ engineStatus: "unconfigured", workerStatus: "idle" });
  api.getVisionSettings.mockResolvedValue({ ...visionSettings });
  api.getAppSettings.mockResolvedValue({ ...appSettings });
  api.getThumbnailCacheStatus.mockResolvedValue({ ...cacheStatus });
  api.getArchiveNamingSettings.mockResolvedValue({ ...namingSettings });
  api.getRecognitionSettings.mockResolvedValue(JSON.parse(JSON.stringify(recognitionSettings)));
  api.getProcessingSettings.mockResolvedValue(JSON.parse(JSON.stringify(processingSettings)));
  api.listBundledFonts.mockResolvedValue([]);
  api.getRecognitionDefaults.mockResolvedValue({
    confidenceThreshold: 0.5,
    margin: 0.05,
    verificationHighThreshold: 0.65,
    verificationLowThreshold: 0.4,
    crossCheckDelta: 0.1,
  });
  api.updateVisionSettings.mockImplementation(async (settings: unknown) => ({ ...(settings as object) }));
  api.updateAppSettings.mockImplementation(async (settings: unknown) => ({ ...(settings as object) }));
  api.updateArchiveNamingSettings.mockImplementation(async (settings: unknown) => ({
    ...(settings as object),
  }));
  api.updateRecognitionSettings.mockImplementation(async (settings: unknown) => ({
    ...(settings as object),
  }));
  api.updateProcessingSettings.mockResolvedValue(undefined);
  api.getProcessResourceStatus.mockResolvedValue({
    capturedAt: "2026-08-11T08:00:00Z",
    approximate: false,
    logicalProcessors: 16,
    groups: [],
    totalCpuPercent: null,
    totalWorkingSetBytes: 0,
    totalPrivateBytes: 0,
  });
  api.getStorageResourceStatus.mockResolvedValue({
    capturedAt: "2026-08-11T08:00:00Z",
    entries: [],
    totalBytes: 0,
  });
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  wrapper?.unmount();
  wrapper = null;
});

describe("Settings category navigation", () => {
  it("renders Chinese categories with every form mounted and only the active pane visible", async () => {
    const page = mountSettings();
    await flushPromises();

    const tabs = page.findAll('[role="tab"]');
    expect(tabs.map((tab) => tab.text())).toEqual([
      "通用设置",
      "归档命名规则",
      "自动角色建议",
      "视觉处理参数",
      "视觉引擎",
      "资源与存储",
    ]);
    expect(tabs[0].attributes("aria-selected")).toBe("true");
    expect(page.findAll('[role="tabpanel"]')).toHaveLength(6);
    expect(page.findAll("form")).toHaveLength(5);

    expect(page.get("#classify-shortcut").isVisible()).toBe(true);
    expect(page.get("#naming-template").isVisible()).toBe(false);
    expect(page.find("#naming-template").exists()).toBe(true);
    expect(page.find(".settings-nav-dirty").exists()).toBe(false);
  });

  it("preserves input drafts across category switches and supports keyboard navigation", async () => {
    const page = mountSettings();
    await flushPromises();

    await page.get("#classify-shortcut").setValue("Ctrl+Shift+X");
    expect(api.updateAppSettings).not.toHaveBeenCalled();
    expect(page.get("#settings-tab-general").text()).toContain("未保存");

    (page.get("#settings-tab-general").element as HTMLElement).focus();
    await page.get('[role="tablist"]').trigger("keydown", { key: "ArrowDown" });
    await flushPromises();

    expect(page.get("#settings-tab-naming").attributes("aria-selected")).toBe("true");
    expect(document.activeElement?.id).toBe("settings-tab-naming");
    expect(page.get("#classify-shortcut").isVisible()).toBe(false);
    expect((page.get("#classify-shortcut").element as HTMLInputElement).value).toBe(
      "Ctrl+Shift+X",
    );

    await page.get('[role="tablist"]').trigger("keydown", { key: "ArrowUp" });
    await flushPromises();

    expect(page.get("#settings-tab-general").attributes("aria-selected")).toBe("true");
    expect(page.get("#classify-shortcut").isVisible()).toBe(true);
    expect((page.get("#classify-shortcut").element as HTMLInputElement).value).toBe(
      "Ctrl+Shift+X",
    );
  });

  it("keeps unsaved recognition threshold drafts when switching categories", async () => {
    const page = mountSettings();
    await flushPromises();

    await page.get("#settings-tab-recognition").trigger("click");
    await page.get("#recognition-threshold").setValue("0.55");
    expect(page.get("#settings-tab-recognition").text()).toContain("未保存");

    await page.get("#settings-tab-general").trigger("click");
    expect(page.get("#recognition-threshold").isVisible()).toBe(false);
    await page.get("#settings-tab-recognition").trigger("click");

    expect((page.get("#recognition-threshold").element as HTMLInputElement).value).toBe(
      "0.55",
    );
  });

  it("mounts resource monitoring only while the resource category is active", async () => {
    vi.useFakeTimers();
    const page = mountSettings();
    await flushPromises();
    expect(api.getProcessResourceStatus).not.toHaveBeenCalled();
    expect(api.getStorageResourceStatus).not.toHaveBeenCalled();

    await page.get("#settings-tab-resources").trigger("click");
    await flushPromises();
    expect(api.getProcessResourceStatus).toHaveBeenCalledTimes(1);
    expect(api.getStorageResourceStatus).toHaveBeenCalledTimes(1);

    await page.get("#settings-tab-general").trigger("click");
    await flushPromises();
    await vi.advanceTimersByTimeAsync(4_000);
    expect(api.getProcessResourceStatus).toHaveBeenCalledTimes(1);
    expect(api.getStorageResourceStatus).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it("keeps per-model recognition drafts across recognizer switches", async () => {
    const page = mountSettings();
    await flushPromises();

    await page.get("#settings-tab-recognition").trigger("click");
    await page.get("#recognition-threshold").setValue("0.55");

    await page.get("#settings-tab-vision").trigger("click");
    await page.get("#recognizer").setValue("arcface");
    await flushPromises();
    await page.get("#settings-tab-recognition").trigger("click");
    expect((page.get("#recognition-threshold").element as HTMLInputElement).value).toBe(
      "0.5",
    );

    await page.get("#settings-tab-vision").trigger("click");
    await page.get("#recognizer").setValue("sface");
    await flushPromises();
    await page.get("#settings-tab-recognition").trigger("click");
    expect((page.get("#recognition-threshold").element as HTMLInputElement).value).toBe(
      "0.55",
    );
  });

  it("warns to rebuild the face bank when the recognizer changes on save", async () => {
    const page = mountSettings();
    await flushPromises();

    await page.get("#settings-tab-vision").trigger("click");
    await page.get("#recognizer").setValue("arcface");
    await flushPromises();
    const saveButton = page
      .findAll("button")
      .find((button) => button.text().includes("保存配置"));
    expect(saveButton).toBeDefined();
    await saveButton!.trigger("click");
    await flushPromises();

    expect(api.updateVisionSettings).toHaveBeenCalled();
    expect(toast.info).toHaveBeenCalledWith(
      expect.stringContaining("识别器已切换"),
    );
  });

  it("announces the bundled sidecar engine as ready in the vision pane", async () => {
    api.runtimeStatus.mockResolvedValue({ engineStatus: "sidecar", workerStatus: "idle" });
    const wrapper = mountSettings();
    await flushPromises();

    await wrapper.get("#settings-tab-vision").trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("内置引擎（bundled sidecar）已就绪");
    expect(wrapper.text()).toContain("无需配置 Python");
  });
});

describe("Settings dirty leave protection", () => {
  it("saves an editable image-processing core limit", async () => {
    const page = mountSettings();
    await flushPromises();

    const input = page.get("#image-processing-core-limit");
    expect(input.attributes("disabled")).toBeUndefined();
    expect(input.attributes("min")).toBe("1");
    expect(input.attributes("max")).toBe("32");
    await input.setValue("3");
    await page.get("form").trigger("submit");
    await flushPromises();

    expect(api.updateAppSettings).toHaveBeenCalledWith(
      expect.objectContaining({ imageProcessingCoreLimit: 3 }),
    );
  });

  it("blocks route leave with a confirm while dirty and proceeds without one after saving", async () => {
    const page = mountSettings();
    await flushPromises();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    const next = vi.fn();

    await page.get("#classify-shortcut").setValue("Ctrl+Shift+X");
    const blocked = await leaveGuard.current?.({}, {}, next);
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(blocked).toBe(false);
    expect(next).not.toHaveBeenCalled();

    confirmSpy.mockClear();
    await page.get("form").trigger("submit");
    await flushPromises();
    expect(api.updateAppSettings).toHaveBeenCalledTimes(1);

    const allowed = await leaveGuard.current?.({}, {}, next);
    expect(confirmSpy).not.toHaveBeenCalled();
    expect(allowed).toBe(true);
  });

  it("prevents window close while dirty and stops once the category is saved", async () => {
    const page = mountSettings();
    await flushPromises();

    const cleanEvent = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(cleanEvent);
    expect(cleanEvent.defaultPrevented).toBe(false);

    await page.get("#classify-shortcut").setValue("Ctrl+Shift+X");
    const dirtyEvent = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(dirtyEvent);
    expect(dirtyEvent.defaultPrevented).toBe(true);

    await page.get("form").trigger("submit");
    await flushPromises();
    const savedEvent = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(savedEvent);
    expect(savedEvent.defaultPrevented).toBe(false);
  });
});

describe("Settings processing hierarchy", () => {
  it("uses semantic secondary headings and saves independent positioning margins", async () => {
    const page = mountSettings();
    await flushPromises();

    await page.get("#settings-tab-processing").trigger("click");
    expect(page.findAll("#settings-pane-processing h3").map((heading) => heading.text())).toEqual([
      "01人脸检测",
      "02文字标注",
      "03头像裁剪",
    ]);

    await page.get("#face-box-expansion").setValue("48");
    await page.get("#canvas-padding").setValue("24");
    await page.get("#settings-pane-processing form").trigger("submit");
    await flushPromises();

    expect(api.updateProcessingSettings).toHaveBeenCalledWith(
      expect.objectContaining({
        annotation: expect.objectContaining({
          faceBoxExpansion: 48,
          padding: 24,
        }),
      }),
    );
  });
  it("reports failures when opening the thumbnail cache directory", async () => {
    api.getThumbnailCacheStatus.mockResolvedValue({
      dir: "C:/cache/thumbnails",
      totalBytes: 1024,
      limitBytes: 1024 * 1024,
    });
    const page = mountSettings();
    await flushPromises();

    openDirectoryExternal.mockRejectedValueOnce(new Error("denied by scope"));
    const button = page
      .findAll("button")
      .find((candidate) => candidate.text().includes("打开缓存目录"));
    expect(button).toBeDefined();
    await button!.trigger("click");
    await flushPromises();

    expect(openDirectoryExternal).toHaveBeenCalledWith("C:/cache/thumbnails");
    expect(toast.error).toHaveBeenCalledWith("无法打开缓存目录：denied by scope");
    expect(recordClientEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        level: "error",
        module: "ui.opener",
        event: "open_cache_directory_failed",
        outcome: "failed",
      }),
    );
  });
});

describe("Settings explicit person annotation toggle", () => {
  it("persists the toggle with general settings", async () => {
    const page = mountSettings();
    await flushPromises();

    expect((page.get("#annotate-person").element as HTMLInputElement).checked).toBe(true);

    await page.get("#annotate-person").setValue(false);
    await page.get("form").trigger("submit");
    await flushPromises();

    expect(api.updateAppSettings).toHaveBeenCalledTimes(1);
    expect(api.updateProcessingSettings).toHaveBeenCalledWith(
      expect.objectContaining({ annotatePerson: false }),
    );
  });
});
