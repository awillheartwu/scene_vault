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
