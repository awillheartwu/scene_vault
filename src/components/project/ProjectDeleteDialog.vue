<script setup lang="ts">
import { onMounted, ref } from "vue";
import { CircleAlert, LoaderCircle, Trash2, X } from "@lucide/vue";
import { captureApi, type ProjectDeletionPreview } from "@/lib/capture-api";
import { toast } from "@/lib/toast";

const props = defineProps<{ projectId: string; projectName: string }>();
const emit = defineEmits<{ (e: "close"): void; (e: "deleted"): void }>();

const preview = ref<ProjectDeletionPreview | null>(null);
const busy = ref(false);
const error = ref("");

onMounted(() => {
  void loadPreview();
});

async function loadPreview() {
  try {
    preview.value = await captureApi.previewProjectDelete(props.projectId);
  } catch (caught) {
    error.value = caught instanceof Error ? caught.message : String(caught);
  }
}

async function confirmDelete() {
  busy.value = true;
  error.value = "";
  try {
    const summary = await captureApi.deleteProject(props.projectId);
    toast.success(
      `已删除「${props.projectName}」：${summary.captureCount} 张截图、${summary.characterCount} 个角色`,
    );
    emit("deleted");
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
    <div class="import-dialog" role="dialog" aria-modal="true" aria-label="删除项目">
      <div class="project-dialog-title">
        <h3>
          <span class="danger-icon"><Trash2 :size="15" /></span>
          删除「{{ projectName }}」？
        </h3>
        <button type="button" class="dialog-close" aria-label="关闭" @click="emit('close')">
          <X :size="16" />
        </button>
      </div>
      <p v-if="error" class="project-dialog-error">{{ error }}</p>
      <template v-else-if="preview">
        <p class="project-dialog-desc">此操作不可撤销，将从应用中移除：</p>
        <ul class="delete-list">
          <li>{{ preview.captureCount }} 张截图记录</li>
          <li>{{ preview.characterCount }} 个角色（含人脸样本）</li>
          <li>{{ preview.sessionCount }} 个捕获会话</li>
          <li v-if="preview.noteCount">{{ preview.noteCount }} 篇项目笔记</li>
          <li v-if="preview.collectionCount">{{ preview.collectionCount }} 个合集</li>
          <li v-if="preview.orphanAssetCount">
            {{ preview.orphanAssetCount }} 条无其他项目引用的素材记录
          </li>
        </ul>
        <p v-if="preview.hasActiveSession" class="session-warning">
          <CircleAlert :size="15" />该项目正在监听中，删除后监听立即停止。
        </p>
        <p class="keep-files-note">磁盘上的截图源文件和归档文件不会删除，仅清除应用内的记录。</p>
      </template>
      <div v-else class="preview-loading">
        <LoaderCircle class="animate-spin" :size="16" />正在统计将删除的数据…
      </div>
      <div class="project-dialog-actions">
        <button type="button" class="secondary-action" :disabled="busy" @click="emit('close')">
          取消
        </button>
        <button
          type="button"
          class="danger-action"
          :disabled="busy || !preview"
          @click="confirmDelete"
        >
          <LoaderCircle v-if="busy" class="animate-spin" :size="16" />确认删除
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
  display: flex;
  align-items: center;
  gap: 9px;
  margin: 0;
  font-size: 15px;
  font-weight: 700;
}

.danger-icon {
  display: grid;
  width: 26px;
  height: 26px;
  place-items: center;
  border-radius: 8px;
  background: color-mix(in srgb, var(--destructive) 12%, transparent);
  color: var(--destructive);
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

.delete-list {
  max-height: 30vh;
  margin: 0;
  padding: 6px 12px;
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--background);
  list-style: none;
}

.delete-list li {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 0;
  color: var(--foreground);
  font-size: 12px;
}

.delete-list li::before {
  content: "";
  width: 5px;
  height: 5px;
  flex: 0 0 auto;
  border-radius: 50%;
  background: var(--accent);
}

.session-warning {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
  color: var(--destructive);
  font-size: 12px;
  line-height: 1.6;
}

.keep-files-note {
  margin: 0;
  color: var(--muted-foreground);
  font-size: 11.5px;
  line-height: 1.6;
}

.project-dialog-error {
  margin: 0;
  color: var(--destructive);
  font-size: 12px;
}

.preview-loading {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 16px 0;
  color: var(--muted-foreground);
  font-size: 12px;
}

.project-dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding-top: 14px;
  border-top: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
}

.danger-action {
  display: inline-flex;
  height: 38px;
  align-items: center;
  justify-content: center;
  gap: 7px;
  padding: 0 14px;
  border: 1px solid transparent;
  border-radius: 9px;
  background: var(--destructive);
  color: #fff;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: filter 0.15s ease, transform 0.15s ease;
}

.danger-action:hover {
  filter: brightness(1.08);
  transform: translateY(-1px);
}

.danger-action:disabled {
  opacity: 0.55;
  cursor: not-allowed;
  transform: none;
}
</style>
