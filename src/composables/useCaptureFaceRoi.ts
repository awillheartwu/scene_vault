import { computed, onBeforeUnmount, ref, watch, type Ref } from 'vue';
import { captureApi, type CaptureItem, type FaceRoi } from '@/lib/capture-api';
import { describeError } from '@/lib/vision-errors';

/** Request ownership follows the image, not the currently visible picker. */
export function useCaptureFaceRoi(item: Readonly<Ref<CaptureItem | null>>, update: (item: CaptureItem) => void) {
  const roi = ref<FaceRoi | null>(null);
  const error = ref('');
  const pending = ref(new Set<string>());
  const revision = ref(0);
  const failedIds = ref(new Set<string>());
  const blocked = computed(() => !!item.value && (failedIds.value.has(item.value.id) || (!!item.value.manualFaceRoiJson && item.value.manualFaceRoiReady !== 1)));
  let generation = 0;
  let disposed = false;
  const busy = computed(() => !!item.value && pending.value.has(item.value.id));
  const loading = ref(false);
  watch(() => item.value?.id, async (id) => {
    const request = ++generation;
    revision.value++;
    roi.value = null;
    error.value = '';
    loading.value = !!id;
    if (!id) return;
    try {
      const value = await captureApi.getCaptureFaceRoi(id);
      if (!disposed && request === generation) roi.value = value;
    } catch (caught) {
      if (!disposed && request === generation) { error.value = String(caught); failedIds.value.add(id); }
    } finally {
      if (request === generation) loading.value = false;
    }
  }, { immediate: true });
  watch(() => [item.value?.processingVersion, item.value?.manualFaceRoiJson, item.value?.manualFaceRoiReady] as const, () => {
    const current = item.value;
    if (!current || current.manualFaceRoiJson === undefined || pending.value.has(current.id)) return;
    try { roi.value = current.manualFaceRoiJson ? JSON.parse(current.manualFaceRoiJson) : null; } catch { error.value = '读取选区失败，请恢复自动选脸'; }
    if (!current.manualFaceRoiJson || current.manualFaceRoiReady === 1) failedIds.value.delete(current.id);
    revision.value++;
  });
  async function save(value: FaceRoi | null) {
    const id = item.value?.id;
    if (!id || pending.value.has(id) || loading.value) return;
    revision.value++;
    failedIds.value.add(id);
    roi.value = value;
    const request = generation;
    const original = item.value!;
    update({ ...original, suggestedCharacterId: null, recognitionConfidence: null, recognitionSource: null, reviewStatus: 'none' });
    pending.value.add(id);
    error.value = '';
    try {
      const updated = await captureApi.setCaptureFaceRoi(id, value);
      failedIds.value.delete(id);
      if (!disposed && request === generation && item.value?.id === id) {
        roi.value = value;
        update(updated);
      }
    } catch (caught) {
      if (!disposed && request === generation && item.value?.id === id) error.value = `更新主脸失败：${describeError(caught)}。可以重试框选，或放弃框选恢复自动选脸。`;
      try {
        const persisted = await captureApi.getCaptureFaceRoi(id);
        if (value === null && persisted === null) failedIds.value.delete(id);
        
        if (!disposed && request === generation && item.value?.id === id) roi.value = persisted;
      } catch { /* Keep the submission guard until intent can be verified. */ }
    } finally {
      pending.value.delete(id);
    }
  }
  onBeforeUnmount(() => { disposed = true; generation++; });
  return { roi, error, busy, blocked, loading, revision, save };
}
