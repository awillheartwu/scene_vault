import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, expect, it, vi } from "vitest";

const events = vi.hoisted(() => ({
  listen: vi.fn(),
  unlisten: vi.fn(),
  handler: undefined as ((event: { payload: Record<string, unknown> }) => void) | undefined,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: events.listen }));

const api = vi.hoisted(() => ({
  readThumbnail: vi.fn(),
  readImage: vi.fn(),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  pathMimeType: () => "image/png",
}));

import CaptureThumbnail from "./CaptureThumbnail.vue";
import CharacterAvatar from "@/components/character/CharacterAvatar.vue";

beforeEach(() => {
  vi.unstubAllGlobals();
  vi.stubGlobal("IntersectionObserver", undefined);
  events.unlisten.mockReset();
  events.listen.mockReset().mockImplementation((_name, handler) => {
    events.handler = handler;
    return Promise.resolve(events.unlisten);
  });
  api.readThumbnail.mockReset().mockResolvedValue(new ArrayBuffer(4));
  api.readImage.mockReset().mockResolvedValue(new ArrayBuffer(4));
  vi.stubGlobal("URL", {
    createObjectURL: vi.fn(() => "blob:thumbnail"),
    revokeObjectURL: vi.fn(),
  });
});

it("does not reload when the parent re-renders with equal item values", async () => {
  const item = {
    id: "capture-1",
    sourcePath: "D:\\shots\\one.png",
    updatedAt: "2026-08-01T00:00:00Z",
  };
  const wrapper = mount(CaptureThumbnail, { props: { item: item as never } });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(1);

  // Parent lists rebuild item objects (and id-only stubs) on every render;
  // equal values must not blank and re-read the thumbnail.
  await wrapper.setProps({ item: { ...item } as never });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(1);

  // A real content change still reloads.
  await wrapper.setProps({
    item: { ...item, sourcePath: "D:\\shots\\new.png" } as never,
  });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(2);
  wrapper.unmount();
});

it("does not request a thumbnail until the card enters the viewport", async () => {
  let onIntersect: IntersectionObserverCallback = () => undefined;
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: IntersectionObserverCallback) {
      onIntersect = callback;
    }
    observe() {}
    disconnect() {}
    unobserve() {}
    takeRecords() { return []; }
    root = null;
    rootMargin = "";
    thresholds = [];
  });

  const wrapper = mount(CaptureThumbnail, {
    props: {
      item: { id: "capture-1", sourcePath: "D:\\shots\\one.png" } as never,
    },
  });
  await flushPromises();
  expect(api.readThumbnail).not.toHaveBeenCalled();

  onIntersect([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(1);
  wrapper.unmount();
});

it("loads the original for an auto-sized image wider than the thumbnail threshold", async () => {
  let onIntersect: IntersectionObserverCallback = () => undefined;
  let onResize: ResizeObserverCallback = () => undefined;
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: IntersectionObserverCallback) { onIntersect = callback; }
    observe() {}
    disconnect() {}
    unobserve() {}
    takeRecords() { return []; }
    root = null;
    rootMargin = "";
    thresholds = [];
  });
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: ResizeObserverCallback) { onResize = callback; }
    observe() {}
    disconnect() {}
    unobserve() {}
  });

  const wrapper = mount(CaptureThumbnail, {
    props: {
      item: { id: "capture-large", sourcePath: "D:\\shots\\large.png" } as never,
      size: "auto",
    },
  });
  const target = wrapper.element as Element;
  onResize(
    [{ target, contentRect: { width: 720 } } as ResizeObserverEntry],
    {} as ResizeObserver,
  );
  onIntersect([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
  await flushPromises();

  expect(api.readImage).toHaveBeenCalledWith("capture-large", "source");
  expect(api.readThumbnail).not.toHaveBeenCalled();
  wrapper.unmount();
});

it("does not request the source when the item reports the file as missing", async () => {
  let onIntersect: IntersectionObserverCallback = () => undefined;
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: IntersectionObserverCallback) {
      onIntersect = callback;
    }
    observe() {}
    disconnect() {}
    unobserve() {}
    takeRecords() { return []; }
    root = null;
    rootMargin = "";
    thresholds = [];
  });

  const wrapper = mount(CaptureThumbnail, {
    props: {
      item: {
        id: "capture-missing",
        sourcePath: "D:\\shots\\missing.png",
        sourceFileState: "missing",
      } as never,
    },
  });
  await flushPromises();
  onIntersect([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
  await flushPromises();

  expect(api.readThumbnail).not.toHaveBeenCalled();
  expect(api.readImage).not.toHaveBeenCalled();
  wrapper.unmount();
});

it("does not request an archived variant that is unreachable", async () => {
  let onIntersect: IntersectionObserverCallback = () => undefined;
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: IntersectionObserverCallback) {
      onIntersect = callback;
    }
    observe() {}
    disconnect() {}
    unobserve() {}
    takeRecords() { return []; }
    root = null;
    rootMargin = "";
    thresholds = [];
  });

  const wrapper = mount(CaptureThumbnail, {
    props: {
      item: {
        id: "capture-unavailable",
        sourcePath: "D:\\shots\\one.png",
        destinationPath: "D:\\archive\\one.png",
        destinationFileState: "unavailable",
      } as never,
      variant: "destination",
    },
  });
  await flushPromises();
  onIntersect([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
  await flushPromises();

  expect(api.readThumbnail).not.toHaveBeenCalled();
  expect(api.readImage).not.toHaveBeenCalled();
  wrapper.unmount();
});

function completedItem(id = "capture-1") {
  return {
    id,
    sourcePath: "D:\\shots\\one.png",
    avatarPath: "D:\\derived\\avatar.png",
    annotatedPath: "D:\\derived\\annotated.png",
    status: "completed",
    updatedAt: "2026-10-05T10:00:00Z",
    processedAt: "2026-10-05T10:00:00Z",
  };
}

it("refreshes mounted pictures and unchanged character avatar stubs through one shared listener", async () => {
  let nextUrl = 0;
  vi.mocked(URL.createObjectURL).mockImplementation(() => `blob:${++nextUrl}`);
  const picture = mount(CaptureThumbnail, {
    props: { item: completedItem() as never, variant: "annotated" },
  });
  const avatar = mount(CharacterAvatar, {
    props: { character: { id: "character-1", name: "Alice", avatarCaptureItemId: "capture-1" } as never },
  });
  await flushPromises();
  const originalPicture = picture.get("img").attributes("src");
  const originalAvatar = avatar.get("img").attributes("src");
  expect(events.listen).toHaveBeenCalledTimes(1);
  expect(events.listen).toHaveBeenCalledWith("capture:item-updated", expect.any(Function));
  expect(api.readThumbnail).toHaveBeenCalledTimes(2);

  events.handler?.({ payload: completedItem("unrelated") });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(2);
  // A new artifact version refreshes both mounted pictures.
  events.handler?.({ payload: { ...completedItem(), processingVersion: 2 } });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(4);
  expect(picture.get("img").attributes("src")).not.toBe(originalPicture);
  expect(avatar.get("img").attributes("src")).not.toBe(originalAvatar);
  expect(URL.revokeObjectURL).toHaveBeenCalledWith(originalAvatar);

  picture.unmount();
  expect(events.unlisten).not.toHaveBeenCalled();
  avatar.unmount();
  expect(events.unlisten).toHaveBeenCalledTimes(1);
  events.handler?.({ payload: completedItem() });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(4);
});

it("does not fetch offscreen pictures when reprocessing completes", async () => {
  let intersect: IntersectionObserverCallback = () => undefined;
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: IntersectionObserverCallback) { intersect = callback; }
    observe() {}
    disconnect() {}
  });
  const wrapper = mount(CaptureThumbnail, {
    props: { item: { id: "capture-1", sourcePath: "old.png" } as never, variant: "annotated" },
  });
  events.handler?.({ payload: completedItem() });
  await flushPromises();
  expect(api.readThumbnail).not.toHaveBeenCalled();
  intersect([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledWith("capture-1", "annotated");
  wrapper.unmount();
});

it("discards an in-flight pre-repair response after completion", async () => {
  let finishOld: (value: ArrayBuffer) => void = () => undefined;
  api.readThumbnail.mockImplementationOnce(() => new Promise<ArrayBuffer>((resolve) => { finishOld = resolve; }));
  const wrapper = mount(CaptureThumbnail, {
    props: { item: completedItem() as never, variant: "avatar" },
  });
  events.handler?.({ payload: { ...completedItem(), processingVersion: 2 } });
  await flushPromises();
  expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
  finishOld(new ArrayBuffer(8));
  await flushPromises();
  expect(URL.createObjectURL).toHaveBeenCalledTimes(1);
  expect(api.readImage).not.toHaveBeenCalled();
  wrapper.unmount();
});

it("refreshes on same-id prop changes and follows a new avatar capture id", async () => {
  const wrapper = mount(CaptureThumbnail, { props: { item: completedItem() as never, variant: "avatar" } });
  await flushPromises();
  await wrapper.setProps({ item: { ...completedItem(), processedAt: "2026-10-05T11:00:00Z" } as never });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(2);
  await wrapper.setProps({ item: completedItem("capture-2") as never });
  await flushPromises();
  const count = api.readThumbnail.mock.calls.length;
  events.handler?.({ payload: completedItem() });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(count);
  events.handler?.({ payload: { ...completedItem("capture-2"), processingVersion: 2 } });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(count + 1);
  wrapper.unmount();
});

it("removes a native listener that finishes registering after unmount", async () => {
  let finish: (unlisten: () => void) => void = () => undefined;
  events.listen.mockImplementationOnce(() => new Promise<() => void>((resolve) => { finish = resolve; }));
  const wrapper = mount(CaptureThumbnail, { props: { item: completedItem() as never } });
  wrapper.unmount();
  finish(events.unlisten);
  await flushPromises();
  expect(events.unlisten).toHaveBeenCalledTimes(1);
});

 it("ignores duplicate events and business-only changes without blanking the image", async () => {
  const item = { ...completedItem(), processingVersion: 1 };
  const wrapper = mount(CaptureThumbnail, { props: { item: item as never, variant: "avatar" } });
  await flushPromises();
  const image = wrapper.get("img").element;
  events.handler?.({ payload: item });
  await flushPromises();
  events.handler?.({ payload: { ...item, updatedAt: "2026-10-05T12:00:00Z", reviewStatus: "confirmed" } });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(1);
  expect(URL.revokeObjectURL).not.toHaveBeenCalled();
  expect(wrapper.get("img").element).toBe(image);
  events.handler?.({ payload: { ...item, updatedAt: "2026-10-05T13:00:00Z", processingVersion: 2 } });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(2);
  wrapper.unmount();
});

it("does not reload the original when derived outputs finish processing or archiving", async () => {
  const item = completedItem();
  const wrapper = mount(CaptureThumbnail, { props: { item: item as never, variant: "source" } });
  await flushPromises();
  events.handler?.({ payload: { ...item, processingVersion: 2, processedAt: "new", archivedAt: "new", updatedAt: "2026-10-05T13:00:00Z" } });
  await flushPromises();
  expect(api.readThumbnail).toHaveBeenCalledTimes(1);
  wrapper.unmount();
});
