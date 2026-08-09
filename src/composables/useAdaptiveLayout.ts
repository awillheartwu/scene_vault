import { ref } from "vue";
import { useMediaQuery } from "@vueuse/core";

export const WIDE_LAYOUT_QUERY = "(min-width: 1440px)";

export function useAdaptiveLayout() {
  // jsdom and static renderers do not expose matchMedia. Treat them as the
  // wide desktop layout so all content remains in the accessibility tree;
  // real WebView2 windows always provide matchMedia.
  const isWideLayout =
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? useMediaQuery(WIDE_LAYOUT_QUERY)
      : ref(true);
  return { isWideLayout };
}
