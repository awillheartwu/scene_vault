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
  size?: "thumb" | "full";
  alt?: string;
}>(), { variant: "source", size: "thumb", alt: "截图缩略图" });
const root = ref<HTMLElement | null>(null);
const url = ref<string | null>(null);
let observer: IntersectionObserver | null = null;
let requested = false;
let requestVersion = 0;

async function loadImage() {
  const version = ++requestVersion;
  if (url.value) URL.revokeObjectURL(url.value);
  url.value = null;
  const variants = props.fallbackVariant
    ? [props.variant, props.fallbackVariant]
    : [props.variant];
  for (const variant of variants) {
    try {
      let bytes: ArrayBuffer;
      try {
        bytes = props.size === "full"
          ? await captureApi.readImage(props.item.id, variant)
          : await captureApi.readThumbnail(props.item.id, variant);
      } catch (error) {
        if (props.size === "full") throw error;
        bytes = await captureApi.readImage(props.item.id, variant);
      }
      if (version !== requestVersion) return;
      const nextUrl = URL.createObjectURL(
        new Blob([bytes], {
          type: props.size === "full"
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
  () => [props.item.id, props.variant, props.size, props.fallbackVariant] as const,
  () => {
    requestVersion += 1;
    if (requested) void loadImage();
  },
);
onBeforeUnmount(() => {
  requestVersion += 1;
  observer?.disconnect();
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
