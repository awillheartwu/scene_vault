import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  readThumbnail: vi.fn(),
  readImage: vi.fn(),
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  pathMimeType: () => "image/png",
}));

import CaptureThumbnail from "./CaptureThumbnail.vue";

beforeEach(() => {
  api.readThumbnail.mockReset().mockResolvedValue(new ArrayBuffer(4));
  api.readImage.mockReset().mockResolvedValue(new ArrayBuffer(4));
  vi.stubGlobal("URL", {
    createObjectURL: vi.fn(() => "blob:thumbnail"),
    revokeObjectURL: vi.fn(),
  });
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
