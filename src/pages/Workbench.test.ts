import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useToasts } from "@/lib/toast";

const { api, eventHandlers } = vi.hoisted(() => ({
  api: {
    listProjects: vi.fn(),
    listProjectCharacterSummaries: vi.fn(),
    listCharacters: vi.fn(),
    listCharacterCaptureItems: vi.fn(),
    listCharacterFaceSamples: vi.fn(),
    setFaceSampleStatus: vi.fn(),
    setFaceSampleFlagged: vi.fn(),
    retryDegradedCaptures: vi.fn(),
    getFaceBankModelStatus: vi.fn(),
    listCategoryItems: vi.fn(),
    getAppSettings: vi.fn(),
    acceptRecognitionSuggestion: vi.fn(),
    reviewRecognitionSuggestion: vi.fn(),
    rejectSuggestionAndEnroll: vi.fn(),
    batchRejectAndEnroll: vi.fn(),
    relabel: vi.fn(),
    renameCharacter: vi.fn(),
    mergeCharacters: vi.fn(),
    setCharacterAvatar: vi.fn(),
    retry: vi.fn(),
    refreshCaptureFaceFeature: vi.fn(),
    setProjectCover: vi.fn(),
    revealPath: vi.fn(),
    readImage: vi.fn(),
    readThumbnail: vi.fn(),
  },
  eventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (name: string, handler: (event: { payload: unknown }) => void) => {
      eventHandlers.set(name, handler);
      return () => eventHandlers.delete(name);
    },
  ),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  captureStatusLabel: (status: string) => status,
  pathFileName: (path: string) => path.split(/[\\/]/).pop() || path,
  pathMimeType: () => "image/png",
  revealPath: api.revealPath,
}));

import Workbench from "./Workbench.vue";

const project = {
  id: "project-1",
  name: "Love & Jealousy",
  description: null,
  coverAssetId: null,
  coverCaptureItemId: null,
  lastSourceDirectory: null,
  lastDestinationDirectory: null,
  destinationDirectory: null,
  createdAt: "2026-08-01T00:00:00Z",
  updatedAt: "2026-08-01T00:00:00Z",
};
const summaries = [
  {
    id: "character-1",
    name: "Ava",
    aliasesJson: "[]",
    avatarAssetId: null,
    avatarCaptureItemId: null,
    captureCount: 2,
    pendingReviewCount: 1,
    sampleCount: 1,
    degradedCount: 0,
    lastCapturedAt: "2026-08-06T00:00:00Z",
    latestCaptureItemId: "item-1",
    latestAvatarPath: "avatar.png",
  },
  {
    id: "character-2",
    name: "Bella",
    aliasesJson: "[]",
    avatarAssetId: null,
    avatarCaptureItemId: null,
    captureCount: 0,
    pendingReviewCount: 0,
    sampleCount: 0,
    degradedCount: 0,
    lastCapturedAt: null,
    latestCaptureItemId: null,
    latestAvatarPath: null,
  },
];
const characters = [
  {
    id: "character-1",
    projectId: "project-1",
    name: "Ava",
    aliasesJson: "[]",
    avatarAssetId: null,
    createdAt: "2026-08-01T00:00:00Z",
    updatedAt: "2026-08-01T00:00:00Z",
  },
  {
    id: "character-2",
    projectId: "project-1",
    name: "Bella",
    aliasesJson: "[]",
    avatarAssetId: null,
    createdAt: "2026-08-01T00:00:00Z",
    updatedAt: "2026-08-01T00:00:00Z",
  },
];
const item = {
  id: "item-1",
  sessionId: "session-1",
  assetId: null,
  characterId: "character-1",
  classification: "person",
  sourcePath: "C:\\shots\\one.png",
  annotatedPath: null,
  avatarPath: null,
  destinationPath: null,
  destinationAvatarPath: null,
  status: "queued",
  faceBoxJson: null,
  suggestedCharacterId: "character-2",
  recognitionConfidence: 0.85,
  recognitionSource: "vision",
  reviewStatus: "pending",
  errorMessage: null,
  failureStage: null,
  attemptCount: 0,
  nextRetryAt: null,
  processingWarningsJson: "[]",
  capturedAt: "2026-08-06T00:00:00Z",
  processedAt: null,
  archivedAt: null,
  createdAt: "2026-08-06T00:00:00Z",
  updatedAt: "2026-08-06T00:00:00Z",
};
const sample = {
  id: "sample-1",
  characterId: "character-1",
  captureItemId: "item-1",
  faceBoxJson: null,
  confidence: 0.85,
  status: "active",
  createdAt: "2026-08-06T00:00:00Z",
  updatedAt: "2026-08-06T00:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  api.listProjects.mockResolvedValue([project]);
  api.listProjectCharacterSummaries.mockResolvedValue(summaries);
  api.listCharacters.mockResolvedValue(characters);
  api.listCharacterCaptureItems.mockResolvedValue([item]);
  api.listCharacterFaceSamples.mockResolvedValue([sample]);
  api.setFaceSampleStatus.mockResolvedValue({ ...sample, status: "revoked" });
  api.setFaceSampleFlagged.mockResolvedValue({ ...sample, flagged: 0 });
  api.retryDegradedCaptures.mockResolvedValue(2);
  api.getFaceBankModelStatus.mockResolvedValue({
    bankModelId: "opencv-sface",
    bankModelVersion: "2021dec",
    activeModelId: "opencv-sface",
    activeModelVersion: "2021dec",
    sampleCount: 1,
    compatible: true,
  });
  api.listCategoryItems.mockResolvedValue([item]);
  api.getAppSettings.mockResolvedValue({
    classifyShortcut: "Ctrl+Shift+S",
    noteShortcut: "Ctrl+Shift+N",
    showPrivateByDefault: false,
    autoSaveNotes: true,
    splitPopupWindows: false,
    autoCloseEmptyPopup: false,
  });
  api.acceptRecognitionSuggestion.mockResolvedValue({
    ...item,
    characterId: "character-2",
    status: "queued",
    suggestedCharacterId: null,
    recognitionConfidence: null,
    recognitionSource: null,
    reviewStatus: "none",
  });
  api.reviewRecognitionSuggestion.mockResolvedValue(item);
  api.rejectSuggestionAndEnroll.mockResolvedValue(item);
  api.batchRejectAndEnroll.mockResolvedValue(1);
  api.renameCharacter.mockResolvedValue({ ...characters[0], name: "Ava 2" });
  api.mergeCharacters.mockResolvedValue(characters[1]);
  api.refreshCaptureFaceFeature.mockResolvedValue(item);
  api.setProjectCover.mockImplementation(async (_projectId, captureItemId) => ({
    ...project,
    coverCaptureItemId: captureItemId,
  }));
  api.setCharacterAvatar.mockResolvedValue({
    ...characters[0],
    avatarAssetId: "asset-1",
  });
  api.readImage.mockResolvedValue(new ArrayBuffer(8));
  api.readThumbnail.mockResolvedValue(new ArrayBuffer(8));
});

afterEach(() => {
  document.body.innerHTML = "";
});

describe("Workbench", () => {
  it("renders the character overview and the first character's captures", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    expect(wrapper.text()).toContain("人物工作台");
    expect(wrapper.text()).toContain("Ava");
    expect(wrapper.text()).toContain("Bella");
    expect(api.listProjectCharacterSummaries).toHaveBeenCalledWith("project-1");
    expect(api.listCharacterCaptureItems).toHaveBeenCalledWith({
      projectId: "project-1",
      characterId: "character-1",
    });
    // The pending suggestion badge shows on Ava's card and the item grid
    // renders the queued capture with its suggestion chip.
    expect(wrapper.find(".pending-badge").text()).toBe("1");
    expect(wrapper.text()).toContain("queued");
    expect(wrapper.text()).toContain("建议待确认");
  });

  it("accepts a pending recognition suggestion from the detail panel", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    const accept = wrapper
      .findAll("button")
      .find((button) => button.text().includes("确认建议"));
    expect(accept).toBeDefined();
    await accept!.trigger("click");
    await flushPromises();

    expect(api.acceptRecognitionSuggestion).toHaveBeenCalledWith("item-1");
    expect(api.reviewRecognitionSuggestion).not.toHaveBeenCalled();
    expect(
      useToasts().toasts.some((toast) =>
        toast.message.includes("完成角色改判，并重新排队"),
      ),
    ).toBe(true);
  });

  it("exposes the active workbench category through aria-selected", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    const tabs = wrapper.findAll('[role="tab"]');
    expect(tabs.find((tab) => tab.text() === "人物")?.attributes("aria-selected")).toBe(
      "true",
    );
    const sceneTab = tabs.find((tab) => tab.text() === "游戏截图");
    await sceneTab!.trigger("click");
    await flushPromises();

    expect(sceneTab!.attributes("aria-selected")).toBe("true");
    expect(
      wrapper
        .findAll('[role="tab"]')
        .find((tab) => tab.text() === "人物")
        ?.attributes("aria-selected"),
    ).toBe("false");
  });

  it("switches the preview variant to the destination archive", async () => {
    const archived = {
      ...item,
      destinationPath: "C:\\archive\\one.png",
      destinationAvatarPath: null,
      reviewStatus: "none",
      suggestedCharacterId: null,
      recognitionConfidence: null,
      recognitionSource: null,
    };
    api.listCharacterCaptureItems.mockResolvedValue([archived]);
    const wrapper = mount(Workbench);
    await flushPromises();

    const archiveTab = wrapper
      .findAll(".variant-tabs button")
      .find((button) => button.text().includes("归档图"));
    expect(archiveTab).toBeDefined();
    await archiveTab!.trigger("click");
    expect(api.readImage).toHaveBeenCalledWith("item-1", "destination");
  });

  it("renames the selected character with the archive-name hint", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    const renameButton = wrapper
      .findAll("button")
      .find((button) => button.attributes("aria-label") === "重命名角色");
    expect(renameButton).toBeDefined();
    await renameButton!.trigger("click");

    await flushPromises();
    const renameDialog = document.body.querySelector('[role="dialog"][aria-label="重命名角色"]');
    expect(renameDialog?.textContent).toContain("已归档文件与历史记录保持原名");
    const input = renameDialog!.querySelector("input") as HTMLInputElement;
    expect(input.value).toBe("Ava");
    input.value = "Ava 2";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    (renameDialog!.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    await flushPromises();

    expect(api.renameCharacter).toHaveBeenCalledWith({
      characterId: "character-1",
      name: "Ava 2",
    });
    // The overview reloads after a successful rename.
    expect(api.listProjectCharacterSummaries.mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(document.body.querySelector('[role="dialog"][aria-label="重命名角色"]')).toBeNull();
  });

  it("merges the selected character into an explicitly chosen target", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    const mergeButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("合并角色"));
    expect(mergeButton).toBeDefined();
    await mergeButton!.trigger("click");
    await flushPromises();

    const dialog = document.body.querySelector('[role="dialog"]');
    expect(dialog).toBeTruthy();
    expect(dialog!.textContent).toContain("Ava");
    const options = Array.from(dialog!.querySelectorAll("option")).map(
      (option) => option.textContent,
    );
    expect(options.some((option) => option?.includes("Ava ·"))).toBe(false);
    expect(options.some((option) => option?.includes("Bella ·"))).toBe(true);

    const select = dialog!.querySelector("select") as HTMLSelectElement;
    select.value = "character-2";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await flushPromises();
    (dialog!.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    await flushPromises();

    expect(api.mergeCharacters).toHaveBeenCalledWith({
      sourceCharacterId: "character-1",
      targetCharacterId: "character-2",
    });
    expect(document.body.querySelector('[role="dialog"]')).toBeNull();
    expect(
      useToasts().toasts.some((entry) => entry.message.includes("已将「Ava」合并到「Bella」")),
    ).toBe(true);
  });

  it("keeps the merge dialog open and announces backend errors", async () => {
    api.mergeCharacters.mockRejectedValueOnce(new Error("角色正在被其他操作更新"));
    const wrapper = mount(Workbench);
    await flushPromises();

    await wrapper
      .findAll("button")
      .find((button) => button.text().includes("合并角色"))!
      .trigger("click");
    await flushPromises();
    const dialog = document.body.querySelector('[role="dialog"]')!;
    const select = dialog.querySelector("select") as HTMLSelectElement;
    select.value = "character-2";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await flushPromises();
    (dialog.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    await flushPromises();

    const alert = dialog.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("角色正在被其他操作更新");
    expect(document.body.querySelector('[role="dialog"]')).toBeTruthy();
  });

  it("sets the selected archived face crop as the representative avatar", async () => {
    api.listCharacterCaptureItems.mockResolvedValue([
      {
        ...item,
        assetId: "asset-1",
        avatarPath: "C:\\cache\\avatar.png",
        destinationPath: "C:\\archive\\one.png",
        destinationAvatarPath: "C:\\archive\\avatar.png",
        status: "completed",
        reviewStatus: "none",
      },
    ]);
    const wrapper = mount(Workbench);
    await flushPromises();

    const setAvatar = wrapper
      .findAll("button")
      .find((button) => button.text().includes("设为代表头像"));
    expect(setAvatar).toBeDefined();
    expect(setAvatar!.attributes("disabled")).toBeUndefined();
    await setAvatar!.trigger("click");
    await flushPromises();

    expect(api.setCharacterAvatar).toHaveBeenCalledWith({
      characterId: "character-1",
      avatarAssetId: "asset-1",
    });
    expect(
      useToasts().toasts.some((entry) => entry.message.includes("设为「Ava」的代表头像")),
    ).toBe(true);
  });

  it("keeps the sample summary in one row and hides avatar restore behind the more menu", async () => {
    api.listProjectCharacterSummaries.mockResolvedValue([
      { ...summaries[0], avatarAssetId: "asset-1", avatarCaptureItemId: "item-1" },
      summaries[1],
    ]);
    const wrapper = mount(Workbench);
    await flushPromises();

    const bar = wrapper.find(".sample-summary-bar");
    expect(bar.exists()).toBe(true);
    expect(bar.find(".sample-summary-copy").exists()).toBe(true);
    expect(bar.find(".sample-summary-actions").exists()).toBe(true);
    const directRestore = bar
      .findAll("button")
      .find((button) => button.text().includes("恢复自动头像"));
    expect(directRestore).toBeUndefined();
    expect(wrapper.find('button[aria-label="更多头像操作"]').exists()).toBe(true);
  });

  it("disables avatar assignment for an unarchived capture and can clear an existing avatar", async () => {
    api.listProjectCharacterSummaries.mockResolvedValue([
      { ...summaries[0], avatarAssetId: "asset-1", avatarCaptureItemId: "item-1" },
      summaries[1],
    ]);
    api.setCharacterAvatar.mockResolvedValue({
      ...characters[0],
      avatarAssetId: null,
    });
    // reka-ui menus do not open in jsdom, so the dropdown shell is stubbed
    // inline and the menu item is asserted to call the same handler the
    // production MoreHorizontal menu invokes.
    const wrapper = mount(Workbench, {
      global: {
        stubs: {
          DropdownMenu: { template: "<div><slot /></div>" },
          DropdownMenuTrigger: { template: "<div><slot /></div>" },
          DropdownMenuContent: { template: "<div><slot /></div>" },
          DropdownMenuItem: {
            emits: ["select"],
            template:
              '<button type="button" @click="$emit(\'select\')"><slot /></button>',
          },
        },
      },
    });
    await flushPromises();

    const setAvatar = wrapper
      .findAll("button")
      .find((button) => button.text().includes("设为代表头像"));
    expect(setAvatar?.attributes("disabled")).toBeDefined();

    const clearAvatar = wrapper
      .findAll("button")
      .find((button) => button.text().includes("恢复自动头像"));
    expect(clearAvatar).toBeDefined();
    await clearAvatar!.trigger("click");
    await flushPromises();

    expect(api.setCharacterAvatar).toHaveBeenCalledWith({
      characterId: "character-1",
      avatarAssetId: null,
    });
  });

  it("renders the explicit representative avatar instead of the newest fallback", async () => {
    api.listProjectCharacterSummaries.mockResolvedValue([
      { ...summaries[0], avatarAssetId: "asset-2", avatarCaptureItemId: "item-avatar" },
      summaries[1],
    ]);
    mount(Workbench);
    await flushPromises();

    expect(api.readThumbnail).toHaveBeenCalledWith("item-avatar", "avatar");
  });

  it("shows only the face-bank count and keeps sample thumbnails collapsed permanently", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    expect(wrapper.text()).toContain("人脸样本库");
    expect(wrapper.text()).toContain("1 条样本");
    expect(api.listCharacterFaceSamples).toHaveBeenCalledWith("character-1");
    expect(wrapper.find(".sample-list").exists()).toBe(false);
    expect(wrapper.find(".sample-strip-toggle").exists()).toBe(false);
  });

  it("lays out the grid header as a title row plus grouped action row", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    const titleRow = wrapper.find(".workbench-grid-header-title");
    expect(titleRow.text()).toContain("Ava");
    expect(titleRow.text()).toContain("1 张截图");
    const groups = wrapper.findAll(".workbench-action-group");
    expect(groups).toHaveLength(3);
    expect(groups[0].text()).toContain("合并角色");
    expect(groups[1].text()).toContain("批量拒绝并登记 (1)");
    expect(groups[2].text()).toContain("当前角色重新识别 (0)");
    expect(groups[2].text()).toContain("全部重新识别 (0)");
    const actions = wrapper.findAll(".workbench-grid-actions button");
    expect(actions).toHaveLength(4);
    const current = actions.find((button) => button.text().includes("当前角色重新识别 (0)"));
    const all = actions.find((button) => button.text().includes("全部重新识别 (0)"));
    expect(current?.attributes("disabled")).toBeDefined();
    expect(all?.attributes("disabled")).toBeDefined();
  });

  it("shows all conditional character actions so their crowded state can be reviewed", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    const actions = wrapper.findAll(".workbench-grid-actions button");
    expect(actions.some((button) => button.text().includes("批量拒绝并登记 (1)"))).toBe(true);
    const current = actions.find((button) => button.text().includes("当前角色重新识别 (0)"));
    const all = actions.find((button) => button.text().includes("全部重新识别 (0)"));
    expect(current?.attributes("disabled")).toBeDefined();
    expect(all?.attributes("disabled")).toBeDefined();
  });

  it("explains a detected but low-quality face in Chinese and marks the card", async () => {
    api.listCharacterCaptureItems.mockResolvedValue([{
      ...item,
      status: "completed",
      faceCount: 1,
      suggestedCharacterId: null,
      recognitionConfidence: null,
      recognitionSource: null,
      reviewStatus: "none",
      processingWarningsJson: '["sample_not_enrolled_low_quality: sharpness 2.2 below 3"]',
    }]);
    api.listCharacterFaceSamples.mockResolvedValue([]);
    const wrapper = mount(Workbench);
    await flushPromises();

    expect(wrapper.find(".face-status-chip").text()).toBe("清晰度不足");
    expect(wrapper.text()).toContain("清晰度 2.2 低于样本门槛 3");
  });

  it("marks and explains captures where no usable face was detected", async () => {
    api.listCharacterCaptureItems.mockResolvedValue([{
      ...item,
      status: "completed",
      faceCount: 0,
      suggestedCharacterId: null,
      recognitionConfidence: null,
      recognitionSource: null,
      reviewStatus: "none",
    }]);
    api.listCharacterFaceSamples.mockResolvedValue([]);
    const wrapper = mount(Workbench);
    await flushPromises();

    expect(wrapper.find(".face-status-chip").text()).toBe("未检测到脸");
    expect(wrapper.text()).toContain("未在这张图片中检测到可用人脸");
  });

  it("refreshes items when a capture changes while the worker runs", async () => {
    mount(Workbench);
    await flushPromises();
    const callsBefore = api.listCharacterCaptureItems.mock.calls.length;

    const handler = eventHandlers.get("capture:item-updated");
    expect(handler).toBeDefined();
    handler!({ payload: { id: "item-1" } });
    await flushPromises();

    expect(api.listCharacterCaptureItems.mock.calls.length).toBeGreaterThan(
      callsBefore,
    );
  });

  it("batch re-recognizes all degraded captures", async () => {
    api.listProjectCharacterSummaries.mockResolvedValue([
      { ...summaries[0], degradedCount: 2 },
      summaries[1],
    ]);
    const wrapper = mount(Workbench);
    await flushPromises();

    const allButton = wrapper
      .findAll("button")
      .find((button) => button.text().includes("全部重新识别"));
    expect(allButton).toBeDefined();
    await allButton!.trigger("click");
    await flushPromises();

    expect(api.retryDegradedCaptures).toHaveBeenCalledWith("project-1", null);
    expect(useToasts().toasts.some((t) => t.message.includes("已重新排队 2 张"))).toBe(
      true,
    );
  });

  it("re-extracts the selected capture's face feature from the correction card", async () => {
    api.listCharacterCaptureItems.mockResolvedValue([
      {
        ...item,
        status: "completed",
        suggestedCharacterId: null,
        recognitionConfidence: null,
        recognitionSource: null,
        reviewStatus: "none",
      },
    ]);
    api.listCharacterFaceSamples.mockResolvedValue([]);
    const wrapper = mount(Workbench);
    await flushPromises();

    const refresh = wrapper
      .findAll("button")
      .find((button) => button.text().includes("重新提取人脸特征"));
    expect(refresh).toBeDefined();
    expect(refresh!.attributes("disabled")).toBeUndefined();
    await refresh!.trigger("click");
    await flushPromises();

    expect(api.refreshCaptureFaceFeature).toHaveBeenCalledWith("item-1");
  });

  it("disables single-image feature refresh while a capture is queued", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    const refresh = wrapper
      .findAll("button")
      .find((button) => button.text().includes("重新提取人脸特征"));
    expect(refresh?.attributes("disabled")).toBeDefined();
  });

  it("restores a flagged sample to matching from the correction card", async () => {
    api.listCharacterFaceSamples.mockResolvedValue([{ ...sample, flagged: 1 }]);
    api.setFaceSampleFlagged.mockResolvedValue({ ...sample, flagged: 0 });
    const wrapper = mount(Workbench);
    await flushPromises();

    const restore = wrapper
      .findAll("button")
      .find((button) => button.text().includes("恢复参与匹配"));
    expect(restore).toBeDefined();
    expect(restore!.attributes("disabled")).toBeUndefined();
    await restore!.trigger("click");
    await flushPromises();

    expect(api.setFaceSampleFlagged).toHaveBeenCalledWith("sample-1", false);
    wrapper.unmount();
  });

  it("marks an active sample as suspicious from the correction card", async () => {
    api.setFaceSampleFlagged.mockResolvedValue({ ...sample, flagged: 1 });
    const wrapper = mount(Workbench);
    await flushPromises();

    const mark = wrapper
      .findAll("button")
      .find((button) => button.text().includes("标记可疑"));
    expect(mark).toBeDefined();
    await mark!.trigger("click");
    await flushPromises();

    expect(api.setFaceSampleFlagged).toHaveBeenCalledWith("sample-1", true);
    wrapper.unmount();
  });

  it("sets the project cover from the correction card and can cancel it", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    const setCover = wrapper
      .findAll("button")
      .find((button) => button.text().includes("设为项目封面"));
    expect(setCover).toBeDefined();
    expect(setCover!.attributes("disabled")).toBeUndefined();
    await setCover!.trigger("click");
    await flushPromises();

    expect(api.setProjectCover).toHaveBeenCalledWith("project-1", "item-1");
    const cancel = wrapper
      .findAll("button")
      .find((button) => button.text().includes("取消项目封面"));
    expect(cancel).toBeDefined();
    await cancel!.trigger("click");
    await flushPromises();

    expect(api.setProjectCover).toHaveBeenCalledWith("project-1", null);
    wrapper.unmount();
  });

  it("keeps the project-cover action available for unclassified and scene categories", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    for (const tab of ["未分类", "游戏截图"]) {
      const tabButton = wrapper
        .findAll(".workbench-tabs button")
        .find((button) => button.text().includes(tab));
      expect(tabButton, `tab ${tab}`).toBeDefined();
      await tabButton!.trigger("click");
      await flushPromises();

      const setCover = wrapper
        .findAll("button")
        .find((button) => button.text().includes("设为项目封面"));
      expect(setCover, `cover action on ${tab}`).toBeDefined();
    }
    wrapper.unmount();
  });

  it("reloads the view when the face bank finishes rebuilding", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();
    const callsBefore = api.listProjectCharacterSummaries.mock.calls.length;

    eventHandlers.get("capture:face-bank-rebuilt")?.({ payload: {} });
    await flushPromises();

    expect(api.listProjectCharacterSummaries.mock.calls.length).toBeGreaterThan(callsBefore);
    wrapper.unmount();
  });

  it("switches to the unclassified tab and loads category items", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    const unclassifiedTab = wrapper
      .findAll(".workbench-tabs button")
      .find((button) => button.text().includes("未分类"));
    expect(unclassifiedTab).toBeDefined();
    await unclassifiedTab!.trigger("click");
    await flushPromises();

    expect(api.listCategoryItems).toHaveBeenCalledWith({
      projectId: "project-1",
      category: "unclassified",
    });
    expect(wrapper.text()).toContain("等待分类的截图会出现在这里");
  });

  it("shows the private tab only when enabled by settings", async () => {
    api.getAppSettings.mockResolvedValue({
      classifyShortcut: "Ctrl+Shift+S",
      noteShortcut: "Ctrl+Shift+N",
      showPrivateByDefault: true,
      autoSaveNotes: true,
      splitPopupWindows: false,
      autoCloseEmptyPopup: false,
    });
    const wrapper = mount(Workbench);
    await flushPromises();

    const tabs = wrapper
      .findAll(".workbench-tabs button")
      .map((button) => button.text());
    expect(tabs).toContain("收藏图");
  });
});

describe("Workbench context menus", () => {
  function menuItems() {
    return Array.from(document.body.querySelectorAll('[role="menuitem"]')) as HTMLElement[];
  }

  it("opens the rename dialog from a character card", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    await wrapper.find(".character-card").trigger("contextmenu", { clientX: 80, clientY: 50 });
    await flushPromises();
    const menu = document.body.querySelector('[role="menu"]');
    expect(menu!.textContent).toContain("重命名…");
    expect(menu!.textContent).toContain("合并到其他角色…");
    expect(menu!.textContent).toContain("批量拒绝并登记");
    expect(menu!.textContent).toContain("当前角色重新识别");

    menuItems()
      .find((entry) => entry.textContent?.includes("重命名"))!
      .click();
    await flushPromises();
    expect(document.body.querySelector('[role="dialog"][aria-label="重命名角色"]')).not.toBeNull();
    wrapper.unmount();
  });

  it("opens the merge dialog from a character card", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    await wrapper.find(".character-card").trigger("contextmenu", { clientX: 80, clientY: 50 });
    await flushPromises();
    menuItems()
      .find((entry) => entry.textContent?.includes("合并到其他角色"))!
      .click();
    await flushPromises();
    expect(document.body.querySelector('[role="dialog"]')?.textContent).toContain("合并角色");
    wrapper.unmount();
  });

  it("batch-rejects pending suggestions from a character card", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    await wrapper.find(".character-card").trigger("contextmenu", { clientX: 80, clientY: 50 });
    await flushPromises();
    menuItems()
      .find((entry) => entry.textContent?.includes("批量拒绝并登记"))!
      .click();
    await flushPromises();
    expect(api.batchRejectAndEnroll).toHaveBeenCalledWith("project-1", "character-1");
    wrapper.unmount();
  });

  it("accepts a pending suggestion and reveals the source image from an item cell", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    await wrapper.find(".workbench-cell").trigger("contextmenu", { clientX: 90, clientY: 60 });
    await flushPromises();
    const menu = document.body.querySelector('[role="menu"]');
    expect(menu!.textContent).toContain("查看详情");
    expect(menu!.textContent).toContain("确认建议");
    expect(menu!.textContent).toContain("拒绝建议");
    expect(menu!.textContent).toContain("拒绝并登记");
    expect(menu!.textContent).toContain("设为项目封面");

    menuItems()
      .find((entry) => entry.textContent?.includes("确认建议"))!
      .click();
    await flushPromises();
    expect(api.acceptRecognitionSuggestion).toHaveBeenCalledWith("item-1");

    await wrapper.find(".workbench-cell").trigger("contextmenu", { clientX: 90, clientY: 60 });
    await flushPromises();
    menuItems()
      .find((entry) => entry.textContent?.includes("显示原图"))!
      .click();
    await flushPromises();
    expect(api.revealPath).toHaveBeenCalledWith("C:\\shots\\one.png");
    wrapper.unmount();
  });

  it("sets the project cover from an item cell", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();

    await wrapper.find(".workbench-cell").trigger("contextmenu", { clientX: 90, clientY: 60 });
    await flushPromises();
    menuItems()
      .find((entry) => entry.textContent?.includes("设为项目封面"))!
      .click();
    await flushPromises();
    expect(api.setProjectCover).toHaveBeenCalledWith("project-1", "item-1");
    wrapper.unmount();
  });

  it("hides suggestion actions in the scene view", async () => {
    const wrapper = mount(Workbench);
    await flushPromises();
    const sceneTab = wrapper
      .findAll('[role="tab"]')
      .find((tab) => tab.text() === "游戏截图");
    await sceneTab!.trigger("click");
    await flushPromises();

    await wrapper.find(".workbench-cell").trigger("contextmenu", { clientX: 90, clientY: 60 });
    await flushPromises();
    const menu = document.body.querySelector('[role="menu"]');
    expect(menu!.textContent).toContain("显示原图");
    expect(menu!.textContent).not.toContain("确认建议");
    expect(menu!.textContent).not.toContain("设为代表头像");
    wrapper.unmount();
  });
});
