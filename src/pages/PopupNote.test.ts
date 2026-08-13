import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { api, eventHandlers } = vi.hoisted(() => ({
  api: {
    getProjectNote: vi.fn(),
    listProjects: vi.fn(),
    getAppSettings: vi.fn(),
    updateProjectNote: vi.fn(),
    openProjectNote: vi.fn(),
    revealProjectNote: vi.fn(),
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
  getCurrentWindow: vi.fn(() => ({ hide: vi.fn(async () => {}) })),
}));

vi.mock("@/lib/capture-api", () => ({ captureApi: api }));

import PopupNote from "./PopupNote.vue";

const project = {
  id: "project-abc",
  name: "ABC",
  description: null,
  coverAssetId: null,
  lastSourceDirectory: null,
  lastDestinationDirectory: null,
  destinationDirectory: "D:\\Archive",
  createdAt: "2026-08-13T00:00:00Z",
  updatedAt: "2026-08-13T00:00:00Z",
};

const note = {
  id: "note-1",
  projectId: project.id,
  destinationDirectory: "D:\\Archive",
  remotePath: "D:\\Archive\\笔记.md",
  content: "录制笔记",
  status: "synced",
  attemptCount: 0,
  nextRetryAt: null,
  lastSyncedContent: "录制笔记",
  errorMessage: null,
  syncedAt: "2026-08-13T00:00:00Z",
  createdAt: "2026-08-13T00:00:00Z",
  updatedAt: "2026-08-13T00:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  eventHandlers.clear();
  localStorage.clear();
  api.getProjectNote.mockResolvedValue(note);
  api.listProjects.mockResolvedValue([project]);
  api.getAppSettings.mockResolvedValue({ autoSaveNotes: true });
});

describe("PopupNote project refresh", () => {
  it("loads a project selected after the hidden popup first mounted", async () => {
    const wrapper = mount(PopupNote);
    await flushPromises();
    expect(wrapper.text()).toContain("还没有选择项目");
    expect(api.getProjectNote).not.toHaveBeenCalled();

    localStorage.setItem("scene-vault.capture.project", project.id);
    eventHandlers.get("note:refresh")?.({ payload: null });
    await flushPromises();

    expect(api.getProjectNote).toHaveBeenCalledWith(project.id);
    expect(wrapper.text()).toContain("ABC");
    expect(wrapper.get("textarea").element).toHaveProperty("value", "录制笔记");
    wrapper.unmount();
  });
});
