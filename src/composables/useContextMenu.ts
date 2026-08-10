import { reactive, type Component } from "vue";

export interface ContextMenuItem {
  id: string;
  label: string;
  icon?: Component;
  danger?: boolean;
  disabled?: boolean;
  separatorBefore?: boolean;
  action?: () => void;
}

export interface ContextMenuState {
  open: boolean;
  x: number;
  y: number;
  items: ContextMenuItem[];
}

/**
 * Interactive or special regions that must keep their native behavior:
 * inputs keep copy/paste, dropdown items and drag regions keep their own
 * handling. A `[data-context-allow]` marker on an interactive region makes it
 * part of the item again (e.g. the clickable main area of a project card).
 */
const EXCLUDE_SELECTOR = [
  "button",
  "input",
  "select",
  "textarea",
  '[role="menuitem"]',
  '[role="option"]',
  "[data-tauri-drag-region]",
  "[data-context-exclude]",
].join(", ");

/**
 * Decides whether a right-click should open a custom menu. Returns false when
 * the click landed on an excluded interactive element, unless that element is
 * the menu-bound item root itself (whole-row buttons) or is explicitly marked
 * with `data-context-allow`.
 */
export function isExcludedContextTarget(event: MouseEvent): boolean {
  const target = event.target;
  if (!(target instanceof Element)) return true;
  const hit = target.closest(EXCLUDE_SELECTOR);
  if (!hit) return false;
  // The item itself may be a button (capture card / history row).
  if (hit === event.currentTarget) return false;
  // Explicitly marked interactive region inside the item (project card main).
  if (hit.closest("[data-context-allow]")) return false;
  return true;
}

/**
 * Input-like controls keep the WebView native context menu (copy/paste and
 * spellcheck). Everything else must not show the browser menu in a desktop
 * app; custom per-item menus call `preventDefault` themselves and run before
 * this document-level listener.
 */
const NATIVE_MENU_KEEP_SELECTOR = 'input, select, textarea, [contenteditable="true"]';

/** Suppresses the WebView native context menu app-wide; returns the uninstaller. */
export function installContextMenuGuard(): () => void {
  const onContextMenu = (event: MouseEvent) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    if (target.closest(NATIVE_MENU_KEEP_SELECTOR)) return;
    event.preventDefault();
  };
  document.addEventListener("contextmenu", onContextMenu);
  return () => document.removeEventListener("contextmenu", onContextMenu);
}

export function useContextMenu() {
  const state = reactive<ContextMenuState>({ open: false, x: 0, y: 0, items: [] });

  /** Opens the menu for a right-click event; returns false when suppressed. */
  function open(event: MouseEvent, items: ContextMenuItem[]): boolean {
    if (isExcludedContextTarget(event)) return false;
    event.preventDefault();
    state.items = items;
    state.x = event.clientX;
    state.y = event.clientY;
    state.open = true;
    return true;
  }

  function close() {
    state.open = false;
    state.items = [];
  }

  function select(item: ContextMenuItem) {
    close();
    item.action?.();
  }

  return { state, open, close, select };
}

export type ContextMenuController = ReturnType<typeof useContextMenu>;
