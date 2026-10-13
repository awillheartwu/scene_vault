import { computed, nextTick, onBeforeUnmount, ref, watch, type Ref } from "vue";
import { captureVariantReadReason } from "@/components/capture/capture-file-state";
import { captureApi, pathMimeType, type CaptureItem } from "@/lib/capture-api";
import { describeError } from "@/lib/vision-errors";
import { reportImageLoadTiming } from "@/lib/image-performance";

type PreviewVariant = "source" | "destination";

/** Recognition and ROI updates do not change the picture being displayed. */
export function useCapturePreview(
  item: Readonly<Ref<CaptureItem | null>>,
  getVariant: () => PreviewVariant,
  onError: (message: string) => void,
) {
  const url = ref<string | null>(null);
  const loading = ref(false);
  const unavailable = ref("");
  const key = computed(() => {
    const current = item.value;
    if (!current) return "";
    const variant = getVariant();
    const identity = variant === "source"
      ? [current.sourcePath, current.fileSize, current.modifiedAtMs, current.contentHash]
      : [current.destinationPath, current.archivedAt];
    return JSON.stringify([current.id, variant, ...identity, captureVariantReadReason(current, variant)]);
  });
  let generation = 0;
  let requestedKey = "";
  let currentImage: string | null = null;
  let pending: Promise<void> | null = null;
  let disposed = false;

  function clear() {
    const previous = url.value;
    url.value = null;
    if (previous) void nextTick(() => URL.revokeObjectURL(previous));
  }

  async function read(current: CaptureItem, variant: PreviewVariant, requestKey: string, request: number) {
    const isCurrent = () => !disposed && request === generation && requestKey === key.value;
    const started = performance.now();
    const metrics: Record<string, number> = {};
    let phase = "read_ms";
    let phaseStarted = started;
    let outcome: "ok" | "failed" | "cancelled" = "cancelled";
    let candidate: string | null = null;
    try {
      const bytes = await captureApi.readImage(current.id, variant);
      metrics.read_ms = performance.now() - phaseStarted;
      if (!isCurrent()) return;
      phase = "blob_ms";
      phaseStarted = performance.now();
      const path = variant === "source" ? current.sourcePath : current.destinationPath!;
      candidate = URL.createObjectURL(new Blob([bytes], { type: pathMimeType(path) }));
      metrics.blob_ms = performance.now() - phaseStarted;
      phase = "decode_ms";
      phaseStarted = performance.now();
      const image = new Image();
      image.src = candidate;
      await image.decode();
      metrics.decode_ms = performance.now() - phaseStarted;
      if (!isCurrent()) return;
      phase = "publish_ms";
      phaseStarted = performance.now();
      const previous = url.value;
      url.value = candidate;
      candidate = null;
      // Keep the old URL valid until Vue has replaced its DOM reference.
      await nextTick();
      if (previous) URL.revokeObjectURL(previous);
      metrics.publish_ms = performance.now() - phaseStarted;
      outcome = isCurrent() ? "ok" : "cancelled";
    } catch (error) {
      if (!isCurrent()) return;
      outcome = "failed";
      clear();
      unavailable.value = `无法加载图片：${describeError(error)}`;
      onError(unavailable.value);
    } finally {
      if (!(phase in metrics)) metrics[phase] = performance.now() - phaseStarted;
      metrics.total_ms = performance.now() - started;
      reportImageLoadTiming(current.id, variant, "preview", metrics, outcome);
      if (candidate) URL.revokeObjectURL(candidate);
      if (request === generation) {
        loading.value = false;
        pending = null;
      }
    }
  }

  function load(): Promise<void> {
    if (disposed) return Promise.resolve();
    const requestKey = key.value;
    if (requestKey === requestedKey) {
      if (pending) return pending;
      if (url.value) return Promise.resolve();
    }
    const current = item.value;
    const variant = getVariant();
    const image = current ? JSON.stringify([current.id, variant]) : null;
    const reason = current ? captureVariantReadReason(current, variant) : null;
    const request = ++generation;
    requestedKey = requestKey;
    if (image !== currentImage || reason || !current) clear();
    currentImage = image;
    unavailable.value = reason ?? "";
    loading.value = false;
    pending = null;
    if (!current || reason) return Promise.resolve();
    loading.value = true;
    pending = read(current, variant, requestKey, request);
    return pending;
  }

  watch(key, () => void load(), { immediate: true });
  onBeforeUnmount(() => {
    disposed = true;
    generation++;
    loading.value = false;
    clear();
  });
  return { url, loading, unavailable, load };
}
