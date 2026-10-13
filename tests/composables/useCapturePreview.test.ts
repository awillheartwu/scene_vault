import { defineComponent, nextTick, ref, type Ref } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCapturePreview } from "@/composables/useCapturePreview";
import type { CaptureItem } from "@/lib/capture-api";

const api = vi.hoisted(() => ({ readImage: vi.fn(), recordClientEvent: vi.fn(async () => {}) }));
vi.mock("@/lib/capture-api", () => ({ captureApi: api, pathMimeType: () => "image/png" }));

const decode = vi.fn<(src: string) => Promise<void>>();
const createUrl = vi.fn();
const revokeUrl = vi.fn();
const wrappers: VueWrapper[] = [];

function item(id: string): CaptureItem {
  return {
    id, sourcePath: `C:/game/${id}.png`, fileSize: 100, modifiedAtMs: 1,
    contentHash: id, destinationPath: null, archivedAt: null, sourceFileState: "available",
    status: "awaiting_label", classification: "unclassified",
  } as CaptureItem;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(finish => { resolve = finish; });
  return { promise, resolve };
}

function setup(selected: Ref<CaptureItem | null>, variant: () => "source" | "destination" = () => "source") {
  let preview!: ReturnType<typeof useCapturePreview>;
  const onError = vi.fn();
  const wrapper = mount(defineComponent({
    setup() { preview = useCapturePreview(selected, variant, onError); return () => null; },
  }));
  wrappers.push(wrapper);
  return { preview, onError, wrapper };
}

beforeEach(() => {
  api.readImage.mockReset().mockResolvedValue(new ArrayBuffer(4));
  decode.mockReset().mockResolvedValue(undefined);
  let count = 0;
  createUrl.mockReset().mockImplementation(() => `blob:preview-${++count}`);
  revokeUrl.mockReset();
  vi.stubGlobal("URL", { createObjectURL: createUrl, revokeObjectURL: revokeUrl });
  vi.stubGlobal("Image", class {
    src = "";
    decode() { return decode(this.src); }
  });
});

afterEach(async () => {
  wrappers.splice(0).forEach(wrapper => wrapper.unmount());
  await nextTick();
  vi.unstubAllGlobals();
});

describe("useCapturePreview", () => {
  it("keeps the decoded preview available when performance reporting fails", async () => {
    api.recordClientEvent.mockRejectedValueOnce(new Error("logging disabled"));
    const selected = ref<CaptureItem | null>(item("a"));
    const { preview, onError } = setup(selected);
    await flushPromises();
    expect(preview.url.value).toBe("blob:preview-1");
    expect(preview.loading.value).toBe(false);
    expect(onError).not.toHaveBeenCalled();
    expect(api.recordClientEvent).toHaveBeenCalledWith(expect.objectContaining({
      captureItemId: "a", event: "load_timing", channel: "perf", outcome: "ok",
      durationMs: expect.any(Number),
    }));
  });

  it("keeps the preview when ROI and recognition state change on the same picture", async () => {
    const selected = ref<CaptureItem | null>(item("a"));
    const { preview } = setup(selected);
    await flushPromises();
    selected.value = {
      ...selected.value!, status: "queued", reviewStatus: "none", processingVersion: 2,
      faceDetectionMode: "normalized", manualFaceRoiJson: '{"x":0,"y":0,"width":0.5,"height":0.5}',
      suggestedCharacterId: "character", updatedAt: "2026-10-12T01:00:00Z",
    };
    await preview.load();
    await flushPromises();
    expect(api.readImage).toHaveBeenCalledTimes(1);
    expect(preview.url.value).toBe("blob:preview-1");
    expect(revokeUrl).not.toHaveBeenCalled();
  });

  it("shares a pending read instead of reading again for an explicit refresh", async () => {
    const bytes = deferred<ArrayBuffer>();
    api.readImage.mockReturnValue(bytes.promise);
    const { preview } = setup(ref<CaptureItem | null>(item("a")));
    const first = preview.load();
    const second = preview.load();
    expect(preview.loading.value).toBe(true);
    expect(api.readImage).toHaveBeenCalledTimes(1);
    bytes.resolve(new ArrayBuffer(4));
    await Promise.all([first, second]);
    expect(preview.url.value).toBe("blob:preview-1");
    expect(preview.loading.value).toBe(false);
  });

  it("retains the same capture until its refreshed picture finishes decoding", async () => {
    const selected = ref<CaptureItem | null>(item("a"));
    const { preview } = setup(selected);
    await flushPromises();
    const ready = deferred<void>();
    decode.mockReturnValueOnce(ready.promise);
    selected.value = { ...selected.value!, sourcePath: "C:/moved/a.png" };
    await flushPromises();
    expect(preview.url.value).toBe("blob:preview-1");
    expect(preview.loading.value).toBe(true);
    expect(revokeUrl).not.toHaveBeenCalled();
    ready.resolve();
    await flushPromises();
    expect(preview.url.value).toBe("blob:preview-2");
    expect(revokeUrl.mock.calls).toEqual([["blob:preview-1"]]);
  });

  it("discards a late read after selecting a different capture", async () => {
    const bytes = deferred<ArrayBuffer>();
    api.readImage.mockReturnValueOnce(bytes.promise);
    const selected = ref<CaptureItem | null>(item("a"));
    const { preview, onError } = setup(selected);
    selected.value = item("b");
    await flushPromises();
    bytes.resolve(new ArrayBuffer(4));
    await flushPromises();
    expect(api.readImage).toHaveBeenLastCalledWith("b", "source");
    expect(preview.url.value).toBe("blob:preview-1");
    expect(createUrl).toHaveBeenCalledTimes(1);
    expect(onError).not.toHaveBeenCalled();
  });

  it("clears the old capture immediately and discards its late decode", async () => {
    const selected = ref<CaptureItem | null>(item("a"));
    const { preview } = setup(selected);
    await flushPromises();
    const oldDecode = deferred<void>();
    const newDecode = deferred<void>();
    decode.mockReturnValueOnce(oldDecode.promise).mockReturnValueOnce(newDecode.promise);
    selected.value = { ...selected.value!, sourcePath: "C:/moved/a.png" };
    await flushPromises();
    selected.value = item("b");
    await flushPromises();
    expect(preview.url.value).toBeNull();
    oldDecode.resolve();
    await flushPromises();
    expect(preview.url.value).toBeNull();
    newDecode.resolve();
    await flushPromises();
    expect(preview.url.value).toBe("blob:preview-3");
    expect(revokeUrl.mock.calls).toEqual([["blob:preview-1"], ["blob:preview-2"]]);
  });

  it.each(["missing", "replaced"] as const)("clears an explicitly %s source without another read", async state => {
    const selected = ref<CaptureItem | null>(item("a"));
    const { preview } = setup(selected);
    await flushPromises();
    selected.value = { ...selected.value!, sourceFileState: state };
    await flushPromises();
    expect(preview.url.value).toBeNull();
    expect(preview.unavailable.value).toContain(state === "missing" ? "缺失" : "替换");
    expect(api.readImage).toHaveBeenCalledTimes(1);
    expect(revokeUrl).toHaveBeenCalledWith("blob:preview-1");
  });

  it("shows an error and releases both previews if the replacement fails to decode", async () => {
    const selected = ref<CaptureItem | null>(item("a"));
    const { preview, onError } = setup(selected);
    await flushPromises();
    decode.mockRejectedValueOnce(new Error("invalid image"));
    selected.value = { ...selected.value!, sourcePath: "C:/moved/a.png" };
    await flushPromises();
    expect(preview.url.value).toBeNull();
    expect(preview.loading.value).toBe(false);
    expect(preview.unavailable.value).toContain("无法加载图片");
    expect(onError).toHaveBeenCalledWith(preview.unavailable.value);
    expect(revokeUrl.mock.calls.map(([url]) => url).sort()).toEqual(["blob:preview-1", "blob:preview-2"]);
  });

  it("does not reuse the source preview when switching to the destination", async () => {
    const selected = ref<CaptureItem | null>({ ...item("a"), destinationPath: "C:/archive/a.png" });
    const variant = ref<"source" | "destination">("source");
    const { preview } = setup(selected, () => variant.value);
    await flushPromises();
    const ready = deferred<void>();
    decode.mockReturnValueOnce(ready.promise);
    variant.value = "destination";
    await flushPromises();
    expect(preview.url.value).toBeNull();
    expect(api.readImage).toHaveBeenLastCalledWith("a", "destination");
    ready.resolve();
    await flushPromises();
    expect(preview.url.value).toBe("blob:preview-2");
  });

  it("releases a late decoded URL after unmounting without reviving the preview", async () => {
    const ready = deferred<void>();
    decode.mockReturnValueOnce(ready.promise);
    const { preview, wrapper } = setup(ref<CaptureItem | null>(item("a")));
    await flushPromises();
    wrapper.unmount();
    wrappers.splice(wrappers.indexOf(wrapper), 1);
    ready.resolve();
    await flushPromises();
    expect(preview.url.value).toBeNull();
    expect(preview.loading.value).toBe(false);
    expect(revokeUrl.mock.calls).toEqual([["blob:preview-1"]]);
  });
});
