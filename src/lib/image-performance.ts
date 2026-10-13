import { captureApi } from "@/lib/capture-api";

export function reportImageLoadTiming(
  captureItemId: string,
  variant: string,
  kind: "preview" | "thumb" | "full",
  metrics: Record<string, number>,
  outcome: "ok" | "failed" | "cancelled",
): void {
  const segments = Object.entries(metrics)
    .filter(([, value]) => Number.isFinite(value) && value >= 0)
    .map(([name, value]) => `${name}=${value.toFixed(3)}`);
  void captureApi.recordClientEvent({
    level: "debug",
    module: "capture.image",
    event: "load_timing",
    operationId: `image_${kind}`,
    captureItemId,
    message: `variant=${variant} kind=${kind} ${segments.join(" ")}`,
    durationMs: metrics.total_ms,
    outcome,
    channel: "perf",
  }).catch(() => undefined);
}
