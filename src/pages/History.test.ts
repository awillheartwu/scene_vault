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
  },
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  captureStatusLabel: (status: string) => status,
  pathFileName: (path: string) => path.split(/[\\/]/).pop() || path,
  pathMimeType: () => "image/png",
  revealPath: vi.fn(),
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
  api.listProjects.mockResolvedValue([project]);
  api.listSessions.mockResolvedValue([session]);
  api.listCharacters.mockResolvedValue([]);
  api.getAppSettings.mockResolvedValue({ showPrivateByDefault: false });
  api.readThumbnail.mockResolvedValue(new ArrayBuffer(4));
  api.readImage.mockResolvedValue(new ArrayBuffer(4));
  api.listHistory.mockResolvedValue({ entries: [entry("1", "2026-08-01T00:00:00Z")], total: 1 });
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
