import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Project } from "@/lib/capture-api";

const { api, eventHandlers } = vi.hoisted(() => ({
  api: {
    getAppSettings: vi.fn(),
    listProjects: vi.fn(),
    listProjectOverviews: vi.fn(),
  },
  eventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@/lib/capture-api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/capture-api")>()),
  captureApi: api,
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => null) }));
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
    hide: vi.fn(async () => {}),
  })),
}));

import PopupClassify from "@/pages/PopupClassify.vue";
import PopupWorkbench from "@/pages/PopupWorkbench.vue";

const savedProject: Project = {
  id: "saved-project",
  name: "Saved Game",
  description: null,
  coverAssetId: null,
  coverCaptureItemId: null,
  lastSourceDirectory: null,
  lastDestinationDirectory: null,
  destinationDirectory: null,
  createdAt: "2026-08-01T00:00:00Z",
  updatedAt: "2026-08-01T00:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  eventHandlers.clear();
  localStorage.clear();
  localStorage.setItem("scene-vault.capture.project", savedProject.id);
  api.listProjects.mockReset().mockResolvedValue([savedProject]);
  api.listProjectOverviews.mockReset().mockResolvedValue([
    { projectId: savedProject.id, engine: "renpy", engineVersions: "8.4.1" },
    { projectId: "queue-project", engine: "renpy", engineVersions: "7.8.6" },
  ]);
});

describe("PopupWorkbench project context", () => {
  it.each([false, true])("keeps the queue project's title and badge over a late saved-project reply (split=%s)", async (splitPopupWindows) => {
    api.getAppSettings.mockResolvedValue({ splitPopupWindows });
    let resolveSaved!: (projects: Project[]) => void;
    api.listProjects.mockReturnValueOnce(new Promise<Project[]>((resolve) => { resolveSaved = resolve; }));
    const wrapper = mount(PopupWorkbench, {
      global: { stubs: { PopupClassify: true, PopupNote: true } },
    });
    try {
      await flushPromises();
      expect(api.listProjects).toHaveBeenCalledOnce();
      const classify = wrapper.getComponent(PopupClassify);
      expect(classify.props("embedded")).toBe(true);
      classify.vm.$emit("project-context", { projectId: "queue-project", projectName: "Queued Game" });
      await flushPromises();
      expect(wrapper.get(".workbench-title .workbench-sub").text()).toBe("Queued Game");
      expect(wrapper.get(".workbench-title .renpy-badge").text()).toBe("Ren'Py 7");
      expect(wrapper.get(".workbench-title .renpy-badge").attributes("title")).toBe("7.8.6");

      resolveSaved([savedProject]);
      await flushPromises();
      eventHandlers.get("note:refresh")!({ payload: {} });
      await flushPromises();
      expect(api.listProjects).toHaveBeenCalledOnce();
      expect(wrapper.get(".workbench-title .workbench-sub").text()).toBe("Queued Game");
      expect(wrapper.get(".workbench-title .renpy-badge").text()).toBe("Ren'Py 7");

      classify.vm.$emit("project-context", { projectId: null, projectName: null });
      await flushPromises();
      expect(wrapper.get(".workbench-title .workbench-sub").text()).toBe("截图分类 · 快速笔记");
      expect(wrapper.find(".workbench-title .renpy-badge").exists()).toBe(false);
    } finally {
      resolveSaved([savedProject]);
      wrapper.unmount();
    }
  });
});
