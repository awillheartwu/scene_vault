import { reactive } from "vue";

export type ToastType = "success" | "error" | "info";

export interface ToastItem {
  id: number;
  type: ToastType;
  message: string;
}

const state = reactive<{ toasts: ToastItem[] }>({ toasts: [] });
let nextId = 1;

const DEFAULT_DURATION = 5000;

function push(type: ToastType, message: string, duration = DEFAULT_DURATION): number {
  const id = nextId++;
  state.toasts.push({ id, type, message });
  if (duration > 0) {
    window.setTimeout(() => dismiss(id), duration);
  }
  return id;
}

export function dismiss(id: number): void {
  const index = state.toasts.findIndex((toast) => toast.id === id);
  if (index >= 0) {
    state.toasts.splice(index, 1);
  }
}

export const toast = {
  success: (message: string, duration?: number) => push("success", message, duration),
  error: (message: string, duration?: number) => push("error", message, duration),
  info: (message: string, duration?: number) => push("info", message, duration),
};

export function useToasts() {
  return state;
}
