import { beforeEach, vi } from "vitest";

// Model the app's initial viewport in the test environment. Narrow-layout
// cases override matchMedia explicitly rather than changing production code.
beforeEach(() => {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    writable: true,
    value: 1440,
  });
  window.matchMedia = vi.fn((query: string): MediaQueryList => {
    const min = query.match(/\(min-width:\s*(\d+)px\)/);
    const max = query.match(/\(max-width:\s*(\d+)px\)/);
    return {
      matches:
        Boolean(min || max) &&
        (!min || window.innerWidth >= Number(min[1])) &&
        (!max || window.innerWidth <= Number(max[1])),
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(() => true),
    };
  });
});
