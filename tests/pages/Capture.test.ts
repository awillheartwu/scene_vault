import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { CaptureImageFacts, CapturePolicyView, ProjectEngineIntegration, ProjectEngineIntegrationView, RenpyScreenshotPolicyView } from "@/lib/capture-api";

const { eventHandlers, eventListeners, api } = vi.hoisted(() => ({
  eventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
  eventListeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
  api: {
    getCaptureFaceRoi: vi.fn(async () => null),
    setCaptureFaceRoi: vi.fn(),
    listCaptureResetCandidates: vi.fn(async () => ['a', 'b']),
    previewCaptureReset: vi.fn(),
    discardCaptureReset: vi.fn(async () => {}),
    listProjectPendingCaptures: vi.fn(async () => []),
    listHistory: vi.fn(async () => ({ entries: [], total: 0 })),
    listProjects: vi.fn(),
    listProjectOverviews: vi.fn(),
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
    recordClientEvent: vi.fn(async () => {}),
    readThumbnail: vi.fn(),
    label: vi.fn(),
    verifyCaptureIdentity: vi.fn(),
    suggestForCapture: vi.fn(),
    retry: vi.fn(),
    previewCaptureDeletion: vi.fn(),
    deleteCaptureItem: vi.fn(),
    setProjectCover: vi.fn(),
    revealPath: vi.fn(),
    createProject: vi.fn(),
    createCharacter: vi.fn(),
    startSession: vi.fn(),
    endSession: vi.fn(),
    getCaptureImageFacts: vi.fn(
      async (): Promise<CaptureImageFacts> => ({ source: null, destination: null }),
    ),
  },
}));

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    const listeners = eventListeners.get(name) ?? new Set<(event: { payload: unknown }) => void>();
    eventListeners.set(name, listeners);
    listeners.add(handler);
    eventHandlers.set(name, event => [...listeners].forEach(listener => listener(event)));
    return () => {
      listeners.delete(handler);
      if (!listeners.size && eventListeners.get(name) === listeners) {
        eventListeners.delete(name);
        eventHandlers.delete(name);
      }
    };
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

vi.mock("@/lib/capture-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/capture-api")>()),
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

import Capture from "@/pages/Capture.vue";
import ProjectSelect from "@/components/project/ProjectSelect.vue";
import RenpyScreenshotSettings from "@/components/capture/RenpyScreenshotSettings.vue";
import { toast } from "@/lib/toast";

const project = {
  id: "project-1",
  name: "Love & Jealousy",
  description: null,
  coverAssetId: null,
  coverCaptureItemId: null,
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
    projectId: project.id,
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

function engineMetadataFixture(speaker: string, suggestedCharacterId: string | null) {
  return {
    captureItemId: "item-1",
    projectId: project.id,
    engine: "renpy",
    schemaVersion: 1,
    bridgeVersion: "1.1.2",
    engineVersion: "8.1.3.23091805",
    speakerRaw: speaker,
    speakerSource: "_last_say_who",
    dialogue: "…",
    suggestedCharacterId,
    virtualWidth: 1920,
    virtualHeight: 1080,
    physicalWidth: 2560,
    physicalHeight: 1440,
    payloadJson: "{}",
    createdAt: "2026-08-01T00:00:01Z",
    updatedAt: "2026-08-01T00:00:01Z",
  };
}

function policyFixture(overrides: Partial<CapturePolicyView> = {}): CapturePolicyView {
  return {
    projectSize: "inherit", resolvedSize: "keep", globalSize: "keep",
    projectFormat: "inherit", resolvedFormat: "keep", globalFormat: "keep",
    projectQuality: null, resolvedQuality: 90, globalQuality: 90,
    ...overrides,
  };
}

function screenshotPolicyFixture(overrides: Partial<RenpyScreenshotPolicyView> = {}): RenpyScreenshotPolicyView {
  return {
    projectAutoHideUi: null, globalAutoHideUi: false, resolvedAutoHideUi: false,
    ...overrides,
  };
}

function integrationFixture(overrides: Partial<ProjectEngineIntegration> = {}): ProjectEngineIntegrationView {
  const engineRoot = overrides.engineRoot ?? "D:\\Games\\Demo";
  return {
    integration: {
      id: "install-1", projectId: project.id, engine: "renpy", engineRoot,
      gameDirectory: `${engineRoot}\\game`, pythonMajor: 3, engineVersion: "8.4.1",
      bridgeVersion: "1.1.6", autoHideUi: false, autoHideUiOverride: null, manifestJson: "[]",
      installedAt: "", updatedAt: "", ...overrides,
    },
    bridgeFilePresent: true, compiledFilePresent: true,
    bundledBridgeVersion: "1.1.6", resolvedAutoHideUi: overrides.autoHideUiOverride ?? false, warning: null,
  };
}

beforeAll(() => {
  Object.defineProperty(HTMLImageElement.prototype, "decode", { configurable: true, value: vi.fn(async () => {}) });
  Object.defineProperty(URL, "createObjectURL", { value: vi.fn(() => "blob:preview") });
  Object.defineProperty(URL, "revokeObjectURL", { value: vi.fn() });
});

beforeEach(() => {
  localStorage.clear();
  eventHandlers.clear();
  eventListeners.clear();
  api.listProjects.mockResolvedValue([project]);
  api.listProjectOverviews.mockReset().mockResolvedValue([]);
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
  api.getCaptureImageFacts.mockResolvedValue({ source: null, destination: null });
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
  invokeMock.mockImplementation(async (command: string) => {
    switch (command) {
      case "get_project_engine_integration":
        return [];
      case "get_project_capture_policy":
        return policyFixture();
      case "get_project_renpy_screenshot_policy":
        return screenshotPolicyFixture();
      case "get_capture_engine_metadata":
        return null;
      default:
        return null;
    }
  });
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
      const engineHealth = wrapper.get(".session-health").findAll("span")
        .find((status) => status.text() === "AI 已配置")!;
      expect(engineHealth.classes()).toContain("online");
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

describe("Capture Ren'Py screenshot settings", () => {
  let installations: ProjectEngineIntegrationView[];
  let policies: Record<string, RenpyScreenshotPolicyView>;
  const save = vi.fn();
  const saveProject = vi.fn();
  const readPolicy = vi.fn();
  const readIntegrations = vi.fn();
  const uninstall = vi.fn();

  function applyResolved(projectId: string) {
    const resolved = policies[projectId]?.resolvedAutoHideUi ?? false;
    installations = installations.map((view) => view.integration.projectId !== projectId ? view : {
      ...view, resolvedAutoHideUi: view.integration.autoHideUiOverride ?? resolved,
    });
  }

  beforeEach(() => {
    installations = [integrationFixture()];
    policies = { [project.id]: screenshotPolicyFixture() };
    readPolicy.mockReset().mockImplementation(async (projectId: string) =>
      policies[projectId] ?? screenshotPolicyFixture(),
    );
    readIntegrations.mockReset().mockImplementation(async (projectId: string) =>
      installations.filter((view) => view.integration.projectId === projectId),
    );
    save.mockReset().mockImplementation(async (input: {
      projectId: string; integrationId: string; autoHideUi: boolean | null;
    }) => {
      installations = installations.map((view) => view.integration.id !== input.integrationId ? view : {
        ...view, integration: { ...view.integration, autoHideUiOverride: input.autoHideUi },
      });
      applyResolved(input.projectId);
      return installations.find((view) => view.integration.id === input.integrationId);
    });
    saveProject.mockReset().mockImplementation(async (input: { projectId: string; autoHideUi: boolean | null }) => {
      const current = policies[input.projectId] ?? screenshotPolicyFixture();
      policies[input.projectId] = {
        ...current, projectAutoHideUi: input.autoHideUi,
        resolvedAutoHideUi: input.autoHideUi ?? current.globalAutoHideUi,
      };
      applyResolved(input.projectId);
      return policies[input.projectId];
    });
    uninstall.mockReset().mockImplementation(async (input: { integrationId: string }) => {
      installations = installations.filter((view) => view.integration.id !== input.integrationId);
      return { removedFiles: [], keptFiles: [] };
    });
    vi.spyOn(toast, "success").mockReturnValue(0);
    vi.spyOn(toast, "error").mockReturnValue(0);
    vi.spyOn(toast, "info").mockReturnValue(0);
    invokeMock.mockClear().mockImplementation(async (
      command: string,
      args?: { input?: { projectId?: string; integrationId?: string; autoHideUi?: boolean | null } },
    ) => {
      if (command === "get_project_engine_integration") return readIntegrations(args?.input?.projectId);
      if (command === "get_project_capture_policy") return policyFixture();
      if (command === "get_project_renpy_screenshot_policy") return readPolicy(args?.input?.projectId);
      if (command === "set_installation_renpy_screenshot_policy") return save(args?.input);
      if (command === "set_project_renpy_screenshot_policy") return saveProject(args?.input);
      if (command === "uninstall_project_engine_bridge") return uninstall(args?.input);
      return null;
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  function deferred<T>() {
    let resolve!: (value: T) => void;
    let reject!: (reason: Error) => void;
    const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
    return { promise, resolve, reject };
  }

  function projectSelect() {
    return document.body.querySelector<HTMLSelectElement>('[aria-label="项目截图设置"]')!;
  }

  function gameSelects() {
    return [...document.body.querySelectorAll<HTMLSelectElement>(".renpy-screenshot-list select")];
  }

  function choose(select: HTMLSelectElement, value: string) {
    select.value = value;
    select.dispatchEvent(new Event("change", { bubbles: true }));
  }

  async function openSettings(wrapper: ReturnType<typeof mount>) {
    await wrapper.get(".session-tools-actions").findAll("button")
      .find((button) => button.text() === "截图设置")!.trigger("click");
    await flushPromises();
    return gameSelects();
  }

  async function closeSettings(wrapper: ReturnType<typeof mount>) {
    wrapper.getComponent(RenpyScreenshotSettings).vm.$emit("update:open", false);
    await flushPromises();
  }

  it("saves all three project defaults without an installed game or a bridge reinstall", async () => {
    installations = [];
    policies[project.id] = screenshotPolicyFixture({ globalAutoHideUi: true, resolvedAutoHideUi: true });
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      expect(wrapper.get(".session-tools-actions").text()).toContain("截图设置");
      expect(await openSettings(wrapper)).toHaveLength(0);
      expect(projectSelect().value).toBe("inherit");
      expect(projectSelect().options[0].text).toBe("跟随全局（启用）");
      for (const [choice, expected] of [["on", true], ["off", false], ["inherit", null]] as const) {
        choose(projectSelect(), choice);
        await flushPromises();
        expect(saveProject).toHaveBeenLastCalledWith({ projectId: project.id, autoHideUi: expected });
        expect(projectSelect().value).toBe(choice);
      }
      expect(invokeMock).toHaveBeenCalledWith("get_project_renpy_screenshot_policy", { input: { projectId: project.id } });
      expect(document.body.textContent).toContain("仍可保存项目默认");
      expect(invokeMock.mock.calls.some(([command]) => command === "install_project_engine_bridge")).toBe(false);
    } finally {
      wrapper.unmount();
    }
  });

  it("changes every inheriting game with the project policy and preserves explicit game choices", async () => {
    installations = [
      integrationFixture(),
      integrationFixture({ id: "install-2", engineRoot: "E:\\Games\\Other" }),
      integrationFixture({ id: "install-3", engineRoot: "F:\\Games\\ExplicitOff", autoHideUiOverride: false, autoHideUi: true }),
    ];
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      const inputs = await openSettings(wrapper);
      const reads = readIntegrations.mock.calls.length;
      choose(projectSelect(), "on");
      await flushPromises();
      expect(projectSelect().value).toBe("on");
      expect(inputs.map((select) => select.value)).toEqual(["inherit", "inherit", "off"]);
      expect(inputs.map((select) => select.options[0].text)).toEqual([
        "跟随项目（启用）", "跟随项目（启用）", "跟随项目（启用）",
      ]);
      expect([...document.body.querySelectorAll(".screenshot-game-state")].map((state) => state.textContent)).toEqual([
        "当前有效：启用", "当前有效：启用", "当前有效：关闭",
      ]);
      expect(readIntegrations.mock.calls.length).toBeGreaterThan(reads);
      expect(readPolicy).toHaveBeenCalledTimes(2);
      expect(save).not.toHaveBeenCalled();
      expect(invokeMock.mock.calls.some(([command]) => command === "install_project_engine_bridge")).toBe(false);
    } finally {
      wrapper.unmount();
    }
  });

  it("saves the chosen game with a nullable policy, holds the engine lock through refresh, and never reinstalls", async () => {
    const first = integrationFixture({ autoHideUiOverride: true });
    const second = integrationFixture({ id: "install-2", engineRoot: "E:\\Games\\Other", engineVersion: "7.8.6" });
    installations = [first, second];
    const pending = deferred<ProjectEngineIntegrationView>();
    const refresh = deferred<RenpyScreenshotPolicyView>();
    save.mockReturnValueOnce(pending.promise);
    const saved = {
      ...second, integration: { ...second.integration, autoHideUiOverride: true },
      resolvedAutoHideUi: true, warning: "保留的编译文件需要确认",
    };
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      const inputs = await openSettings(wrapper);
      expect(inputs.map((input) => input.value)).toEqual(["on", "inherit"]);
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
      expect(api.label).not.toHaveBeenCalled();
      choose(inputs[1], "on");
      await flushPromises();
      expect(invokeMock).toHaveBeenCalledWith("set_installation_renpy_screenshot_policy", {
        input: { projectId: project.id, integrationId: second.integration.id, autoHideUi: true },
      });
      expect(inputs.map((input) => input.value)).toEqual(["on", "inherit"]);
      expect(inputs.every((input) => input.disabled)).toBe(true);
      expect(projectSelect().disabled).toBe(true);
      const install = wrapper.findAll("button").find((button) => button.text() === "重新安装")!;
      const remove = wrapper.findAll("button").find((button) => button.text() === "卸载")!;
      expect(install.attributes("disabled")).toBeDefined();
      expect(remove.attributes("disabled")).toBeDefined();
      const dialog = wrapper.getComponent(RenpyScreenshotSettings);
      dialog.vm.$emit("installation-change", first, false);
      dialog.vm.$emit("project-change", true);
      await flushPromises();
      expect(save).toHaveBeenCalledTimes(1);
      expect(saveProject).not.toHaveBeenCalled();
      expect(toast.success).not.toHaveBeenCalled();

      installations = [first, saved];
      readPolicy.mockReturnValueOnce(refresh.promise);
      pending.resolve(saved);
      await flushPromises();
      expect(dialog.props("saving")).toBe(true);
      expect(install.attributes("disabled")).toBeDefined();
      expect(inputs[1].value).toBe("on");
      expect(toast.success).not.toHaveBeenCalled();
      refresh.resolve(policies[project.id]);
      await flushPromises();
      expect(inputs.every((input) => !input.disabled)).toBe(true);
      expect(toast.success).toHaveBeenCalledWith(
        "截图设置已保存；已更新并重启过的插件将在下一次截图时应用。",
      );
      expect(document.body.textContent).toContain(saved.warning);

      choose(inputs[0], "off");
      await flushPromises();
      expect(save).toHaveBeenLastCalledWith({ projectId: project.id, integrationId: first.integration.id, autoHideUi: false });
      expect(inputs[0].value).toBe("off");
      choose(inputs[0], "inherit");
      await flushPromises();
      expect(save).toHaveBeenLastCalledWith({ projectId: project.id, integrationId: first.integration.id, autoHideUi: null });
      expect(inputs[0].value).toBe("inherit");
      expect(invokeMock.mock.calls.some(([command]) => command === "install_project_engine_bridge")).toBe(false);
    } finally {
      pending.resolve(saved);
      refresh.resolve(policies[project.id]);
      wrapper.unmount();
    }
  });

  it.each(["rejected", "unchanged"])("preserves the persisted game choice when a save is %s", async (outcome) => {
    const warning = "截图设置写入失败";
    if (outcome === "rejected") save.mockRejectedValueOnce(new Error(warning));
    else save.mockResolvedValueOnce({ ...installations[0], warning });
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      const [input] = await openSettings(wrapper);
      choose(input, "on");
      await flushPromises();
      expect(input.value).toBe("inherit");
      expect(input.disabled).toBe(false);
      expect(document.body.querySelector('.screenshot-settings-warning[role="alert"]')?.textContent).toContain(warning);
      expect(toast.success).not.toHaveBeenCalled();
      choose(input, "on");
      await flushPromises();
      expect(input.value).toBe("on");
      expect(save).toHaveBeenCalledTimes(2);
    } finally {
      wrapper.unmount();
    }
  });

  it.each(["rejected", "unchanged"])("preserves the persisted project default when a save is %s", async (outcome) => {
    if (outcome === "rejected") saveProject.mockRejectedValueOnce(new Error("项目写入失败"));
    else saveProject.mockResolvedValueOnce(screenshotPolicyFixture());
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      await openSettings(wrapper);
      choose(projectSelect(), "on");
      await flushPromises();
      expect(projectSelect().value).toBe("inherit");
      expect(projectSelect().disabled).toBe(false);
      expect(document.body.querySelector('.screenshot-settings-warning[role="alert"]')).not.toBeNull();
      expect(toast.success).not.toHaveBeenCalled();
      choose(projectSelect(), "on");
      await flushPromises();
      expect(projectSelect().value).toBe("on");
    } finally {
      wrapper.unmount();
    }
  });

  it("blocks editing on a load failure and retries both the policy and integrations", async () => {
    readPolicy.mockRejectedValueOnce(new Error("读取失败"));
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      const inputs = await openSettings(wrapper);
      expect(projectSelect().disabled).toBe(true);
      expect(inputs.every((input) => input.disabled)).toBe(true);
      expect(document.body.textContent).toContain("截图设置加载失败：读取失败");
      wrapper.getComponent(RenpyScreenshotSettings).vm.$emit("project-change", true);
      wrapper.getComponent(RenpyScreenshotSettings).vm.$emit("installation-change", installations[0], true);
      await flushPromises();
      expect(save).not.toHaveBeenCalled();
      expect(saveProject).not.toHaveBeenCalled();

      document.body.querySelector<HTMLButtonElement>(".screenshot-settings-error button")!.click();
      await flushPromises();
      expect(readPolicy).toHaveBeenCalledTimes(2);
      expect(projectSelect().disabled).toBe(false);
      expect(gameSelects()[0].disabled).toBe(false);
      expect(document.body.querySelector(".screenshot-settings-error")).toBeNull();
    } finally {
      wrapper.unmount();
    }
  });

  it("shows a saved-but-unreadable state without resaving when refresh fails", async () => {
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      await openSettings(wrapper);
      readIntegrations.mockRejectedValueOnce(new Error("刷新失败"));
      choose(projectSelect(), "on");
      await flushPromises();
      expect(projectSelect().value).toBe("on");
      expect(projectSelect().disabled).toBe(true);
      expect(document.body.textContent).toContain("设置已保存，但最新状态读取失败");
      expect(toast.success).not.toHaveBeenCalled();
      document.body.querySelector<HTMLButtonElement>(".screenshot-settings-error button")!.click();
      await flushPromises();
      expect(projectSelect().value).toBe("on");
      expect(projectSelect().disabled).toBe(false);
      expect(gameSelects()[0].options[0].text).toBe("跟随项目（启用）");
      expect(saveProject).toHaveBeenCalledTimes(1);
      expect(document.body.textContent).not.toContain("最新状态读取失败");
    } finally {
      wrapper.unmount();
    }
  });

  it("keeps the update and restart requirement visible for stale plugins when saving", async () => {
    installations = [integrationFixture({ bridgeVersion: "1.1.5" })];
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      await openSettings(wrapper);
      expect(document.body.textContent).toContain("插件需要更新（1.1.5 → 1.1.6）");
      choose(projectSelect(), "on");
      await flushPromises();
      expect(toast.success).toHaveBeenCalledWith(
        "截图设置已保存；旧插件需要重新安装并重启游戏一次，更新后才会应用。",
      );
    } finally {
      wrapper.unmount();
    }
  });

  it.each(["success", "failure"])("retains the lock on a project switch and discards the old save's late %s", async (outcome) => {
    const previous = installations[0];
    const nextProject = { ...project, id: "project-2", name: "Other project" };
    installations.push(integrationFixture({ id: "install-2", projectId: nextProject.id, engineRoot: "E:\\Games\\Other" }));
    api.listProjects.mockResolvedValue([project, nextProject]);
    const pending = deferred<ProjectEngineIntegrationView>();
    save.mockReturnValueOnce(pending.promise);
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      choose((await openSettings(wrapper))[0], "on");
      await flushPromises();
      wrapper.getComponent(ProjectSelect).vm.$emit("update:modelValue", nextProject.id);
      await flushPromises();
      expect(wrapper.getComponent(RenpyScreenshotSettings).props("open")).toBe(false);
      expect(wrapper.findAll("button").find((button) => button.text() === "重新安装")!.attributes("disabled")).toBeDefined();
      if (outcome === "success") pending.resolve({
        ...previous, integration: { ...previous.integration, autoHideUiOverride: true },
        resolvedAutoHideUi: true, warning: "旧项目警告",
      });
      else pending.reject(new Error("旧项目保存失败"));
      await flushPromises();
      const inputs = await openSettings(wrapper);
      expect(inputs).toHaveLength(1);
      expect(inputs[0].value).toBe("inherit");
      expect(inputs[0].disabled).toBe(false);
      expect(document.body.querySelector(".screenshot-game-name")?.textContent?.trim()).toBe("Other");
      expect(document.body.textContent).not.toContain("旧项目");
      expect(save).toHaveBeenCalledTimes(1);
      expect(save).toHaveBeenCalledWith({ projectId: project.id, integrationId: previous.integration.id, autoHideUi: true });
      expect(toast.success).not.toHaveBeenCalled();
      expect(toast.error).not.toHaveBeenCalled();
    } finally {
      wrapper.unmount();
    }
  });

  it("does not apply a project's old save after switching away and back", async () => {
    const nextProject = { ...project, id: "project-2", name: "Other project" };
    api.listProjects.mockResolvedValue([project, nextProject]);
    const pending = deferred<RenpyScreenshotPolicyView>();
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      await openSettings(wrapper);
      saveProject.mockReturnValueOnce(pending.promise);
      choose(projectSelect(), "on");
      await flushPromises();
      const select = wrapper.getComponent(ProjectSelect);
      select.vm.$emit("update:modelValue", nextProject.id);
      await flushPromises();
      select.vm.$emit("update:modelValue", project.id);
      await flushPromises();
      pending.resolve(screenshotPolicyFixture({ projectAutoHideUi: true, resolvedAutoHideUi: true }));
      await flushPromises();
      expect((await openSettings(wrapper))[0].value).toBe("inherit");
      expect(projectSelect().value).toBe("inherit");
      expect(toast.success).not.toHaveBeenCalled();
    } finally {
      wrapper.unmount();
    }
  });

  it("discards an old policy load after switching projects", async () => {
    const nextProject = { ...project, id: "project-2", name: "Other project" };
    installations.push(integrationFixture({ id: "install-2", projectId: nextProject.id, engineRoot: "E:\\Games\\Other" }));
    api.listProjects.mockResolvedValue([project, nextProject]);
    const pending = deferred<RenpyScreenshotPolicyView>();
    readPolicy.mockReturnValueOnce(pending.promise);
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      await openSettings(wrapper);
      expect(projectSelect().disabled).toBe(true);
      wrapper.getComponent(ProjectSelect).vm.$emit("update:modelValue", nextProject.id);
      await flushPromises();
      await openSettings(wrapper);
      expect(projectSelect().disabled).toBe(false);
      pending.resolve(screenshotPolicyFixture({ projectAutoHideUi: true, globalAutoHideUi: true, resolvedAutoHideUi: true }));
      await flushPromises();
      expect(projectSelect().value).toBe("inherit");
      expect(projectSelect().options[0].text).toBe("跟随全局（关闭）");
      expect(document.body.querySelector(".screenshot-game-name")?.textContent?.trim()).toBe("Other");
    } finally {
      wrapper.unmount();
    }
  });

  it("does not restore stale installation rows from a read started before saving", async () => {
    const previous = installations[0];
    const pendingRead = deferred<ProjectEngineIntegrationView[]>();
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      await openSettings(wrapper);
      readIntegrations.mockReturnValueOnce(pendingRead.promise);
      await closeSettings(wrapper);
      await openSettings(wrapper);
      await closeSettings(wrapper);
      await openSettings(wrapper);
      choose(projectSelect(), "on");
      await flushPromises();
      pendingRead.resolve([previous]);
      await flushPromises();
      expect(gameSelects()[0].options[0].text).toBe("跟随项目（启用）");
      expect(document.body.querySelector(".screenshot-game-state")?.textContent).toBe("当前有效：启用");
    } finally {
      wrapper.unmount();
    }
  });

  it("does not restore an uninstalled game from an earlier dialog read", async () => {
    const previous = installations[0];
    const pendingRead = deferred<ProjectEngineIntegrationView[]>();
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      readIntegrations.mockReturnValueOnce(pendingRead.promise);
      await openSettings(wrapper);
      await closeSettings(wrapper);
      const remove = wrapper.findAll("button").find((button) => button.text() === "卸载")!;
      await remove.trigger("click");
      await wrapper.findAll("button").find((button) => button.text() === "确认卸载？")!.trigger("click");
      await flushPromises();
      expect(uninstall).toHaveBeenCalledWith({ projectId: project.id, integrationId: previous.integration.id });
      pendingRead.resolve([previous]);
      await flushPromises();
      expect(await openSettings(wrapper)).toHaveLength(0);
      expect(projectSelect().disabled).toBe(false);
    } finally {
      wrapper.unmount();
    }
  });

  it.each(["success", "failure"])("ignores a late %s after unmount without refreshing or showing a Toast", async (outcome) => {
    const previous = installations[0];
    const pending = deferred<ProjectEngineIntegrationView>();
    save.mockReturnValueOnce(pending.promise);
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      choose((await openSettings(wrapper))[0], "on");
      await flushPromises();
      wrapper.unmount();
      const reads = [readPolicy.mock.calls.length, readIntegrations.mock.calls.length];
      if (outcome === "success") pending.resolve({
        ...previous, integration: { ...previous.integration, autoHideUiOverride: true }, resolvedAutoHideUi: true,
      });
      else pending.reject(new Error("卸载后保存失败"));
      await flushPromises();
      expect([readPolicy.mock.calls.length, readIntegrations.mock.calls.length]).toEqual(reads);
      expect(toast.success).not.toHaveBeenCalled();
      expect(toast.error).not.toHaveBeenCalled();
      expect(toast.info).not.toHaveBeenCalled();
    } finally {
      wrapper.unmount();
    }
  });
});

describe("Capture quick-label flow", () => {
  it("refreshes ordinary image facts when the same capture is replaced", async () => {
    api.getCaptureImageFacts.mockResolvedValue({
      source: { width: 3840, height: 2160, format: "png" }, destination: null,
    });
    const wrapper = mount(Capture);
    try {
      await flushPromises();
      const heading = wrapper.get(".stage-heading-actions");
      const file = heading.get(".capture-heading-file");
      const chips = heading.get(".engine-chips");
      expect(file.text()).toContain("001.png");
      expect(chips.text()).toBe("截图 3840×2160");
      expect(file.element.compareDocumentPosition(chips.element) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      api.getCaptureImageFacts.mockResolvedValue({ source: null, destination: null });
      eventHandlers.get("capture:item-updated")?.({
        payload: { ...item(), sourceFileState: "replaced", updatedAt: "2026-08-01T00:00:03Z" },
      });
      await flushPromises();
      expect(wrapper.text()).not.toContain("截图 3840×2160");
      expect(api.getCaptureImageFacts).toHaveBeenLastCalledWith("item-1");
    } finally {
      wrapper.unmount();
    }
  });

  it("serializes archive setting changes and shows the inherited JPEG quality", async () => {
    api.listSessions.mockResolvedValue([]);
    const initial = policyFixture({ globalFormat: "jpeg", resolvedFormat: "jpeg" });
    let finishSave!: (policy: CapturePolicyView) => void;
    const saving = new Promise<CapturePolicyView>((resolve) => { finishSave = resolve; });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_project_engine_integration") return [];
      if (command === "get_project_capture_policy") return initial;
      if (command === "set_project_capture_policy") return saving;
      return null;
    });
    const wrapper = mount(Capture, { attachTo: document.body });
    try {
      await flushPromises();
      expect(wrapper.get(".archive-spec-summary").text()).toBe("保持原样 · JPEG q90");
      await wrapper.get('.archive-stage button[aria-haspopup="dialog"]').trigger("click");
      await flushPromises();
      const dialog = document.body.querySelector(".archive-policy-dialog")!;
      expect(dialog.textContent).toContain("跟随全局（90）");
      const size = dialog.querySelector('[aria-label="归档尺寸"]') as HTMLSelectElement;
      size.value = "native";
      size.dispatchEvent(new Event("change", { bubbles: true }));
      await flushPromises();
      expect(invokeMock).toHaveBeenCalledWith("set_project_capture_policy", {
        input: { projectId: project.id, archiveOutputSize: "native", archiveOutputFormat: "inherit", archiveOutputQuality: null },
      });
      expect(size.disabled).toBe(true);
      expect((dialog.querySelector('[aria-label="归档格式"]') as HTMLSelectElement).disabled).toBe(true);
      expect(dialog.textContent).toContain("正在保存…");
      finishSave({ ...initial, projectSize: "native", resolvedSize: "native" });
      await flushPromises();
      expect(size.disabled).toBe(false);
      expect(wrapper.get(".archive-spec-summary").text()).toBe("按游戏设定尺寸 · JPEG q90");
    } finally {
      finishSave(initial);
      wrapper.unmount();
    }
  });

  it("renders the session rail, opens all source directories and collapses its tools", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([]);
    api.listSourceDirectories.mockResolvedValue([
      { id: "dir-1", projectId: project.id, directory: "D:\\Game", enabled: true, createdAt: "" },
      { id: "dir-2", projectId: project.id, directory: "D:\\Other Game", enabled: false, createdAt: "" },
    ]);
    const wrapper = mount(Capture, { attachTo: document.body });
    await flushPromises();

    expect(wrapper.findAll(".session-stage-heading h2").map((heading) => heading.text())).toEqual([
      "选择项目",
      "截图来源",
      "归档",
    ]);
    expect(wrapper.get(".session-tools-actions").isVisible()).toBe(true);

    await wrapper.get(".session-tools-toggle").trigger("click");
    await wrapper.vm.$nextTick();
    expect(wrapper.get(".session-tools-toggle").attributes("aria-expanded")).toBe("false");
    expect(wrapper.get(".session-tools-actions").attributes("style")).toContain("display: none");
    expect(wrapper.text()).toContain("展开会话工具");
    expect(wrapper.text()).toContain("继续使用你习惯的截图方式，新截图会自动出现");

    const sourceTrigger = wrapper.get('[aria-label="查看截图来源目录"]');
    expect(sourceTrigger.attributes("aria-haspopup")).toBe("dialog");
    expect(sourceTrigger.get(".source-dir-name").text()).toBe("D:\\Game");
    expect(sourceTrigger.get(".source-directory-more").attributes("title")).toBe("共 2 个来源目录");
    expect(wrapper.find(".sources-stage .source-dir-list").exists()).toBe(false);
    await sourceTrigger.trigger("click");
    await flushPromises();
    const dialog = document.body.querySelector(".source-directories-dialog")!;
    expect([...dialog.querySelectorAll(".source-dir-name")].map((name) => name.textContent)).toEqual([
      "D:\\Game", "D:\\Other Game",
    ]);
    expect([...dialog.querySelectorAll(".source-dir-state")].map((state) => state.textContent?.trim())).toEqual([
      "监听中", "已暂停",
    ]);
    wrapper.unmount();
  });

  it("loads project badges without a selected capture and refreshes them when the menu opens", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([]);
    api.listProjectOverviews.mockResolvedValue([
      { projectId: project.id, engine: "renpy", engineVersions: "8.4.1" },
    ]);
    const wrapper = mount(Capture);
    try {
      await flushPromises();
      const select = wrapper.getComponent(ProjectSelect);
      expect(select.get(".project-select-trigger .renpy-badge").text()).toBe("Ren'Py 8");
      expect(wrapper.find(".capture-heading-title .renpy-badge").exists()).toBe(false);
      const overviewCalls = api.listProjectOverviews.mock.calls.length;
      api.listProjectOverviews.mockResolvedValue([
        { projectId: project.id, engine: "renpy", engineVersions: "7.8.6,8.4.1" },
      ]);
      select.vm.$emit("open");
      await flushPromises();
      expect(api.listProjectOverviews).toHaveBeenCalledTimes(overviewCalls + 1);
      expect(select.get(".project-select-trigger .renpy-badge").text()).toBe("Ren'Py 7/8");
      expect(select.get(".project-select-trigger .renpy-badge").attributes("title")).toBe("7.8.6、8.4.1");
    } finally {
      wrapper.unmount();
    }
  });

  it("opens the project creation form when requested from the home page", async () => {
    localStorage.setItem("scene-vault.capture.create-project", "1");

    const wrapper = mount(Capture);
    await flushPromises();

    expect(wrapper.find('[aria-label="项目名称"]').exists()).toBe(true);
    expect(localStorage.getItem("scene-vault.capture.create-project")).toBeNull();
    wrapper.unmount();
  });

  it("shows a small avatar in the character list when the character has a capture", async () => {
    api.listCharacters.mockResolvedValue([
      { ...characters[0], avatarCaptureItemId: "avatar-item" },
      characters[1],
    ]);
    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper.findAll("button").find((button) => button.text().includes("人物"))!.trigger("click");
    await flushPromises();

    expect(api.readThumbnail).toHaveBeenCalledWith("avatar-item", "avatar");
    const row = wrapper.findAll(".character-row").find((entry) => entry.text().includes("Mira"))!;
    expect(row.find(".avatar img").exists()).toBe(true);
    wrapper.unmount();
  });

  it("starts framing on the face the last scan picked", async () => {
    const detected = { ...item(), faceBoxJson: '{"x":300,"y":150,"width":300,"height":300}' };
    api.listProjectRecentCaptures.mockResolvedValue([detected]);
    api.suggestForCapture.mockResolvedValue(detected);
    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper.findAll("button").find((button) => button.text().includes("人物"))!.trigger("click");
    await wrapper.vm.$nextTick();
    const image = wrapper.get(".face-roi-selector img").element;
    Object.defineProperty(image, "naturalWidth", { value: 1000 });
    Object.defineProperty(image, "naturalHeight", { value: 1000 });
    await wrapper.findAll("button").find((button) => button.text() === "框选主脸")!.trigger("click");

    // The capture page and the popup share the selector, so the default box has
    // to come from the same detected face on both sides.
    expect(wrapper.get(".roi-box").attributes("style")).toContain("left: 30%");
    expect(wrapper.get(".roi-box").attributes("style")).toContain("top: 15%");
    expect(wrapper.get(".roi-box").attributes("style")).toContain("width: 30%");
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

  it("preselects the dialogue speaker suggestion when no face suggestion exists", async () => {
    api.listProjectOverviews.mockResolvedValue([
      { projectId: project.id, engine: "renpy", engineVersions: "8.4.1" },
    ]);
    api.getCaptureImageFacts.mockResolvedValue({
      source: { width: 3840, height: 2160, format: "png" },
      destination: null,
    });
    invokeMock.mockImplementation(async (command: string) => {
      switch (command) {
        case "get_project_engine_integration":
          return [];
        case "get_project_capture_policy":
          return policyFixture();
        case "get_capture_engine_metadata":
          return engineMetadataFixture("Ethan", "char-2");
        default:
          return null;
      }
    });
    const wrapper = mount(Capture);
    await flushPromises();

    const chips = wrapper.find(".stage-heading-actions .engine-chips");
    expect(chips.exists()).toBe(true);
    expect(wrapper.get(".project-select-trigger .renpy-badge").text()).toBe("Ren'Py 8");
    expect(wrapper.get(".project-select-trigger .renpy-badge").attributes("title")).toBe("8.4.1");
    expect(chips.text()).toContain("说话人 Ethan");
    expect(chips.text()).toContain("引擎 1920×1080");
    expect(chips.text()).toContain("截图 3840×2160");

    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();

    const engineRow = wrapper
      .findAll(".suggestion-row")
      .find((row) => row.text().includes("引擎建议：Ethan"));
    expect(engineRow).toBeTruthy();
    expect(engineRow!.classes()).toContain("selected");

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flushPromises();
    expect(api.label).toHaveBeenCalledWith("item-1", "char-2", "person");
    wrapper.unmount();
  });

  it("lets a late face suggestion take over the auto-picked dialogue suggestion", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      switch (command) {
        case "get_project_engine_integration":
          return [];
        case "get_project_capture_policy":
          return policyFixture();
        case "get_capture_engine_metadata":
          return engineMetadataFixture("Ethan", "char-2");
        default:
          return null;
      }
    });
    let releaseVision: () => void = () => {};
    api.suggestForCapture.mockImplementation(
      () =>
        new Promise((resolve) => {
          releaseVision = () =>
            resolve({
              ...item(),
              suggestedCharacterId: "char-1",
              recognitionConfidence: 0.91,
              recognitionSource: "face_bank",
              reviewStatus: "pending",
            });
        }),
    );

    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();

    const engineRow = wrapper
      .findAll(".suggestion-row")
      .find((row) => row.text().includes("引擎建议：Ethan"));
    expect(engineRow).toBeTruthy();
    expect(engineRow!.classes()).toContain("selected");

    releaseVision();
    await flushPromises();

    const visionRow = wrapper
      .findAll(".suggestion-row")
      .find((row) => row.text().includes("推荐：Mira"));
    expect(visionRow).toBeTruthy();
    expect(visionRow!.classes()).toContain("selected");
    expect(engineRow!.classes()).not.toContain("selected");
    wrapper.unmount();
  });

  it("offers to create a character from an unknown speaker and selects it", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      switch (command) {
        case "get_project_engine_integration":
          return [];
        case "get_project_capture_policy":
          return policyFixture();
        case "get_capture_engine_metadata":
          return engineMetadataFixture("Wuwu", null);
        default:
          return null;
      }
    });
    api.createCharacter.mockResolvedValue({
      id: "char-wuwu",
      projectId: project.id,
      name: "Wuwu",
      aliasesJson: "[]",
      avatarAssetId: null,
      createdAt: "",
      updatedAt: "",
    });
    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();

    const createRow = wrapper
      .findAll(".suggestion-row")
      .find((row) => row.text().includes("创建角色「Wuwu」"));
    expect(createRow).toBeTruthy();
    await createRow!.trigger("click");
    await flushPromises();

    expect(api.createCharacter).toHaveBeenCalledWith(project.id, "Wuwu");
    expect(
      wrapper.findAll(".suggestion-row").some((row) => row.text().includes("创建角色「Wuwu」")),
    ).toBe(false);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flushPromises();
    expect(api.label).toHaveBeenCalledWith("item-1", "char-wuwu", "person");
    wrapper.unmount();
  });

  it("marks and preselects a double match between speaker and face suggestion", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      switch (command) {
        case "get_project_engine_integration":
          return [];
        case "get_project_capture_policy":
          return policyFixture();
        case "get_capture_engine_metadata":
          return engineMetadataFixture("Ethan", "char-2");
        default:
          return null;
      }
    });
    api.suggestForCapture.mockResolvedValue({
      ...item(),
      suggestedCharacterId: "char-2",
      recognitionConfidence: 0.84,
      recognitionSource: "face_bank",
      reviewStatus: "pending",
    });
    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();

    const visionRow = wrapper
      .findAll(".suggestion-row")
      .find((row) => row.text().includes("推荐：Ethan"));
    expect(visionRow).toBeTruthy();
    expect(visionRow!.text()).toContain("相似度 84% · 说话人匹配");
    expect(visionRow!.classes()).toContain("selected");
    expect(
      wrapper.findAll(".suggestion-row").some((row) => row.text().includes("引擎建议")),
    ).toBe(false);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flushPromises();
    expect(api.label).toHaveBeenCalledWith("item-1", "char-2", "person");
    wrapper.unmount();
  });

  it("does not carry the previous capture's automatic pick onto the next image", async () => {
    const one = { ...item(), id: "item-1", sourcePath: "D:\\shots\\one.png" };
    const two = {
      ...item(),
      id: "item-2",
      sourcePath: "D:\\shots\\two.png",
      capturedAt: "2026-08-01T00:00:02Z",
    };
    api.listProjectRecentCaptures.mockResolvedValue([one, two]);
    invokeMock.mockImplementation(
      async (command: string, args?: { input?: { captureItemId?: string } }) => {
        switch (command) {
          case "get_project_engine_integration":
            return [];
          case "get_project_capture_policy":
            return policyFixture();
          case "get_capture_engine_metadata":
            return args?.input?.captureItemId === "item-2"
              ? engineMetadataFixture("Ethan", "char-2")
              : null;
          default:
            return null;
        }
      },
    );
    api.suggestForCapture.mockImplementation(async (captureItemId: string) =>
      captureItemId === "item-2"
        ? {
            ...two,
            suggestedCharacterId: "char-2",
            recognitionConfidence: 0.84,
            reviewStatus: "pending",
          }
        : { ...one, reviewStatus: "none" },
    );
    const wrapper = mount(Capture);
    await flushPromises();

    // Work on item-2 (two.png) explicitly, whatever the initial pick was.
    const findCard = (name: string) =>
      wrapper
        .findAll(".capture-card")
        .find((card) => (card.attributes("aria-label") ?? "").includes(name));
    await findCard("two.png")!.trigger("click");
    await flushPromises();

    // Its dialogue and face suggestions agree, so Ethan is auto-picked.
    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();
    const visionRow = wrapper
      .findAll(".suggestion-row")
      .find((row) => row.text().includes("推荐：Ethan"));
    expect(visionRow!.classes()).toContain("selected");

    // Switching to item-1 (which has no suggestions at all) must drop that
    // automatic pick instead of submitting it for the new image.
    await findCard("one.png")!.trigger("click");
    await flushPromises();
    if (!wrapper.find("#label-title").text().includes("选择主角色")) {
      await wrapper
        .findAll("button")
        .find((button) => button.text().includes("人物"))!
        .trigger("click");
      await flushPromises();
    }
    expect(wrapper.find("#label-title").text()).toContain("选择主角色");
    expect(wrapper.findAll(".character-row").length).toBeGreaterThan(0);
    expect(wrapper.find(".character-row.selected").exists()).toBe(false);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flushPromises();
    expect(api.label).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("keeps a manual re-pick of the automatic character when switching images", async () => {
    const one = { ...item(), id: "item-1", sourcePath: "D:\\shots\\one.png" };
    const two = {
      ...item(),
      id: "item-2",
      sourcePath: "D:\\shots\\two.png",
      capturedAt: "2026-08-01T00:00:02Z",
    };
    api.listProjectRecentCaptures.mockResolvedValue([one, two]);
    invokeMock.mockImplementation(
      async (command: string, args?: { input?: { captureItemId?: string } }) => {
        switch (command) {
          case "get_project_engine_integration":
            return [];
          case "get_project_capture_policy":
            return policyFixture();
          case "get_capture_engine_metadata":
            return args?.input?.captureItemId === "item-2"
              ? engineMetadataFixture("Ethan", "char-2")
              : null;
          default:
            return null;
        }
      },
    );
    api.suggestForCapture.mockImplementation(async (captureItemId: string) =>
      captureItemId === "item-2"
        ? {
            ...two,
            suggestedCharacterId: "char-2",
            recognitionConfidence: 0.84,
            reviewStatus: "pending",
          }
        : { ...one, reviewStatus: "none" },
    );
    const wrapper = mount(Capture);
    await flushPromises();

    const findCard = (name: string) =>
      wrapper
        .findAll(".capture-card")
        .find((card) => (card.attributes("aria-label") ?? "").includes(name));
    await findCard("two.png")!.trigger("click");
    await flushPromises();
    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("人物"))!
      .trigger("click");
    await flushPromises();

    // Confirm the auto-picked Ethan manually, even though it is the same
    // character: that turns the pick into a human decision.
    const ethanRow = wrapper
      .findAll(".character-row")
      .find((row) => row.text().includes("Ethan"));
    expect(ethanRow).toBeTruthy();
    await ethanRow!.trigger("click");

    // Switching images must keep a manual pick.
    await findCard("one.png")!.trigger("click");
    await flushPromises();
    if (!wrapper.find("#label-title").text().includes("选择主角色")) {
      await wrapper
        .findAll("button")
        .find((button) => button.text().includes("人物"))!
        .trigger("click");
      await flushPromises();
    }
    expect(wrapper.find("#label-title").text()).toContain("选择主角色");
    expect(wrapper.find(".character-row.selected").text()).toContain("Ethan");
    wrapper.unmount();
  });

  it("launches the project game from the session toolbar", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      switch (command) {
        case "get_project_engine_integration":
          return [
            {
              integration: {
                id: "install-1",
                projectId: project.id,
                engine: "renpy",
                engineRoot: "D:\\Games\\Demo",
                gameDirectory: "D:\\Games\\Demo\\game",
                pythonMajor: 3,
                engineVersion: null,
                bridgeVersion: "1.1.3",
                autoHideUi: false,
                autoHideUiOverride: null,
                metadataDirectory: null,
                manifestJson: "[]",
                installedAt: "",
                updatedAt: "",
              },
              bridgeFilePresent: true,
              compiledFilePresent: false,
              bundledBridgeVersion: "1.1.3",
              resolvedAutoHideUi: false,
              warning: null,
            },
          ];
        case "get_project_capture_policy":
          return policyFixture();
        case "get_capture_engine_metadata":
          return null;
        case "list_project_games":
          return [
            {
              integrationId: "install-1",
              engineRoot: "D:\\Games\\Demo",
              executable: "D:\\Games\\Demo\\Demo.exe",
              gameName: "Demo",
              recommended: true,
            },
          ];
        case "launch_project_game":
          return {
            integrationId: "install-1",
            engineRoot: "D:\\Games\\Demo",
            executable: "D:\\Games\\Demo\\Demo.exe",
            gameName: "Demo",
            recommended: true,
          };
        default:
          return null;
      }
    });
    const wrapper = mount(Capture);
    await flushPromises();

    const launchButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("启动游戏"));
    expect(launchButton).toBeTruthy();
    await launchButton!.trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("launch_project_game", {
      input: {
        projectId: project.id,
        integrationId: "install-1",
        executable: "D:\\Games\\Demo\\Demo.exe",
      },
    });
    wrapper.unmount();
  });

  it("lets the user pick the executable when a folder has several programs", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      switch (command) {
        case "get_project_engine_integration":
          return [
            {
              integration: {
                id: "install-1",
                projectId: project.id,
                engine: "renpy",
                engineRoot: "D:\\Games\\Demo",
                gameDirectory: "D:\\Games\\Demo\\game",
                pythonMajor: 3,
                engineVersion: null,
                bridgeVersion: "1.1.3",
                autoHideUi: false,
                autoHideUiOverride: null,
                metadataDirectory: null,
                manifestJson: "[]",
                installedAt: "",
                updatedAt: "",
              },
              bridgeFilePresent: true,
              compiledFilePresent: false,
              bundledBridgeVersion: "1.1.3",
              resolvedAutoHideUi: false,
              warning: null,
            },
          ];
        case "get_project_capture_policy":
          return policyFixture();
        case "get_capture_engine_metadata":
          return null;
        case "list_project_games":
          return [
            {
              integrationId: "install-1",
              engineRoot: "D:\\Games\\Demo",
              executable: "D:\\Games\\Demo\\Story.exe",
              gameName: "Demo",
              recommended: true,
            },
            {
              integrationId: "install-1",
              engineRoot: "D:\\Games\\Demo",
              executable: "D:\\Games\\Demo\\URM.exe",
              gameName: "Demo",
              recommended: false,
            },
          ];
        case "launch_project_game":
          return {
            integrationId: "install-1",
            engineRoot: "D:\\Games\\Demo",
            executable: "D:\\Games\\Demo\\URM.exe",
            gameName: "Demo",
            recommended: false,
          };
        default:
          return null;
      }
    });
    const wrapper = mount(Capture);
    await flushPromises();

    const launchButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("启动游戏"));
    await launchButton!.trigger("click");
    await flushPromises();

    const rows = wrapper.findAll(".game-launch-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("Story.exe（推荐）");
    await rows[1].find("button").trigger("click");
    await flushPromises();

    expect(invokeMock).toHaveBeenCalledWith("launch_project_game", {
      input: {
        projectId: project.id,
        integrationId: "install-1",
        executable: "D:\\Games\\Demo\\URM.exe",
      },
    });
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
    expect(wrapper.get(".session-health").findAll("span")
      .find((status) => status.text() === "AI 未配置")!.classes()).not.toContain("online");
    const startRecognition = wrapper
      .findAll("button")
      .find((button) => button.text().includes("开始识别导入截图"));
    expect(startRecognition?.attributes("disabled")).toBeDefined();
    expect(startRecognition?.attributes("title")).toContain("AI 未配置");
    expect(api.startImportedRecognition).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("shows the bundled sidecar engine as ready and enables imported recognition", async () => {
    api.runtimeStatus.mockResolvedValue({
      engineStatus: "sidecar",
      workerStatus: "idle",
      activeCaptureItemId: null,
      queuedCount: 0,
      archivePendingCount: 0,
      lastError: null,
    });
    api.deferredImportRecognitionCount.mockResolvedValue(3);
    const wrapper = mount(Capture);
    await flushPromises();

    expect(wrapper.text()).toContain("内置引擎");
    expect(wrapper.get(".session-health").findAll("span")
      .find((status) => status.text() === "内置引擎")!.classes()).toContain("online");
    const startRecognition = wrapper
      .findAll("button")
      .find((button) => button.text().includes("开始识别导入截图"));
    expect(startRecognition?.attributes("disabled")).toBeUndefined();
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
    expect(wrapper.text()).toContain("30 张待分类");
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

  it("removes a capture whose source was deleted before archive", async () => {
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
    expect(wrapper.findAll(".capture-card").length).toBe(2);

    api.listProjectRecentCaptures.mockResolvedValue([second]);
    eventHandlers.get("capture:item-purged")?.({ payload: { captureItemId: "item-1" } });
    await flushPromises();

    const remaining = wrapper.findAll(".capture-card");
    expect(remaining.length).toBe(1);
    expect(remaining[0].attributes("aria-label")).toContain("two.png");
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

  it("offers removing an awaiting capture from the working strip", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([item()]);
    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper.find(".capture-card").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')!.textContent).toContain("从 Scene Vault 移除");
    wrapper.unmount();
  });

  it("hides removal while a capture is being processed", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([item("queued")]);
    const wrapper = mount(Capture);
    await flushPromises();

    await wrapper.find(".capture-card").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')!.textContent).not.toContain("从 Scene Vault 移除");
    wrapper.unmount();
  });

  it("shows a replaced source and does not read the mismatched image", async () => {
    api.listProjectRecentCaptures.mockResolvedValue([
      { ...item(), sourceFileState: "replaced", destinationPath: null },
    ]);
    api.readImage.mockClear();
    const wrapper = mount(Capture);
    await flushPromises();

    expect(wrapper.text()).toContain("原图已被替换");
    expect(api.readImage).not.toHaveBeenCalled();
    wrapper.unmount();
  });
});
