import { useMediaQuery } from "@vueuse/core";

export const WIDE_LAYOUT_QUERY = "(min-width: 1440px)";

export function useAdaptiveLayout() {
  const isWideLayout = useMediaQuery(WIDE_LAYOUT_QUERY);
  return { isWideLayout };
}
