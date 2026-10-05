<script setup lang="ts">
import { computed, ref } from "vue";
import { LoaderCircle, Search } from "@lucide/vue";
import {
  captureApi,
  pickDirectory,
  type ProjectFileReconcileResult,
} from "@/lib/capture-api";
import { describeError } from "@/lib/vision-errors";
import TaskDialog from "@/components/common/TaskDialog.vue";

const props = defineProps<{ projectId: string }>();
const emit = defineEmits<{
  (event: "close"): void;
  (event: "updated"): void;
}>();

const busy = ref(false);
const result = ref<ProjectFileReconcileResult | null>(null);
const error = ref("");

const hasFindings = computed(() => {
  const value = result.value;
  if (!value) return false;
  return [
    value.sourceRelocatedCount,
    value.sourceMissingCount,
    value.sourceReplacedCount,
    value.sourceAmbiguousCount,
    value.destinationMissingCount,
    value.destinationUnavailableCount,
    value.destinationRelocatedCount,
    value.destinationAmbiguousCount,
    value.destinationContentMismatchCount,
    value.sourceUnavailableDirectoryCount,
  ].some((count) => count > 0);
});

async function run(archiveSearchDirectory: string | null = null) {
  if (busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    result.value = await captureApi.reconcileProjectFiles(props.projectId, archiveSearchDirectory);
    emit("updated");
  } catch (caught) {
    error.value = describeError(caught);
  } finally {
    busy.value = false;
  }
}

async function relocateFromPickedArchiveDirectory() {
  const directory = await pickDirectory();
  if (!directory) return;
  await run(directory);
}
</script>

<template>
  <TaskDialog
    title="检查项目文件"
    description="核对全项目的原图路径与归档目标状态；归档目录被改名或搬迁后按标识找回。不会导入或识别新文件。"
    :icon="Search"
    :width="560"
    @close="emit('close')"
  >
    <p v-if="error" class="task-error" role="alert">{{ error }}</p>
    <ul v-if="result" class="task-result">
      <li v-if="result.sourceRelocatedCount">
        已重新定位 {{ result.sourceRelocatedCount }} 张原图，并保留人物、分类、归档和人脸数据
      </li>
      <li v-if="result.sourceMissingCount">{{ result.sourceMissingCount }} 张原图缺失</li>
      <li v-if="result.sourceReplacedCount">
        {{ result.sourceReplacedCount }} 张原图路径已出现不同内容
      </li>
      <li v-if="result.sourceAmbiguousCount">
        {{ result.sourceAmbiguousCount }} 张原图存在多个同名同内容候选，未自动重定位
      </li>
      <li v-if="result.destinationMissingCount">
        {{ result.destinationMissingCount }} 个归档/头像目标文件缺失
      </li>
      <li v-if="result.destinationUnavailableCount">
        {{ result.destinationUnavailableCount }} 个归档/头像目标当前不可访问
      </li>
      <li v-if="result.destinationRelocatedCount">
        已找回 {{ result.destinationRelocatedCount }} 个归档/头像目标文件；文件仍在原位置，只更新数据库指向
      </li>
      <li v-if="result.destinationAmbiguousCount" class="task-warning">
        {{ result.destinationAmbiguousCount }} 个归档目标存在多个同标识候选，未自动改写
      </li>
      <li v-if="result.destinationContentMismatchCount" class="task-warning">
        {{ result.destinationContentMismatchCount }} 个归档目标存在同标识但内容不同的文件，未改写
      </li>
      <li v-if="result.sourceUnavailableDirectoryCount" class="task-warning">
        {{ result.sourceUnavailableDirectoryCount }} 个来源目录不可访问；为避免误判，未批量改写其缺失状态
      </li>
      <li v-if="!hasFindings">所有已登记文件状态正常</li>
    </ul>
    <template #actions>
      <button
        v-if="result && (result.destinationMissingCount || result.destinationRelocatedCount)"
        type="button"
        class="secondary-action"
        :disabled="busy"
        title="归档目录被改名或搬迁后，指定要搜索的目录，再按归档标识找回"
        @click="relocateFromPickedArchiveDirectory"
      >
        指定归档目录找回…
      </button>
      <button type="button" class="primary-action" :disabled="busy" @click="run()">
        <LoaderCircle v-if="busy" :size="15" class="animate-spin" />
        <Search v-else :size="15" />{{ busy ? "检查中…" : "开始检查" }}
      </button>
    </template>
  </TaskDialog>
</template>

<style scoped>
.task-result {
  display: grid;
  gap: 6px;
  margin: 2px 0;
  padding-left: 18px;
  color: var(--foreground);
  font-size: 11.5px;
}

.task-warning {
  color: var(--warn);
}

.task-error {
  margin: 0;
  color: var(--warn);
  font-size: 11.5px;
}
</style>
