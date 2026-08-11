import { captureApi, type LogLevel } from "@/lib/capture-api";

interface ClientEvent {
  level: LogLevel;
  module: string;
  event: string;
  message?: string;
  operationId?: string;
  durationMs?: number;
  outcome?: string;
  errorCode?: string;
}

let bridgeFailureReported = false;

function safeMessage(value: unknown): string {
  const raw = value instanceof Error ? `${value.name}: ${value.message}` : String(value);
  return raw
    .replace(/[A-Za-z]:[\\/][^\s]+/g, "[local-path]")
    .replace(/(?:\/[^\s/]+){2,}/g, "[local-path]")
    .replace(/[\r\n]+/g, " | ")
    .slice(0, 500);
}

export async function recordClientEvent(event: ClientEvent): Promise<void> {
  try {
    await captureApi.recordClientEvent({ ...event, message: safeMessage(event.message ?? event.event) });
  } catch (error) {
    if (!bridgeFailureReported) {
      bridgeFailureReported = true;
      console.warn("Client log bridge is unavailable", error);
    }
  }
}

export function installClientErrorLogging(): void {
  window.addEventListener("error", (event) => {
    void recordClientEvent({
      level: "error",
      module: "runtime",
      event: "window_error",
      message: safeMessage(event.error ?? event.message),
      outcome: "failed",
      errorCode: event.error instanceof Error ? event.error.name : "window_error",
    });
  });
  window.addEventListener("unhandledrejection", (event) => {
    void recordClientEvent({
      level: "error",
      module: "runtime",
      event: "unhandled_rejection",
      message: safeMessage(event.reason),
      outcome: "failed",
      errorCode: event.reason instanceof Error ? event.reason.name : "unhandled_rejection",
    });
  });
}
