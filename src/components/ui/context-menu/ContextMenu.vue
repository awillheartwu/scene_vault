<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from "vue";
import type { ContextMenuController } from "@/composables/useContextMenu";

const props = defineProps<{ menu: ContextMenuController }>();

const root = ref<HTMLElement | null>(null);
const itemEls = ref<(HTMLElement | null)[]>([]);
const focusedIndex = ref(0);
const GAP = 8;

function close() {
  props.menu.close();
}

function clampPosition() {
  const el = root.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const maxX = window.innerWidth - rect.width - GAP;
  const maxY = window.innerHeight - rect.height - GAP;
  el.style.left = `${Math.max(GAP, Math.min(props.menu.state.x, maxX))}px`;
  el.style.top = `${Math.max(GAP, Math.min(props.menu.state.y, maxY))}px`;
}

function firstEnabledIndex(): number {
  return props.menu.state.items.findIndex((item) => !item.disabled);
}

function lastEnabledIndex(): number {
  const items = props.menu.state.items;
  for (let index = items.length - 1; index >= 0; index--) {
    if (!items[index].disabled) return index;
  }
  return -1;
}

function nextEnabled(from: number, direction: 1 | -1): number {
  const count = props.menu.state.items.length;
  for (let step = 1; step <= count; step++) {
    const index = (from + direction * step + count) % count;
    if (!props.menu.state.items[index].disabled) return index;
  }
  return -1;
}

function focusItemAt(index: number) {
  if (index < 0) {
    root.value?.focus();
    return;
  }
  focusedIndex.value = index;
  itemEls.value[index]?.focus();
}

function onKeydown(event: KeyboardEvent) {
  if (!props.menu.state.items.length) return;
  switch (event.key) {
    case "ArrowDown":
      event.preventDefault();
      focusItemAt(nextEnabled(focusedIndex.value, 1));
      break;
    case "ArrowUp":
      event.preventDefault();
      focusItemAt(nextEnabled(focusedIndex.value, -1));
      break;
    case "Home":
      event.preventDefault();
      focusItemAt(firstEnabledIndex());
      break;
    case "End":
      event.preventDefault();
      focusItemAt(lastEnabledIndex());
      break;
    case "Enter":
    case " ":
    case "Spacebar": {
      const item = props.menu.state.items[focusedIndex.value];
      if (item && !item.disabled) {
        event.preventDefault();
        props.menu.select(item);
      }
      break;
    }
    case "Escape":
      event.preventDefault();
      close();
      break;
  }
}

function onPointerDown(event: PointerEvent) {
  if (!root.value?.contains(event.target as Node)) close();
}

function onScroll() {
  close();
}

function onResize() {
  close();
}

function onWindowBlur() {
  props.menu.close({ restoreFocus: false });
}

watch(
  () => props.menu.state.open,
  (open) => {
    if (open) {
      window.addEventListener("pointerdown", onPointerDown, true);
      document.addEventListener("scroll", onScroll, true);
      window.addEventListener("resize", onResize);
      window.addEventListener("blur", onWindowBlur);
    } else {
      window.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
      window.removeEventListener("blur", onWindowBlur);
    }
  },
);

watch(
  () => [props.menu.state.open, props.menu.state.x, props.menu.state.y] as const,
  ([open]) => {
    if (!open) return;
    void nextTick(() => {
      clampPosition();
      focusItemAt(firstEnabledIndex());
    });
  },
);

onBeforeUnmount(() => {
  window.removeEventListener("pointerdown", onPointerDown, true);
  document.removeEventListener("scroll", onScroll, true);
  window.removeEventListener("resize", onResize);
  window.removeEventListener("blur", onWindowBlur);
});
</script>

<template>
  <Teleport to="body">
    <div
      v-if="menu.state.open"
      ref="root"
      role="menu"
      tabindex="-1"
      class="fixed z-50 max-h-80 min-w-48 overflow-y-auto rounded-md border bg-popover p-1 text-popover-foreground shadow-md outline-none"
      :style="{ left: `${menu.state.x}px`, top: `${menu.state.y}px` }"
      @keydown="onKeydown"
      @contextmenu.prevent
    >
      <template v-for="(item, index) in menu.state.items" :key="item.id">
        <div v-if="item.separatorBefore" role="separator" class="context-menu-separator" />
        <button
          :ref="(el) => { itemEls[index] = el as HTMLElement | null }"
          type="button"
          role="menuitem"
          tabindex="-1"
          class="context-menu-item"
          :class="{ 'context-menu-danger': item.danger, 'context-menu-disabled': item.disabled }"
          :disabled="item.disabled"
          @click="menu.select(item)"
        >
          <component
            :is="item.icon"
            v-if="item.icon"
            class="context-menu-icon"
            :size="15"
            aria-hidden="true"
          />
          <span>{{ item.label }}</span>
        </button>
      </template>
    </div>
  </Teleport>
</template>

<style scoped>
.context-menu-item {
  display: flex;
  width: 100%;
  align-items: center;
  gap: 0.5rem;
  border-radius: 0.25rem;
  padding: 0.375rem 0.5rem;
  font-size: 0.875rem;
  line-height: 1.25rem;
  color: var(--popover-foreground);
  outline: none;
  user-select: none;
}

.context-menu-item:focus-visible {
  background: var(--accent);
  color: var(--accent-foreground);
}

.context-menu-item:disabled {
  pointer-events: none;
  opacity: 0.5;
}

.context-menu-item.context-menu-danger {
  color: var(--destructive);
}

.context-menu-item.context-menu-danger:focus-visible {
  background: color-mix(in srgb, var(--destructive) 10%, transparent);
  color: var(--destructive);
}

.context-menu-item.context-menu-danger .context-menu-icon {
  color: var(--destructive);
}

.context-menu-icon {
  flex: none;
  width: 1rem;
  height: 1rem;
  color: var(--muted-foreground);
}

.context-menu-separator {
  margin: 0.25rem 0.25rem;
  height: 1px;
  background: var(--border);
}
</style>
