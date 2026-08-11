import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const { eventHandlers, api } = vi.hoisted(() => ({
  eventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
  api: {
    listProjects: vi.fn(),
    listCharacters: vi.fn(),
    listSessions: vi.fn(),
    listItems: vi.fn(),
    listProjectRecentCaptures: vi.fn(),
    listUnimportedCaptures: vi.fn(),
    importDirectoryCaptures: vi.fn(),
    deferredImportRecognitionCount: vi.fn(),
    startImportedRecognition: vi.fn(),
    listSourceDirectories: vi.fn(),
    addSourceDirectory: vi.fn(),
    removeSourceDirectory: vi.fn(),
    setSourceDirectoryEnabled: vi.fn(),
    setProjectDestination: vi.fn(),
    runtimeStatus: vi.fn(),
    readImage: vi.fn(),
    readThumbnail: vi.fn(),
    label: vi.fn(),
    verifyCaptureIdentity: vi.fn(),
    suggestForCapture: vi.fn(),
    retry: vi.fn(),
    setProjectCover: vi.fn(),
    revealPath: vi.fn(),
    createProject: vi.fn(),
    createCharacter: vi.fn(),
    startSession: vi.fn(),
    endSession: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    eventHandlers.set(name, handler);
    return () => eventHandlers.delete(name);
  }),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  captureStatusLabel: (status: string) => ({
    awaiting_label: "待分类",
    queued: "排队中",
    processing: "处理中",
    archive_pending: "等待归档",
    completed: "已完成",
    failed: "处理失败",
  })[status] ?? status,
  captureClassificationLabel: (classification: string) => ({
    unclassified: "未分类",
    person: "人物",
    scene: "游戏截图",
    private: "收藏",
  })[classification] ?? classification,
  pathFileName: (path: string) => path.split(/[\\/]/).pop() || path,
  pathMimeType: () => "image/png",
  pickDirectory: vi.fn(async () => null),
  revealPath: api.revealPath,
}));

import Capture from "./Capture.vue";
import { toast } from "@/lib/toast";

const project = {
  id: "project-1",
  name: "Love & Jealousy",
  description: null,
  coverAssetId: null,
  lastSourceDirectory: null,
  lastDestinationDirectory: null,
  destinationDirectory: "D:\\Archive",
  createdAt: "2026-08-01T00:00:00Z",
  updatedAt: "2026-08-01T00:00:00Z",
};
const session = {
  id: "session-1",
  projectId: project.id,
  status: "active",
  startedAt: "2026-08-01T00:00:00Z",
  endedAt: null,
  createdAt: "2026-08-01T00:00:00Z",
  updatedAt: "2026-08-01T00:00:00Z",
};
const characters = [
  { id: "char-1", projectId: project.id, name: "Mira", aliasesJson: "[]", avatarAssetId: null, createdAt: "", updatedAt: "" },
  { id: "char-2", projectId: project.id, name: "Ethan", aliasesJson: "[]", avatarAssetId: null, createdAt: "", updatedAt: "" },
];

function item(status = "awaiting_label") {
  return {
    id: "item-1",
    sessionId: session.id,
    assetId: null,
    characterId: null,
    classification: "unclassified",
    sourcePath: "D:\\Game\\Screenshots\\001.png",
    annotatedPath: null,
    avatarPath: null,
    destinationPath: null,
    destinationAvatarPath: null,
    status,
    faceBoxJson: null,
    errorMessage: status === "failed" ? "NAS offline" : null,
    failureStage: status === "failed" ? "archive" : null,
    attemptCount: status === "failed" ? 3 : 0,
    nextRetryAt: null,
    processingWarningsJson: "[]",
    capturedAt: "2026-08-01T00:00:01Z",
    processedAt: null,
    archivedAt: null,
    createdAt: "2026-08-01T00:00:01Z",
    updatedAt: "2026-08-01T00:00:01Z",
  };
}

beforeAll(() => {
  Object.defineProperty(URL, "createObjectURL", { value: vi.fn(() => "blob:preview") });
  Object.defineProperty(URL, "revokeObjectURL", { value: vi.fn() });
});

beforeEach(() => {
  localStorage.clear();
  eventHandlers.clear();
  api.listProjects.mockResolvedValue([project]);
  api.listCharacters.mockResolvedValue(characters);
  api.listSessions.mockResolvedValue([session]);
  api.listSourceDirectories.mockResolvedValue([]);
  api.listProjectRecentCaptures.mockResolvedValue([item()]);
  api.listUnimportedCaptures.mockResolvedValue([]);
  api.importDirectoryCaptures.mockResolvedValue(0);
  api.deferredImportRecognitionCount.mockResolvedValue(0);
  api.startImportedRecognition.mockResolvedValue(0);
  api.runtimeStatus.mockResolvedValue({
    engineStatus: "configured",
    workerStatus: "idle",
    activeCaptureItemId: null,
    queuedCount: 0,
    archivePendingCount: 0,
    lastError: null,
  });
  api.readImage.mockResolvedValue(new ArrayBuffer(4));
  api.readThumbnail.mockResolvedValue(new ArrayBuffer(4));
  api.verifyCaptureIdentity.mockResolvedValue({
    score: 0.9,
    bestOtherScore: null,
    bestOtherCharacterId: null,
    hasSamples: true,
    hasFeature: true,
    level: "ok",
  });
  api.suggestForCapture.mockResolvedValue(item());
  api.label.mockImplementation(async (_itemId: string, characterId: string | null, classification = "person") => ({
    ...item("queued"),
    characterId,
    classification,
  }));
  api.retry.mockResolvedValue(item("archive_pending"));
});

function setWideLayout(matches: boolean) {
  window.matchMedia = vi.fn(
    (query: string): MediaQueryList =>
      ({
        matches,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      }) as MediaQueryList,
  ) as unknown as typeof window.matchMedia;
}

describe("Capture adaptive label panel trigger", () => {
  afterEach(() => {
    delete (window as { matchMedia?: typeof window.matchMedia }).matchMedia;
  });

  it("uses the static classification panel without a redundant trigger on wide layouts", async () => {
    setWideLayout(true);
    const wrapper = mount(Capture);
    try {
      await flushPromises();
      expect(wrapper.find(".responsive-detail-panel.is-static").exists()).toBe(true);
      expect(wrapper.find(".label-panel-trigger").exists()).toBe(false);
      expect(wrapper.get(".stage-progress").text()).toContain("图片处理状态");
      expect(wrapper.get(".stage-progress").text()).toContain("空闲");
      expect(wrapper.findAll(".project-controls .icon-button")).toHaveLength(2);
    } finally {
      wrapper.unmount();
    }
  });

  it("shows a working classification trigger and drawer on narrow layouts", async () => {
    setWideLayout(false);
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      const trigger = wrapper.get(".label-panel-trigger");
      expect(trigger.attributes("aria-expanded")).toBe("false");
      await trigger.trigger("click");
      await flushPromises();
      expect(document.body.querySelector(".responsive-detail-panel.is-drawer")).not.toBeNull();
      expect(trigger.attributes("aria-expanded")).toBe("true");
    } finally {
      wrapper.unmount();
    }
  });
});

describe("Capture quick-label flow", () => {
  it("renders the three-stage session rail and collapses its conditional tools", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([]);
    api.listSourceDirectories.mockResolvedValue([
      { id: "dir-1", projectId: project.id, directory: "D:\\Game", enabled: true, createdAt: "" },
    ]);
    const wrapper = mount(Capture);
    await flushPromises();

    expect(wrapper.findAll(".session-stage-heading h2").map((heading) => heading.text())).toEqual([
      "选择项目",
      "截图来源",
      "归档位置",
    ]);
    expect(wrapper.get(".session-tools-actions").isVisible()).toBe(true);

    await wrapper.get(".session-tools-toggle").trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.get(".session-tools-toggle").attributes("aria-expanded")).toBe("false");
    expect(wrapper.get(".session-tools-actions").attributes("style")).toContain("display: none");
    expect(wrapper.text()).toContain("展开会话工具");
    expect(wrapper.text()).toContain("继续使用你习惯的截图方式，新截图会自动出现");
    wrapper.unmount();
  });

  it("opens the project creation form when requested from the home page", async () => {
    localStorage.setItem("scene-vault.capture.create-project", "1");

    const wrapper = mount(Capture);
    await flushPromises();

    expect(wrapper.find('[aria-label="项目名称"]').exists()).toBe(true);
    expect(localStorage.getItem("scene-vault.capture.create-project")).toBeNull();
    wrapper.unmount();
  });

  it("classifies as person, selects a recent character with 1, and confirms with Enter", async () => {
    const wrapper = mount(Capture);
    await flushPromises();

    const personButton = wrapper.findAll("button").find((button) => button.text().includes("人物"));
    expect(personButton).toBeTruthy();
    await personButton!.trigger("click");
    await wrapper.vm.$nextTick();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "1" }));
    await wrapper.vm.$nextTick();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flushPromises();

    expect(api.label).toHaveBeenCalledWith("item-1", "char-1", "person");
    expect(wrapper.text()).toContain("排队中");
    wrapper.unmount();
  });

  it("classifies a scene shot with one click and queues it without a character", async () => {
    const wrapper = mount(Capture);
    await flushPromises();

    const sceneButton = wrapper.findAll("button").find((button) => button.text().includes("游戏截图"));
    await sceneButton!.trigger("click");
    await flushPromises();

    expect(api.label).toHaveBeenCalledWith("item-1", null, "scene");
    expect(wrapper.text()).toContain("排队中");
    wrapper.unmount();
  });

  it("classifies a private shot with one click", async () => {
    const wrapper = mount(Capture);
    await flushPromises();

    const privateButton = wrapper.findAll("button").find((button) => button.text().includes("收藏"));
    await privateButton!.trigger("click");
    await flushPromises();

    expect(api.label).toHaveBeenCalledWith("item-1", null, "private");
    wrapper.unmount();
  });

  it("receives live items and exposes a retry action for archive failures", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([]);
    const wrapper = mount(Capture);
    await flushPromises();

    eventHandlers.get("capture:item-created")?.({ payload: item("failed") });
    await wrapper.vm.$nextTick();
    expect(wrapper.text()).toContain("归档失败");

    const retry = wrapper.findAll("button").find((button) => button.text().includes("重试"));
    expect(retry).toBeTruthy();
    await retry!.trigger("click");
    await flushPromises();
    expect(api.retry).toHaveBeenCalledWith("item-1");
    wrapper.unmount();
  });

  it("defers imported recognition and does not load the imported thumbnails", async () => {
    const candidate = {
      path: "D:\\Game\\Screenshots\\old.png",
      fileSize: 1024,
      modifiedAtMs: 1,
    };
    let imported = false;
    api.listUnimportedCaptures.mockResolvedValue([candidate]);
    api.importDirectoryCaptures.mockImplementation(async () => {
      imported = true;
      return 43;
    });
    api.deferredImportRecognitionCount.mockImplementation(async () => imported ? 43 : 0);
    api.startImportedRecognition.mockResolvedValue(43);
    api.listProjectRecentCaptures.mockResolvedValue([]);
    const wrapper = mount(Capture);
    await flushPromises();
    const recentCallsBeforeImport = api.listProjectRecentCaptures.mock.calls.length;

    const openImport = wrapper.findAll("button").find((button) => button.text().includes("导入截图"));
    expect(openImport).toBeTruthy();
    await openImport!.trigger("click");
    await flushPromises();
    const importDialog = document.body.querySelector('[role="dialog"][aria-label="导入已有截图"]');
    const confirmImport = Array.from(importDialog!.querySelectorAll("button")).find((button) => button.textContent?.includes("全部导入"));
    expect(confirmImport).toBeTruthy();
    confirmImport!.click();
    await flushPromises();

    expect(api.importDirectoryCaptures).toHaveBeenCalledWith(session.id, [candidate.path]);
    expect(api.listProjectRecentCaptures).toHaveBeenCalledTimes(recentCallsBeforeImport + 1);
    expect(api.listItems).not.toHaveBeenCalled();
    expect(api.readThumbnail).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("开始识别导入截图（43）");

    const startRecognition = wrapper
      .findAll("button")
      .find((button) => button.text().includes("开始识别导入截图"));
    await startRecognition!.trigger("click");
    await flushPromises();
    expect(api.startImportedRecognition).toHaveBeenCalledWith(session.id);
    expect(wrapper.text()).not.toContain("开始识别导入截图（43）");
    wrapper.unmount();
  });

  it("keeps the list fresh after starting imported recognition", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([item()]);
    api.deferredImportRecognitionCount.mockResolvedValue(3);
    api.startImportedRecognition.mockResolvedValue(3);
    const wrapper = mount(Capture);
    await flushPromises();
    const callsBeforeStart = api.listProjectRecentCaptures.mock.calls.length;

    const startRecognition = wrapper
      .findAll("button")
      .find((button) => button.text().includes("开始识别导入截图"));
    expect(startRecognition).toBeTruthy();
    await startRecognition!.trigger("click");
    await flushPromises();

    expect(api.startImportedRecognition).toHaveBeenCalledWith(session.id);
    expect(api.listProjectRecentCaptures).toHaveBeenCalledTimes(callsBeforeStart + 1);
    expect(wrapper.text()).toContain("001.png");
    wrapper.unmount();
  });

  it("disables imported-recognition start and explains why the AI engine is unconfigured", async () => {
    api.runtimeStatus.mockResolvedValue({
      engineStatus: "unconfigured",
      workerStatus: "idle",
      activeCaptureItemId: null,
      queuedCount: 0,
      archivePendingCount: 0,
      lastError: null,
    });
    api.deferredImportRecognitionCount.mockResolvedValue(3);
    const wrapper = mount(Capture);
    await flushPromises();

    expect(wrapper.text()).toContain("AI 未配置");
    const startRecognition = wrapper
      .findAll("button")
      .find((button) => button.text().includes("开始识别导入截图"));
    expect(startRecognition?.attributes("disabled")).toBeDefined();
    expect(startRecognition?.attributes("title")).toContain("AI 未配置");
    expect(api.startImportedRecognition).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("shows every pending capture in the strip without a small cap", async () => {
    const many = Array.from({ length: 30 }, (_, index) => ({
      ...item(),
      id: `item-${index}`,
      capturedAt: `2026-08-01T00:00:${String(index).padStart(2, "0")}Z`,
      createdAt: `2026-08-01T00:00:${String(index).padStart(2, "0")}Z`,
      updatedAt: `2026-08-01T00:00:${String(index).padStart(2, "0")}Z`,
    }));
    api.listProjectRecentCaptures.mockResolvedValue(many);

    const wrapper = mount(Capture);
    await flushPromises();

    expect(api.listProjectRecentCaptures).toHaveBeenCalledWith(project.id, 100);
    expect(wrapper.findAll(".capture-card").length).toBe(30);
    expect(wrapper.text()).toContain("30 张等待标记");
    wrapper.unmount();
  });

  it("refreshes the face-bank suggestion when person is chosen", async () => {
    const base = { ...item(), faceCount: 1, suggestedCharacterId: null, reviewStatus: "none" };
    api.listProjectRecentCaptures.mockResolvedValue([base]);
    api.suggestForCapture.mockResolvedValue({
      ...base,
      suggestedCharacterId: "char-1",
      recognitionConfidence: 0.81,
      reviewStatus: "pending",
    });

    const wrapper = mount(Capture);
    await flushPromises();
    const personButton = wrapper.findAll("button").find((button) => button.text().includes("人物"));
    await personButton!.trigger("click");
    await flushPromises();

    expect(api.suggestForCapture).toHaveBeenCalledWith("item-1");
    expect(wrapper.text()).toContain("推荐：Mira");
    wrapper.unmount();
  });

  it("re-runs the suggestion even when the local snapshot has no face count", async () => {
    const base = { ...item(), faceCount: 0, suggestedCharacterId: null, reviewStatus: "none" };
    api.listProjectRecentCaptures.mockResolvedValue([base]);
    api.suggestForCapture.mockResolvedValue({
      ...base,
      faceCount: 1,
      suggestedCharacterId: "char-1",
      recognitionConfidence: 0.9,
      reviewStatus: "pending",
    });
    const wrapper = mount(Capture);
    await flushPromises();

    const personButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"));
    await personButton!.trigger("click");
    await flushPromises();

    expect(api.suggestForCapture).toHaveBeenCalledWith("item-1");
    expect(wrapper.text()).toContain("推荐：Mira");
    wrapper.unmount();
  });

  it("refreshes recent captures when the face bank finishes rebuilding", async () => {
    const wrapper = mount(Capture);
    await flushPromises();
    const callsBefore = api.listProjectRecentCaptures.mock.calls.length;

    eventHandlers.get("capture:face-bank-rebuilt")?.({ payload: {} });
    await flushPromises();

    expect(api.listProjectRecentCaptures.mock.calls.length).toBeGreaterThan(callsBefore);
    wrapper.unmount();
  });

  it("keeps the user's selected capture when another awaiting item is updated", async () => {
    const first = { ...item(), id: "item-1", sourcePath: "D:\\shots\\one.png" };
    const second = {
      ...item(),
      id: "item-2",
      sourcePath: "D:\\shots\\two.png",
      capturedAt: "2026-08-01T00:00:02Z",
    };
    api.listProjectRecentCaptures.mockResolvedValue([first, second]);
    const wrapper = mount(Capture);
    await flushPromises();

    const cards = wrapper.findAll(".capture-card");
    await cards[1].trigger("click");
    expect(wrapper.find(".capture-card.selected").attributes("aria-label")).toContain("two.png");

    eventHandlers.get("capture:item-updated")?.({
      payload: { ...first, faceCount: 1, suggestedCharacterId: "char-1" },
    });
    await flushPromises();

    expect(wrapper.find(".capture-card.selected").attributes("aria-label")).toContain("two.png");
    wrapper.unmount();
  });

  it("starts a session and reports the projects it auto-stopped", async () => {
    api.listSessions.mockResolvedValue([]);
    api.listSourceDirectories.mockResolvedValue([
      { id: "dir-1", projectId: project.id, directory: "D:\\Game", enabled: true, createdAt: "" },
    ]);
    api.startSession.mockResolvedValue({ session, stoppedProjects: ["BAD"] });

    const wrapper = mount(Capture);
    await flushPromises();
    const start = wrapper.findAll("button").find((button) => button.text().includes("开始会话"));
    expect(start).toBeTruthy();
    await start!.trigger("click");
    await flushPromises();

    expect(api.startSession).toHaveBeenCalledWith(project.id);
    expect(wrapper.text()).toContain("停止会话");
    wrapper.unmount();
  });
});

describe("Capture item context menu", () => {
  function menuItems() {
    return Array.from(document.body.querySelectorAll('[role="menuitem"]')) as HTMLElement[];
  }

  it("offers labeling for awaiting captures and opens the label panel", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([item()]);
    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper.find(".capture-card").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')!.textContent).toContain("标记角色/分类");

    menuItems()
      .find((entry) => entry.textContent?.includes("标记角色/分类"))!
      .click();
    await flushPromises();
    expect(api.suggestForCapture).toHaveBeenCalledWith("item-1");
    wrapper.unmount();
  });

  it("offers retry for failed captures", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([item("failed")]);
    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper.find(".capture-card").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')!.textContent).toContain("重新识别");

    menuItems()
      .find((entry) => entry.textContent?.includes("重新识别"))!
      .click();
    await flushPromises();
    expect(api.retry).toHaveBeenCalledWith("item-1");
    wrapper.unmount();
  });

  it("reveals the source image and sets the project cover", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([
      { ...item("archive_pending"), destinationPath: "D:\\Archive\\001.png" },
    ]);
    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper.find(".capture-card").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    const menu = document.body.querySelector('[role="menu"]');
    expect(menu!.textContent).toContain("显示原图");
    expect(menu!.textContent).toContain("显示归档图");
    expect(menu!.textContent).toContain("设为项目封面");

    menuItems()
      .find((entry) => entry.textContent?.includes("显示原图"))!
      .click();
    await flushPromises();
    expect(api.revealPath).toHaveBeenCalledWith("D:\\Game\\Screenshots\\001.png");

    await wrapper.find(".capture-card").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    menuItems()
      .find((entry) => entry.textContent?.includes("设为项目封面"))!
      .click();
    await flushPromises();
    expect(api.setProjectCover).toHaveBeenCalledWith("project-1", "item-1");
    wrapper.unmount();
  });

  it("reports reveal failures instead of leaving an unhandled rejection", async () => {
    api.revealPath.mockRejectedValueOnce(new Error("source is unavailable"));
    const toastError = vi.spyOn(toast, "error");
    const wrapper = mount(Capture);
    try {
      await flushPromises();
      await wrapper.find(".capture-card").trigger("contextmenu", {
        clientX: 100,
        clientY: 60,
      });
      await flushPromises();

      menuItems()
        .find((entry) => entry.textContent?.includes("显示原图"))!
        .click();
      await flushPromises();

      expect(toastError).toHaveBeenCalledWith("source is unavailable");
    } finally {
      toastError.mockRestore();
      wrapper.unmount();
    }
  });

  it("disables mutating menu actions while another operation is running", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([item("failed")]);
    let finishRetry!: (value: ReturnType<typeof item>) => void;
    api.retry.mockImplementationOnce(
      () => new Promise((resolve) => { finishRetry = resolve; }),
    );
    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper.find(".capture-card").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    menuItems()
      .find((entry) => entry.textContent?.includes("重新识别"))!
      .click();
    await flushPromises();

    await wrapper.find(".capture-card").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    const retry = menuItems().find((entry) => entry.textContent?.includes("重新识别"));
    expect(retry?.hasAttribute("disabled")).toBe(true);

    finishRetry(item("archive_pending"));
    await flushPromises();
    wrapper.unmount();
  });
});
