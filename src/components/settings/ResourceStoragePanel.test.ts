import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { api, openPathExternal, revealPath, toast } = vi.hoisted(() => ({
  api: {
    getProcessResourceStatus: vi.fn(),
    getStorageResourceStatus: vi.fn(),
    cleanupResource: vi.fn(),
    getLogSettings: vi.fn(),
    updateLogSettings: vi.fn(),
  },
  openPathExternal: vi.fn(),
  revealPath: vi.fn(),
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  openPathExternal,
  revealPath,
}));

vi.mock("@/lib/toast", () => ({ toast }));

import ResourceStoragePanel from "./ResourceStoragePanel.vue";

const processStatus = {
  capturedAt: "2026-08-11T08:00:00Z",
  approximate: false,
  logicalProcessors: 16,
  groups: [
    {
      role: "rust",
      processCount: 1,
      pids: [10],
      cpuPercent: 1.2,
      workingSetBytes: 100,
      peakWorkingSetBytes: 200,
      privateBytes: 80,
    },
  ],
  totalCpuPercent: 1.2,
  totalWorkingSetBytes: 100,
  totalPrivateBytes: 80,
};

const storageStatus = {
  capturedAt: "2026-08-11T08:00:00Z",
  totalBytes: 100,
  entries: [
    {
      kind: "thumbnail_cache",
      label: "缩略图缓存",
      path: "C:\\cache\\thumbnails",
      totalBytes: 100,
      fileCount: 2,
      cleanupAvailable: true,
      cleanupDescription: "可安全重建",
    },
  ],
};

beforeEach(() => {
  vi.useFakeTimers();
  api.getProcessResourceStatus.mockResolvedValue(processStatus);
  api.getStorageResourceStatus.mockResolvedValue(storageStatus);
  api.cleanupResource.mockResolvedValue({
    kind: "thumbnail_cache",
    removedFiles: 2,
    reclaimedBytes: 100,
    message: "已清理",
  });
  api.getLogSettings.mockResolvedValue({
    retentionDays: 14,
    maxFileSizeMb: 5,
    maxArchivedFiles: 20,
    automaticCleanup: true,
  });
  api.updateLogSettings.mockImplementation(async (settings) => settings);
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("ResourceStoragePanel lifecycle", () => {
  it("uses full-width-safe text actions for refresh controls", async () => {
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    const actions = wrapper.findAll(".section-action");
    expect(actions).toHaveLength(3);
    expect(actions.map((button) => button.text())).toEqual(["刷新", "重新扫描", "保存策略"]);
    expect(wrapper.findAll("button.icon-action")).toHaveLength(0);
    expect(wrapper.text()).toContain("Python AI Worker");
    expect(wrapper.text()).toContain("首次 AI 请求时按需启动");
    wrapper.unmount();
  });

  it("samples processes while mounted and stops after unmount", async () => {
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();
    expect(api.getProcessResourceStatus).toHaveBeenCalledTimes(1);
    expect(api.getStorageResourceStatus).toHaveBeenCalledTimes(1);
    expect(api.getLogSettings).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(2_000);
    await flushPromises();
    expect(api.getProcessResourceStatus).toHaveBeenCalledTimes(2);
    expect(api.getStorageResourceStatus).toHaveBeenCalledTimes(1);

    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(4_000);
    expect(api.getProcessResourceStatus).toHaveBeenCalledTimes(2);
  });

  it("saves the visible log retention and cleanup policy", async () => {
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();
    const retention = wrapper.get<HTMLInputElement>('.log-policy-fields input[type="number"]');
    await retention.setValue(30);
    await wrapper.get(".log-policy-card").trigger("submit");
    await flushPromises();
    expect(api.updateLogSettings).toHaveBeenCalledWith(expect.objectContaining({
      retentionDays: 30,
      automaticCleanup: true,
    }));
    expect(toast.success).toHaveBeenCalledWith("日志保留与清理策略已保存并立即生效");
  });

  it("confirms cleanup, invokes the generic cleanup interface, and rescans storage", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    await wrapper.get(".cleanup-action").trigger("click");
    await flushPromises();

    expect(api.cleanupResource).toHaveBeenCalledWith("thumbnail_cache");
    expect(api.getStorageResourceStatus).toHaveBeenCalledTimes(2);
    expect(toast.success).toHaveBeenCalled();
    wrapper.unmount();
  });
});
