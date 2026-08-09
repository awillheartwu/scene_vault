import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { api } = vi.hoisted(() => ({
  api: {
    listProjects: vi.fn(),
    listSessions: vi.fn(),
    listCharacters: vi.fn(),
    listHistory: vi.fn(),
    getAppSettings: vi.fn(),
    readThumbnail: vi.fn(),
    readImage: vi.fn(),
  },
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  captureStatusLabel: (status: string) => status,
  pathFileName: (path: string) => path.split(/[\\/]/).pop() || path,
  pathMimeType: () => "image/png",
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
