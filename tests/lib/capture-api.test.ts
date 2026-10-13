import { beforeEach, expect, it, vi } from "vitest";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  save: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  revealItemInDir: vi.fn(),
}));

import { captureApi, engineApi } from "@/lib/capture-api";

beforeEach(() => {
  tauri.invoke.mockReset().mockResolvedValue(undefined);
});

it("records failed media latency in the perf channel without an IPC error", async () => {
  tauri.invoke.mockRejectedValueOnce("not found: capture source image");

  await expect(captureApi.readThumbnail("capture-1", "source")).rejects.toBe(
    "not found: capture source image",
  );

  expect(tauri.invoke).toHaveBeenCalledTimes(2);
  expect(tauri.invoke).toHaveBeenNthCalledWith(1, "read_capture_thumbnail", {
    input: { captureItemId: "capture-1", variant: "source" },
  });
  expect(tauri.invoke).toHaveBeenNthCalledWith(2, "record_client_event", {
    input: expect.objectContaining({
      event: "read_timing",
      captureItemId: "capture-1",
      channel: "perf",
      outcome: "failed",
      durationMs: expect.any(Number),
    }),
  });
});

it("records a read timing for successful media reads", async () => {
  const bytes = new ArrayBuffer(8);
  tauri.invoke.mockImplementation((command: string) =>
    command === "read_capture_thumbnail" ? Promise.resolve(bytes) : Promise.resolve(undefined),
  );

  await expect(captureApi.readThumbnail("capture-1", "source")).resolves.toBe(bytes);

  expect(tauri.invoke).toHaveBeenNthCalledWith(1, "read_capture_thumbnail", {
    input: { captureItemId: "capture-1", variant: "source" },
  });
  expect(tauri.invoke).toHaveBeenNthCalledWith(2, "record_client_event", {
    input: expect.objectContaining({
      level: "debug",
      module: "capture.image",
      event: "read_timing",
      operationId: "read_capture_thumbnail",
      captureItemId: "capture-1",
      durationMs: expect.any(Number),
    }),
  });
});

it("continues reporting failures from non-media IPC commands", async () => {
  tauri.invoke
    .mockRejectedValueOnce("database unavailable")
    .mockResolvedValueOnce(undefined);

  await expect(captureApi.listProjects()).rejects.toBe("database unavailable");

  expect(tauri.invoke).toHaveBeenNthCalledWith(1, "list_projects", undefined);
  expect(tauri.invoke).toHaveBeenNthCalledWith(2, "record_client_event", {
    input: expect.objectContaining({
      level: "error",
      operationId: "list_projects",
    }),
  });
});

it("sends a project id and an optional archive search directory to the file checker", async () => {
  tauri.invoke.mockResolvedValueOnce({});

  await captureApi.reconcileProjectFiles("project-1");

  expect(tauri.invoke).toHaveBeenCalledWith("reconcile_project_files", {
    input: { projectId: "project-1", archiveSearchDirectory: null },
  });

  tauri.invoke.mockResolvedValueOnce({});

  await captureApi.reconcileProjectFiles("project-1", "/volume/renamed-archive");

  expect(tauri.invoke).toHaveBeenLastCalledWith("reconcile_project_files", {
    input: {
      projectId: "project-1",
      archiveSearchDirectory: "/volume/renamed-archive",
    },
  });
});

it("engine api keeps camelCase payloads for the bridge commands", async () => {
  tauri.invoke.mockResolvedValue({ archiveOutputSize: "keep" });

  await engineApi.detectEngine("D:\\games\\demo");
  await engineApi.getProjectIntegration("project-1");
  await engineApi.installBridge("project-1", "D:\\games\\demo");
  await engineApi.uninstallBridge("project-1");
  await engineApi.uninstallBridge("project-1", "install-9");
  await engineApi.getCaptureMetadata("capture-1");
  await engineApi.getCapturePolicy("project-1");
  await engineApi.setArchiveOutputSize("project-1", "1080p");

  expect(tauri.invoke).toHaveBeenNthCalledWith(1, "detect_engine", {
    input: { path: "D:\\games\\demo" },
  });
  expect(tauri.invoke).toHaveBeenNthCalledWith(2, "get_project_engine_integration", {
    input: { projectId: "project-1" },
  });
  expect(tauri.invoke).toHaveBeenNthCalledWith(3, "install_project_engine_bridge", {
    input: { projectId: "project-1", path: "D:\\games\\demo" },
  });
  expect(tauri.invoke).toHaveBeenNthCalledWith(4, "uninstall_project_engine_bridge", {
    input: { projectId: "project-1" },
  });
  expect(tauri.invoke).toHaveBeenNthCalledWith(5, "uninstall_project_engine_bridge", {
    input: { projectId: "project-1", integrationId: "install-9" },
  });
  expect(tauri.invoke).toHaveBeenNthCalledWith(6, "get_capture_engine_metadata", {
    input: { captureItemId: "capture-1" },
  });
  expect(tauri.invoke).toHaveBeenNthCalledWith(7, "get_project_capture_policy", {
    input: { projectId: "project-1" },
  });
  expect(tauri.invoke).toHaveBeenNthCalledWith(8, "set_project_archive_output_size", {
    input: { projectId: "project-1", archiveOutputSize: "1080p" },
  });
});
