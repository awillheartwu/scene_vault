<script setup lang="ts">
import { AlertCircle, CheckCircle2, Info, X } from "@lucide/vue";
import { dismiss, useToasts } from "@/lib/toast";

const { toasts } = useToasts();
</script>

<template>
  <div class="toast-host" role="status" aria-live="polite">
    <TransitionGroup name="toast">
      <div v-for="item in toasts" :key="item.id" class="toast-item" :data-type="item.type">
        <CheckCircle2 v-if="item.type === 'success'" :size="16" class="toast-icon" />
        <AlertCircle v-else-if="item.type === 'error'" :size="16" class="toast-icon" />
        <Info v-else :size="16" class="toast-icon" />
        <span class="toast-text">{{ item.message }}</span>
        <button type="button" class="toast-close" aria-label="关闭提示" @click="dismiss(item.id)">
          <X :size="14" />
        </button>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.toast-host {
  position: fixed;
  top: 16px;
  right: 16px;
  z-index: 9999;
  display: flex;
  flex-direction: column;
  gap: 8px;
  max-width: min(380px, calc(100vw - 32px));
  pointer-events: none;
}

.toast-item {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 10px 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: color-mix(in srgb, var(--card) 92%, transparent);
  box-shadow: var(--card-shadow), var(--inner-highlight);
  backdrop-filter: var(--panel-blur);
  color: var(--foreground);
  font-size: 12px;
  pointer-events: auto;
}

.toast-icon {
  flex: none;
}

.toast-item[data-type="success"] .toast-icon {
  color: var(--ok);
}

.toast-item[data-type="error"] .toast-icon {
  color: var(--destructive);
}

.toast-item[data-type="info"] .toast-icon {
  color: var(--accent);
}

.toast-text {
  min-width: 0;
  flex: 1;
  line-height: 1.45;
}

.toast-close {
  display: grid;
  width: 22px;
  height: 22px;
  flex: none;
  place-items: center;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
}

.toast-close:hover {
  background: var(--muted);
  color: var(--foreground);
}

.toast-enter-active,
.toast-leave-active {
  transition:
    opacity 0.18s ease,
    transform 0.18s ease;
}

.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(-6px);
}
</style>
