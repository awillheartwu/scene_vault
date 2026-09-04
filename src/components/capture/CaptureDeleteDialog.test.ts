import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  previewCaptureDeletion: vi.fn(),
  deleteCaptureItem: vi.fn(),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  pathFileName: (path: string) => path.split(/[\\/]/).pop() || path,
}));

import CaptureDeleteDialog from "./CaptureDeleteDialog.vue";
import type { CaptureItem } from "@/lib/capture-api";

const item: CaptureItem = {
  id: "capture-1",
  sessionId: "session-1",
  projectId: "project-1",
  assetId: null,
  characterId: null,
  classification: "scene",
  sourcePath: "D:\\shots\\one.png",
  fileSize: 1,
  modifiedAtMs: null,
  contentHash: "hash-1",
  annotatedPath: null,
  avatarPath: null,
  destinationPath: "D:\\archive\\one.png",
  destinationAvatarPath: null,
  status: "completed",
  faceBoxJson: null,
  faceCount: null,
  suggestedCharacterId: null,
  recognitionConfidence: null,
  recognitionSource: null,
  reviewStatus: "none",
  errorMessage: null,
  failureStage: null,
  attemptCount: 0,
  nextRetryAt: null,
  processingWarningsJson: "[]",
  capturedAt: "2026-08-01T00:00:00Z",
  processedAt: null,
  archivedAt: null,
  createdAt: "2026-08-01T00:00:00Z",
  updatedAt: "2026-08-01T00:00:00Z",
};

const preview = {
  captureCount: 1,
  destinationFileCount: 1,
  networkDestinationFileCount: 0,
  localDerivedFileCount: 1,
  sourceFilesPreserved: 1,
};

beforeEach(() => {
  vi.clearAllMocks();
  api.previewCaptureDeletion.mockResolvedValue(preview);
  api.deleteCaptureItem.mockResolvedValue({
    completed: true,
    recordsDeleted: 1,
    deletedCaptureItemIds: ["capture-1"],
    destinationFilesRecycled: 1,
    destinationFilesPermanentlyDeleted: 0,
    destinationFilesAlreadyMissing: 0,
    failures: [],
  });
});

function mountDialog() {
  return mount(CaptureDeleteDialog, {
    props: { captureItem: item },
    attachTo: document.body,
  });
}

describe("CaptureDeleteDialog", () => {
  it("loads the preview and defaults to deleting destination files", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    expect(api.previewCaptureDeletion).toHaveBeenCalledWith("capture-1");
    const dialog = document.body.querySelector('[role="dialog"]');
    expect(dialog!.textContent).toContain("1 个目标文件");
    expect(dialog!.textContent).toContain("原图保持原位置");
    const checkbox = dialog!.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(checkbox.checked).toBe(true);
    wrapper.unmount();
  });

  it("deletes with destination files by default and emits deleted on success", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const buttons = document.body.querySelectorAll("button");
    const confirm = Array.from(buttons).find((button) => button.textContent?.includes("确认移除记录"))!;
    confirm.click();
    await flushPromises();

    expect(api.deleteCaptureItem).toHaveBeenCalledWith({
      captureItemId: "capture-1",
      deleteDestinationFiles: true,
      allowPermanentNetworkDelete: false,
    });
    expect(wrapper.emitted("deleted")).toHaveLength(1);
    wrapper.unmount();
  });

  it("passes false when the user unchecks the destination-file option", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const checkbox = document.body.querySelector('input[type="checkbox"]') as HTMLInputElement;
    checkbox.click();
    await flushPromises();

    const confirm = Array.from(document.body.querySelectorAll("button"))
      .find((button) => button.textContent?.includes("确认移除记录"))!;
    confirm.click();
    await flushPromises();

    expect(api.deleteCaptureItem).toHaveBeenCalledWith({
      captureItemId: "capture-1",
      deleteDestinationFiles: false,
      allowPermanentNetworkDelete: false,
    });
    wrapper.unmount();
  });

  it("keeps the dialog open and shows failures on a partial delete", async () => {
    api.deleteCaptureItem.mockResolvedValue({
      completed: false,
      recordsDeleted: 0,
      deletedCaptureItemIds: [],
      destinationFilesRecycled: 0,
      destinationFilesPermanentlyDeleted: 0,
      destinationFilesAlreadyMissing: 0,
      failures: [{ path: "D:\\archive\\one.png", error: "recycle denied" }],
    });
    const wrapper = mountDialog();
    await flushPromises();

    const confirm = Array.from(document.body.querySelectorAll("button"))
      .find((button) => button.textContent?.includes("确认移除记录"))!;
    confirm.click();
    await flushPromises();

    expect(wrapper.emitted("deleted")).toBeUndefined();
    expect(document.body.querySelector('[role="dialog"]')).not.toBeNull();
    expect(document.body.textContent).toContain("未能删除");
    expect(document.body.textContent).toContain("recycle denied");
    wrapper.unmount();
  });

  it("requires a second confirmation before permanently deleting network targets", async () => {
    api.previewCaptureDeletion.mockResolvedValue({
      ...preview,
      networkDestinationFileCount: 1,
    });
    const wrapper = mountDialog();
    await flushPromises();

    const first = Array.from(document.body.querySelectorAll("button"))
      .find((button) => button.textContent?.includes("确认移除记录"))!;
    first.click();
    await flushPromises();

    expect(api.deleteCaptureItem).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("NAS/网络文件无法进入 Windows 回收站");

    const second = Array.from(document.body.querySelectorAll("button"))
      .find((button) => button.textContent?.includes("永久删除网络目标并移除"))!;
    second.click();
    await flushPromises();

    expect(api.deleteCaptureItem).toHaveBeenCalledWith({
      captureItemId: "capture-1",
      deleteDestinationFiles: true,
      allowPermanentNetworkDelete: true,
    });
    wrapper.unmount();
  });
});
