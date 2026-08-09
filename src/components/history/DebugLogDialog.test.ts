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

import DebugLogDialog from "./DebugLogDialog.vue";

beforeEach(() => {
  vi.clearAllMocks();
  api.listDebugLogs.mockResolvedValue({
    records: [{
      timestamp: "2026-08-09T02:03:04.000Z",
      level: "error",
      module: "capture.worker",
      message: "processing failed",
    }],
    matchedCount: 1,
    truncated: false,
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

describe("DebugLogDialog", () => {
  it("loads records and applies time, level, and module filters", async () => {
    const wrapper = mount(DebugLogDialog);
    try {
      await flushPromises();
      expect(document.body.textContent).toContain("processing failed");
      expect(document.body.textContent).toContain("capture.worker");

      const selects = document.body.querySelectorAll<HTMLSelectElement>(".log-toolbar select");
      const input = document.body.querySelector<HTMLInputElement>(".module-filter input");
      expect(selects).toHaveLength(2);
      selects[1].value = "error";
      selects[1].dispatchEvent(new Event("change", { bubbles: true }));
      if (input) {
        input.value = "worker";
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }
      document.body.querySelector<HTMLButtonElement>(".log-filter-button")?.click();
      await flushPromises();

      expect(api.listDebugLogs).toHaveBeenLastCalledWith(expect.objectContaining({
        levels: ["error"],
        module: "worker",
        limit: 500,
      }));
    } finally {
      wrapper.unmount();
    }
  });

  it("copies a bounded diagnostic summary and runs policy cleanup", async () => {
    const wrapper = mount(DebugLogDialog);
    try {
      await flushPromises();
      document.body.querySelector<HTMLButtonElement>(".copy-diagnostics")?.click();
      await flushPromises();
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith("diagnostic summary");
      expect(toastSuccess).toHaveBeenCalledWith("诊断信息已复制，可直接粘贴给 Agent");

      const cleanupButton = Array.from(document.body.querySelectorAll<HTMLButtonElement>(".log-dialog-footer button"))
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
    const wrapper = mount(DebugLogDialog);
    try {
      await flushPromises();
      expect(document.body.textContent).toContain("可能包含本地路径");
      document.body.querySelector<HTMLButtonElement>(".copy-diagnostics")?.click();
      await flushPromises();
      expect(execCommand).toHaveBeenCalledWith("copy");
      expect(toastSuccess).toHaveBeenCalledWith("诊断信息已复制，可直接粘贴给 Agent");
    } finally {
      wrapper.unmount();
    }
  });
});
