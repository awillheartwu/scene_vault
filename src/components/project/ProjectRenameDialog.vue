<script setup lang="ts">
import { ref } from "vue";
import { LoaderCircle, X } from "@lucide/vue";
import { captureApi } from "@/lib/capture-api";
import { toast } from "@/lib/toast";

const props = defineProps<{ projectId: string; projectName: string }>();
const emit = defineEmits<{ (e: "close"): void; (e: "saved", name: string): void }>();

const name = ref(props.projectName);
const busy = ref(false);
const error = ref("");

async function save() {
  const trimmed = name.value.trim();
  if (!trimmed) {
    error.value = "项目名称不能为空";
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    const updated = await captureApi.renameProject(props.projectId, trimmed);
    toast.success(`已重命名为「${updated.name}」`);
    emit("saved", updated.name);
  } catch (caught) {
    error.value = caught instanceof Error ? caught.message : String(caught);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <Teleport to="body">
    <div class="import-overlay" @click.self="emit('close')">
    <div class="import-dialog" role="dialog" aria-modal="true" aria-label="重命名项目">
      <div class="project-dialog-title">
        <h3>重命名项目</h3>
        <button type="button" class="dialog-close" aria-label="关闭" @click="emit('close')">
          <X :size="16" />
        </button>
      </div>
      <p class="project-dialog-desc">修改项目名称，已登记的截图、角色和配置保持不变。</p>
      <form @submit.prevent="save">
        <input v-model="name" autofocus aria-label="项目名称" placeholder="项目名称" />
        <p v-if="error" class="project-dialog-error">{{ error }}</p>
      </form>
      <div class="project-dialog-actions">
        <button type="button" class="secondary-action" :disabled="busy" @click="emit('close')">
          取消
        </button>
        <button type="submit" class="primary-action" :disabled="busy">
          <LoaderCircle v-if="busy" class="animate-spin" :size="16" />保存
        </button>
    </div>
    </div>
    </div>
  </Teleport>
</template>

<style scoped>
.project-dialog-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.project-dialog-title h3 {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
}

.dialog-close {
  display: grid;
  width: 28px;
  height: 28px;
  place-items: center;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
}

.dialog-close:hover {
  background: var(--muted);
  color: var(--foreground);
}

.project-dialog-desc {
  margin: 0;
  color: var(--muted-foreground);
  font-size: 12px;
  line-height: 1.6;
}

.import-dialog input {
  height: 38px;
  padding: 0 12px;
  border: 1px solid var(--input);
  border-radius: 10px;
  background: var(--background);
  color: var(--foreground);
  font-size: 13px;
}

.import-dialog input:focus {
  border-color: var(--ring);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ring) 18%, transparent);
  outline: 0;
}

.import-dialog form {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.project-dialog-error {
  margin: 0;
  color: var(--destructive);
  font-size: 12px;
}

.project-dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding-top: 14px;
  border-top: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
}
</style>
