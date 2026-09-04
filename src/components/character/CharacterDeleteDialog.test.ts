import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  previewCharacterDeletion: vi.fn(),
  deleteCharacter: vi.fn(),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
}));

import CharacterDeleteDialog from "./CharacterDeleteDialog.vue";

const source = {
  id: "character-1",
  name: "Ava",
  aliasesJson: "[]",
  avatarAssetId: null,
  avatarCaptureItemId: null,
  captureCount: 2,
  pendingReviewCount: 1,
  sampleCount: 1,
  degradedCount: 0,
  lastCapturedAt: null,
  latestCaptureItemId: "item-1",
  latestAvatarPath: null,
};

const preview = {
  captureCount: 2,
  destinationFileCount: 1,
  networkDestinationFileCount: 0,
  localDerivedFileCount: 1,
  sourceFilesPreserved: 2,
};

beforeEach(() => {
  vi.clearAllMocks();
  api.previewCharacterDeletion.mockResolvedValue(preview);
  api.deleteCharacter.mockResolvedValue({
    completed: true,
    recordsDeleted: 2,
    deletedCaptureItemIds: ["item-1", "item-2"],
    destinationFilesRecycled: 1,
    destinationFilesPermanentlyDeleted: 0,
    destinationFilesAlreadyMissing: 0,
    failures: [],
  });
});

function mountDialog() {
  return mount(CharacterDeleteDialog, {
    props: { source },
    attachTo: document.body,
  });
}

describe("CharacterDeleteDialog", () => {
  it("loads the cascade preview and defaults to deleting destination files", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    expect(api.previewCharacterDeletion).toHaveBeenCalledWith({
      characterId: "character-1",
      deleteDestinationFiles: false,
    });
    const dialog = document.body.querySelector('[role="dialog"]');
    expect(dialog!.textContent).toContain("2 条该角色的截图记录");
    const checkbox = dialog!.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(checkbox.checked).toBe(true);
    wrapper.unmount();
  });

  it("cascades deletion with destination files and emits deleted", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const confirm = Array.from(document.body.querySelectorAll("button"))
      .find((button) => button.textContent?.includes("确认删除角色"))!;
    confirm.click();
    await flushPromises();

    expect(api.deleteCharacter).toHaveBeenCalledWith({
      characterId: "character-1",
      deleteDestinationFiles: true,
      allowPermanentNetworkDelete: false,
    });
    expect(wrapper.emitted("deleted")).toHaveLength(1);
    wrapper.unmount();
  });

  it("keeps the dialog open and lists failures on a partial delete", async () => {
    api.deleteCharacter.mockResolvedValue({
      completed: false,
      recordsDeleted: 0,
      deletedCaptureItemIds: [],
      destinationFilesRecycled: 1,
      destinationFilesPermanentlyDeleted: 0,
      destinationFilesAlreadyMissing: 0,
      failures: [{ path: "D:\\archive\\two.png", error: "access denied" }],
    });
    const wrapper = mountDialog();
    await flushPromises();

    const confirm = Array.from(document.body.querySelectorAll("button"))
      .find((button) => button.textContent?.includes("确认删除角色"))!;
    confirm.click();
    await flushPromises();

    expect(wrapper.emitted("deleted")).toBeUndefined();
    expect(document.body.querySelector('[role="dialog"]')).not.toBeNull();
    expect(document.body.textContent).toContain("未能删除");
    expect(document.body.textContent).toContain("access denied");
    wrapper.unmount();
  });

  it("requires a second confirmation before permanently deleting network targets", async () => {
    api.previewCharacterDeletion.mockResolvedValue({
      ...preview,
      networkDestinationFileCount: 1,
    });
    const wrapper = mountDialog();
    await flushPromises();

    const first = Array.from(document.body.querySelectorAll("button"))
      .find((button) => button.textContent?.includes("确认删除角色"))!;
    first.click();
    await flushPromises();
    expect(api.deleteCharacter).not.toHaveBeenCalled();

    const second = Array.from(document.body.querySelectorAll("button"))
      .find((button) => button.textContent?.includes("永久删除网络目标并删除角色"))!;
    second.click();
    await flushPromises();

    expect(api.deleteCharacter).toHaveBeenCalledWith({
      characterId: "character-1",
      deleteDestinationFiles: true,
      allowPermanentNetworkDelete: true,
    });
    wrapper.unmount();
  });
});
