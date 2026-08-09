import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { eventHandlers } = vi.hoisted(() => ({
  eventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    eventHandlers.set(name, handler);
    return () => eventHandlers.delete(name);
  }),
}));

import CaptureProgress from "./CaptureProgress.vue";

beforeEach(() => eventHandlers.clear());

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
