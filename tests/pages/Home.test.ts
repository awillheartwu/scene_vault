import { enableAutoUnmount, flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectArchiveUsage, ProjectOverviewSummary } from "@/lib/capture-api";

const { api, push, launchGameMock } = vi.hoisted(() => ({
  api: {
    listProjectOverviews: vi.fn<() => Promise<ProjectOverviewSummary[]>>(),
    listProjectArchiveUsage: vi.fn<(
      requestId: string,
      onProgress: (usage: ProjectArchiveUsage) => void,
    ) => Promise<ProjectArchiveUsage[]>>(),
    cancelProjectArchiveUsage: vi.fn<(requestId: string) => Promise<void>>(),
    runtimeStatus: vi.fn(),
    readThumbnail: vi.fn(),
    readImage: vi.fn(),
    recordClientEvent: vi.fn(async () => {}),
    renameProject: vi.fn(),
    previewProjectDelete: vi.fn(),
    deleteProject: vi.fn(),
  },
  push: vi.fn(),
  launchGameMock: vi.fn(),
}));

vi.mock("vue-router", () => ({
  useRouter: () => ({ push }),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  engineApi: { launchGame: launchGameMock },
  pathMimeType: () => "image/jpeg",
}));

import Home from "@/pages/Home.vue";
import ProjectArchiveSummary from "@/components/project/ProjectArchiveSummary.vue";
import ProjectHomeCard from "@/components/project/ProjectHomeCard.vue";
import ProjectHomeRow from "@/components/project/ProjectHomeRow.vue";

enableAutoUnmount(afterEach);

const projects: ProjectOverviewSummary[] = [
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
    totalBytes: 123456789,
    engine: null,
    engineVersions: null,
    archiveOutputSize: "native",
    archiveOutputFormat: "jpeg",
    archiveOutputQuality: 100,
    lastActivityAt: "2026-08-08T04:36:00Z",
    latestCaptureItemId: "capture-bad",
    coverCaptureItemId: null,
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
    totalBytes: 987654321,
    engine: null,
    engineVersions: null,
    archiveOutputSize: "keep",
    archiveOutputFormat: "png",
    archiveOutputQuality: 90,
    lastActivityAt: "2026-08-07T23:10:00Z",
    latestCaptureItemId: "capture-summer",
    coverCaptureItemId: null,
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
    totalBytes: 0,
    engine: null,
    engineVersions: null,
    archiveOutputSize: "keep",
    archiveOutputFormat: "keep",
    archiveOutputQuality: 90,
    lastActivityAt: "2026-08-06T12:00:00Z",
    latestCaptureItemId: null,
    coverCaptureItemId: null,
  },
];

// Deliberately differ from the recorded capture counts and source sizes.
const archiveUsages: ProjectArchiveUsage[] = [
  {
    projectId: "project-bad",
    state: "ready",
    mainImageCount: 5,
    avatarImageCount: 2,
    totalBytes: 2 * 1024 ** 2,
    scannedAt: "2026-10-21T02:00:00Z",
    message: null,
  },
  {
    projectId: "project-summer",
    state: "ready",
    mainImageCount: 9,
    avatarImageCount: 1,
    totalBytes: 1024 ** 2,
    scannedAt: "2026-10-21T02:00:00Z",
    message: null,
  },
  {
    projectId: "project-eternum",
    state: "unconfigured",
    mainImageCount: null,
    avatarImageCount: null,
    totalBytes: null,
    scannedAt: "2026-10-21T02:00:00Z",
    message: null,
  },
];

function projectNames(wrapper: VueWrapper) {
  return wrapper.findAll('button[aria-label^="打开项目："]').map((button) =>
    (button.attributes("aria-label") ?? "").replace("打开项目：", ""),
  );
}

function menuItem(label: string): HTMLElement {
  const item = Array.from(document.body.querySelectorAll<HTMLElement>('[role="menuitem"]'))
    .find((item) => item.textContent?.includes(label));
  expect(item).toBeDefined();
  return item!;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

beforeAll(() => {
  Object.defineProperty(URL, "createObjectURL", { value: vi.fn(() => "blob:thumbnail") });
  Object.defineProperty(URL, "revokeObjectURL", { value: vi.fn() });
});

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  api.listProjectOverviews.mockReset().mockResolvedValue(projects);
  api.listProjectArchiveUsage.mockReset().mockResolvedValue(archiveUsages);
  api.cancelProjectArchiveUsage.mockReset().mockResolvedValue(undefined);
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
  it("shows every project with its capture workflow status", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    expect(projectNames(wrapper)).toEqual(["BAD", "Summer Heat", "Eternum"]);
    expect(wrapper.text()).toContain("待分类 3");
    expect(wrapper.text()).toContain("全部已归档");

    wrapper.unmount();
  });

  it("shows directory snapshots in both views and keeps offline archives unknown", async () => {
    api.listProjectArchiveUsage.mockResolvedValue([
      archiveUsages[0],
      {
        ...archiveUsages[1],
        state: "unavailable",
        mainImageCount: null,
        avatarImageCount: null,
        totalBytes: null,
        message: "NAS offline",
      },
      archiveUsages[2],
    ]);
    const wrapper = mount(Home);
    await flushPromises();

    const cards = wrapper.findAllComponents(ProjectHomeCard);
    const readySummary = cards[0].getComponent(ProjectArchiveSummary);
    expect(readySummary.attributes("aria-label")).toContain("主图：5 张");
    expect(readySummary.attributes("aria-label")).toContain("总大小：2.0 MB");
    expect(cards[1].getComponent(ProjectArchiveSummary).text()).toContain("未连接");
    expect(wrapper.text()).not.toContain("0 B");
    expect(wrapper.text()).not.toContain("117.7 MB");
    expect(wrapper.text()).not.toContain("941.9 MB");

    await wrapper.get('[aria-label="列表视图"]').trigger("click");

    const rows = wrapper.findAllComponents(ProjectHomeRow);
    expect(rows[0].get('[aria-label="归档，主图 5 张"]').text()).toContain("5");
    expect(rows[0].get('[aria-label="总大小：2.0 MB"]').text()).toMatch(/2\.0\s*MB/);
    expect(rows[1].get('[aria-label="未连接，主图 —"]').text()).toContain("未连接");
    expect(rows[1].get('[aria-label="总大小：—"]').text()).toBe("—");
    expect(localStorage.getItem("scene-vault.home.project-view")).toBe("list");
    wrapper.unmount();

    const reopened = mount(Home);
    await flushPromises();
    expect(reopened.get('[aria-label="列表视图"]').attributes("aria-pressed")).toBe("true");
    expect(projectNames(reopened)).toEqual(["BAD", "Summer Heat", "Eternum"]);
  });

  it.each(["grid", "list"])("launches a supported game from the %s view", async (view) => {
    localStorage.setItem("scene-vault.home.project-view", view);
    api.listProjectOverviews.mockResolvedValue([
      { ...projects[0], engine: "renpy", engineVersions: "7.7.3,8.1.3" },
      ...projects.slice(1),
    ]);
    launchGameMock.mockResolvedValue({
      integrationId: "install-1",
      engineRoot: "D:\\Games\\BAD",
      executable: "D:\\Games\\BAD\\BAD.exe",
      gameName: "BAD",
    });

    const wrapper = mount(Home);
    await flushPromises();

    expect(wrapper.get('button[aria-label="启动游戏 Summer Heat"]').attributes("disabled"))
      .toBeDefined();
    await wrapper.get('button[aria-label="启动游戏 BAD"]').trigger("click");
    await flushPromises();
    expect(launchGameMock).toHaveBeenCalledWith("project-bad");
  });

  it.each(["captures", "size"] as const)(
    "sorts by archive %s while keeping unknown statistics after complete results in both directions",
    async (mode) => {
      const states = ["loading", "unconfigured", "unavailable", "missing", "partial", "error"] as const;
      const unknownProjects = states.map((state) => ({
        ...projects[0],
        projectId: state,
        name: `A ${state}`,
        destinationConfigured: state !== "unconfigured",
      }));
      const completeProjects = [
        { ...projects[0], projectId: "small", name: "Small", captureCount: 1000, totalBytes: 10 ** 9 },
        { ...projects[1], projectId: "large", name: "Large", captureCount: 1, totalBytes: 1 },
        { ...projects[2], projectId: "empty", name: "Empty", destinationConfigured: true },
      ];
      const usages: ProjectArchiveUsage[] = [
        { ...archiveUsages[0], projectId: "small", mainImageCount: 2, totalBytes: 1024 },
        { ...archiveUsages[1], projectId: "large", mainImageCount: 8, totalBytes: 8192 },
        { ...archiveUsages[0], projectId: "empty", mainImageCount: 0, avatarImageCount: 0, totalBytes: 0 },
      ];
      for (const state of states) {
        if (state !== "loading") usages.push({ ...archiveUsages[2], projectId: state, state });
      }
      api.listProjectOverviews.mockResolvedValue([...unknownProjects, ...completeProjects]);
      api.listProjectArchiveUsage.mockResolvedValue(usages);
      const wrapper = mount(Home);
      await flushPromises();

      await wrapper.get('[aria-label="项目排序"] select').setValue(mode);
      expect(projectNames(wrapper).slice(0, 3)).toEqual(["Large", "Small", "Empty"]);
      const unknownNames = new Set(unknownProjects.map((project) => project.name));
      expect(new Set(projectNames(wrapper).slice(3))).toEqual(unknownNames);

      await wrapper.get('[aria-label="当前倒序，点击切换为正序"]').trigger("click");
      expect(projectNames(wrapper).slice(0, 3)).toEqual(["Empty", "Small", "Large"]);
      expect(new Set(projectNames(wrapper).slice(3))).toEqual(unknownNames);
    },
  );

  it("sorts projects by creation time and pending work", async () => {
    api.listProjectOverviews.mockResolvedValue([
      projects[0],
      { ...projects[1], captureCount: 12, failedCount: 3 },
      projects[2],
    ]);
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get('[aria-label="项目排序"] select').setValue("created");
    expect(projectNames(wrapper)).toEqual(["Eternum", "Summer Heat", "BAD"]);
    await wrapper.get('[aria-label="当前倒序，点击切换为正序"]').trigger("click");
    expect(projectNames(wrapper)).toEqual(["BAD", "Summer Heat", "Eternum"]);

    await wrapper.get('[aria-label="项目排序"] select').setValue("attention");
    expect(projectNames(wrapper)).toEqual(["Summer Heat", "BAD", "Eternum"]);
  });

  it("prefers the pinned project cover over the latest capture", async () => {
    api.listProjectOverviews.mockResolvedValue([
      { ...projects[0], coverCaptureItemId: "capture-cover" },
    ]);
    const wrapper = mount(Home, {
      global: {
        stubs: {
          CaptureThumbnail: {
            props: ["item"],
            template: "<div class='thumb-stub' :data-item='item && item.id' />",
          },
        },
      },
    });
    await flushPromises();

    expect(wrapper.get(".thumb-stub").attributes("data-item")).toBe("capture-cover");
    expect(wrapper.find('[data-item="capture-bad"]').exists()).toBe(false);
    wrapper.unmount();
  });

  it("filters projects without losing the library context", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get('input[type="search"]').setValue("  SUMMER  ");

    expect(projectNames(wrapper)).toEqual(["Summer Heat"]);
    await wrapper.get('input[type="search"]').setValue("main capture");
    expect(projectNames(wrapper)).toEqual(["BAD"]);
    await wrapper.get('input[type="search"]').setValue("no matching project");
    expect(projectNames(wrapper)).toEqual([]);
    await wrapper.get('input[type="search"]').setValue("");
    expect(projectNames(wrapper)).toEqual(["BAD", "Summer Heat", "Eternum"]);

    wrapper.unmount();
  });

  it.each(["grid", "list"])("selects a project before opening the workbench or capture in %s", async (view) => {
    localStorage.setItem("scene-vault.home.project-view", view);
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get('button[aria-label="打开项目：BAD"]').trigger("click");
    expect(localStorage.getItem("scene-vault.capture.project")).toBe("project-bad");
    expect(push).toHaveBeenCalledWith("/workbench");

    await wrapper.get('button[aria-label="继续捕获 Summer Heat"]').trigger("click");
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

  it("renames a project, cancels its old scan and ignores late results after reloading", async () => {
    const previousScan = deferred<ProjectArchiveUsage[]>();
    const currentScan = deferred<ProjectArchiveUsage[]>();
    api.listProjectArchiveUsage
      .mockReturnValueOnce(previousScan.promise)
      .mockReturnValueOnce(currentScan.promise);
    api.listProjectOverviews
      .mockResolvedValueOnce(projects)
      .mockResolvedValue([{ ...projects[0], name: "Renamed" }, ...projects.slice(1)]);
    const wrapper = mount(Home);
    await flushPromises();
    const [previousId, previousProgress] = api.listProjectArchiveUsage.mock.calls[0];
    previousProgress(archiveUsages[0]);
    await flushPromises();

    await wrapper.get('[aria-label="BAD 更多操作"]').trigger("click");
    await flushPromises();
    menuItem("重命名").click();
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
    expect(projectNames(wrapper)).toEqual(["Renamed", "Summer Heat", "Eternum"]);
    expect(document.body.querySelector('[role="dialog"][aria-label="重命名项目"]')).toBeNull();
    expect(api.cancelProjectArchiveUsage).toHaveBeenCalledTimes(1);
    expect(api.cancelProjectArchiveUsage).toHaveBeenCalledWith(previousId);
    const [currentId, currentProgress] = api.listProjectArchiveUsage.mock.calls[1];
    expect(currentId).not.toBe(previousId);

    const staleUsage = { ...archiveUsages[0], mainImageCount: 999, totalBytes: 999 };
    previousProgress(staleUsage);
    previousScan.resolve([staleUsage]);
    await flushPromises();
    expect(wrapper.findComponent(ProjectArchiveSummary).attributes("aria-busy")).toBe("true");

    currentProgress(archiveUsages[0]);
    await flushPromises();
    expect(wrapper.findComponent(ProjectArchiveSummary).attributes("aria-label"))
      .toContain("主图：5 张");

    wrapper.unmount();
    expect(api.cancelProjectArchiveUsage).toHaveBeenCalledTimes(2);
    expect(api.cancelProjectArchiveUsage).toHaveBeenCalledWith(currentId);
    currentScan.resolve(archiveUsages);
    await flushPromises();
  });

  it("deletes a project after previewing what will be removed", async () => {
    localStorage.setItem("scene-vault.capture.project", "project-bad");
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get('[aria-label="BAD 更多操作"]').trigger("click");
    await flushPromises();
    menuItem("删除项目").click();
    await flushPromises();

    expect(api.previewProjectDelete).toHaveBeenCalledWith("project-bad");
    const dialog = document.body.querySelector('[role="dialog"][aria-label="删除项目"]');
    expect(dialog).toBeTruthy();
    expect(dialog!.textContent).toContain("12 张截图记录");
    expect(dialog!.textContent).toContain("正在监听中");
    expect(dialog!.textContent).toContain("截图源文件和归档文件不会删除");
    expect(api.deleteProject).not.toHaveBeenCalled();

    api.listProjectOverviews.mockResolvedValue(projects.slice(1));
    (dialog!.querySelector(".danger-action") as HTMLButtonElement).click();
    await flushPromises();

    expect(api.deleteProject).toHaveBeenCalledWith("project-bad");
    expect(localStorage.getItem("scene-vault.capture.project")).toBeNull();
    expect(projectNames(wrapper)).toEqual(["Summer Heat", "Eternum"]);
    expect(document.body.querySelector('[role="dialog"][aria-label="删除项目"]')).toBeNull();

    wrapper.unmount();
  });

  it("sorts projects by name ascending and flips to descending", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get('[aria-label="项目排序"] select').setValue("name");
    expect(projectNames(wrapper)).toEqual(["BAD", "Eternum", "Summer Heat"]);

    await wrapper.get('[aria-label="当前正序，点击切换为倒序"]').trigger("click");
    expect(projectNames(wrapper)).toEqual(["Summer Heat", "Eternum", "BAD"]);
    expect(localStorage.getItem("scene-vault.home.sort")).toBe("name");
    expect(localStorage.getItem("scene-vault.home.sort-direction")).toBe("desc");

    wrapper.unmount();
  });

  it("paginates projects and resets the page after filtering or changing page size", async () => {
    const many = Array.from({ length: 60 }, (_, index) => ({
      ...projects[0],
      projectId: `project-${index}`,
      name: `Project ${String(index).padStart(2, "0")}`,
      createdAt: "2026-08-01T00:00:00Z",
    }));
    api.listProjectOverviews.mockResolvedValue(many);
    const wrapper = mount(Home);
    await flushPromises();

    expect(projectNames(wrapper)).toHaveLength(50);
    expect(wrapper.text()).toContain("共 60 条");

    await wrapper.get('[aria-label="下一页"]').trigger("click");
    await flushPromises();
    expect(projectNames(wrapper)).toHaveLength(10);
    expect(projectNames(wrapper)[0]).toBe("Project 50");

    await wrapper.get('input[type="search"]').setValue("Project 00");
    expect(projectNames(wrapper)).toEqual(["Project 00"]);
    expect(wrapper.get('[aria-label="上一页"]').attributes("disabled")).toBeDefined();
    await wrapper.get('input[type="search"]').setValue("");
    await wrapper.get('[aria-label="下一页"]').trigger("click");
    await wrapper.get('[aria-label="每页条数"]').setValue("100");
    expect(projectNames(wrapper)).toHaveLength(60);
    expect(wrapper.get('[aria-label="上一页"]').attributes("disabled")).toBeDefined();

    wrapper.unmount();
  });
});

describe("Home archive requests", () => {
  it("applies streamed statistics and keeps completed results when the remaining scan fails", async () => {
    const scan = deferred<ProjectArchiveUsage[]>();
    api.listProjectArchiveUsage.mockReturnValueOnce(scan.promise);
    const wrapper = mount(Home);
    await flushPromises();
    expect(wrapper.findComponent(ProjectArchiveSummary).attributes("aria-busy")).toBe("true");

    const [, onProgress] = api.listProjectArchiveUsage.mock.calls[0];
    onProgress(archiveUsages[0]);
    await flushPromises();
    expect(wrapper.findComponent(ProjectArchiveSummary).attributes("aria-label"))
      .toContain("主图：5 张");

    scan.reject(new Error("Archive scan interrupted"));
    await flushPromises();
    const summaries = wrapper.findAllComponents(ProjectArchiveSummary);
    expect(summaries[0].attributes("aria-label")).toContain("主图：5 张");
    expect(summaries[1].text()).toContain("无法读取");
    expect(summaries[1].attributes("aria-label")).toContain("Archive scan interrupted");
    expect(summaries[1].text()).not.toContain("0 B");
  });

  it("cancels on leaving and ignores the old progress and final response after re-entry", async () => {
    const previousScan = deferred<ProjectArchiveUsage[]>();
    api.listProjectArchiveUsage.mockReturnValueOnce(previousScan.promise);
    const previous = mount(Home);
    await flushPromises();
    const [previousId, previousProgress] = api.listProjectArchiveUsage.mock.calls[0];
    previous.unmount();
    expect(api.cancelProjectArchiveUsage).toHaveBeenCalledWith(previousId);

    const current = mount(Home);
    await flushPromises();
    expect(api.listProjectArchiveUsage.mock.calls[1][0]).not.toBe(previousId);
    const staleUsage = { ...archiveUsages[0], mainImageCount: 999 };
    previousProgress(staleUsage);
    previousScan.resolve([staleUsage]);
    await flushPromises();
    expect(current.findComponent(ProjectArchiveSummary).attributes("aria-label"))
      .toContain("主图：5 张");
  });

  it("does not start a scan when project loading finishes after leaving", async () => {
    const overviews = deferred<ProjectOverviewSummary[]>();
    api.listProjectOverviews.mockReturnValueOnce(overviews.promise);
    const wrapper = mount(Home);
    wrapper.unmount();

    overviews.resolve(projects);
    await flushPromises();
    expect(api.listProjectArchiveUsage).not.toHaveBeenCalled();
  });
});

describe("Home project context menu", () => {
  function closeMenu() {
    document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
  }

  it("opens the selected project or capture flow only after choosing a menu action", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.findComponent(ProjectHomeCard).trigger("contextmenu", { clientX: 140, clientY: 80 });
    await flushPromises();

    expect(push).not.toHaveBeenCalled();
    menuItem("打开项目").click();
    await flushPromises();
    expect(localStorage.getItem("scene-vault.capture.project")).toBe("project-bad");
    expect(push).toHaveBeenLastCalledWith("/workbench");

    await wrapper.findAllComponents(ProjectHomeCard)[1].trigger("contextmenu", { clientX: 140, clientY: 80 });
    await flushPromises();
    menuItem("继续捕获").click();
    await flushPromises();
    expect(localStorage.getItem("scene-vault.capture.project")).toBe("project-summer");
    expect(push).toHaveBeenLastCalledWith("/capture");
    wrapper.unmount();
  });

  it("allows the card main area but ignores footer buttons", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.get('button[aria-label="打开项目：BAD"]').trigger("contextmenu", { clientX: 140, clientY: 80 });
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
    closeMenu();
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')).toBeNull();

    await wrapper.get('button[aria-label="继续捕获 BAD"]').trigger("contextmenu", { clientX: 140, clientY: 80 });
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')).toBeNull();
    wrapper.unmount();
  });

  it("opens the rename dialog from the context menu", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.findComponent(ProjectHomeCard).trigger("contextmenu", { clientX: 140, clientY: 80 });
    await flushPromises();
    menuItem("重命名").click();
    await flushPromises();

    expect(document.body.querySelector('[role="dialog"][aria-label="重命名项目"]')).not.toBeNull();
    wrapper.unmount();
  });

  it("opens the delete dialog from the context menu", async () => {
    const wrapper = mount(Home);
    await flushPromises();

    await wrapper.findComponent(ProjectHomeCard).trigger("contextmenu", { clientX: 140, clientY: 80 });
    await flushPromises();
    menuItem("删除项目").click();
    await flushPromises();

    expect(document.body.querySelector('[role="dialog"][aria-label="删除项目"]')).not.toBeNull();
    wrapper.unmount();
  });

  it("opens from the list row but not from its action buttons", async () => {
    const wrapper = mount(Home);
    await flushPromises();
    await wrapper.get('[aria-label="列表视图"]').trigger("click");
    await flushPromises();

    await wrapper.findComponent(ProjectHomeRow).trigger("contextmenu", { clientX: 140, clientY: 80 });
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
    closeMenu();
    await flushPromises();

    await wrapper.get('[aria-label="BAD 更多操作"]').trigger("contextmenu", { clientX: 140, clientY: 80 });
    await flushPromises();
    expect(document.body.querySelector('[role="menu"]')).toBeNull();
    wrapper.unmount();
  });
});
