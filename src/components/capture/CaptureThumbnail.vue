<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ImageOff } from "@lucide/vue";
import { captureApi, pathMimeType, type CaptureItem } from "@/lib/capture-api";

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
  if (url.value) URL.revokeObjectURL(url.value);
  url.value = null;
  const variants = props.fallbackVariant
    ? [props.variant, props.fallbackVariant]
    : [props.variant];
  for (const variant of variants) {
    try {
      let bytes: ArrayBuffer;
      try {
        bytes = imageSize === "full"
          ? await captureApi.readImage(props.item.id, variant)
          : await captureApi.readThumbnail(props.item.id, variant);
      } catch (error) {
        if (imageSize === "full") throw error;
        bytes = await captureApi.readImage(props.item.id, variant);
      }
      if (version !== requestVersion) return;
      const nextUrl = URL.createObjectURL(
        new Blob([bytes], {
          type: imageSize === "full"
            ? pathMimeType(props.item.sourcePath)
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
watch(
  () => [
    props.item.id,
    props.variant,
    props.size,
    props.fullWidthThreshold,
    props.fallbackVariant,
    automaticSize.value,
  ] as const,
  () => {
    requestVersion += 1;
    if (requested) void loadImage();
  },
);
onBeforeUnmount(() => {
  requestVersion += 1;
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
