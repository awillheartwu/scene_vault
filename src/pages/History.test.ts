import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { api } = vi.hoisted(() => ({
  api: {
    listProjects: vi.fn(),
    listSessions: vi.fn(),
    listCharacters: vi.fn(),
    listHistory: vi.fn(),
    getAppSettings: vi.fn(),
    readThumbnail: vi.fn(),
    readImage: vi.fn(),
    retry: vi.fn(),
    revealPath: vi.fn(),
    listDebugLogs: vi.fn(),
    getLogStatus: vi.fn(),
  },
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  captureStatusLabel: (status: string) => status,
  pathFileName: (path: string) => path.split(/[\\/]/).pop() || path,
  pathMimeType: () => "image/png",
  revealPath: api.revealPath,
}));

import History from "./History.vue";

const project = {
  id: "project-1",
  name: "P",
  description: null,
  coverAssetId: null,
  lastSourceDirectory: null,
  lastDestinationDirectory: null,
  destinationDirectory: "D:\\Archive",
  createdAt: "2026-08-01T00:00:00Z",
  updatedAt: "2026-08-01T00:00:00Z",
};
const otherProject = {
  ...project,
  id: "project-2",
  name: "Other",
};
const session = {
  id: "session-1",
  projectId: project.id,
  status: "active",
  startedAt: "2026-08-01T00:00:00Z",
  endedAt: null,
  createdAt: "",
  updatedAt: "",
};

function entry(id: string, capturedAt: string) {
  return {
    id,
    projectId: project.id,
    projectName: "P",
    sessionStatus: "active",
    sessionId: session.id,
    characterId: null,
    classification: "scene",
    characterName: null,
    assetId: null,
    sourcePath: `D:\\s\\${id}.png`,
    annotatedPath: null,
    avatarPath: null,
    destinationPath: null,
    destinationAvatarPath: null,
    status: "completed",
    faceBoxJson: null,
    errorMessage: null,
    failureStage: null,
    attemptCount: 0,
    nextRetryAt: null,
    processingWarningsJson: "[]",
    capturedAt,
    processedAt: null,
    archivedAt: null,
    createdAt: capturedAt,
    updatedAt: capturedAt,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  api.listProjects.mockResolvedValue([project]);
  api.listSessions.mockResolvedValue([session]);
  api.listCharacters.mockResolvedValue([]);
  api.getAppSettings.mockResolvedValue({ showPrivateByDefault: false });
  api.readThumbnail.mockResolvedValue(new ArrayBuffer(4));
  api.readImage.mockResolvedValue(new ArrayBuffer(4));
  api.listHistory.mockResolvedValue({ entries: [entry("1", "2026-08-01T00:00:00Z")], total: 1 });
  api.listDebugLogs.mockResolvedValue({ records: [], matchedCount: 0, truncated: false });
  api.getLogStatus.mockResolvedValue({
    directory: "C:\\Logs\\Scene Vault",
    fileCount: 1,
    totalBytes: 512,
    retentionDays: 14,
    maxFileBytes: 5 * 1024 * 1024,
    maxArchivedFiles: 20,
  });
});

describe("History project continuity", () => {
  it("opens the project currently selected by Capture and Workbench", async () => {
    localStorage.setItem("scene-vault.capture.project", otherProject.id);
    api.listProjects.mockResolvedValue([project, otherProject]);
    const wrapper = mount(History);
    await flushPromises();

    expect(wrapper.get(".history-filters select").element).toHaveProperty(
      "value",
      otherProject.id,
    );
    expect(api.listSessions).toHaveBeenCalledWith(otherProject.id);
    expect(api.listCharacters).toHaveBeenCalledWith(otherProject.id);
    expect(api.listHistory).toHaveBeenCalledWith(
      expect.objectContaining({ projectId: otherProject.id }),
    );
    wrapper.unmount();
  });

  it("falls back to the first available project when the saved project was deleted", async () => {
    localStorage.setItem("scene-vault.capture.project", "deleted-project");
    const wrapper = mount(History);
    await flushPromises();

    expect(api.listHistory).toHaveBeenCalledWith(
      expect.objectContaining({ projectId: project.id }),
    );
    expect(localStorage.getItem("scene-vault.capture.project")).toBe(project.id);
    wrapper.unmount();
  });
});

describe("History content tabs", () => {
  it("switches from screenshot history to the inline debug log panel", async () => {
    const wrapper = mount(History, { attachTo: document.body });
    try {
      await flushPromises();
      expect(wrapper.find(".history-workspace").exists()).toBe(true);

      const logTab = wrapper.findAll('[data-slot="tabs-trigger"]')
        .find((tab) => tab.text().includes("调试日志"));
      expect(logTab).toBeDefined();
      await logTab?.trigger("mousedown", { button: 0, ctrlKey: false });
      await flushPromises();

      expect(wrapper.find(".debug-log-panel").exists()).toBe(true);
      expect(wrapper.find(".history-workspace").exists()).toBe(false);
      expect(api.listDebugLogs).toHaveBeenCalledOnce();
    } finally {
      wrapper.unmount();
    }
  });
});

describe("History pagination", () => {
  it("loads the requested page and switches page sizes", async () => {
    api.listHistory.mockResolvedValue({ entries: [], total: 120 });
    const wrapper = mount(History);
    await flushPromises();

    expect(api.listHistory).toHaveBeenCalledWith(expect.objectContaining({ limit: 100, offset: 0 }));
    expect(wrapper.text()).toContain("共 120 条");

    await wrapper.get('[aria-label="下一页"]').trigger("click");
    await flushPromises();
    expect(api.listHistory).toHaveBeenLastCalledWith(expect.objectContaining({ offset: 100 }));

    const select = wrapper.get(".pagination-size select");
    await select.setValue("50");
    await flushPromises();
    expect(api.listHistory).toHaveBeenLastCalledWith(expect.objectContaining({ limit: 50, offset: 0 }));

    wrapper.unmount();
  });
});

describe("History context menu", () => {
  function menuItems() {
    return Array.from(document.body.querySelectorAll('[role="menuitem"]')) as HTMLElement[];
  }

  it("selects the row on right-click and reveals the source image", async () => {
    api.listHistory.mockResolvedValue({
      entries: [
        entry("1", "2026-08-01T00:00:00Z"),
        entry("2", "2026-08-02T00:00:00Z"),
      ],
      total: 2,
    });
    const wrapper = mount(History);
    await flushPromises();

    const rows = wrapper.findAll(".history-row");
    await rows[1].trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();

    const menu = document.body.querySelector('[role="menu"]');
    expect(menu).not.toBeNull();
    expect(menu!.textContent).toContain("查看详情");
    expect(menu!.textContent).toContain("显示原图");
    expect(rows[1].classes()).toContain("selected");

    const revealItem = menuItems().find((item) => item.textContent?.includes("显示原图"));
    expect(revealItem).toBeDefined();
    revealItem!.click();
    await flushPromises();
    expect(api.revealPath).toHaveBeenCalledWith("D:\\s\\2.png");
    wrapper.unmount();
  });

  it("opens the detail panel from the context menu", async () => {
    api.listHistory.mockResolvedValue({
      entries: [entry("1", "2026-08-01T00:00:00Z")],
      total: 1,
    });
    const wrapper = mount(History, { attachTo: document.body });
    await flushPromises();

    await wrapper.find(".history-row").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    const detailItem = menuItems().find((item) => item.textContent?.includes("查看详情"));
    detailItem!.click();
    await flushPromises();

    expect(wrapper.find(".detail-preview").exists()).toBe(true);
    wrapper.unmount();
  });

  it("hides reprocess for recognized captures without annotation", async () => {
    api.listHistory.mockResolvedValue({
      entries: [
        {
          ...entry("9", "2026-08-09T00:00:00Z"),
          classification: "person",
          annotatedPath: null,
          faceBoxJson: '{"x":10,"y":20,"width":30,"height":40}',
        },
      ],
      total: 1,
    });
    const wrapper = mount(History);
    await flushPromises();

    await wrapper.find(".history-row").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')!.textContent).not.toContain("重新识别");
    wrapper.unmount();
  });

  it("offers reprocess for degraded person captures", async () => {
    api.listHistory.mockResolvedValue({
      entries: [
        {
          ...entry("9", "2026-08-09T00:00:00Z"),
          classification: "person",
          annotatedPath: null,
        },
      ],
      total: 1,
    });
    const wrapper = mount(History);
    await flushPromises();

    await wrapper.find(".history-row").trigger("contextmenu", { clientX: 100, clientY: 60 });
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')!.textContent).toContain("重新识别");

    const reprocessItem = menuItems().find((item) => item.textContent?.includes("重新识别"));
    reprocessItem!.click();
    await flushPromises();
    expect(api.retry).toHaveBeenCalledWith("9");
    wrapper.unmount();
  });
});

describe("History responsive detail", () => {
  afterEach(() => {
    if (typeof window.matchMedia === "function") {
      delete (window as { matchMedia?: typeof window.matchMedia }).matchMedia;
    }
  });

  it("auto-opens a focus-trapped detail drawer on row selection and closes it without losing selection", async () => {
    window.matchMedia = vi.fn(
      (query: string): MediaQueryList =>
        ({
          matches: false,
          media: query,
          onchange: null,
          addListener: vi.fn(),
          removeListener: vi.fn(),
          addEventListener: vi.fn(),
          removeEventListener: vi.fn(),
          dispatchEvent: vi.fn(),
        }) as MediaQueryList,
    ) as unknown as typeof window.matchMedia;

    api.listHistory.mockResolvedValue({
      entries: [entry("1", "2026-08-01T00:00:00Z"), entry("2", "2026-08-02T00:00:00Z")],
      total: 2,
    });
    const wrapper = mount(History);
    try {
      await flushPromises();

      expect(document.body.querySelector(".responsive-detail-panel.is-drawer")).toBeNull();

      const rows = wrapper.findAll(".history-row");
      expect(rows).toHaveLength(2);
      await rows[1].trigger("click");
      await flushPromises();

      const drawer = document.body.querySelector(".responsive-detail-panel.is-drawer");
      expect(drawer).not.toBeNull();
      expect(drawer?.getAttribute("role")).toBe("dialog");
      expect(drawer?.getAttribute("aria-labelledby")).not.toBeNull();
      const labelledBy = drawer?.getAttribute("aria-labelledby");
      expect(labelledBy && document.getElementById(labelledBy)?.textContent).toContain("2.png");
      expect(drawer?.textContent).toContain("2.png");
      expect(rows[1].classes()).toContain("selected");

      const close = document.body.querySelector<HTMLButtonElement>('[aria-label="关闭详情"]');
      expect(close).not.toBeNull();
      close?.click();
      await flushPromises();

      expect(document.body.querySelector(".responsive-detail-panel.is-drawer")).toBeNull();
      expect(rows[1].classes()).toContain("selected");

      await rows[1].trigger("click");
      await flushPromises();
      expect(document.body.querySelector(".responsive-detail-panel.is-drawer")).not.toBeNull();

      await rows[0].trigger("click");
      await flushPromises();
      expect(document.body.querySelector(".responsive-detail-panel.is-drawer")?.textContent).toContain("1.png");
    } finally {
      wrapper.unmount();
    }
  });

  it("shows the detail as a static panel at wide layout and switches it on row selection", async () => {
    api.listHistory.mockResolvedValue({
      entries: [entry("1", "2026-08-01T00:00:00Z"), entry("2", "2026-08-02T00:00:00Z")],
      total: 2,
    });
    const wrapper = mount(History);
    try {
      await flushPromises();

      const panel = wrapper.find(".responsive-detail-panel.is-static");
      expect(panel.exists()).toBe(true);
      expect(panel.text()).toContain("1.png");
      expect(document.body.querySelector('[aria-label="关闭详情"]')).toBeNull();

      await wrapper.findAll(".history-row")[1].trigger("click");
      await flushPromises();

      expect(panel.text()).toContain("2.png");
      expect(document.body.querySelector(".responsive-detail-panel.is-drawer")).toBeNull();
    } finally {
      wrapper.unmount();
    }
  });
});
