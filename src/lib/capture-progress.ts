import { onBeforeUnmount, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { captureApi } from "@/lib/capture-api";

export interface ProgressPayload {
  captureItemId: string;
  sourcePath: string;
  stage: string;
  percent: number;
}

export interface RuntimeStatusPayload {
  engineStatus: string;
  workerStatus: string;
  activeCaptureItemId: string | null;
  activeCaptureSourcePath: string | null;
  queuedCount: number;
  archivePendingCount: number;
  prelabelPendingCount: number;
  lastError: string | null;
}

export const STAGE_LABELS: Record<string, string> = {
  read: "读取截图",
  detect_face: "检测人脸",
  extract_feature: "特征提取",
  annotate: "标注",
  crop_avatar: "裁剪头像",
  done: "完成",
};

/**
 * Live capture-processing progress: subscribes to the Rust progress and
 * runtime-status events and exposes the current stage/percent/queue length.
 */
export function useCaptureProgress() {
  const activeItemId = ref<string | null>(null);
  const activeSourcePath = ref<string | null>(null);
  const stage = ref<string | null>(null);
  const percent = ref(0);
  const queuedCount = ref(0);
  const prelabelPendingCount = ref(0);
  const engineStatus = ref<string | null>(null);
  let unlisteners: UnlistenFn[] = [];

  async function start() {
    try {
      // Seed the engine state immediately; the worker only emits
      // runtime-status when something changes, so an idle page would
      // otherwise never learn that the AI engine is unconfigured.
      try {
        const status = await captureApi.runtimeStatus();
        engineStatus.value = status.engineStatus;
        queuedCount.value = status.queuedCount;
        prelabelPendingCount.value = status.prelabelPendingCount ?? 0;
      } catch {
        // Browser previews do not expose the Tauri bridge.
      }
      unlisteners.push(
        await listen<ProgressPayload>("capture:progress", (event) => {
          activeItemId.value = event.payload.captureItemId;
          activeSourcePath.value = event.payload.sourcePath;
          stage.value = event.payload.stage;
          percent.value = event.payload.percent;
        }),
        await listen<RuntimeStatusPayload>("capture:runtime-status", (event) => {
          engineStatus.value = event.payload.engineStatus;
          queuedCount.value = event.payload.queuedCount;
          prelabelPendingCount.value = event.payload.prelabelPendingCount ?? 0;
          const nextActiveItemId = event.payload.activeCaptureItemId;
          if (nextActiveItemId) {
            if (activeItemId.value !== nextActiveItemId) {
              stage.value = null;
              percent.value = 0;
            }
            activeItemId.value = nextActiveItemId;
            activeSourcePath.value = event.payload.activeCaptureSourcePath;
          } else {
            // The pre-label pass keeps items in awaiting_label, so it never
            // reports an active capture. Keep the current image visible
            // between serial items until the whole batch is really done.
            if (
              event.payload.queuedCount === 0 &&
              event.payload.archivePendingCount === 0 &&
              prelabelPendingCount.value === 0
            ) {
              activeItemId.value = null;
              activeSourcePath.value = null;
              stage.value = null;
              percent.value = 0;
            }
          }
        }),
      );
    } catch {
      // Browser previews do not expose the Tauri event bridge.
    }
  }

  onBeforeUnmount(() => {
    unlisteners.forEach((unlisten) => unlisten());
  });

  return {
    start,
    activeItemId,
    activeSourcePath,
    stage,
    percent,
    queuedCount,
    prelabelPendingCount,
    engineStatus,
  };
}

export function stageLabel(stage: string | null): string {
  return (stage && STAGE_LABELS[stage]) || stage || "处理中";
}
