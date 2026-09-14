import { flushPromises, mount } from "@vue/test-utils";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const { api, eventHandlers } = vi.hoisted(() => ({
  api: {
    getCaptureFaceRoi: vi.fn(async () => null),
    setCaptureFaceRoi: vi.fn(),
    previewCaptureReset: vi.fn(),
    discardCaptureReset: vi.fn(async () => {}),
    classifyPopupContext: vi.fn(),
    getAppSettings: vi.fn(),
    runtimeStatus: vi.fn(),
    readImage: vi.fn(),
    suggestForCapture: vi.fn(),
    label: vi.fn(),
    verifyCaptureIdentity: vi.fn(),
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
    hide: vi.fn(async () => {}),
  })),
}));

vi.mock("@/lib/capture-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/capture-api")>()),
  captureApi: api,
  pathMimeType: () => "image/png",
}));

import PopupClassify from "./PopupClassify.vue";

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

beforeAll(() => {
  Object.defineProperty(URL, "createObjectURL", { value: vi.fn(() => "blob:preview") });
  Object.defineProperty(URL, "revokeObjectURL", { value: vi.fn() });
});

beforeEach(() => {
  vi.clearAllMocks();
  eventHandlers.clear();
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
  api.suggestForCapture.mockResolvedValue(item);
});

describe("PopupClassify", () => {
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
    expect(wrapper.text()).toContain("1/1");
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

    expect(wrapper.text()).not.toContain("1/1");
    expect(wrapper.text()).toContain("待分类截图");
    wrapper.unmount();
  });

  it("removes a purged capture from the queue immediately", async () => {
    const wrapper = mount(PopupClassify);
    await flushPromises();
    expect(wrapper.text()).toContain("1/1");

    eventHandlers.get("capture:item-purged")?.({ payload: { captureItemId: item.id } });
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).not.toContain("1/1");
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
