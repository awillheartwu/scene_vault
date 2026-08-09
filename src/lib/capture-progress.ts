import { onBeforeUnmount, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ProgressPayload {
  captureItemId: string;
  stage: string;
  percent: number;
}

export interface RuntimeStatusPayload {
  engineStatus: string;
  workerStatus: string;
  activeCaptureItemId: string | null;
  queuedCount: number;
  archivePendingCount: number;
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
  const stage = ref<string | null>(null);
  const percent = ref(0);
  const queuedCount = ref(0);
  let unlisteners: UnlistenFn[] = [];

  async function start() {
    try {
      unlisteners.push(
        await listen<ProgressPayload>("capture:progress", (event) => {
          activeItemId.value = event.payload.captureItemId;
          stage.value = event.payload.stage;
          percent.value = event.payload.percent;
        }),
        await listen<RuntimeStatusPayload>("capture:runtime-status", (event) => {
          queuedCount.value = event.payload.queuedCount;
          const nextActiveItemId = event.payload.activeCaptureItemId;
          if (nextActiveItemId) {
            if (activeItemId.value !== nextActiveItemId) {
              stage.value = null;
              percent.value = 0;
            }
            activeItemId.value = nextActiveItemId;
          } else {
            if (event.payload.queuedCount === 0 && event.payload.archivePendingCount === 0) {
              activeItemId.value = null;
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

  return { start, activeItemId, stage, percent, queuedCount };
}

export function stageLabel(stage: string | null): string {
  return (stage && STAGE_LABELS[stage]) || stage || "处理中";
}
