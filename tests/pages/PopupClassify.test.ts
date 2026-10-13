import { flushPromises, mount } from "@vue/test-utils";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { CaptureImageFacts } from "@/lib/capture-api";

const { api, eventHandlers } = vi.hoisted(() => ({
  api: {
    getCaptureFaceRoi: vi.fn(async () => null),
    setCaptureFaceRoi: vi.fn(),
    previewCaptureReset: vi.fn(),
    discardCaptureReset: vi.fn(async () => {}),
    classifyPopupContext: vi.fn(),
    listProjectOverviews: vi.fn(),
    getAppSettings: vi.fn(),
    runtimeStatus: vi.fn(),
    readImage: vi.fn(),
    recordClientEvent: vi.fn(async () => {}),
    readThumbnail: vi.fn(),
    suggestForCapture: vi.fn(),
    label: vi.fn(),
    verifyCaptureIdentity: vi.fn(),
    createCharacter: vi.fn(),
    getCaptureImageFacts: vi.fn(async (): Promise<CaptureImageFacts> => ({ source: null, destination: null })),
  },
  eventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    eventHandlers.set(name, handler);
    return () => eventHandlers.delete(name);
  }),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({
    label: "classify",
    isAlwaysOnTop: vi.fn(async () => false),
    setAlwaysOnTop: vi.fn(async () => {}),
    hide: hideMock,
  })),
}));

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
const hideMock = vi.hoisted(() => vi.fn(async () => {}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

vi.mock("@/lib/capture-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/capture-api")>()),
  captureApi: api,
  pathMimeType: () => "image/png",
}));

import PopupClassify from "@/pages/PopupClassify.vue";

const character = {
  id: "char-1",
  projectId: "project-1",
  name: "Mira",
  aliasesJson: "[]",
  avatarAssetId: null,
  createdAt: "",
  updatedAt: "",
};
const item = {
  id: "item-1",
  projectId: "project-1",
  sessionId: "session-1",
  assetId: null,
  characterId: null,
  classification: "unclassified",
  sourcePath: "D:\\Game\\Screenshots\\001.png",
  annotatedPath: null,
  avatarPath: null,
  destinationPath: null,
  destinationAvatarPath: null,
  status: "awaiting_label",
  faceBoxJson: null,
  errorMessage: null,
  failureStage: null,
  attemptCount: 0,
  nextRetryAt: null,
  processingWarningsJson: "[]",
  suggestedCharacterId: null,
  recognitionConfidence: null,
  recognitionSource: null,
  reviewStatus: "none",
  verificationScore: null,
  verificationStatus: "unverified",
  bestOtherScore: null,
  bestOtherCharacterId: null,
  faceCount: 1,
  capturedAt: "2026-08-01T00:00:00Z",
  processedAt: null,
  archivedAt: null,
  createdAt: "2026-08-01T00:00:00Z",
  updatedAt: "2026-08-01T00:00:00Z",
};

function engineMetadataFixture(speaker: string) {
  return {
    captureItemId: "item-1",
    projectId: "project-1",
    engine: "renpy",
    schemaVersion: 1,
    bridgeVersion: "1.1.2",
    engineVersion: "8.1.3.23091805",
    speakerRaw: speaker,
    speakerSource: "_last_say_who",
    dialogue: "…",
    suggestedCharacterId: null,
    virtualWidth: 1920,
    virtualHeight: 1080,
    physicalWidth: 2560,
    physicalHeight: 1440,
    payloadJson: "{}",
    createdAt: "2026-08-01T00:00:01Z",
    updatedAt: "2026-08-01T00:00:01Z",
  };
}

beforeAll(() => {
  Object.defineProperty(HTMLImageElement.prototype, "decode", { configurable: true, value: vi.fn(async () => {}) });
  Object.defineProperty(URL, "createObjectURL", { value: vi.fn(() => "blob:preview") });
  Object.defineProperty(URL, "revokeObjectURL", { value: vi.fn() });
});

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  eventHandlers.clear();
  api.listProjectOverviews.mockReset().mockResolvedValue([]);
  api.getCaptureImageFacts.mockReset().mockResolvedValue({ source: null, destination: null });
  api.classifyPopupContext.mockResolvedValue({
    items: [item],
    projectId: "project-1",
    projectName: null,
    characters: [character],
  });
  api.getAppSettings.mockResolvedValue({ autoCloseEmptyPopup: false });
  api.runtimeStatus.mockResolvedValue({
    engineStatus: "configured",
    workerStatus: "idle",
    activeCaptureItemId: null,
    activeCaptureSourcePath: null,
    queuedCount: 0,
    archivePendingCount: 0,
    prelabelPendingCount: 0,
    lastError: null,
  });
  api.readImage.mockResolvedValue(new ArrayBuffer(4));
  api.readThumbnail.mockResolvedValue(new ArrayBuffer(4));
  api.suggestForCapture.mockResolvedValue(item);
  invokeMock.mockImplementation(async () => null);
});

describe("PopupClassify", () => {
  it("emits the actual queue project context when embedded despite a different saved project", async () => {
    localStorage.setItem("scene-vault.capture.project", "saved-project");
    api.classifyPopupContext.mockResolvedValue({
      items: [item], projectId: "project-1", projectName: "Queued Game", characters: [character],
    });
    const wrapper = mount(PopupClassify, { props: { embedded: true } });
    try {
      await flushPromises();
      expect(wrapper.emitted("project-context")).toEqual([
        [{ projectId: "project-1", projectName: "Queued Game" }],
      ]);
      expect(wrapper.find(".popup-header").exists()).toBe(false);
      expect(api.listProjectOverviews).not.toHaveBeenCalled();
      api.classifyPopupContext.mockResolvedValue({
        items: [], projectId: "project-2", projectName: "Next Game", characters: [],
      });
      eventHandlers.get("classify:refresh")!({ payload: {} });
      await flushPromises();
      expect(wrapper.emitted("project-context")?.slice(-1)[0]).toEqual([
        { projectId: "project-2", projectName: "Next Game" },
      ]);
      expect(api.listProjectOverviews).not.toHaveBeenCalled();
    } finally {
      wrapper.unmount();
    }
  });

  it("keeps an awaiting-label capture in the queue and shows its fresh suggestion", async () => {
    const wrapper = mount(PopupClassify);
    await flushPromises();

    eventHandlers.get("capture:item-updated")?.({
      payload: {
        ...item,
        suggestedCharacterId: "char-1",
        recognitionConfidence: 0.9,
        recognitionSource: "face_bank",
        reviewStatus: "pending",
      },
    });
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).toContain("推荐：Mira");
    expect(wrapper.text()).toContain("这张截图属于哪一类？");
    wrapper.unmount();
  });

  it("shows a small avatar for characters that have a capture", async () => {
    api.classifyPopupContext.mockResolvedValue({
      items: [item],
      projectId: "project-1",
      projectName: null,
      characters: [{ ...character, avatarCaptureItemId: "avatar-item" }],
    });
    api.suggestForCapture.mockResolvedValue({
      ...item,
      suggestedCharacterId: "char-1",
      recognitionConfidence: 0.81,
      reviewStatus: "pending",
    });
    const wrapper = mount(PopupClassify);
    await flushPromises();

    await wrapper.findAll("button").find((button) => button.text().includes("人物"))!.trigger("click");
    await flushPromises();

    expect(api.readThumbnail).toHaveBeenCalledWith("avatar-item", "avatar");
    expect(wrapper.find(".popup-character .popup-avatar img").exists()).toBe(true);
    wrapper.unmount();
  });

  it("removes a capture from the queue once it is classified elsewhere", async () => {
    const wrapper = mount(PopupClassify);
    await flushPromises();

    eventHandlers.get("capture:item-updated")?.({
      payload: {
        ...item,
        classification: "person",
        characterId: "char-1",
        status: "queued",
      },
    });
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).not.toContain("这张截图属于哪一类？");
    expect(wrapper.text()).toContain("待分类截图");
    wrapper.unmount();
  });

  it("removes a purged capture from the queue immediately", async () => {
    const wrapper = mount(PopupClassify);
    await flushPromises();
    expect(wrapper.text()).toContain("这张截图属于哪一类？");

    eventHandlers.get("capture:item-purged")?.({ payload: { captureItemId: item.id } });
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).not.toContain("这张截图属于哪一类？");
    expect(wrapper.text()).toContain("待分类截图");
    wrapper.unmount();
  });

  it("re-runs the face-bank suggestion when person is chosen", async () => {
    api.suggestForCapture.mockResolvedValue({
      ...item,
      suggestedCharacterId: "char-1",
      recognitionConfidence: 0.81,
      reviewStatus: "pending",
    });
    const wrapper = mount(PopupClassify);
    await flushPromises();

    const personButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"));
    expect(personButton).toBeDefined();
    await personButton!.trigger("click");
    await flushPromises();

    expect(api.suggestForCapture).toHaveBeenCalledWith("item-1");
    expect(wrapper.text()).toContain("推荐：Mira");
    wrapper.unmount();
  });

  it("shows the engine speaker suggestion and classifies the capture on click", async () => {
    api.listProjectOverviews.mockResolvedValue([
      { projectId: "project-1", engine: "renpy", engineVersions: "7.8.6, 8.4.1" },
    ]);
    api.getCaptureImageFacts.mockResolvedValue({
      source: { width: 3840, height: 2160, format: "png" }, destination: null,
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_capture_engine_metadata") return engineMetadataFixture("Mira");
      return null;
    });
    api.verifyCaptureIdentity.mockResolvedValue({
      score: 0.9,
      bestOtherScore: null,
      bestOtherCharacterId: null,
      hasSamples: true,
      hasFeature: true,
      level: "unverified",
    });
    api.label.mockResolvedValue({
      ...item,
      classification: "person",
      characterId: "char-1",
      status: "queued",
    });
    const wrapper = mount(PopupClassify);
    await flushPromises();
    const chips = wrapper.find(".popup-image-heading .engine-chips");
    expect(chips.exists()).toBe(true);
    expect(wrapper.get(".popup-image-heading .popup-image-label").text()).toBe("待分类截图");
    expect(wrapper.get(".popup-header .renpy-badge").text()).toBe("Ren'Py 7/8");
    expect(wrapper.get(".popup-header .renpy-badge").attributes("title")).toBe("7.8.6、8.4.1");
    expect(chips.text()).toContain("说话人 Mira");
    expect(chips.text()).toContain("引擎 1920×1080");
    expect(chips.text()).toContain("截图 3840×2160");
    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();

    const row = wrapper
      .findAll(".popup-suggestion")
      .find((button) => button.text().includes("引擎建议：Mira"));
    expect(row).toBeTruthy();
    await row!.trigger("click");
    await flushPromises();

    expect(api.verifyCaptureIdentity).toHaveBeenCalledWith({
      captureItemId: "item-1",
      characterId: "char-1",
    });
    expect(api.label).toHaveBeenCalledWith("item-1", "char-1", "person");
    wrapper.unmount();
  });

  it("creates a character from an unknown speaker and classifies it", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_capture_engine_metadata") return engineMetadataFixture("Wuwu");
      return null;
    });
    api.createCharacter.mockResolvedValue({
      id: "char-wuwu",
      projectId: "project-1",
      name: "Wuwu",
      aliasesJson: "[]",
      avatarAssetId: null,
      createdAt: "",
      updatedAt: "",
    });
    api.verifyCaptureIdentity.mockResolvedValue({
      score: 0.9,
      bestOtherScore: null,
      bestOtherCharacterId: null,
      hasSamples: true,
      hasFeature: true,
      level: "unverified",
    });
    api.label.mockResolvedValue({
      ...item,
      classification: "person",
      characterId: "char-wuwu",
      status: "queued",
    });
    const wrapper = mount(PopupClassify);
    await flushPromises();
    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();

    const createRow = wrapper
      .findAll(".popup-suggestion")
      .find((button) => button.text().includes("创建角色「Wuwu」"));
    expect(createRow).toBeTruthy();
    await createRow!.trigger("click");
    await flushPromises();

    expect(api.createCharacter).toHaveBeenCalledWith("project-1", "Wuwu");
    expect(api.label).toHaveBeenCalledWith("item-1", "char-wuwu", "person");
    wrapper.unmount();
  });

  it("refreshes the engine suggestion when late metadata arrives", async () => {
    const wrapper = mount(PopupClassify);
    await flushPromises();
    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();
    expect(
      wrapper.findAll(".popup-suggestion").some((button) => button.text().includes("引擎建议")),
    ).toBe(false);

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_capture_engine_metadata") return engineMetadataFixture("Mira");
      return null;
    });
    eventHandlers.get("capture:engine-metadata-updated")?.({
      payload: { captureItemId: "item-1" },
    });
    await flushPromises();

    const row = wrapper
      .findAll(".popup-suggestion")
      .find((button) => button.text().includes("引擎建议：Mira"));
    expect(row).toBeTruthy();
    wrapper.unmount();
  });

  it("marks a double match between speaker and face suggestion", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_capture_engine_metadata") return engineMetadataFixture("Mira");
      return null;
    });
    api.suggestForCapture.mockResolvedValue({
      ...item,
      suggestedCharacterId: "char-1",
      recognitionConfidence: 0.84,
      reviewStatus: "pending",
    });
    const wrapper = mount(PopupClassify);
    await flushPromises();
    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();

    const row = wrapper
      .findAll(".popup-suggestion")
      .find((button) => button.text().includes("推荐：Mira"));
    expect(row).toBeTruthy();
    expect(row!.text()).toContain("相似度 84% · 说话人匹配");
    wrapper.unmount();
  });

  it("enlarges the picture while framing and restores the roster afterwards", async () => {
    api.classifyPopupContext.mockResolvedValue({
      items: [{ ...item, faceBoxJson: '{"x":300,"y":150,"width":300,"height":300}' }],
      projectId: "project-1",
      projectName: null,
      characters: [character],
    });
    // Choosing 人物 asks the engine again; the backend answers with the stored
    // face box, so the fixture has to keep it.
    api.suggestForCapture.mockResolvedValue({
      ...item,
      faceBoxJson: '{"x":300,"y":150,"width":300,"height":300}',
    });
    const wrapper = mount(PopupClassify);
    await flushPromises();

    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();

    const image = wrapper.get(".face-roi-selector img").element;
    Object.defineProperty(image, "naturalWidth", { value: 1000 });
    Object.defineProperty(image, "naturalHeight", { value: 1000 });
    const frameButton = wrapper.findAll("button").find((button) => button.text().includes("框选主脸"));
    expect(frameButton).toBeDefined();
    await frameButton!.trigger("click");
    // Same selector and helper as the capture page: the default box must start
    // on the detected face here too.
    expect(wrapper.get(".roi-box").attributes("style")).toContain("left: 30%");
    expect(wrapper.get(".roi-box").attributes("style")).toContain("width: 30%");
    expect(wrapper.get(".popup-page").classes()).toContain("framing");
    expect(wrapper.get(".popup-framing-note").text()).toContain("正在框选主脸");

    await wrapper.findAll("button").find((button) => button.text() === "取消")!.trigger("click");
    expect(wrapper.get(".popup-page").classes()).not.toContain("framing");
    expect(wrapper.find(".popup-framing-note").exists()).toBe(false);
    wrapper.unmount();
  });
});

it("keeps a refreshed queue when an older empty load completes and does not auto-close", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const wrapper = mount(PopupClassify);
  try {
    await flushPromises();
    let finishOld!: (context: { items: typeof item[]; projectId: string; projectName: string; characters: typeof character[] }) => void;
    api.classifyPopupContext.mockReturnValueOnce(new Promise(resolve => { finishOld = resolve; }));
    api.getAppSettings.mockResolvedValueOnce({ autoCloseEmptyPopup: true });
    eventHandlers.get("classify:refresh")!({ payload: {} });
    await flushPromises();
    api.classifyPopupContext.mockResolvedValueOnce({
      items: [{ ...item, id: "new-capture" }], projectId: "project-1",
      projectName: "Fresh project", characters: [character],
    });
    eventHandlers.get("capture:item-created")!({ payload: {} });
    await flushPromises();
    finishOld({ items: [], projectId: "project-1", projectName: "Stale project", characters: [character] });
    await flushPromises();
    expect(wrapper.text()).toContain("Fresh project");
    expect(wrapper.text()).not.toContain("Stale project");
    expect(api.readImage).toHaveBeenCalledWith("new-capture", "source");
    await vi.advanceTimersByTimeAsync(3001);
    expect(hideMock).not.toHaveBeenCalled();
  } finally {
    wrapper.unmount();
    vi.useRealTimers();
  }
});

it("applies the newest auto-close setting even after its queue load was superseded", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const empty = { items: [], projectId: "project-1", projectName: "Game", characters: [character] };
  api.classifyPopupContext.mockResolvedValue(empty);
  api.getAppSettings.mockResolvedValue({ autoCloseEmptyPopup: true });
  const wrapper = mount(PopupClassify);
  try {
    await flushPromises();
    let finishSettings!: (settings: { autoCloseEmptyPopup: boolean }) => void;
    api.getAppSettings.mockReturnValueOnce(new Promise(resolve => { finishSettings = resolve; }));
    eventHandlers.get("classify:refresh")!({ payload: {} });
    await flushPromises();
    eventHandlers.get("capture:item-created")!({ payload: {} });
    await flushPromises();
    finishSettings({ autoCloseEmptyPopup: false });
    await flushPromises();
    await vi.advanceTimersByTimeAsync(3001);
    expect(hideMock).not.toHaveBeenCalled();
  } finally {
    wrapper.unmount();
    vi.useRealTimers();
  }
});
