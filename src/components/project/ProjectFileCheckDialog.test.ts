import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({ reconcileProjectFiles: vi.fn() }));
const pickDirectoryMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  pickDirectory: pickDirectoryMock,
}));

import ProjectFileCheckDialog from "./ProjectFileCheckDialog.vue";

const reconcileResult = {
  sourceScannedDirectoryCount: 1,
  sourceScannedFileCount: 12,
  sourceCheckedCount: 2,
  sourceRelocatedCount: 1,
  sourceMissingCount: 0,
  sourceReplacedCount: 0,
  sourceAmbiguousCount: 0,
  sourceUnavailableDirectoryCount: 0,
  destinationMissingCount: 0,
  destinationUnavailableCount: 0,
  destinationRelocatedCount: 0,
  destinationAmbiguousCount: 0,
  destinationContentMismatchCount: 0,
  destinationScannedFileCount: 0,
};

beforeEach(() => {
  vi.clearAllMocks();
  api.reconcileProjectFiles.mockResolvedValue(reconcileResult);
});

function mountDialog() {
  return mount(ProjectFileCheckDialog, {
    props: { projectId: "project-1" },
    attachTo: document.body,
  });
}

function clickButton(label: string) {
  const button = [...document.body.querySelectorAll("button")].find((entry) =>
    entry.textContent?.includes(label),
  );
  expect(button).toBeDefined();
  button!.click();
}

describe("ProjectFileCheckDialog", () => {
  it("checks the project files and lists the findings", async () => {
    const wrapper = mountDialog();
    await flushPromises();
    expect(document.body.textContent).toContain("检查项目文件");

    clickButton("开始检查");
    await flushPromises();

    expect(api.reconcileProjectFiles).toHaveBeenCalledWith("project-1", null);
    expect(document.body.textContent).toContain("已重新定位 1 张原图");
    expect(wrapper.emitted("updated")).toHaveLength(1);
    wrapper.unmount();
  });

  it("searches a picked archive directory when targets are missing", async () => {
    api.reconcileProjectFiles.mockResolvedValueOnce({
      ...reconcileResult,
      sourceRelocatedCount: 0,
      destinationMissingCount: 1,
    });
    const wrapper = mountDialog();
    await flushPromises();
    clickButton("开始检查");
    await flushPromises();

    pickDirectoryMock.mockResolvedValueOnce("/volume/renamed-archive");
    clickButton("指定归档目录找回…");
    await flushPromises();

    expect(api.reconcileProjectFiles).toHaveBeenLastCalledWith(
      "project-1",
      "/volume/renamed-archive",
    );
    wrapper.unmount();
  });
});
