<script setup lang="ts">
import { ref } from "vue";
import { Archive, LoaderCircle } from "@lucide/vue";
import { captureApi } from "@/lib/capture-api";
import { describeError } from "@/lib/vision-errors";
import TaskDialog from "@/components/common/TaskDialog.vue";

const props = defineProps<{ projectId: string }>();
const emit = defineEmits<{ (event: "close"): void }>();

const busy = ref(false);
const message = ref("");
const failed = ref(false);

async function rebuild() {
  if (busy.value) return;
  busy.value = true;
  message.value = "";
  try {
    const result = await captureApi.rebuildArchiveManifest(props.projectId);
    message.value = result.message;
    failed.value = result.failedCount > 0;
  } catch (caught) {
    message.value = describeError(caught);
    failed.value = true;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <TaskDialog
    title="更新评分清单"
    description="为打分流程生成或更新归档目录里的 project.json（人物与图片对应表）。不会移动或改写任何归档文件。"
    :icon="Archive"
    @close="emit('close')"
  >
    <p v-if="message" class="manifest-message" :class="{ 'manifest-error': failed }" role="status">
      {{ message }}
    </p>
    <template #actions>
      <button type="button" class="primary-action" :disabled="busy" @click="rebuild">
        <LoaderCircle v-if="busy" :size="15" class="animate-spin" />
        <Archive v-else :size="15" />{{ busy ? "写入中…" : "开始更新" }}
      </button>
    </template>
  </TaskDialog>
</template>

<style scoped>
.manifest-message {
  margin: 0;
  color: var(--foreground);
  font-size: 11.5px;
  line-height: 1.6;
}

.manifest-error {
  color: var(--warn);
}
</style>
