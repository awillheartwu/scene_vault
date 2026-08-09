import { flushPromises, mount } from "@vue/test-utils";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const { api, push } = vi.hoisted(() => ({
  api: {
    listProjectOverviews: vi.fn(),
    runtimeStatus: vi.fn(),
    readThumbnail: vi.fn(),
    readImage: vi.fn(),
    renameProject: vi.fn(),
    previewProjectDelete: vi.fn(),
    deleteProject: vi.fn(),
  },
  push: vi.fn(),
}));

vi.mock("vue-router", () => ({
  useRouter: () => ({ push }),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  pathMimeType: () => "image/jpeg",
}));

import Home from "./Home.vue";

const projects = [
  {
    projectId: "project-bad",
    name: "BAD",
    description: "Main capture project",
    createdAt: "2026-08-05T08:00:00Z",
    sourceCount: 2,
    destinationConfigured: true,
    sessionCount: 3,
    activeSessionCount: 1,
    captureCount: 12,
    awaitingCount: 3,
    processingCount: 1,
    completedCount: 8,
    failedCount: 0,
    lastActivityAt: "2026-08-08T04:36:00Z",
    latestCaptureItemId: "capture-bad",
  },
  {
    projectId: "project-summer",
    name: "Summer Heat",
    description: null,
    createdAt: "2026-08-06T08:00:00Z",
    sourceCount: 1,
    destinationConfigured: true,
    sessionCount: 1,
    activeSessionCount: 0,
    captureCount: 9,
    awaitingCount: 0,
    processingCount: 0,
    completedCount: 9,
    failedCount: 0,
    lastActivityAt: "2026-08-07T23:10:00Z",
    latestCaptureItemId: "capture-summer",
  },
  {
    projectId: "project-eternum",
    name: "Eternum",
    description: null,
    createdAt: "2026-08-07T08:00:00Z",
    sourceCount: 0,
    destinationConfigured: false,
    sessionCount: 0,
    activeSessionCount: 0,
    captureCount: 0,
    awaitingCount: 0,
    processingCount: 0,
    completedCount: 0,
    failedCount: 0,
    lastActivityAt: "2026-08-06T12:00:00Z",
    latestCaptureItemId: null,
  },
];

beforeAll(() => {
  Object.defineProperty(URL, "createObjectURL", { value: vi.fn(() => "blob:thumbnail") });
  Object.defineProperty(URL, "revokeObjectURL", { value: vi.fn() });
});

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  api.listProjectOverviews.mockResolvedValue(projects);
  api.runtimeStatus.mockResolvedValue({
    engineStatus: "configured",
    workerStatus: "idle",
    activeCaptureItemId: null,
    queuedCount: 1,
    archivePendingCount: 0,
    lastError: null,
  });
  api.readThumbnail.mockResolvedValue(new ArrayBuffer(4));
  api.readImage.mockResolvedValue(new ArrayBuffer(4));
  api.renameProject.mockResolvedValue({ ...projects[0], name: "Renamed" });
  api.previewProjectDelete.mockResolvedValue({
    captureCount: 12,
    characterCount: 2,
    sessionCount: 3,
    hasActiveSession: true,
    noteCount: 1,
    collectionCount: 0,
    orphanAssetCount: 1,
  });
  api.deleteProject.mockResolvedValue({
    captureCount: 12,
    characterCount: 2,
    sessionCount: 3,
    hasActiveSession: true,
    noteCount: 1,
    collectionCount: 0,
    orphanAssetCount: 1,
  });
});

describe("Home project library", () => {
  it("shows every project in the default grid instead of selecting one project", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    expect(wrapper.find(".project-grid").exists()).toBe(true);
    expect(wrapper.text()).toContain("BAD");
    expect(wrapper.text()).toContain("Summer Heat");
    expect(wrapper.text()).toContain("Eternum");
    expect(wrapper.find('[aria-label="选择项目"]').exists()).toBe(false);
    expect(wrapper.text()).toContain("待分类 3");
    expect(wrapper.text()).toContain("全部已归档");

    wrapper.unmount();
  });

  it("switches to the compact list and remembers the view", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get('[aria-label="列表视图"]').trigger("click");

    expect(wrapper.find(".project-list").exists()).toBe(true);
    expect(wrapper.get('[aria-label="列表视图"]').attributes("aria-pressed")).toBe("true");
    expect(localStorage.getItem("scene-vault.home.project-view")).toBe("list");

    wrapper.unmount();
  });

  it("filters projects without losing the library context", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get('input[type="search"]').setValue("summer");

    expect(wrapper.text()).toContain("Summer Heat");
    expect(wrapper.text()).not.toContain("Eternum");
    expect(wrapper.findAll(".project-card")).toHaveLength(1);

    wrapper.unmount();
  });

  it("opens the selected project in the workbench and capture flow", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.findAll(".enter-project-button")[0].trigger("click");
    expect(localStorage.getItem("scene-vault.capture.project")).toBe("project-bad");
    expect(push).toHaveBeenCalledWith("/workbench");

    await wrapper.findAll(".capture-project-button")[1].trigger("click");
    expect(localStorage.getItem("scene-vault.capture.project")).toBe("project-summer");
    expect(push).toHaveBeenCalledWith("/capture");

    wrapper.unmount();
  });

  it("opens the capture page with the create-project form requested", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get(".new-project-button").trigger("click");

    expect(localStorage.getItem("scene-vault.capture.create-project")).toBe("1");
    expect(push).toHaveBeenCalledWith("/capture");

    wrapper.unmount();
  });

  it("renames a project from the card action", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get('[aria-label="重命名 BAD"]').trigger("click");
    await flushPromises();

    const dialog = document.body.querySelector('[role="dialog"][aria-label="重命名项目"]');
    expect(dialog).toBeTruthy();
    const input = dialog!.querySelector('input[aria-label="项目名称"]') as HTMLInputElement;
    input.value = "  Renamed  ";
    input.dispatchEvent(new Event("input"));
    (dialog!.querySelector("form") as HTMLFormElement).dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );
    await flushPromises();

    expect(api.renameProject).toHaveBeenCalledWith("project-bad", "Renamed");
    expect(api.listProjectOverviews).toHaveBeenCalledTimes(2);
    expect(document.body.querySelector('[role="dialog"][aria-label="重命名项目"]')).toBeNull();

    wrapper.unmount();
  });

  it("deletes a project after previewing what will be removed", async () => {
    localStorage.setItem("scene-vault.capture.project", "project-bad");
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get('[aria-label="删除 BAD"]').trigger("click");
    await flushPromises();

    expect(api.previewProjectDelete).toHaveBeenCalledWith("project-bad");
    const dialog = document.body.querySelector('[role="dialog"][aria-label="删除项目"]');
    expect(dialog).toBeTruthy();
    expect(dialog!.textContent).toContain("12 张截图记录");
    expect(dialog!.textContent).toContain("正在监听中");

    (dialog!.querySelector(".danger-action") as HTMLButtonElement).click();
    await flushPromises();

    expect(api.deleteProject).toHaveBeenCalledWith("project-bad");
    expect(localStorage.getItem("scene-vault.capture.project")).toBeNull();
    expect(document.body.querySelector('[role="dialog"][aria-label="删除项目"]')).toBeNull();

    wrapper.unmount();
  });

  it("sorts projects by name ascending and flips to descending", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    const nameButton = wrapper
      .findAll(".sort-switch button")
      .find((button) => button.text() === "名称");
    await nameButton!.trigger("click");
    expect(wrapper.findAll(".project-card")[0].text()).toContain("BAD");
    expect(wrapper.findAll(".project-card")[2].text()).toContain("Summer Heat");

    await wrapper.get('[aria-label="当前正序，点击切换为倒序"]').trigger("click");
    expect(wrapper.findAll(".project-card")[0].text()).toContain("Summer Heat");
    expect(wrapper.findAll(".project-card")[2].text()).toContain("BAD");
    expect(localStorage.getItem("scene-vault.home.sort")).toBe("name");
    expect(localStorage.getItem("scene-vault.home.sort-direction")).toBe("desc");

    wrapper.unmount();
  });

  it("sorts projects by capture count and creation time", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    const capturesButton = wrapper
      .findAll(".sort-switch button")
      .find((button) => button.text() === "图片数");
    await capturesButton!.trigger("click");
    expect(wrapper.findAll(".project-card")[0].text()).toContain("BAD");
    await wrapper.get('[aria-label="当前倒序，点击切换为正序"]').trigger("click");
    expect(wrapper.findAll(".project-card")[0].text()).toContain("Eternum");

    const createdButton = wrapper
      .findAll(".sort-switch button")
      .find((button) => button.text() === "创建时间");
    await createdButton!.trigger("click");
    expect(wrapper.findAll(".project-card")[0].text()).toContain("Eternum");
    await wrapper.get('[aria-label="当前倒序，点击切换为正序"]').trigger("click");
    expect(wrapper.findAll(".project-card")[0].text()).toContain("BAD");

    wrapper.unmount();
  });

  it("paginates the project grid and switches page sizes", async () => {
    const many = Array.from({ length: 60 }, (_, index) => ({
      ...projects[0],
      projectId: `project-${index}`,
      name: `Project ${String(index).padStart(2, "0")}`,
      createdAt: "2026-08-01T00:00:00Z",
    }));
    api.listProjectOverviews.mockResolvedValue(many);
    const wrapper = mount(Home);
    await flushPromises();

    expect(wrapper.findAll(".project-card")).toHaveLength(50);
    expect(wrapper.text()).toContain("共 60 条");

    await wrapper.get('[aria-label="下一页"]').trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".project-card")).toHaveLength(10);

    const sizeSelect = wrapper.get(".pagination-size select");
    await sizeSelect.setValue("100");
    await flushPromises();
    expect(wrapper.findAll(".project-card")).toHaveLength(60);

    wrapper.unmount();
  });
});
