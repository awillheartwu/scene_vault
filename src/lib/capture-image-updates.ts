import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CaptureItem } from "./capture-api";

type Subscriber = (item: CaptureItem) => void;
const subscribers = new Map<string, Set<Subscriber>>();
let connection: { unlisten?: UnlistenFn } | undefined;

/** One native listener per webview; retain only mounted capture ids. */
export function subscribeCaptureImageUpdates(id: string, subscriber: Subscriber): () => void {
  let group = subscribers.get(id);
  if (!group) subscribers.set(id, group = new Set());
  group.add(subscriber);
  if (!connection) {
    const current: { unlisten?: UnlistenFn } = {};
    connection = current;
    void listen<CaptureItem>("capture:item-updated", ({ payload }) => {
      if (connection !== current) return;
      subscribers.get(payload.id)?.forEach((notify) => notify(payload));
    }).then((unlisten) => {
      if (connection !== current) unlisten();
      else current.unlisten = unlisten;
    }).catch(() => {
      // Browser previews have no native event bridge. A later mount retries.
      if (connection === current) connection = undefined;
    });
  }
  return () => {
    group.delete(subscriber);
    if (!group.size) subscribers.delete(id);
    if (!subscribers.size) {
      const previous = connection;
      connection = undefined;
      previous?.unlisten?.();
    }
  };
}
