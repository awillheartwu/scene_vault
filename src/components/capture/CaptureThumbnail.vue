<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { ImageOff } from "@lucide/vue";
import { captureApi, pathMimeType, type CaptureItem } from "@/lib/capture-api";
import { subscribeCaptureImageUpdates } from "@/lib/capture-image-updates";
import { captureVariantReadReason } from "./capture-file-state";

const props = withDefaults(defineProps<{
  item: CaptureItem;
  variant?: "source" | "annotated" | "avatar" | "destination";
  /** Tries this variant when the primary one cannot be read (e.g. no avatar). */
  fallbackVariant?: "source" | "annotated" | "avatar" | "destination";
  /** "thumb" loads the cached 320px JPEG; "full" reads the original file. */
  size?: "thumb" | "full" | "auto";
  /** In auto mode, switch to the original once the rendered width exceeds this value. */
  fullWidthThreshold?: number;
  alt?: string;
}>(), {
  variant: "source",
  size: "thumb",
  fullWidthThreshold: 360,
  alt: "截图缩略图",
});
const root = ref<HTMLElement | null>(null);
const url = ref<string | null>(null);
const automaticSize = ref<"thumb" | "full">("thumb");
let observer: IntersectionObserver | null = null;
let resizeObserver: ResizeObserver | null = null;
let requested = false;
let unsubscribe: (() => void) | undefined;
const eventItem = shallowRef<CaptureItem | null>(null);
const imageItem = computed(() => {
  const incoming = eventItem.value;
  if (!incoming || incoming.id !== props.item.id) return props.item;
  // Parent lists can lag behind the completion event; avatar stubs have no date.
  return Date.parse(props.item.updatedAt) > Date.parse(incoming.updatedAt)
    ? props.item : incoming;
});
watch(() => props.item.id, (id) => {
  unsubscribe?.();
  eventItem.value = null;
  unsubscribe = subscribeCaptureImageUpdates(id, (item) => {
    const previous = imageItem.value;
    if (Date.parse(item.updatedAt) < Date.parse(previous.updatedAt)) return;
    eventItem.value = item;
  });
}, { immediate: true });
let requestVersion = 0;

function resolvedSize(): "thumb" | "full" {
  return props.size === "auto" ? automaticSize.value : props.size;
}

function updateAutomaticSize(width: number) {
  if (props.size !== "auto") return;
  automaticSize.value = width > props.fullWidthThreshold ? "full" : "thumb";
}

async function loadImage() {
  const version = ++requestVersion;
  const imageSize = resolvedSize();
  const item = imageItem.value;
  if (url.value) URL.revokeObjectURL(url.value);
  url.value = null;
  const variants = props.fallbackVariant
    ? [props.variant, props.fallbackVariant]
    : [props.variant];
  for (const variant of variants) {
    // Known-missing, replaced or unreachable files must not trigger pointless
    // image reads; the backend state is authoritative when present.
    if (captureVariantReadReason(item, variant)) continue;
    try {
      let bytes: ArrayBuffer;
      try {
        bytes = imageSize === "full"
          ? await captureApi.readImage(item.id, variant)
          : await captureApi.readThumbnail(item.id, variant);
      } catch (error) {
        if (version !== requestVersion) return;
        if (imageSize === "full") throw error;
        bytes = await captureApi.readImage(item.id, variant);
      }
      if (version !== requestVersion) return;
      const nextUrl = URL.createObjectURL(
        new Blob([bytes], {
          type: imageSize === "full"
            ? pathMimeType(item.sourcePath)
            : "image/jpeg",
        }),
      );
      if (version !== requestVersion) {
        URL.revokeObjectURL(nextUrl);
        return;
      }
      url.value = nextUrl;
      return;
    } catch {
      if (version !== requestVersion) return;
      /* try the next variant */
    }
  }
}

function requestImage() {
  if (requested) return;
  requested = true;
  observer?.disconnect();
  observer = null;
  void loadImage();
}

onMounted(() => {
  updateAutomaticSize(root.value?.getBoundingClientRect().width ?? 0);
  if (props.size === "auto" && typeof ResizeObserver !== "undefined" && root.value) {
    resizeObserver = new ResizeObserver((entries) => {
      const width = entries.find((entry) => entry.target === root.value)?.contentRect.width;
      if (width != null) updateAutomaticSize(width);
    });
    resizeObserver.observe(root.value);
  }
  if (typeof IntersectionObserver === "undefined" || !root.value) {
    requestImage();
    return;
  }
  observer = new IntersectionObserver(
    (entries) => {
      if (entries.some((entry) => entry.isIntersecting)) requestImage();
    },
    { rootMargin: "80px" },
  );
  observer.observe(root.value);
});
// Business updates and duplicate events do not change the actual picture.
// Compare only the file identity/readability of the requested variants.
function variantKey(item: CaptureItem, variant: "source" | "annotated" | "avatar" | "destination") {
  const readable = captureVariantReadReason(item, variant);
  if (variant === "source") {
    return [variant, item.sourcePath, item.fileSize, item.modifiedAtMs, item.contentHash, readable];
  }
  if (variant === "destination") {
    return [variant, item.destinationPath, item.archivedAt, readable];
  }
  const local = variant === "avatar" ? item.avatarPath : item.annotatedPath;
  return [variant, local, variant === "avatar" ? item.destinationAvatarPath : null,
    item.processedAt, item.processingVersion,
    variant === "avatar" && !local ? item.archivedAt : null, readable];
}
const imageKey = computed(() => {
  const item = imageItem.value;
  return JSON.stringify([item.id, resolvedSize(), variantKey(item, props.variant),
    props.fallbackVariant ? variantKey(item, props.fallbackVariant) : null]);
});
watch(imageKey, () => {
  requestVersion += 1;
  if (requested) void loadImage();
});
onBeforeUnmount(() => {
  requestVersion += 1;
  unsubscribe?.();
  observer?.disconnect();
  resizeObserver?.disconnect();
  if (url.value) URL.revokeObjectURL(url.value);
});
</script>

<template>
  <div ref="root" class="h-full w-full">
    <img
      v-if="url"
      :src="url"
      :alt="alt"
      class="h-full w-full object-cover"
      decoding="async"
      loading="lazy"
    />
    <div v-else class="grid h-full w-full place-items-center bg-white/5 text-slate-600">
      <ImageOff :size="22" aria-hidden="true" />
    </div>
  </div>
</template>
