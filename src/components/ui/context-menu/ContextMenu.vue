<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from "vue";
import type { ContextMenuController } from "@/composables/useContextMenu";

const props = defineProps<{ menu: ContextMenuController }>();

const root = ref<HTMLElement | null>(null);
const itemEls = ref<(HTMLElement | null)[]>([]);
const focusedIndex = ref(0);
const GAP = 8;
const MIN_HEIGHT = 120;

function close() {
  props.menu.close();
}

function place() {
  const el = root.value;
  if (!el) return;
  const { x, y } = props.menu.state;
  // Measure the natural height first, then give the menu as much room as the
  // cursor side offers instead of a fixed cap: a long menu should show in full
  // whenever the window has space for it, and only scroll when it does not.
  el.style.maxHeight = "";
  const rect = el.getBoundingClientRect();
  const limit = window.innerHeight - GAP * 2;
  const below = window.innerHeight - y - GAP;
  const above = y - GAP;
  const openUp = rect.height > below && above > below;
  const room = Math.min(limit, openUp ? above : below);
  const maxHeight = Math.max(Math.min(MIN_HEIGHT, limit), room);
  el.style.maxHeight = `${maxHeight}px`;
  el.style.left = `${Math.max(GAP, Math.min(x, window.innerWidth - rect.width - GAP))}px`;
  el.style.top = `${Math.max(GAP, openUp ? y - Math.min(rect.height, maxHeight) : y)}px`;
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

function onScroll(event: Event) {
  // The menu scrolls itself when it is taller than the window; that scroll
  // must not close it. Any other scroll detaches the menu from its target.
  if (event.target instanceof Node && root.value?.contains(event.target)) return;
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
  () => [props.menu.state.open, props.menu.state.x, props.menu.state.y, props.menu.state.items] as const,
  ([open]) => {
    if (!open) return;
    void nextTick(() => {
      place();
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
      class="fixed z-50 min-w-48 overflow-y-auto rounded-md border bg-popover p-1 text-popover-foreground shadow-md outline-none"
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
  /* Matches the dropdown menu separator (reka: -mx-1 my-1 h-px) so both menus
     draw the same hairline instead of one being inset by the container padding. */
  margin: 0.25rem -0.25rem;
  height: 1px;
  background: var(--border);
}
</style>
