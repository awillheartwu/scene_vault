import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { api, openPath, toastSuccess } = vi.hoisted(() => ({
  api: {
    listDebugLogs: vi.fn(),
    getLogStatus: vi.fn(),
    cleanupDebugLogs: vi.fn(),
    getDiagnosticSummary: vi.fn(),
  },
  openPath: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  openPathExternal: openPath,
}));

vi.mock("@/lib/toast", () => ({
  toast: { success: toastSuccess },
}));

import DebugLogPanel from "./DebugLogPanel.vue";

beforeEach(() => {
  vi.clearAllMocks();
  api.listDebugLogs.mockResolvedValue({
    records: [{
      timestamp: "2026-08-09T02:03:04.000Z",
      level: "error",
      module: "capture.worker",
      message: "processing failed",
      event: "processing_failed",
      outcome: "failed",
      requestId: "req-1",
    }],
    matchedCount: 1,
    truncated: false,
    offset: 0,
    nextOffset: null,
    hasMore: false,
  });
  api.getLogStatus.mockResolvedValue({
    directory: "C:\\Logs\\Scene Vault",
    fileCount: 2,
    totalBytes: 2048,
    retentionDays: 14,
    maxFileBytes: 5 * 1024 * 1024,
    maxArchivedFiles: 20,
  });
  api.cleanupDebugLogs.mockResolvedValue({
    removedFiles: 1,
    status: {
      directory: "C:\\Logs\\Scene Vault",
      fileCount: 1,
      totalBytes: 1024,
      retentionDays: 14,
      maxFileBytes: 5 * 1024 * 1024,
      maxArchivedFiles: 20,
    },
  });
  api.getDiagnosticSummary.mockResolvedValue("diagnostic summary");
  Object.assign(navigator, {
    clipboard: { writeText: vi.fn().mockResolvedValue(undefined) },
  });
});

describe("DebugLogPanel", () => {
  it("loads records and applies time, level, and module filters", async () => {
    const wrapper = mount(DebugLogPanel, { attachTo: document.body });
    try {
      await flushPromises();
      expect(document.body.textContent).toContain("processing failed");
      expect(document.body.textContent).toContain("capture.worker");

      const selects = document.body.querySelectorAll<HTMLSelectElement>(".log-toolbar select");
      const input = document.body.querySelector<HTMLInputElement>(".module-filter input");
      expect(selects).toHaveLength(3);
      selects[1].value = "error";
      selects[1].dispatchEvent(new Event("change", { bubbles: true }));
      if (input) {
        input.value = "worker";
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }
      const eventInput = document.body.querySelector<HTMLInputElement>(".event-filter input");
      const correlationInput = document.body.querySelector<HTMLInputElement>(".correlation-filter input");
      if (eventInput) {
        eventInput.value = "processing";
        eventInput.dispatchEvent(new Event("input", { bubbles: true }));
      }
      if (correlationInput) {
        correlationInput.value = "req-1";
        correlationInput.dispatchEvent(new Event("input", { bubbles: true }));
      }
      document.body.querySelector<HTMLButtonElement>(".log-filter-button")?.click();
      await flushPromises();

      expect(api.listDebugLogs).toHaveBeenLastCalledWith(expect.objectContaining({
        levels: ["error"],
        module: "worker",
        event: "processing",
        correlationId: "req-1",
        offset: 0,
        limit: 100,
      }));
    } finally {
      wrapper.unmount();
    }
  });

  it("loads the next page on demand without replacing existing rows", async () => {
    api.listDebugLogs
      .mockResolvedValueOnce({
        records: [{ timestamp: "2026-08-09T02:00:00Z", level: "info", module: "first", message: "page one" }],
        matchedCount: 2,
        truncated: true,
        offset: 0,
        nextOffset: 1,
        hasMore: true,
      })
      .mockResolvedValueOnce({
        records: [{ timestamp: "2026-08-09T01:00:00Z", level: "info", module: "second", message: "page two" }],
        matchedCount: 2,
        truncated: false,
        offset: 1,
        nextOffset: null,
        hasMore: false,
      });
    const wrapper = mount(DebugLogPanel, { attachTo: document.body });
    try {
      await flushPromises();
      await wrapper.get(".log-load-sentinel button").trigger("click");
      await flushPromises();
      expect(wrapper.text()).toContain("page one");
      expect(wrapper.text()).toContain("page two");
      expect(api.listDebugLogs).toHaveBeenLastCalledWith(expect.objectContaining({ offset: 1, limit: 100 }));
    } finally {
      wrapper.unmount();
    }
  });

  it("disconnects infinite-loading observation when leaving the log page", async () => {
    const observer = { observe: vi.fn(), disconnect: vi.fn() };
    vi.stubGlobal("IntersectionObserver", vi.fn(function () { return observer; }));
    const wrapper = mount(DebugLogPanel);
    try {
      await flushPromises();
      expect(observer.observe).toHaveBeenCalledOnce();
      wrapper.unmount();
      expect(observer.disconnect).toHaveBeenCalledOnce();
    } finally {
      if (wrapper.exists()) wrapper.unmount();
      vi.unstubAllGlobals();
    }
  });

  it("copies a bounded diagnostic summary and runs policy cleanup", async () => {
    const wrapper = mount(DebugLogPanel, { attachTo: document.body });
    try {
      await flushPromises();
      document.body.querySelector<HTMLButtonElement>(".copy-diagnostics")?.click();
      await flushPromises();
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith("diagnostic summary");
      expect(toastSuccess).toHaveBeenCalledWith("诊断信息已复制，可直接粘贴给 Agent");

      const cleanupButton = Array.from(document.body.querySelectorAll<HTMLButtonElement>(".log-panel-footer button"))
        .find((button) => button.textContent?.includes("按策略清理"));
      cleanupButton?.click();
      await flushPromises();
      expect(api.cleanupDebugLogs).toHaveBeenCalledOnce();
      expect(api.listDebugLogs).toHaveBeenCalledTimes(2);
    } finally {
      wrapper.unmount();
    }
  });

  it("falls back to local selection copy when WebView clipboard permission is denied", async () => {
    vi.mocked(navigator.clipboard.writeText).mockRejectedValueOnce(new Error("denied"));
    const execCommand = vi.fn().mockReturnValue(true);
    Object.assign(document, { execCommand });
    const wrapper = mount(DebugLogPanel, { attachTo: document.body });
    try {
      await flushPromises();
      expect(document.body.textContent).toContain("离开后不占用监控资源");
      document.body.querySelector<HTMLButtonElement>(".copy-diagnostics")?.click();
      await flushPromises();
      expect(execCommand).toHaveBeenCalledWith("copy");
      expect(toastSuccess).toHaveBeenCalledWith("诊断信息已复制，可直接粘贴给 Agent");
    } finally {
      wrapper.unmount();
    }
  });
});
