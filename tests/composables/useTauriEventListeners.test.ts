import { flushPromises, mount } from "@vue/test-utils";
import { defineComponent, h, onMounted } from "vue";
import { beforeEach, expect, it, vi } from "vitest";
import type { EventCallback, EventName } from "@tauri-apps/api/event";

const bridge = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: bridge.listen }));

import { useTauriEventListeners } from "@/composables/useTauriEventListeners";

beforeEach(() => { bridge.listen.mockReset(); });

it("cleans registered and late listeners and ignores events after unmount", async () => {
  const first = vi.fn();
  const late = vi.fn();
  const handler = vi.fn();
  const callbacks: EventCallback<unknown>[] = [];
  let finishLate!: (unlisten: () => void) => void;
  bridge.listen.mockImplementation((name: EventName, callback: EventCallback<unknown>) => {
    callbacks.push(callback);
    return name === "first" ? Promise.resolve(first) : new Promise((resolve) => { finishLate = resolve; });
  });
  let listeners!: ReturnType<typeof useTauriEventListeners>;
  const wrapper = mount(defineComponent({
    setup() {
      listeners = useTauriEventListeners();
      onMounted(async () => {
        await listeners.listen("first", handler);
        await listeners.listen("late", handler);
      });
      return () => h("div");
    },
  }));
  await flushPromises();
  callbacks[0]({ event: "first", id: 1, payload: null });
  expect(handler).toHaveBeenCalledOnce();
  wrapper.unmount();
  expect(first).toHaveBeenCalledOnce();
  callbacks[0]({ event: "first", id: 1, payload: null });
  finishLate(late);
  await flushPromises();
  callbacks[1]({ event: "late", id: 2, payload: null });
  expect(late).toHaveBeenCalledOnce();
  expect(handler).toHaveBeenCalledOnce();
  await listeners.listen("after-unmount", handler);
  expect(bridge.listen).toHaveBeenCalledTimes(2);
});

it("retains cleanup ownership when a later registration fails", async () => {
  const unlisten = vi.fn();
  bridge.listen.mockResolvedValueOnce(unlisten).mockRejectedValueOnce(new Error("bridge closed"));
  const wrapper = mount(defineComponent({
    setup() {
      const listeners = useTauriEventListeners();
      onMounted(async () => {
        try {
          await listeners.listen("first", () => {});
          await listeners.listen("failed", () => {});
        } catch { /* Consumers may continue without the unavailable bridge. */ }
      });
      return () => h("div");
    },
  }));
  await flushPromises();
  wrapper.unmount();
  expect(unlisten).toHaveBeenCalledOnce();
});
