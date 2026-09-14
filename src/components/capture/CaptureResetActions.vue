<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { CheckSquare, ChevronDown, RotateCcw, X } from '@lucide/vue';
import { captureApi, type CaptureResetInput } from '@/lib/capture-api';
import CaptureResetDialog from './CaptureResetDialog.vue';
const props = withDefaults(defineProps<{ filter: CaptureResetInput; selectedIds: string[]; selectableIds?: string[]; hasItems?: boolean; triggerless?: boolean; selectAllCandidates?: () => Promise<string[]> }>(), { hasItems: true, selectableIds: () => [] });
const selecting = defineModel<boolean>('selecting', { default: false });
const emit = defineEmits<{ updated: []; clear: []; selectAll: [ids: string[]] }>();
const input = ref<CaptureResetInput>();
const open = ref(false);
const menu = ref<HTMLDetailsElement | null>(null);
const selectingAll = ref(false);
const allCandidateIds = ref<string[] | null>(null);
// A reset job lives only while its dialog is open, so switching project or page
// simply closes the dialog instead of recovering anything.
watch(() => props.filter.projectId, () => { open.value = false; selecting.value = false; });
function onMenuPointerDown(event: PointerEvent) {
  if (menu.value && !menu.value.contains(event.target as Node)) closeMenu();
}
function onMenuFocusOut(event: FocusEvent) {
  const next = event.relatedTarget as Node | null;
  if (!menu.value || !next || !menu.value.contains(next)) closeMenu();
}
function onMenuScroll(event: Event) {
  // Scrolling the menu itself must keep it open; any other scroll detaches it.
  if (event.target instanceof Node && menu.value?.contains(event.target)) return;
  closeMenu();
}
function watchMenuDismissal() {
  window.addEventListener('pointerdown', onMenuPointerDown, true);
  window.addEventListener('focusout', onMenuFocusOut, true);
  document.addEventListener('scroll', onMenuScroll, true);
  window.addEventListener('resize', closeMenu);
  window.addEventListener('blur', closeMenu);
}
function stopWatchingMenuDismissal() {
  window.removeEventListener('pointerdown', onMenuPointerDown, true);
  window.removeEventListener('focusout', onMenuFocusOut, true);
  document.removeEventListener('scroll', onMenuScroll, true);
  window.removeEventListener('resize', closeMenu);
  window.removeEventListener('blur', closeMenu);
}
function closeMenu() { if (menu.value) menu.value.open = false; stopWatchingMenuDismissal(); }
// The native details element toggles after this click handler runs, so its
// open state is read on the next tick.
function syncMenuDismissal() { if (menu.value?.open) watchMenuDismissal(); else stopWatchingMenuDismissal(); }
function toggleMenu() { void nextTick(syncMenuDismissal); }
function preview(ids?: string[]) {
  if (!props.filter.projectId || (ids && !ids.length)) return;
  input.value = ids ? { projectId: props.filter.projectId, captureItemIds: [...ids], includePrivate: true } : { ...props.filter, captureItemIds: props.filter.captureItemIds ? [...props.filter.captureItemIds] : undefined };
  open.value = true;
  closeMenu();
}
function startSelection() { selecting.value = true; allCandidateIds.value = null; emit('clear'); closeMenu(); }
function cancelSelection() { selecting.value = false; emit('clear'); }
function finishSelection() {
  if (!props.selectedIds.length) { cancelSelection(); return; }
  preview(props.selectedIds);
}
async function toggleAll() {
  if (selectingAll.value) return;
  if (allSelected.value) { emit('clear'); return; }
  selectingAll.value = true;
  const context = JSON.stringify(props.filter);
  try {
    const ids = props.selectAllCandidates ? await props.selectAllCandidates() : await captureApi.listCaptureResetCandidates(props.filter);
    if (!selecting.value || context !== JSON.stringify(props.filter)) return;
    allCandidateIds.value = ids;
    emit('selectAll', ids);
  } catch { /* 读取失败时保持当前选择，用户可以再点一次 */ }
  finally { selectingAll.value = false; }
}
watch(() => JSON.stringify(props.filter), () => { allCandidateIds.value = null; });
const allSelected = computed(() => !!allCandidateIds.value?.length && allCandidateIds.value.every(id => props.selectedIds.includes(id)));
function started() { selecting.value = false; emit('clear'); }
defineExpose({ preview, startSelection });
onBeforeUnmount(() => stopWatchingMenuDismissal());
</script>
<template>
  <div class="reset-actions" aria-label="图片管理">
    <template v-if="selecting">
      <span class="reset-selection-count">已选 {{ selectedIds.length }} 张</span>
      <button type="button" class="reset-button" :disabled="selectingAll" @click="toggleAll"><CheckSquare :size="14" />{{ selectingAll ? '读取中…' : allSelected ? '取消全选' : '全选筛选结果' }}</button>
      <button type="button" class="reset-button reset-selected" :disabled="selectingAll" @click="finishSelection"><RotateCcw :size="14" />结束多选</button>
      <button type="button" class="reset-button" @click="cancelSelection"><X :size="14" />取消</button>
    </template>
    <details v-else-if="!triggerless && hasItems" ref="menu" class="reset-menu" @click="toggleMenu" @toggle="syncMenuDismissal" @keydown.esc.stop="closeMenu">
      <summary class="reset-button">管理<ChevronDown :size="13" /></summary>
      <div class="reset-menu-content">
        <button type="button" :disabled="!filter.projectId" @click="startSelection"><CheckSquare :size="14" />撤销图片</button>
      </div>
    </details>
    <CaptureResetDialog v-if="open" :input="input" @close="open = false" @updated="emit('updated')" @started="started" />
  </div>
</template>
<style scoped>
.reset-actions { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
.reset-button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; min-height: 32px; padding: 5px 10px; border: 1px solid var(--border); border-radius: 7px; background: var(--secondary); color: var(--muted-foreground); font-size: 12px; line-height: 20px; white-space: nowrap; cursor: pointer; }
.reset-button:hover, .reset-button.active { color: var(--foreground); border-color: color-mix(in srgb, var(--accent) 45%, var(--border)); }
.reset-selected { color: var(--accent); background: color-mix(in srgb, var(--accent) 8%, var(--card)); }
.reset-selection-count { color: var(--muted-foreground); font-size: 12px; white-space: nowrap; }
.reset-menu { position: relative; }
.reset-menu summary { list-style: none; }
.reset-menu summary::-webkit-details-marker { display: none; }
.reset-menu-content { position: absolute; left: 0; top: calc(100% + 6px); z-index: 30; min-width: 210px; max-height: 280px; overflow: auto; padding: 5px; border: 1px solid var(--border); border-radius: 9px; background: var(--card); box-shadow: var(--card-shadow); }
.reset-menu-content button { display: flex; gap: 7px; align-items: center; width: 100%; padding: 9px 10px; border: 0; border-radius: 5px; background: transparent; color: var(--foreground); font-size: 12px; text-align: left; cursor: pointer; }
.reset-menu-content button:hover { background: var(--secondary); }
button:disabled { opacity: .45; cursor: not-allowed; }
button:focus-visible, summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
