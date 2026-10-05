import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({ rebuildArchiveManifest: vi.fn() }));

vi.mock("@/lib/capture-api", () => ({ captureApi: api }));

import ArchiveManifestDialog from "./ArchiveManifestDialog.vue";

beforeEach(() => {
  vi.clearAllMocks();
  api.rebuildArchiveManifest.mockResolvedValue({
    projectId: "project-1",
    manifestCount: 1,
    entryCount: 41,
    characterCount: 41,
    writtenCount: 1,
    unchangedCount: 0,
    skippedCount: 0,
    failedCount: 0,
    message: "已更新评分清单：41 张图、41 个人物",
  });
});

describe("ArchiveManifestDialog", () => {
  it("writes the manifest and shows the result inline", async () => {
    const wrapper = mount(ArchiveManifestDialog, {
      props: { projectId: "project-1" },
      attachTo: document.body,
    });
    await flushPromises();

    const button = [...document.body.querySelectorAll("button")].find((entry) =>
      entry.textContent?.includes("开始更新"),
    )!;
    button.click();
    await flushPromises();

    expect(api.rebuildArchiveManifest).toHaveBeenCalledWith("project-1");
    expect(document.body.textContent).toContain("已更新评分清单：41 张图、41 个人物");
    wrapper.unmount();
  });
});
