<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { Star, UserRound } from "@lucide/vue";
import type { AnnotationSettings } from "@/lib/capture-api";

/**
 * Visual picker for the annotation text position around the detected face.
 *
 * A circle represents the face; four clickable stars around it are snap
 * targets for the discrete directions (`above` / `right` / `below` / `left`).
 * The round marker can be dragged (or the stage clicked) to pick a fully
 * custom position, stored as `textOffsetX/Y` in face-box units.
 *
 * Engine fallback order stays fixed (above -> right -> below -> left) and is
 * shown as colored rank badges on the stars.
 */

const FACE_POSITIONS = [
  { value: "above", label: "上方", fx: 0.5, fy: 0.08 },
  { value: "right", label: "右侧", fx: 0.92, fy: 0.5 },
  { value: "below", label: "下方", fx: 0.5, fy: 0.92 },
  { value: "left", label: "左侧", fx: 0.08, fy: 0.5 },
] as const;

const FALLBACK_CHAIN: readonly string[] = ["above", "right", "below", "left"];

/** Star centers sit at offset +-1.2; this maps offsets to stage pixels. */
const OFFSET_SCALE = 0.35;
const MAX_OFFSET = 1.3;
const SNAP_RADIUS = 20;
const DRAG_THRESHOLD = 4;

const props = defineProps<{
  modelValue: AnnotationSettings;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: AnnotationSettings): void;
}>();

const stageRef = ref<HTMLElement | null>(null);
const stageSize = ref({ w: 260, h: 236 });
const markerRef = ref<HTMLElement | null>(null);

interface DragState {
  active: boolean;
  moved: boolean;
  startX: number;
  startY: number;
  grabbed: "star" | "marker" | null;
  grabbedStar: string | null;
}

const drag = ref<DragState>({
  active: false,
  moved: false,
  startX: 0,
  startY: 0,
  grabbed: null,
  grabbedStar: null,
});

let lastPointerUpAt = 0;

const position = computed<string>(() => props.modelValue.faceTextPosition || "above");
const isCustom = computed(() => position.value === "custom");
const offsetX = computed(() => props.modelValue.textOffsetX);
const offsetY = computed(() => props.modelValue.textOffsetY);

/** 1-based rank for a star: 1 when it is the discrete primary, else chain order. */
function starRank(value: string): number {
  if (!isCustom.value) {
    const chain = [position.value, ...FALLBACK_CHAIN.filter((d) => d !== position.value)];
    return chain.indexOf(value) + 1;
  }
  return FALLBACK_CHAIN.indexOf(value) + 2;
}

function starPoint(value: string, w: number, h: number): { x: number; y: number } {
  const star = FACE_POSITIONS.find((p) => p.value === value);
  return star ? { x: star.fx * w, y: star.fy * h } : { x: w / 2, y: h / 2 };
}

function offsetFromPoint(px: number, py: number, w: number, h: number) {
  const scaleX = w * OFFSET_SCALE;
  const scaleY = h * OFFSET_SCALE;
  return {
    x: clamp((px - w / 2) / scaleX, -MAX_OFFSET, MAX_OFFSET),
    y: clamp((py - h / 2) / scaleY, -MAX_OFFSET, MAX_OFFSET),
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function round2(value: number): number {
  return Math.round(value * 100) / 100;
}

function nearestStar(px: number, py: number, w: number, h: number): string | null {
  let best: string | null = null;
  let bestDistance = SNAP_RADIUS;
  for (const star of FACE_POSITIONS) {
    const point = starPoint(star.value, w, h);
    const distance = Math.hypot(px - point.x, py - point.y);
    if (distance < bestDistance) {
      bestDistance = distance;
      best = star.value;
    }
  }
  return best;
}

function eventPoint(event: PointerEvent): { x: number; y: number; w: number; h: number } | null {
  const rect = stageRef.value?.getBoundingClientRect();
  if (!rect || rect.width === 0 || rect.height === 0) return null;
  return {
    x: event.clientX - rect.left,
    y: event.clientY - rect.top,
    w: rect.width,
    h: rect.height,
  };
}

function setDiscrete(value: string) {
  emit("update:modelValue", {
    ...props.modelValue,
    faceTextPosition: value,
    textOffsetX: null,
    textOffsetY: null,
  });
}

function setCustom(x: number, y: number) {
  emit("update:modelValue", {
    ...props.modelValue,
    faceTextPosition: "custom",
    textOffsetX: x,
    textOffsetY: y,
  });
}

function onStarClick(value: string) {
  if (Date.now() - lastPointerUpAt < 300) return;
  setDiscrete(value);
}

function onPointerDown(event: PointerEvent) {
  if (event.button !== 0) return;
  const point = eventPoint(event);
  if (!point) return;
  const target = event.target as HTMLElement;
  const starButton = target.closest?.(".pos-btn") as HTMLElement | null;
  drag.value = {
    active: true,
    moved: false,
    startX: point.x,
    startY: point.y,
    grabbed: starButton ? "star" : target.closest?.(".marker") ? "marker" : null,
    grabbedStar: starButton?.dataset?.position ?? null,
  };
  try {
    stageRef.value?.setPointerCapture?.(event.pointerId);
  } catch {
    /* pointer capture is optional */
  }
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
}

function onPointerMove(event: PointerEvent) {
  if (!drag.value.active) return;
  const point = eventPoint(event);
  if (!point) return;
  const moved = Math.hypot(point.x - drag.value.startX, point.y - drag.value.startY);
  if (!drag.value.moved && moved < DRAG_THRESHOLD) return;
  if (!drag.value.moved) {
    drag.value.moved = true;
  }
  const offset = offsetFromPoint(point.x, point.y, point.w, point.h);
  setCustom(round2(offset.x), round2(offset.y));
}

function onPointerUp(event: PointerEvent) {
  if (!drag.value.active) return;
  lastPointerUpAt = Date.now();
  const state = drag.value;
  drag.value = { ...drag.value, active: false };
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);

  const point = eventPoint(event);
  if (!point) return;

  if (!state.moved) {
    if (state.grabbed === "star" && state.grabbedStar) {
      setDiscrete(state.grabbedStar);
    } else if (state.grabbed === null) {
      // Plain click on the stage: place the marker there.
      const offset = offsetFromPoint(point.x, point.y, point.w, point.h);
      setCustom(round2(offset.x), round2(offset.y));
    }
    return;
  }

  const snap = nearestStar(point.x, point.y, point.w, point.h);
  if (snap) {
    setDiscrete(snap);
  }
}

function onMarkerKeydown(event: KeyboardEvent) {
  const step = event.shiftKey ? 0.25 : 0.1;
  let x = offsetX.value ?? 0;
  let y = offsetY.value ?? 0;
  if (!isCustom.value) {
    const point = starPoint(position.value, stageSize.value.w, stageSize.value.h);
    const offset = offsetFromPoint(point.x, point.y, stageSize.value.w, stageSize.value.h);
    x = offset.x;
    y = offset.y;
  }
  switch (event.key) {
    case "ArrowLeft":
      x -= step;
      break;
    case "ArrowRight":
      x += step;
      break;
    case "ArrowUp":
      y -= step;
      break;
    case "ArrowDown":
      y += step;
      break;
    default:
      return;
  }
  event.preventDefault();
  setCustom(round2(clamp(x, -MAX_OFFSET, MAX_OFFSET)), round2(clamp(y, -MAX_OFFSET, MAX_OFFSET)));
}

function measureStage() {
  const rect = stageRef.value?.getBoundingClientRect();
  if (rect && rect.width > 0 && rect.height > 0) {
    stageSize.value = { w: rect.width, h: rect.height };
  }
}

let resizeObserver: ResizeObserver | null = null;

onMounted(() => {
  measureStage();
  if (typeof ResizeObserver !== "undefined" && stageRef.value) {
    resizeObserver = new ResizeObserver(measureStage);
    resizeObserver.observe(stageRef.value);
  }
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
});

/** Marker position in stage pixels. */
const markerPoint = computed(() => {
  const { w, h } = stageSize.value;
  if (isCustom.value && offsetX.value !== null && offsetY.value !== null) {
    return {
      x: w / 2 + offsetX.value * w * OFFSET_SCALE,
      y: h / 2 + offsetY.value * h * OFFSET_SCALE,
    };
  }
  return starPoint(position.value, w, h);
});

const primaryLabel = computed(() => {
  if (isCustom.value) {
    const x = offsetX.value ?? 0;
    const y = offsetY.value ?? 0;
    return `自定义位置（横向 ${x >= 0 ? "+" : ""}${x.toFixed(1)} · 纵向 ${y >= 0 ? "+" : ""}${y.toFixed(1)}）`;
  }
  return FACE_POSITIONS.find((p) => p.value === position.value)?.label ?? "上方";
});

const chainPreview = computed(() => {
  const names: Record<string, string> = {
    above: "上方",
    right: "右侧",
    below: "下方",
    left: "左侧",
  };
  const chain = isCustom.value
    ? [...FALLBACK_CHAIN]
    : [position.value, ...FALLBACK_CHAIN.filter((d) => d !== position.value)];
  return chain.map((d) => names[d]).join(" → ");
});
</script>

<template>
  <div class="face-position-picker">
    <div
      ref="stageRef"
      class="picker-stage"
      @pointerdown="onPointerDown"
    >
      <div class="face-circle" aria-hidden="true">
        <UserRound :size="34" stroke-width="1.5" />
      </div>
      <button
        v-for="star in FACE_POSITIONS"
        :key="star.value"
        type="button"
        class="pos-btn"
        :data-position="star.value"
        :class="{ selected: !isCustom && position === star.value }"
        :style="{
          '--fx': `${star.fx * 100}%`,
          '--fy': `${star.fy * 100}%`,
          '--rank': starRank(star.value),
        }"
        :title="
          !isCustom && position === star.value
            ? `${star.label}（首选）`
            : `${star.label}（降级 ${starRank(star.value) - 1}）`
        "
        :aria-pressed="!isCustom && position === star.value"
        @click="onStarClick(star.value)"
      >
        <Star class="pos-icon" :size="18" :fill="!isCustom && position === star.value ? 'currentColor' : 'none'" />
        <span class="rank-badge">{{ starRank(star.value) }}</span>
      </button>
      <div
        ref="markerRef"
        class="marker"
        role="slider"
        tabindex="0"
        :aria-label="`文字位置：${isCustom ? '自定义' : primaryLabel}`"
        :style="{
          left: `${markerPoint.x}px`,
          top: `${markerPoint.y}px`,
        }"
        title="拖动或按方向键微调（Shift 加快）"
        @keydown="onMarkerKeydown"
      >
        <span v-if="isCustom" class="marker-badge">1</span>
      </div>
    </div>
    <p class="picker-hint">
      首选 <strong>{{ primaryLabel }}</strong>；放不下时按顺序降级：{{ chainPreview }}
    </p>
    <p class="picker-caption">点星星选方向 · 拖动圆点或点击空白自由定位 · 方向键微调（Shift 加快）</p>
  </div>
</template>

<style scoped>
.face-position-picker {
  width: 100%;
  max-width: 260px;
}

.picker-stage {
  position: relative;
  width: 100%;
  height: 236px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--card) 60%, transparent);
  touch-action: none;
  user-select: none;
}

.face-circle {
  position: absolute;
  top: 50%;
  left: 50%;
  width: 84px;
  height: 84px;
  margin: -42px 0 0 -42px;
  border-radius: 9999px;
  border: 2px solid var(--primary);
  background: color-mix(in srgb, var(--primary) 8%, var(--card));
  color: color-mix(in srgb, var(--primary) 75%, var(--foreground));
  display: flex;
  align-items: center;
  justify-content: center;
  pointer-events: none;
}

.pos-btn {
  position: absolute;
  top: var(--fy);
  left: var(--fx);
  margin: -17px 0 0 -17px;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border-radius: 9999px;
  border: 1px solid var(--border);
  background: var(--card);
  color: color-mix(in srgb, var(--primary) calc(var(--rank) * 22%), var(--muted-foreground));
  cursor: pointer;
  transition:
    transform 120ms ease,
    box-shadow 120ms ease,
    border-color 120ms ease,
    color 120ms ease;
}

.pos-btn:hover {
  transform: scale(1.1);
  border-color: var(--primary);
}

.pos-btn.selected {
  border-color: var(--primary);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 25%, transparent);
  color: var(--primary);
}

.pos-icon {
  pointer-events: none;
}

.rank-badge {
  position: absolute;
  top: -5px;
  right: -5px;
  min-width: 15px;
  height: 15px;
  border-radius: 9999px;
  background: var(--primary);
  color: var(--primary-foreground);
  font-size: 10px;
  line-height: 15px;
  text-align: center;
  padding: 0 3px;
}

.marker {
  position: absolute;
  width: 22px;
  height: 22px;
  margin: -11px 0 0 -11px;
  border-radius: 9999px;
  background: var(--primary);
  border: 3px solid color-mix(in srgb, var(--primary) 30%, var(--card));
  box-shadow: 0 2px 8px rgb(0 0 0 / 0.35);
  cursor: grab;
  z-index: 2;
}

.marker:active {
  cursor: grabbing;
}

.marker:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 3px;
}

.marker-badge {
  position: absolute;
  top: -7px;
  right: -7px;
  min-width: 15px;
  height: 15px;
  border-radius: 9999px;
  background: var(--foreground);
  color: var(--background);
  font-size: 10px;
  line-height: 15px;
  text-align: center;
  padding: 0 3px;
}

.picker-hint {
  margin-top: 8px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--muted-foreground);
}

.picker-caption {
  margin-top: 2px;
  font-size: 10.5px;
  line-height: 1.5;
  color: color-mix(in srgb, var(--muted-foreground) 80%, transparent);
}
</style>
