<script setup lang="ts">
import { computed } from "vue";
import { ChevronLeft, ChevronRight } from "@lucide/vue";

const props = withDefaults(
  defineProps<{
    page: number;
    pageSize: number;
    total: number;
    pageSizeOptions?: number[];
  }>(),
  { pageSizeOptions: () => [50, 100, 200, 500, 1000] },
);

const emit = defineEmits<{
  (e: "update:page", page: number): void;
  (e: "update:pageSize", size: number): void;
}>();

const totalPages = computed(() => Math.max(1, Math.ceil(props.total / props.pageSize)));

// Compact window around the current page with ellipsis for long lists.
const pages = computed<(number | "...")[]>(() => {
  const total = totalPages.value;
  if (total <= 7) {
    return Array.from({ length: total }, (_, index) => index + 1);
  }
  const current = props.page;
  const candidates = [1, 2, total - 1, total, current - 1, current, current + 1].filter(
    (page) => page >= 1 && page <= total,
  );
  const sorted = [...new Set(candidates)].sort((left, right) => left - right);
  const output: (number | "...")[] = [];
  let previous = 0;
  for (const page of sorted) {
    if (page - previous > 1) output.push("...");
    output.push(page);
    previous = page;
  }
  return output;
});

function go(page: number) {
  if (page >= 1 && page <= totalPages.value && page !== props.page) {
    emit("update:page", page);
  }
}
</script>

<template>
  <div class="pagination" role="navigation" aria-label="分页">
    <span class="pagination-total">共 {{ total }} 条</span>
    <button
      type="button"
      class="pagination-button"
      :disabled="page <= 1"
      aria-label="上一页"
      @click="go(page - 1)"
    >
      <ChevronLeft :size="15" aria-hidden="true" />
    </button>
    <template v-for="(item, index) in pages" :key="index">
      <span v-if="item === '...'" class="pagination-ellipsis">…</span>
      <button
        v-else
        type="button"
        class="pagination-button"
        :class="{ active: item === page }"
        :aria-current="item === page ? 'page' : undefined"
        @click="go(item)"
      >
        {{ item }}
      </button>
    </template>
    <button
      type="button"
      class="pagination-button"
      :disabled="page >= totalPages"
      aria-label="下一页"
      @click="go(page + 1)"
    >
      <ChevronRight :size="15" aria-hidden="true" />
    </button>
    <label class="pagination-size">
      每页
      <select
        :value="pageSize"
        aria-label="每页条数"
        @change="emit('update:pageSize', Number(($event.target as HTMLSelectElement).value))"
      >
        <option v-for="size in pageSizeOptions" :key="size" :value="size">{{ size }}</option>
      </select>
    </label>
  </div>
</template>

<style scoped>
.pagination {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 6px;
  padding: 12px 0 2px;
}

.pagination-total {
  margin-right: 6px;
  color: var(--muted-foreground);
  font-size: 11.5px;
}

.pagination-button {
  display: grid;
  min-width: 30px;
  height: 30px;
  place-items: center;
  padding: 0 7px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: color-mix(in srgb, var(--card) 58%, transparent);
  color: var(--muted-foreground);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: border-color 0.15s ease, color 0.15s ease, background-color 0.15s ease;
}

.pagination-button:hover:not(:disabled) {
  border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  color: var(--accent);
}

.pagination-button.active {
  border-color: var(--accent);
  background: var(--accent);
  color: var(--accent-foreground);
}

.pagination-button:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.pagination-ellipsis {
  color: var(--muted-foreground);
  font-size: 12px;
}

.pagination-size {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-left: 6px;
  color: var(--muted-foreground);
  font-size: 11.5px;
}

.pagination-size select {
  height: 30px;
  padding: 0 8px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--background);
  color: var(--foreground);
  font-size: 12px;
}
</style>
