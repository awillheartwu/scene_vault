import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  previewFaceRepair: vi.fn(),
  startFaceRepair: vi.fn(),
  getFaceRepairStatus: vi.fn(),
}));

vi.mock("@/lib/capture-api", () => ({ captureApi: api }));

import CaptureFaceRepairDialog from "./CaptureFaceRepairDialog.vue";

const candidate = {
  id: "capture-1",
  missingFaceBox: false,
  sourcePath: "D:\\shots\\one.png",
  fileName: "one.png",
  storedWidth: 500,
  storedHeight: 600,
  storedConfidence: 0.63,
};

const status = (overrides: Record<string, unknown> = {}) => ({
  state: "running",
  projectId: "project-1",
  taskId: "task-1",
  total: 1,
  processed: 0,
  requeued: 0,
  kept: 0,
  failed: 0,
  currentFile: null,
  message: null,
  ...overrides,
});

beforeEach(() => {
  vi.resetAllMocks();
  api.getFaceRepairStatus.mockResolvedValue(status({ state: "idle", total: 0 }));
  api.previewFaceRepair.mockResolvedValue({ candidates: [candidate], retries: [] });
  api.startFaceRepair.mockResolvedValue(status());
});

const wrappers: ReturnType<typeof mount>[] = [];
afterEach(() => {
  wrappers.splice(0).forEach((wrapper) => wrapper.unmount());
  vi.useRealTimers();
  document.body.innerHTML = "";
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function startButton() {
  return [...document.body.querySelectorAll("button")].find((entry) =>
    entry.textContent?.includes("开始核对"),
  )!;
}

function mountDialog() {
  const wrapper = mount(CaptureFaceRepairDialog, {
    props: { projectId: "project-1" },
    attachTo: document.body,
  });
  wrappers.push(wrapper);
  return wrapper;
}

describe("CaptureFaceRepairDialog", () => {
  it("previews the project's suspicious captures before starting", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    expect(api.previewFaceRepair).toHaveBeenCalledWith("project-1");
    expect(document.body.textContent).toContain("1 张低置信度脸框需要核对");
    wrapper.unmount();
  });

  it("starts the repair, polls the progress and reports the summary", async () => {
    vi.useFakeTimers();
    try {
      const wrapper = mountDialog();
      await flushPromises();

      const start = [...document.body.querySelectorAll("button")].find((entry) =>
        entry.textContent?.includes("开始核对"),
      )!;
      start.click();
      await flushPromises();
      expect(api.startFaceRepair).toHaveBeenCalledWith("project-1");

      api.getFaceRepairStatus.mockResolvedValue(
        status({ state: "done", processed: 1, requeued: 1 }),
      );
      await vi.advanceTimersByTimeAsync(800);
      await flushPromises();

      expect(document.body.textContent).toContain("已重新排队");
      expect(wrapper.emitted("updated")).toHaveLength(1);
      wrapper.unmount();
    } finally {
      vi.useRealTimers();
    }
  });
  it("counts missing boxes separately and guides detection failures to manual retry", async () => {
    api.previewFaceRepair.mockResolvedValue({
      candidates: [candidate, { ...candidate, id: "missing", missingFaceBox: true }],
      retries: [{ id: "retry", fileName: "retry.png" }],
    });
    mountDialog();
    await flushPromises();
    expect(document.body.textContent).toContain("1 张低置信度脸框需要核对");
    expect(document.body.textContent).toContain("1 张已完成人物图缺少旧脸框");
    expect(document.body.textContent).toContain("已有检测失败项不包含在本次核对中");
    expect(document.body.textContent).toContain("手动重试");
    expect(startButton().disabled).toBe(false);
  });

  it("allows a missing-box-only preview to start", async () => {
    api.previewFaceRepair.mockResolvedValue({ candidates: [{ ...candidate, missingFaceBox: true }], retries: [] });
    mountDialog();
    await flushPromises();
    startButton().click();
    await flushPromises();
    expect(api.startFaceRepair).toHaveBeenCalledWith("project-1");
  });

  it.each([
    ["done", 0, "核对完成"],
    ["partial", 1, "核对结束，部分失败"],
    ["failed", 2, "核对失败"],
  ])("shows %s outcome and the backend message", async (state, failed, label) => {
    vi.useFakeTimers();
    api.getFaceRepairStatus.mockResolvedValue(status({ total: 2 }));
    const wrapper = mountDialog();
    await flushPromises();
    api.getFaceRepairStatus.mockResolvedValue(status({ state, total: 2, processed: 2, failed, message: failed ? "probe failed: model unavailable" : null }));
    await vi.advanceTimersByTimeAsync(700);
    expect(document.body.textContent).toContain(label);
    if (failed) expect(document.body.textContent).toContain("probe failed: model unavailable");
    expect(wrapper.emitted("updated")).toHaveLength(1);
    const calls = api.getFaceRepairStatus.mock.calls.length;
    await vi.advanceTimersByTimeAsync(2100);
    expect(api.getFaceRepairStatus).toHaveBeenCalledTimes(calls);
  });

  it("blocks for another project's run without displaying its progress or emitting its completion", async () => {
    vi.useFakeTimers();
    api.getFaceRepairStatus.mockResolvedValue(status({ projectId: "project-2", currentFile: "foreign-private.png", processed: 43 }));
    const wrapper = mountDialog();
    await flushPromises();
    expect(document.body.textContent).toContain("其他项目（project-2）正在核对");
    expect(document.body.textContent).not.toContain("foreign-private.png");
    expect(document.body.textContent).not.toContain("43/");
    expect(startButton().disabled).toBe(true);
    api.getFaceRepairStatus.mockResolvedValue(status({ projectId: "project-2", state: "done" }));
    await vi.advanceTimersByTimeAsync(700);
    expect(startButton().disabled).toBe(false);
    expect(wrapper.emitted("updated")).toBeUndefined();
    expect(document.body.textContent).not.toContain("核对完成");
  });

  it("does not apply a replaced task's completion to the tracked task", async () => {
    vi.useFakeTimers();
    api.getFaceRepairStatus.mockResolvedValue(status());
    const wrapper = mountDialog();
    await flushPromises();
    api.getFaceRepairStatus.mockResolvedValue(status({ taskId: "new-task", state: "done", requeued: 99 }));
    await vi.advanceTimersByTimeAsync(700);
    expect(wrapper.emitted("updated")).toBeUndefined();
    expect(document.body.textContent).not.toContain("99");
    expect(document.body.textContent).not.toContain("核对完成");
  });

  it("serializes slow polls and ignores their completion after unmount", async () => {
    vi.useFakeTimers();
    api.getFaceRepairStatus.mockResolvedValue(status());
    const wrapper = mountDialog();
    await flushPromises();
    const pending = deferred<ReturnType<typeof status>>();
    api.getFaceRepairStatus.mockReturnValue(pending.promise);
    await vi.advanceTimersByTimeAsync(2800);
    expect(api.getFaceRepairStatus).toHaveBeenCalledTimes(2);
    wrapper.unmount();
    pending.resolve(status({ state: "done", processed: 1 }));
    await flushPromises();
    await vi.advanceTimersByTimeAsync(1400);
    expect(api.getFaceRepairStatus).toHaveBeenCalledTimes(2);
    expect(wrapper.emitted("updated")).toBeUndefined();
  });

  it.each(["status", "preview", "start"])("does not restart polling after unmount during %s", async (stage) => {
    vi.useFakeTimers();
    const pending = deferred<any>();
    if (stage === "status") api.getFaceRepairStatus.mockReturnValue(pending.promise);
    if (stage === "preview") api.previewFaceRepair.mockReturnValue(pending.promise);
    if (stage === "start") api.startFaceRepair.mockReturnValue(pending.promise);
    const wrapper = mountDialog();
    await flushPromises();
    if (stage === "start") { startButton().click(); await flushPromises(); }
    wrapper.unmount();
    pending.resolve(stage === "preview" ? { candidates: [candidate], retries: [] } : status());
    await flushPromises();
    await vi.advanceTimersByTimeAsync(2100);
    expect(api.getFaceRepairStatus).toHaveBeenCalledTimes(1);
    expect(wrapper.emitted("updated")).toBeUndefined();
  });

  it("ignores a late preview after changing projects", async () => {
    const pending = deferred<any>();
    api.previewFaceRepair.mockReturnValueOnce(pending.promise);
    const wrapper = mountDialog();
    await flushPromises();
    api.previewFaceRepair.mockResolvedValue({ candidates: [], retries: [] });
    await wrapper.setProps({ projectId: "project-2" });
    await flushPromises();
    pending.resolve({ candidates: [candidate], retries: [] });
    await flushPromises();
    expect(document.body.textContent).toContain("0 张低置信度脸框需要核对");
    expect(startButton().disabled).toBe(true);
  });

  it("reports an initialization error and refreshes the global reservation", async () => {
    api.startFaceRepair.mockRejectedValue(new Error("engine initialization failed"));
    mountDialog();
    await flushPromises();
    startButton().click();
    await flushPromises();
    expect(document.body.textContent).toContain("engine initialization failed");
    expect(startButton().disabled).toBe(false);
    expect(api.getFaceRepairStatus).toHaveBeenCalledTimes(2);
  });

});
