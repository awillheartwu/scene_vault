import { onBeforeUnmount } from "vue";
import {
  listen,
  type EventCallback,
  type EventName,
  type UnlistenFn,
} from "@tauri-apps/api/event";

/** Own each listener as soon as its asynchronous registration completes. */
export function useTauriEventListeners() {
  let active = true;
  const unlisteners: UnlistenFn[] = [];

  async function listenEvent<T>(event: EventName, handler: EventCallback<T>): Promise<void> {
    if (!active) return;
    const unlisten = await listen<T>(event, (payload) => {
      if (active) handler(payload);
    });
    if (active) {
      unlisteners.push(unlisten);
    } else {
      unlisten();
    }
  }

  onBeforeUnmount(() => {
    active = false;
    unlisteners.splice(0).forEach((unlisten) => unlisten());
  });

  return { listen: listenEvent, isActive: () => active };
}
