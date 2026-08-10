import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { eventHandlers, api } = vi.hoisted(() => ({
  eventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
  api: {
    runtimeStatus: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    eventHandlers.set(name, handler);
    return () => eventHandlers.delete(name);
  }),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
}));

import CaptureProgress from "./CaptureProgress.vue";

beforeEach(() => {
  eventHandlers.clear();
  api.runtimeStatus.mockResolvedValue({
    engineStatus: "configured",
    workerStatus: "idle",
    activeCaptureItemId: null,
    activeCaptureSourcePath: null,
    queuedCount: 0,
    archivePendingCount: 0,
    prelabelPendingCount: 0,
    lastError: null,
  });
});

describe("CaptureProgress", () => {
  it("reserves a stable idle status area when persistent", async () => {
    const wrapper = mount(CaptureProgress, { props: { persistent: true } });
    await flushPromises();

    expect(wrapper.get(".capture-progress").classes()).toContain("idle");
    expect(wrapper.text()).toContain("图片处理状态");
    expect(wrapper.text()).toContain("空闲");
    wrapper.unmount();
  });

  it("stays hidden while idle when persistence is not requested", async () => {
    const wrapper = mount(CaptureProgress);
    await flushPromises();

    expect(wrapper.find(".capture-progress").exists()).toBe(false);
    wrapper.unmount();
  });

  it("explains that imported screenshots await explicit start", async () => {
    const wrapper = mount(CaptureProgress, {
      props: { persistent: true, deferredCount: 43 },
    });
    await flushPromises();

    expect(wrapper.find('[data-state="deferred-import"]').text()).toContain("已登记 43 张截图");
    wrapper.unmount();
  });

  it("warns that an unconfigured AI engine cannot recognize imports", async () => {
    api.runtimeStatus.mockResolvedValue({
      engineStatus: "unconfigured",
      workerStatus: "idle",
      activeCaptureItemId: null,
      activeCaptureSourcePath: null,
      queuedCount: 0,
      archivePendingCount: 0,
      lastError: null,
    });
    const wrapper = mount(CaptureProgress, {
      props: { persistent: true, deferredCount: 43 },
    });
    await flushPromises();

    const hint = wrapper.find('[data-state="engine-unconfigured"]');
    expect(hint.exists()).toBe(true);
    expect(hint.text()).toContain("AI 未配置");
    wrapper.unmount();
  });

  it("shows the remaining imported-recognition batch in the idle status line", async () => {
    api.runtimeStatus.mockResolvedValue({
      engineStatus: "configured",
      workerStatus: "idle",
      activeCaptureItemId: null,
      activeCaptureSourcePath: null,
      queuedCount: 0,
      archivePendingCount: 0,
      prelabelPendingCount: 17,
      lastError: null,
    });
    const wrapper = mount(CaptureProgress, { props: { persistent: true } });
    await flushPromises();

    const batch = wrapper.find('[data-state="batch-remaining"]');
    expect(batch.exists()).toBe(true);
    expect(wrapper.get(".capture-progress-text").text()).toContain("本批识别 剩余 17 张");
    wrapper.unmount();
  });

  it("shows the batch remaining next to the current image and clears when done", async () => {
    api.runtimeStatus.mockResolvedValue({
      engineStatus: "configured",
      workerStatus: "idle",
      activeCaptureItemId: null,
      activeCaptureSourcePath: null,
      queuedCount: 0,
      archivePendingCount: 0,
      prelabelPendingCount: 0,
      lastError: null,
    });
    const wrapper = mount(CaptureProgress, { props: { persistent: true } });
    await flushPromises();

    // A batch item is in flight; a runtime-status between items must not
    // blank the bar while the batch still has remaining images.
    eventHandlers.get("capture:progress")?.({
      payload: {
        captureItemId: "item-5",
        sourcePath: "D:\\Screenshots\\screenshot0005.png",
        stage: "extract_feature",
        percent: 40,
      },
    });
    await wrapper.vm.$nextTick();
    eventHandlers.get("capture:runtime-status")?.({
      payload: {
        engineStatus: "configured",
        workerStatus: "idle",
        activeCaptureItemId: null,
        activeCaptureSourcePath: null,
        queuedCount: 0,
        archivePendingCount: 0,
        prelabelPendingCount: 16,
        lastError: null,
      },
    });
    await wrapper.vm.$nextTick();
    expect(wrapper.text()).toContain("screenshot0005.png");
    expect(wrapper.get(".capture-progress-value").text()).toContain("40%");
    expect(wrapper.get(".capture-progress-value").text()).toContain("剩余 16 张");

    // The last item finishes: the batch row hides and the bar idles.
    eventHandlers.get("capture:runtime-status")?.({
      payload: {
        engineStatus: "configured",
        workerStatus: "idle",
        activeCaptureItemId: null,
        activeCaptureSourcePath: null,
        queuedCount: 0,
        archivePendingCount: 0,
        prelabelPendingCount: 0,
        lastError: null,
      },
    });
    await wrapper.vm.$nextTick();
    expect(wrapper.find('[data-state="batch-remaining"]').exists()).toBe(false);
    expect(wrapper.get(".capture-progress").classes()).toContain("idle");
    wrapper.unmount();
  });

  it("shows an active item immediately from runtime status before detailed progress arrives", async () => {
    const wrapper = mount(CaptureProgress, { props: { persistent: true } });
    await flushPromises();

    eventHandlers.get("capture:runtime-status")?.({
      payload: {
        engineStatus: "configured",
        workerStatus: "running",
        activeCaptureItemId: "item-1",
        activeCaptureSourcePath: "D:\\Screenshots\\screenshot0044.png",
        queuedCount: 2,
        archivePendingCount: 0,
        lastError: null,
      },
    });
    await wrapper.vm.$nextTick();

    expect(wrapper.get(".capture-progress").classes()).not.toContain("idle");
    expect(wrapper.text()).toContain("处理中");
    expect(wrapper.text()).toContain("screenshot0044.png");
    expect(wrapper.text()).toMatch(/0%\s+· 后续 2 张/);

    eventHandlers.get("capture:progress")?.({
      payload: {
        captureItemId: "item-1",
        sourcePath: "D:\\Screenshots\\screenshot0044.png",
        stage: "detect_face",
        percent: 45,
      },
    });
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).toContain("检测人脸");
    expect(wrapper.text()).toMatch(/45%\s+· 后续 2 张/);
    expect(wrapper.get(".capture-progress-bar").attributes("style")).toContain("width: 45%");
    wrapper.unmount();
  });
});
