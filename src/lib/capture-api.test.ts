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

import { captureApi } from "./capture-api";

beforeEach(() => {
  tauri.invoke.mockReset();
});

it("does not report speculative media read failures as IPC errors", async () => {
  tauri.invoke.mockRejectedValueOnce("not found: capture source image");

  await expect(captureApi.readThumbnail("capture-1", "source")).rejects.toBe(
    "not found: capture source image",
  );

  expect(tauri.invoke).toHaveBeenCalledTimes(1);
  expect(tauri.invoke).toHaveBeenCalledWith("read_capture_thumbnail", {
    input: { captureItemId: "capture-1", variant: "source" },
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

it("sends a project id to the project-wide file checker", async () => {
  tauri.invoke.mockResolvedValueOnce({});

  await captureApi.reconcileProjectFiles("project-1");

  expect(tauri.invoke).toHaveBeenCalledWith("reconcile_project_files", {
    input: { projectId: "project-1" },
  });
});
