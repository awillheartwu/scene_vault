<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { AlertCircle, FileImage, LoaderCircle, Trash2, X } from "@lucide/vue";
import {
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import {
  captureApi,
  pathFileName,
  type CaptureDeletionPreview,
  type CaptureDeletionResult,
  type CaptureItem,
} from "@/lib/capture-api";
import { toast } from "@/lib/toast";

const props = defineProps<{ captureItem: CaptureItem }>();
const emit = defineEmits<{
  (event: "close"): void;
  (event: "deleted", result: CaptureDeletionResult): void;
}>();

const preview = ref<CaptureDeletionPreview | null>(null);
const previewError = ref("");
const busy = ref(false);
const deleteDestinationFiles = ref(true);
const networkConfirmation = ref(false);
const partial = ref<CaptureDeletionResult | null>(null);

const fileName = computed(() => pathFileName(props.captureItem.sourcePath));

const destinationCountText = computed(() => {
  const count = preview.value?.destinationFileCount ?? 0;
  if (count === 0) return "当前没有可移入回收站的目标文件";
  const network = preview.value?.networkDestinationFileCount ?? 0;
  if (network === 0) return `勾选后会把 ${count} 个目标文件移入回收站`;
  return `${count - network} 个本地目标移入回收站，${network} 个 NAS/网络目标需永久删除`;
});

onMounted(() => {
  void loadPreview();
});

async function loadPreview() {
  previewError.value = "";
  try {
    preview.value = await captureApi.previewCaptureDeletion(props.captureItem.id);
  } catch (caught) {
    previewError.value = caught instanceof Error ? caught.message : String(caught);
  }
}

function close() {
  if (!busy.value) emit("close");
}

async function confirmDelete() {
  if (!preview.value || busy.value) return;
  if (
    deleteDestinationFiles.value &&
    preview.value.networkDestinationFileCount > 0 &&
    !networkConfirmation.value
  ) {
    networkConfirmation.value = true;
    return;
  }
  busy.value = true;
  previewError.value = "";
  try {
    const result = await captureApi.deleteCaptureItem({
      captureItemId: props.captureItem.id,
      deleteDestinationFiles: deleteDestinationFiles.value,
      allowPermanentNetworkDelete: networkConfirmation.value,
    });
    if (!result.completed) {
      partial.value = result;
      return;
    }
    partial.value = null;
    toast.success(`已从 Scene Vault 移除「${fileName.value}」，原图保留。`);
    emit("deleted", result);
  } catch (caught) {
    previewError.value = caught instanceof Error ? caught.message : String(caught);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <DialogRoot :open="true" @update:open="!$event && close()">
    <DialogPortal>
      <DialogOverlay class="import-overlay" />
      <DialogContent class="import-dialog delete-dialog">
        <div class="delete-title">
          <DialogTitle>
            <span class="delete-icon"><Trash2 :size="15" /></span>
            从 Scene Vault 移除「{{ fileName }}」？
          </DialogTitle>
          <button
            type="button"
            class="dialog-close"
            aria-label="关闭移除截图确认"
            :disabled="busy"
            @click="close"
          >
            <X :size="16" />
          </button>
        </div>

        <DialogDescription v-if="!networkConfirmation" class="delete-description">
          将删除这条截图在应用内的记录，不可撤销。源图不会被应用删除。
        </DialogDescription>
        <DialogDescription v-else class="delete-description delete-danger">
          NAS/网络文件无法进入 Windows 回收站。继续后将永久删除这些目标文件，此操作不可恢复。
        </DialogDescription>

        <p v-if="previewError" class="delete-error" role="alert">{{ previewError }}</p>
        <div v-else-if="!preview" class="preview-loading">
          <LoaderCircle class="animate-spin" :size="16" />正在确认删除范围…
        </div>

        <template v-else>
          <ul v-if="!networkConfirmation" class="delete-effects">
            <li>{{ preview.captureCount }} 条截图记录（含人脸框、Face Bank 样本和素材关系）</li>
            <li v-if="preview.localDerivedFileCount > 0">
              本地标注图/头像派生文件 {{ preview.localDerivedFileCount }} 个，将随记录清理
            </li>
            <li v-if="preview.sourceFilesPreserved > 0">
              {{ preview.sourceFilesPreserved }} 张原图保持原位置，不会被删除
            </li>
          </ul>

          <label v-if="!networkConfirmation" class="delete-target-option">
            <input v-model="deleteDestinationFiles" type="checkbox" :disabled="busy" @change="networkConfirmation = false" />
            <span>
              同时删除应用生成的归档图/头像归档文件（移入 Windows 回收站）
              <small>{{ destinationCountText }}</small>
            </span>
          </label>

          <div v-else class="partial-warning network-delete-warning" role="alert">
            <strong>将永久删除 {{ preview.networkDestinationFileCount }} 个 NAS/网络目标文件</strong>
            <span>本地目标仍会进入回收站；原图始终保留。若不接受永久删除，请返回并取消勾选目标文件。</span>
          </div>

          <div v-if="partial" class="partial-warning" role="alert">
            <strong>有 {{ partial.failures.length }} 个目标文件未能删除，截图记录已保留。</strong>
            <span>
              已回收 {{ partial.destinationFilesRecycled }} 个，永久删除 {{ partial.destinationFilesPermanentlyDeleted }} 个，{{ partial.destinationFilesAlreadyMissing }} 个此前已缺失。
              处理相应文件后可再次点击删除。
            </span>
            <ul v-if="partial.failures.length" class="partial-failures">
              <li v-for="failure in partial.failures" :key="failure.path">
                <code>{{ failure.path }}</code>
                {{ failure.error }}
              </li>
            </ul>
          </div>

          <p class="keep-files-note">
            <FileImage :size="14" aria-hidden="true" />
            原图与游戏/截图源文件始终保留在磁盘上；本次只是把应用记录与目标文件移出 Scene Vault。
          </p>

          <div class="delete-actions">
            <button type="button" class="secondary-action" :disabled="busy" @click="networkConfirmation ? (networkConfirmation = false) : close()">
              {{ networkConfirmation ? "返回" : "取消" }}
            </button>
            <button type="button" class="danger-action" :disabled="busy" @click="confirmDelete">
              <LoaderCircle v-if="busy" class="animate-spin" :size="16" />
              <AlertCircle v-else :size="16" aria-hidden="true" />
              {{ networkConfirmation ? "永久删除网络目标并移除" : partial ? "重试移除记录" : "确认移除记录" }}
            </button>
          </div>
        </template>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped src="./delete-dialog.css"></style>
