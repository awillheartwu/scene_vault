<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef } from "vue";
import {
  ClipboardCopy,
  ExternalLink,
  Filter,
  FolderOpen,
  LoaderCircle,
  RefreshCw,
  ShieldCheck,
  Trash2,
} from "@lucide/vue";
import {
  captureApi,
  openPathExternal,
  type LogLevel,
  type LogRecord,
  type LogStatus,
} from "@/lib/capture-api";
import { toast } from "@/lib/toast";

type TimeRange = "15m" | "1h" | "24h" | "7d";
const PAGE_SIZE = 100;

const records = shallowRef<LogRecord[]>([]);
const status = ref<LogStatus | null>(null);
const timeRange = ref<TimeRange>("24h");
const level = ref<LogLevel | "">("");
const moduleQuery = ref("");
const eventQuery = ref("");
const correlationQuery = ref("");
const outcome = ref("");
const matchedCount = ref(0);
const busy = ref(false);
const loadingMore = ref(false);
const hasMore = ref(false);
const nextOffset = ref<number | null>(null);
const errorMessage = ref("");
const logList = ref<HTMLElement | null>(null);
const loadSentinel = ref<HTMLElement | null>(null);
let pageWindow: { since: string; until: string } | null = null;
let pageObserver: IntersectionObserver | null = null;

const statusSummary = computed(() => {
  if (!status.value) return "正在读取日志策略…";
  const dropped = status.value.droppedRecords ? ` · 本次运行丢弃 ${status.value.droppedRecords} 条` : "";
  return `${formatBytes(status.value.totalBytes)} · ${status.value.fileCount} 个文件 · 保留 ${status.value.retentionDays} 天${dropped}`;
});

function rangeStart(until = Date.now()): string {
  const durationMs = {
    "15m": 15 * 60_000,
    "1h": 60 * 60_000,
    "24h": 24 * 60 * 60_000,
    "7d": 7 * 24 * 60 * 60_000,
  }[timeRange.value];
  return new Date(until - durationMs).toISOString();
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function formatTimestamp(value: string): string {
  return new Date(value).toLocaleString([], {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function levelLabel(value: LogLevel): string {
  return { debug: "调试", info: "信息", warn: "警告", error: "错误" }[value];
}

async function load(reset = true) {
  if (busy.value || loadingMore.value) return;
  if (!reset && (!hasMore.value || nextOffset.value === null)) return;
  if (reset) {
    const until = Date.now();
    pageWindow = { since: rangeStart(until), until: new Date(until).toISOString() };
    busy.value = true;
    records.value = [];
  } else {
    loadingMore.value = true;
  }
  errorMessage.value = "";
  try {
    const window = pageWindow ?? { since: rangeStart(), until: new Date().toISOString() };
    const offset = reset ? 0 : nextOffset.value ?? 0;
    const [result, nextStatus] = await Promise.all([
      captureApi.listDebugLogs({
        since: window.since,
        until: window.until,
        levels: level.value ? [level.value] : [],
        module: moduleQuery.value.trim() || null,
        event: eventQuery.value.trim() || null,
        correlationId: correlationQuery.value.trim() || null,
        outcome: outcome.value || null,
        offset,
        limit: PAGE_SIZE,
      }),
      reset ? captureApi.getLogStatus() : Promise.resolve(null),
    ]);
    records.value = reset ? result.records : [...records.value, ...result.records];
    matchedCount.value = result.matchedCount;
    hasMore.value = result.hasMore;
    nextOffset.value = result.nextOffset;
    if (nextStatus) status.value = nextStatus;
  } catch (caught) {
    errorMessage.value = caught instanceof Error ? caught.message : String(caught);
  } finally {
    busy.value = false;
    loadingMore.value = false;
  }
}

async function copyText(value: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch {
      // WebView2 can expose the API while denying clipboard permission.
    }
  }
  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.appendChild(textarea);
  textarea.select();
  const copied = document.execCommand("copy");
  textarea.remove();
  if (!copied) throw new Error("clipboard is unavailable");
}

async function copyDiagnostics() {
  busy.value = true;
  errorMessage.value = "";
  try {
    await copyText(await captureApi.getDiagnosticSummary());
    toast.success("诊断信息已复制，可直接粘贴给 Agent");
  } catch (caught) {
    errorMessage.value = caught instanceof Error ? caught.message : String(caught);
  } finally {
    busy.value = false;
  }
}

async function cleanup() {
  busy.value = true;
  errorMessage.value = "";
  try {
    const result = await captureApi.cleanupDebugLogs();
    status.value = result.status;
    toast.success(result.removedFiles ? `已清理 ${result.removedFiles} 个过期日志文件` : "没有需要清理的过期日志");
    busy.value = false;
    await load(true);
  } catch (caught) {
    errorMessage.value = caught instanceof Error ? caught.message : String(caught);
    busy.value = false;
  }
}

async function openDirectory() {
  if (!status.value) return;
  try {
    await openPathExternal(status.value.directory);
  } catch (caught) {
    errorMessage.value = caught instanceof Error ? caught.message : String(caught);
  }
}

function setupInfiniteLoading() {
  if (typeof IntersectionObserver === "undefined" || !loadSentinel.value) return;
  pageObserver = new IntersectionObserver(
    (entries) => {
      if (entries.some((entry) => entry.isIntersecting)) void load(false);
    },
    { root: logList.value, rootMargin: "160px 0px" },
  );
  pageObserver.observe(loadSentinel.value);
}

onMounted(async () => {
  await load(true);
  await nextTick();
  setupInfiniteLoading();
});

onBeforeUnmount(() => {
  pageObserver?.disconnect();
  pageObserver = null;
});
</script>

<template>
  <section class="debug-log-panel" aria-labelledby="debug-log-heading">
    <div class="log-panel-heading">
      <div>
        <h2 id="debug-log-heading">本地运行日志</h2>
        <p>{{ statusSummary }}</p>
      </div>
      <button type="button" class="copy-diagnostics" :disabled="busy" @click="copyDiagnostics">
        <ClipboardCopy :size="16" />复制诊断信息
      </button>
    </div>

    <div class="log-privacy-note">
      <ShieldCheck :size="16" />
      <span>日志保存在本机；本页不实时订阅，仅在打开、刷新和滚动加载时读取，离开后不占用监控资源。</span>
    </div>

    <div class="log-toolbar" aria-label="日志筛选">
      <Filter :size="16" aria-hidden="true" />
      <label>
        时间范围
        <select v-model="timeRange">
          <option value="15m">最近 15 分钟</option>
          <option value="1h">最近 1 小时</option>
          <option value="24h">最近 24 小时</option>
          <option value="7d">最近 7 天</option>
        </select>
      </label>
      <label>
        日志级别
        <select v-model="level">
          <option value="">全部级别</option>
          <option value="error">错误</option>
          <option value="warn">警告</option>
          <option value="info">信息</option>
          <option value="debug">调试</option>
        </select>
      </label>
      <label class="module-filter">
        模块
        <input v-model="moduleQuery" type="search" placeholder="例如 capture.worker" @keyup.enter="load(true)" />
      </label>
      <label class="event-filter">
        事件
        <input v-model="eventQuery" type="search" placeholder="例如 request_failed" @keyup.enter="load(true)" />
      </label>
      <label>
        结果
        <select v-model="outcome">
          <option value="">全部结果</option>
          <option value="succeeded">成功</option>
          <option value="degraded">降级</option>
          <option value="failed">失败</option>
          <option value="retrying">重试中</option>
        </select>
      </label>
      <label class="correlation-filter">
        关联 ID
        <input v-model="correlationQuery" type="search" placeholder="请求 / 项目 / 截图 ID" @keyup.enter="load(true)" />
      </label>
      <button type="button" class="log-filter-button" :disabled="busy || loadingMore" @click="load(true)">
        <LoaderCircle v-if="busy" class="animate-spin" :size="15" />
        <RefreshCw v-else :size="15" />
        应用筛选
      </button>
    </div>

    <p v-if="errorMessage" class="log-error" role="alert">{{ errorMessage }}</p>

    <div class="log-result-meta" aria-live="polite">
      <span>匹配 {{ matchedCount }} 条</span>
      <span>已加载 {{ records.length }} 条</span>
    </div>

    <div ref="logList" class="log-list" role="list" aria-label="调试日志记录" :aria-busy="busy || loadingMore">
      <article v-for="(record, index) in records" :key="`${record.timestamp}-${record.module}-${index}`" class="log-row" role="listitem">
        <time :datetime="record.timestamp">{{ formatTimestamp(record.timestamp) }}</time>
        <span class="log-level" :data-level="record.level">{{ levelLabel(record.level) }}</span>
        <code>{{ record.module }}</code>
        <div class="log-content">
          <p>{{ record.message }}</p>
          <div v-if="record.event || record.outcome || record.requestId || record.captureItemId" class="log-fields">
            <span v-if="record.event">事件 {{ record.event }}</span>
            <span v-if="record.outcome">结果 {{ record.outcome }}</span>
            <span v-if="record.requestId">请求 {{ record.requestId }}</span>
            <span v-if="record.captureItemId">截图 {{ record.captureItemId }}</span>
            <span v-if="record.durationMs != null">耗时 {{ record.durationMs.toFixed(1) }} ms</span>
          </div>
        </div>
      </article>
      <div v-if="!records.length && !busy" class="log-empty">当前筛选范围内没有日志。</div>
      <div v-if="busy && !records.length" class="log-empty"><LoaderCircle class="animate-spin" :size="18" />正在读取日志…</div>
      <div ref="loadSentinel" class="log-load-sentinel" aria-live="polite">
        <button v-if="hasMore" type="button" :disabled="loadingMore" @click="load(false)">
          <LoaderCircle v-if="loadingMore" class="animate-spin" :size="15" />
          {{ loadingMore ? "正在加载…" : `继续加载（剩余 ${Math.max(0, matchedCount - records.length)} 条）` }}
        </button>
        <span v-else-if="records.length">已加载当前筛选范围内的全部日志</span>
      </div>
    </div>

    <footer class="log-panel-footer">
      <button type="button" :disabled="busy || !status" @click="openDirectory">
        <FolderOpen :size="16" />打开日志目录<ExternalLink :size="12" />
      </button>
      <button type="button" :disabled="busy" @click="cleanup">
        <Trash2 :size="16" />按策略清理
      </button>
    </footer>
  </section>
</template>

<style scoped>
.debug-log-panel {
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1 1 0;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--card);
  box-shadow: var(--card-shadow);
}

.log-panel-heading,
.log-toolbar,
.log-panel-footer {
  display: flex;
  align-items: center;
}

.log-panel-heading {
  justify-content: space-between;
  gap: 16px;
  padding: 14px 16px;
  border-bottom: 1px solid var(--border);
}
.log-panel-heading h2 { margin: 0; font-size: 15px; font-weight: 700; }
.log-panel-heading p { margin: 4px 0 0; color: var(--muted-foreground); font-size: 11px; }
.copy-diagnostics { display: inline-flex; height: 36px; flex: none; align-items: center; gap: 7px; padding: 0 12px; border: 1px solid transparent; border-radius: 8px; background: var(--accent-gradient); color: var(--accent-foreground); font-size: 12px; font-weight: 600; }

.log-privacy-note {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  border-bottom: 1px solid color-mix(in srgb, var(--info) 24%, var(--border));
  background: color-mix(in srgb, var(--info) 8%, var(--card));
  color: var(--muted-foreground);
  font-size: 11px;
}
.log-privacy-note svg { flex: none; color: var(--info); }

.log-toolbar {
  flex-wrap: wrap;
  gap: 10px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
  background: color-mix(in srgb, var(--secondary) 52%, var(--card));
  color: var(--muted-foreground);
}
.log-toolbar > svg { flex: none; }
.log-toolbar label { display: flex; align-items: center; gap: 7px; font-size: 11px; font-weight: 600; }
.log-toolbar select,
.log-toolbar input { height: 34px; border: 1px solid var(--border); border-radius: 8px; background: var(--background); color: var(--foreground); font-size: 12px; }
.log-toolbar select { padding: 0 28px 0 9px; }
.log-toolbar input { width: 190px; padding: 0 10px; }
.module-filter, .event-filter, .correlation-filter { flex: 1 1 220px; }
.module-filter input, .event-filter input, .correlation-filter input { min-width: 130px; flex: 1; }
.log-filter-button { display: inline-flex; height: 34px; align-items: center; gap: 6px; padding: 0 12px; border: 1px solid color-mix(in srgb, var(--accent) 40%, var(--border)); border-radius: 8px; background: color-mix(in srgb, var(--accent) 10%, var(--secondary)); color: var(--foreground); font-size: 12px; font-weight: 600; }

.log-error { margin: 0; padding: 9px 16px; border-bottom: 1px solid color-mix(in srgb, var(--destructive) 35%, var(--border)); background: color-mix(in srgb, var(--destructive) 10%, var(--card)); color: var(--destructive); font-size: 12px; }
.log-result-meta { display: flex; justify-content: space-between; gap: 12px; padding: 7px 16px; border-bottom: 1px solid var(--border); color: var(--muted-foreground); font-size: 11px; }
.log-result-meta span:last-child { color: var(--warn); }

.log-list { min-height: 0; flex: 1 1 0; overflow: auto; background: var(--background); }
.log-row { display: grid; grid-template-columns: 132px 54px minmax(130px, 180px) minmax(240px, 1fr); align-items: start; gap: 12px; min-height: 42px; padding: 10px 16px; border-bottom: 1px solid color-mix(in srgb, var(--border) 72%, transparent); font-size: 12px; }
.log-row:hover { background: color-mix(in srgb, var(--muted) 70%, transparent); }
.log-row time { color: var(--muted-foreground); font-variant-numeric: tabular-nums; }
.log-row code { overflow: hidden; color: var(--accent); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.log-row p { overflow-wrap: anywhere; margin: 0; color: var(--foreground); line-height: 1.45; }
.log-content { min-width: 0; }
.log-fields { display: flex; flex-wrap: wrap; gap: 5px 10px; margin-top: 4px; color: var(--muted-foreground); font-size: 10px; }
.log-level { padding: 2px 7px; border-radius: 99px; background: var(--muted); color: var(--muted-foreground); font-size: 10px; font-weight: 700; text-align: center; }
.log-level[data-level="info"] { background: color-mix(in srgb, var(--info) 16%, var(--card)); color: var(--info); }
.log-level[data-level="warn"] { background: color-mix(in srgb, var(--warn) 17%, var(--card)); color: var(--warn); }
.log-level[data-level="error"] { background: color-mix(in srgb, var(--destructive) 15%, var(--card)); color: var(--destructive); }
.log-empty { display: flex; min-height: 180px; align-items: center; justify-content: center; gap: 8px; color: var(--muted-foreground); font-size: 12px; }
.log-load-sentinel { display: flex; min-height: 48px; align-items: center; justify-content: center; padding: 8px; color: var(--muted-foreground); font-size: 11px; }
.log-load-sentinel button { display: inline-flex; min-height: 32px; align-items: center; gap: 6px; padding: 0 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--secondary); color: var(--foreground); font-size: 11px; }

.log-panel-footer { gap: 8px; padding: 9px 14px; border-top: 1px solid var(--border); background: var(--card); }
.log-panel-footer button { display: inline-flex; height: 34px; align-items: center; gap: 7px; padding: 0 11px; border: 1px solid var(--border); border-radius: 8px; background: var(--secondary); color: var(--foreground); font-size: 11px; font-weight: 600; }
.log-panel-footer button:hover:not(:disabled) { border-color: color-mix(in srgb, var(--accent) 42%, var(--border)); }

button:disabled { cursor: not-allowed; opacity: .55; }
button:focus-visible,
select:focus-visible,
input:focus-visible { outline: 2px solid var(--ring); outline-offset: 2px; }

@media (max-width: 760px) {
  .log-panel-heading { align-items: stretch; flex-direction: column; }
  .copy-diagnostics { justify-content: center; }
  .log-privacy-note { align-items: flex-start; }
  .log-toolbar { align-items: stretch; }
  .log-toolbar > svg { display: none; }
  .log-toolbar label { min-width: calc(50% - 5px); flex: 1 1 160px; align-items: stretch; flex-direction: column; }
  .log-toolbar select,
  .log-toolbar input { width: 100%; }
  .log-filter-button { width: 100%; justify-content: center; }
  .log-row { grid-template-columns: 116px 50px minmax(0, 1fr); gap: 8px; }
  .log-row p { grid-column: 1 / -1; }
  .log-panel-footer { display: grid; grid-template-columns: 1fr 1fr; }
  .log-panel-footer button { justify-content: center; }
}
</style>
