<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { ScanFace, Check, X, RotateCcw, LoaderCircle } from '@lucide/vue';
import type { FaceRoi } from '@/lib/capture-api';
import { normalizeFaceBox, type FaceBox } from '@/lib/face-box';
const props = defineProps<{ imageUrl: string | null; itemId: string; modelValue: FaceRoi | null; faceBox?: FaceBox | null; busy?: boolean; loading?: boolean; disabled?: boolean; error?: string; inline?: boolean }>();
const emit = defineEmits<{ confirm: [roi: FaceRoi | null] }>();
const editing = defineModel<boolean>('editing', { default: false });
const trigger = ref<HTMLButtonElement | null>(null);
const keyboardBox = ref<HTMLElement | null>(null);
const draft = ref<FaceRoi | null>(null);
const image = ref<HTMLImageElement | null>(null);
const bounds = ref({left:0,top:0,width:0,height:0});
let observer: ResizeObserver | null = null;
let start: { x: number; y: number } | null = null;
function measure() {
  const img=image.value;
  if (!img) return;
  const rect=img.getBoundingClientRect();
  const scale=img.naturalWidth && img.naturalHeight ? Math.min(rect.width/img.naturalWidth,rect.height/img.naturalHeight) : 1;
  const width=img.naturalWidth ? img.naturalWidth*scale : rect.width;
  const height=img.naturalHeight ? img.naturalHeight*scale : rect.height;
  bounds.value={left:(rect.width-width)/2,top:(rect.height-height)/2,width,height};
}
watch(image, img=>{observer?.disconnect();if(img && typeof ResizeObserver !== 'undefined'){observer=new ResizeObserver(measure);observer.observe(img);}void nextTick(measure);});
async function toggle() {
  editing.value = !editing.value;
  draft.value = props.modelValue ? { ...props.modelValue } : defaultDraft();
  measure();
  await nextTick();
  if (editing.value) keyboardBox.value?.focus();
}
// Framing starts on the face the last scan picked, so the user only adjusts it.
function defaultDraft(): FaceRoi {
  return normalizeFaceBox(props.faceBox, image.value?.naturalWidth ?? 0, image.value?.naturalHeight ?? 0)
    ?? { x: .25, y: .25, width: .5, height: .5 };
}
function cancel() { editing.value = false; start = null; void nextTick(() => trigger.value?.focus()); }
function key(event: KeyboardEvent) {
  if (event.key === 'Escape') { event.preventDefault(); cancel(); return; }
  if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key) || props.busy || props.loading || props.disabled) return;
  event.preventDefault();
  const box = draft.value ?? { x: .25, y: .25, width: .5, height: .5 };
  const dx = event.key === 'ArrowRight' ? .01 : event.key === 'ArrowLeft' ? -.01 : 0;
  const dy = event.key === 'ArrowDown' ? .01 : event.key === 'ArrowUp' ? -.01 : 0;
  draft.value = event.shiftKey
    ? { ...box, width: Math.max(.01, Math.min(1 - box.x, box.width + dx)), height: Math.max(.01, Math.min(1 - box.y, box.height + dy)) }
    : { ...box, x: Math.max(0, Math.min(1 - box.width, box.x + dx)), y: Math.max(0, Math.min(1 - box.height, box.y + dy)) };
}
watch(() => props.itemId, () => { editing.value = false; draft.value = null; start = null; });
watch(() => props.modelValue, value => { draft.value = value ? { ...value } : null; });
const visibleBox=computed(()=>editing.value ? draft.value : props.modelValue);
const boxStyle = computed(() => visibleBox.value ? {
  left: `${visibleBox.value.x * 100}%`, top: `${visibleBox.value.y * 100}%`,
  width: `${visibleBox.value.width * 100}%`, height: `${visibleBox.value.height * 100}%`,
} : {});
const areaStyle=computed(()=>({left:`${bounds.value.left}px`,top:`${bounds.value.top}px`,width:`${bounds.value.width}px`,height:`${bounds.value.height}px`}));
function point(event: PointerEvent) {
  measure();
  const rect = image.value?.getBoundingClientRect();
  const b=bounds.value;
  if (!rect || !b.width || !b.height) return null;
  return { x: Math.max(0, Math.min(1, (event.clientX - rect.left-b.left) / b.width)), y: Math.max(0, Math.min(1, (event.clientY - rect.top-b.top) / b.height)) };
}
function down(event: PointerEvent) {
  if (!editing.value || props.busy || props.loading || props.disabled || event.button !== 0) return;
  start = point(event);
  draft.value = null;
  (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
}
function move(event: PointerEvent) {
  const end = point(event);
  if (!start || !end) return;
  draft.value = { x: Math.min(start.x, end.x), y: Math.min(start.y, end.y), width: Math.abs(start.x - end.x), height: Math.abs(start.y - end.y) };
}
function confirm(value: FaceRoi | null) { emit('confirm', value); cancel(); }
onBeforeUnmount(()=>observer?.disconnect());
</script>
<template>
  <div class="face-roi-selector" :class="{ inline, editing }" @keydown.stop>
    <img ref="image" :src="imageUrl || undefined" alt="截图预览，可框选主脸" draggable="false" @load="measure" />
    <div v-if="editing || modelValue" ref="keyboardBox" class="roi-image" :style="areaStyle" :tabindex="editing ? 0 : -1" role="group" aria-label="主脸选区：方向键移动，Shift 加方向键调整大小" @keydown="key" @pointerdown.prevent="down" @pointermove="move" @pointerup="move($event); start = null" @pointercancel="start = null">
      <span v-if="visibleBox" class="roi-box" :style="boxStyle"><span class="roi-caption">主脸范围</span></span>
    </div>
    <div class="roi-toolbar">
      <span v-if="busy || loading" class="roi-status" role="status"><LoaderCircle :size="14" class="animate-spin" />{{ busy ? '识别中…' : '读取选区…' }}</span>
      <template v-if="editing">
        <button type="button" class="roi-confirm" :disabled="busy || loading || disabled || !draft || draft.width <= 0 || draft.height <= 0" @click="confirm(draft)"><Check :size="14" />确认主脸</button>
        <button type="button" @click="cancel"><X :size="14" />取消</button>
      </template>
      <button v-else ref="trigger" type="button" :disabled="busy || loading || disabled || !imageUrl" :aria-expanded="editing" @click="toggle"><ScanFace :size="15" />{{ modelValue ? '重新框选' : '框选主脸' }}</button>
      <button v-if="modelValue || error" type="button" :disabled="busy || loading || disabled" title="放弃框选，恢复自动选脸" aria-label="放弃框选，恢复自动选脸" @click="confirm(null)"><RotateCcw :size="14" /></button>
    </div>
    <p v-if="editing" class="roi-hint">拖动框选一张脸 · 方向键微调 · Shift 调整大小</p>
    <p v-if="error" role="alert" class="roi-message error">{{ error }}</p>
  </div>
</template>
<style scoped>
.face-roi-selector { position: relative; min-height: 220px; height: 40vh; width: 100%; overflow: hidden; background: var(--background); }
.face-roi-selector.inline { position: absolute; inset: 0; min-height: 0; width: 100%; height: 100%; background: transparent; }
.face-roi-selector > img { display: block; width: 100%; height: 100%; object-fit: contain; user-select: none; }
.roi-image { position: absolute; touch-action: none; pointer-events: none; }
.editing .roi-image { pointer-events: auto; cursor: crosshair; }
.roi-box { position: absolute; border: 2px solid var(--accent); background: color-mix(in srgb, var(--accent) 9%, transparent); box-sizing: border-box; pointer-events: none; }
.editing .roi-box { box-shadow: 0 0 0 2000px #0005; }
.roi-caption { position: absolute; top: 0; left: 0; padding: 3px 6px; background: var(--accent); color: var(--accent-foreground); font-size: 10px; line-height: 16px; white-space: nowrap; }
.roi-toolbar { position: absolute; right: 12px; top: 12px; z-index: 5; display: flex; align-items: center; gap: 5px; padding: 4px; border: 1px solid var(--border); border-radius: 9px; background: color-mix(in srgb, var(--card) 95%, transparent); box-shadow: var(--card-shadow); }
.roi-toolbar button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; min-height: 30px; padding: 5px 8px; border: 0; border-radius: 5px; background: transparent; color: var(--foreground); font-size: 12px; cursor: pointer; }
.roi-toolbar button:hover { background: var(--secondary); }
.roi-toolbar .roi-confirm { background: var(--accent); color: var(--accent-foreground); }
.roi-toolbar button:disabled { opacity: .45; cursor: not-allowed; }
.roi-toolbar button:focus-visible, .roi-image:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.roi-status { display: flex; align-items: center; gap: 5px; padding: 0 5px; color: var(--muted-foreground); font-size: 11px; }
.roi-message { position: absolute; z-index: 5; left: 12px; bottom: 70px; max-width: calc(100% - 24px); padding: 7px 10px; margin: 0; border: 1px solid var(--border); border-radius: 7px; background: var(--card); color: var(--muted-foreground); font-size: 11px; }
.roi-message.error { color: var(--warn); }
/* The hint stays out of the way in the top-left corner: it must never block a
   drag (pointer-events) nor be replaced by a failure message. */
.roi-hint { position: absolute; z-index: 5; left: 12px; top: 12px; max-width: min(62%, 420px); padding: 6px 9px; margin: 0; border: 1px solid var(--border); border-radius: 7px; background: color-mix(in srgb, var(--card) 90%, transparent); color: var(--muted-foreground); font-size: 11px; pointer-events: none; }
</style>
