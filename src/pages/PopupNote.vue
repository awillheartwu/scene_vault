<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { marked } from "marked";
import createDOMPurify from "dompurify";
defineProps<{ embedded?: boolean }>();
import {
  CheckCircle2,
  CloudUpload,
  Eye,
  ExternalLink,
  FileText,
  FolderOpen,
  LoaderCircle,
  Save,
  PenLine,
  TriangleAlert,
  X,
} from "@lucide/vue";
import {
  captureApi,
  type ProjectNote,
} from "@/lib/capture-api";

const note = ref<ProjectNote | null>(null);
const projectName = ref("");
const draft = ref("");
const busy = ref(false);
const saving = ref(false);
const errorMessage = ref("");
const opened = ref(false);
const lastSavedAt = ref<string | null>(null);
const previewMode = ref(false);
const autoSaveNotes = ref(true);
const unlisteners: UnlistenFn[] = [];

const syncLabel = computed(() => {
  if (!note.value) return "";
  if (note.value.status === "synced") return "已同步到归档目录";
  if (note.value.status === "failed") return "同步失败，自动重试中";
  return "等待同步…";
});

const syncOk = computed(() => note.value?.status === "synced");
const dirty = computed(() => note.value !== null && draft.value !== note.value.content);
const renderedMarkdown = computed(() => renderMarkdown(draft.value));

function resolveProjectId(): string | null {
  const saved = localStorage.getItem("scene-vault.capture.project");
  return saved || null;
}

async function load() {
  const projectId = resolveProjectId();
  if (!projectId) {
    note.value = null;
    projectName.value = "";
    errorMessage.value = "还没有选择项目，请先在主窗口的捕获页选择一个项目。";
    return;
  }
  busy.value = true;
  errorMessage.value = "";
  try {
    const [loaded, projects, appSettings] = await Promise.all([
      captureApi.getProjectNote(projectId),
      captureApi.listProjects(),
      captureApi.getAppSettings(),
    ]);
    note.value = loaded;
    draft.value = loaded.content;
    autoSaveNotes.value = appSettings.autoSaveNotes;
    projectName.value =
      projects.find((project) => project.id === projectId)?.name ?? "当前项目";
  } catch (error) {
    errorMessage.value = noteErrorText(error);
  } finally {
    busy.value = false;
  }
}

async function save() {
  if (saving.value) return;
  const projectId = resolveProjectId();
  if (!projectId || !note.value) return;
  // Nothing to save if the draft matches the persisted content.
  if (draft.value === note.value.content) return;
  saving.value = true;
  errorMessage.value = "";
  try {
    note.value = await captureApi.updateProjectNote(projectId, draft.value);
    lastSavedAt.value = new Date().toISOString();
  } catch (error) {
    errorMessage.value = noteErrorText(error);
  } finally {
    saving.value = false;
  }
}

function onBlur() {
  if (autoSaveNotes.value && dirty.value && note.value) {
    void save();
  }
}

function onWindowBlur() {
  // The setting covers the whole popup, not just the textarea: switching back
  // to the game or another app also saves the draft.
  if (autoSaveNotes.value && dirty.value && note.value) {
    void save();
  }
}

async function openNote() {
  const projectId = resolveProjectId();
  if (!projectId) return;
  try {
    const result = await captureApi.openProjectNote(projectId);
    opened.value = result.opened;
    if (!result.opened) errorMessage.value = "笔记文件尚未同步到磁盘，请先保存。";
  } catch (error) {
    errorMessage.value = noteErrorText(error);
  }
}

async function revealNote() {
  const projectId = resolveProjectId();
  if (!projectId) return;
  try {
    const result = await captureApi.revealProjectNote(projectId);
    if (!result.opened) errorMessage.value = "笔记文件尚未同步到磁盘，请先保存。";
  } catch (error) {
    errorMessage.value = noteErrorText(error);
  }
}

async function close() {
  try {
    await getCurrentWindow().hide();
  } catch {
    window.close();
  }
}

function onKeydown(event: KeyboardEvent) {
  if (event.target instanceof HTMLTextAreaElement) return;
  if (event.key === "Escape") {
    void close();
  } else if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
    event.preventDefault();
    void save();
  }
}

function normalizeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function noteErrorText(error: unknown): string {
  const message = normalizeError(error);
  return message.toLowerCase().includes("not found")
    ? "项目不存在或已被删除，请在主窗口的捕获页重新选择项目。"
    : message;
}

// Popular, widely maintained Markdown parser. The old hand-rolled renderer
// was replaced in favor of GFM support (tables, task lists, fenced code).
// The webview is sandboxed, so links open via target=_blank (system browser).
marked.setOptions({ gfm: true, breaks: true });
const mdRenderer = new marked.Renderer();
mdRenderer.link = ({ href, title, tokens }) =>
  `<a href="${href}" target="_blank" rel="noopener noreferrer"${title ? ` title="${title}"` : ""}>${tokens.map((token) => token.raw).join("")}</a>`;
marked.use({ renderer: mdRenderer });

// DOMPurify degrades to a no-op when it cannot build a real DOM sanitizer.
// Binding the factory explicitly to the popup window guarantees the browser
// code path (DOMParser + HTMLTemplateElement) inside the Tauri webview.
const purify = createDOMPurify(window);

const FALLBACK_ALLOWED = new Set([
  "A", "B", "BLOCKQUOTE", "BR", "CODE", "DEL", "EM", "H1", "H2", "H3", "H4",
  "H5", "H6", "HR", "INPUT", "LI", "OL", "P", "PRE", "STRONG", "TABLE",
  "TBODY", "TD", "TH", "THEAD", "TR", "UL",
]);
const FALLBACK_FORBIDDEN = new Set([
  "SCRIPT", "STYLE", "IFRAME", "OBJECT", "EMBED", "LINK", "META", "BASE",
  "FORM", "BUTTON",
]);

function fallbackSanitize(html: string): string {
  const doc = new DOMParser().parseFromString(`<div id="__md">${html}</div>`, "text/html");
  const root = doc.getElementById("__md");
  if (!root) return "";
  const sanitizeNode = (node: Element) => {
    const tag = node.tagName;
    if (FALLBACK_FORBIDDEN.has(tag)) {
      node.remove();
      return;
    }
    if (!FALLBACK_ALLOWED.has(tag)) {
      node.replaceWith(...Array.from(node.childNodes));
      return;
    }
    for (const attr of Array.from(node.attributes)) {
      const name = attr.name.toLowerCase();
      const value = attr.value;
      const safeAttr = ["href", "target", "rel", "title", "class", "checked", "disabled", "type"].includes(name);
      const safeHref = name !== "href" || /^(https?:|mailto:|#)/i.test(value);
      if (!safeAttr || !safeHref) node.removeAttribute(attr.name);
    }
    for (const child of Array.from(node.children)) sanitizeNode(child);
  };
  for (const child of Array.from(root.children)) sanitizeNode(child);
  return root.innerHTML;
}

function togglePreview() {
  previewMode.value = !previewMode.value;
}

function renderMarkdown(source: string): string {
  try {
    const html = marked.parse(source, { async: false }) as string;
    if (purify.isSupported) {
      return purify.sanitize(html, { ADD_ATTR: ["target", "rel"] });
    }
    return fallbackSanitize(html);
  } catch (error) {
    console.error("[note] markdown render failed:", error);
    return `<pre>${String(error)}</pre>`;
  }
}

watch(
  () => note.value?.status,
  () => {
    // Refresh the draft after a sync cycle completes elsewhere.
    if (note.value?.status === "synced" && draft.value !== note.value.content) {
      draft.value = note.value.content;
    }
  },
);

onMounted(async () => {
  document.body.style.minWidth = "0";
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("blur", onWindowBlur);
  try {
    unlisteners.push(
      await listen("note:refresh", () => void load()),
      await listen<unknown>("note:updated", () => {
        if (!document.hasFocus()) void load();
      }),
    );
  } catch {
    // Browser previews do not expose the Tauri event bridge.
  }
  await load();
});

onBeforeUnmount(() => {
  document.body.style.minWidth = "";
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("blur", onWindowBlur);
  unlisteners.forEach((unlisten) => unlisten());
});

// Expose save so the workbench container can auto-save when switching tabs.
defineExpose({
  saveIfDirty: () => {
    if (dirty.value && note.value) void save();
  },
});
</script>

<template>
  <section class="note-page" :class="{ embedded }">
    <header class="note-header" data-tauri-drag-region="deep">
      <div v-if="!embedded" class="note-title" data-tauri-drag-region="deep">
        <span class="note-dot" :class="{ synced: syncOk }" />
        <strong>笔记</strong>
        <span class="note-project">{{ projectName || "未选择项目" }}</span>
      </div>
      <div class="note-header-actions">
        <button
          type="button"
          class="note-icon"
          :class="{ active: previewMode }"
          :title="previewMode ? '切换到编辑' : '预览 Markdown'"
          aria-label="切换预览"
          @click="togglePreview"
        >
          <Eye v-if="!previewMode" :size="15" />
          <PenLine v-else :size="15" />
        </button>
        <button type="button" class="note-icon" title="在文件夹中显示" aria-label="在文件夹中显示" @click="revealNote">
          <FolderOpen :size="15" />
        </button>
        <button type="button" class="note-icon" title="用系统编辑器打开" aria-label="打开笔记文件" @click="openNote">
          <ExternalLink :size="15" />
        </button>
        <button v-if="!embedded" type="button" class="note-icon" aria-label="关闭" @click="close">
          <X :size="16" />
        </button>
      </div>
    </header>

    <div v-if="errorMessage" class="note-error" role="alert">
      <TriangleAlert :size="15" />{{ errorMessage }}
    </div>

    <div v-if="busy && !note" class="note-loading">
      <LoaderCircle class="animate-spin" :size="20" />加载中…
    </div>

    <template v-else>
      <textarea
        v-if="!previewMode"
        v-model="draft"
        class="note-editor"
        @blur="onBlur"
        :placeholder="note ? '记录此刻的想法… 支持 # 标题、**加粗**、1. 列表（数字后要有空格），点击右上角眼睛预览' : '暂无项目'"
        spellcheck="false"
        :disabled="!note"
      />
      <div v-else class="note-preview markdown-body" v-html="renderedMarkdown" />

      <footer class="note-footer">
        <span class="note-sync" :class="{ ok: syncOk }">
          <CloudUpload v-if="!syncOk" :size="13" />
          <CheckCircle2 v-else :size="13" />
          {{ note ? syncLabel : "" }}
        </span>
        <span v-if="note" class="note-meta">
          <FileText :size="12" />{{ previewMode ? "预览模式" : "Markdown" }}
        </span>
        <button
          type="button"
          class="note-save"
          :disabled="!note || !dirty || saving"
          @click="save"
        >
          <LoaderCircle v-if="saving" class="animate-spin" :size="15" />
          <Save v-else :size="15" />
          {{ dirty ? "保存" : "已保存" }}
        </button>
      </footer>
    </template>
  </section>
</template>

<style scoped>
.note-page {
  display: flex;
  width: 100%;
  height: 100vh;
  flex-direction: column;
  background: var(--bg-gradient);
  color: var(--foreground);
  user-select: none;
}

.note-page.embedded {
  height: 100%;
}

.note-header {
  display: flex;
  height: 44px;
  flex: none;
  align-items: center;
  justify-content: space-between;
  padding: 0 8px 0 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 72%, transparent);
  background: color-mix(in srgb, var(--sidebar) 72%, transparent);
  backdrop-filter: var(--panel-blur);
}

.note-page:not(.embedded) .note-header {
  height: 48px;
  padding: 0 10px 0 14px;
}

.note-title {
  display: flex;
  min-width: 0;
  align-items: center;
  gap: 8px;
  font-size: 12.5px;
}

.note-title strong {
  flex: none;
  font-weight: 700;
  letter-spacing: 0.04em;
}

.note-project {
  overflow: hidden;
  color: var(--muted-foreground);
  font-size: 11.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.note-dot {
  width: 8px;
  height: 8px;
  flex: none;
  border-radius: 50%;
  background: var(--warn);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--warn) 18%, transparent);
}

.note-dot.synced {
  background: var(--ok);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ok) 18%, transparent);
}

.note-header-actions {
  display: flex;
  flex: none;
  margin-left: auto;
  gap: 2px;
}

.note-icon {
  display: grid;
  width: 32px;
  height: 32px;
  place-items: center;
  border: 1px solid transparent;
  border-radius: 9px;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
  transition:
    background-color var(--motion-fast),
    border-color var(--motion-fast),
    color var(--motion-fast);
}

.note-page:not(.embedded) .note-icon {
  width: 40px;
  height: 40px;
  border-radius: 10px;
}

.note-icon:hover {
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--foreground);
}

.note-icon:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.note-error {
  display: flex;
  flex: none;
  align-items: center;
  gap: 8px;
  margin: 10px 12px 0;
  padding: 8px 11px;
  border: 1px solid color-mix(in srgb, var(--destructive) 45%, var(--border));
  border-radius: 10px;
  background: color-mix(in srgb, var(--destructive) 12%, var(--card));
  color: var(--destructive);
  font-size: 11.5px;
  line-height: 1.5;
}

.note-loading {
  display: flex;
  flex: 1;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--muted-foreground);
  font-size: 12px;
}

.note-editor {
  min-height: 0;
  flex: 1;
  margin: 12px 12px 0;
  padding: 12px 14px;
  resize: none;
  border: 1px solid var(--border);
  border-radius: 12px;
  outline: none;
  background: color-mix(in srgb, var(--card) 84%, transparent);
  color: var(--foreground);
  font-family: "Cascadia Code", "JetBrains Mono", "Consolas", monospace;
  font-size: 13px;
  line-height: 1.65;
  box-shadow: var(--inner-highlight);
  user-select: text;
}

.note-editor:focus,
.note-editor:focus-visible {
  border-color: color-mix(in srgb, var(--accent) 55%, var(--border));
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ring) 22%, transparent);
  outline: none;
}

.note-preview {
  min-height: 0;
  flex: 1;
  margin: 12px 12px 0;
  padding: 12px 16px;
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: color-mix(in srgb, var(--card) 84%, transparent);
  box-shadow: var(--inner-highlight);
  user-select: text;
}

.note-icon.active {
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--accent);
}

.note-editor::placeholder {
  color: var(--muted-foreground);
}

.note-footer {
  display: flex;
  flex: none;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
}

.note-sync {
  display: inline-flex;
  flex: none;
  align-items: center;
  gap: 5px;
  color: var(--warn);
  font-size: 11px;
}

.note-sync.ok {
  color: var(--ok);
}

.note-meta {
  display: flex;
  min-width: 0;
  flex: 1;
  align-items: center;
  gap: 5px;
  overflow: hidden;
  color: var(--muted-foreground);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.note-save {
  display: inline-flex;
  height: 40px;
  flex: none;
  align-items: center;
  gap: 6px;
  padding: 0 14px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: var(--accent-gradient);
  color: var(--accent-foreground);
  font-size: 12px;
  font-weight: 700;
  box-shadow: var(--glow);
  cursor: pointer;
}

.note-save:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.note-save:disabled {
  opacity: 0.55;
  cursor: not-allowed;
  box-shadow: none;
}
</style>
<style>
/* Non-scoped on purpose: these style the v-html markdown preview, whose nodes
   carry no scoped data attributes. Scoped selectors would silently no-op. */
.markdown-body {
  font-size: 13px;
  line-height: 1.7;
}

.markdown-body h1,
.markdown-body h2,
.markdown-body h3,
.markdown-body h4,
.markdown-body h5,
.markdown-body h6 {
  margin: 0.9em 0 0.45em;
  color: var(--foreground);
  line-height: 1.3;
}

.markdown-body h1 {
  font-size: 1.6em;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
  padding-bottom: 0.25em;
}

.markdown-body h2 {
  font-size: 1.35em;
}

.markdown-body h3 {
  font-size: 1.18em;
}

.markdown-body h4 {
  font-size: 1.08em;
}

.markdown-body h5,
.markdown-body h6 {
  font-size: 1em;
}

.markdown-body p {
  margin: 0.45em 0;
}

.markdown-body ul,
.markdown-body ol {
  margin: 0.4em 0;
  padding-left: 1.6em;
  /* Tailwind v4 preflight resets ol/ul/menu to list-style:none; restore the
     markers inside the rendered note preview. */
  list-style: revert;
}

.markdown-body ul {
  list-style-type: disc;
}

.markdown-body ol {
  list-style-type: decimal;
}

.markdown-body li {
  margin: 0.2em 0;
}

.markdown-body li > p {
  margin: 0.15em 0;
}

.markdown-body strong {
  color: var(--foreground);
  font-weight: 700;
}

.markdown-body code {
  padding: 0.15em 0.4em;
  border-radius: 5px;
  background: color-mix(in srgb, var(--muted-foreground) 14%, transparent);
  color: var(--accent);
  font-family: "Cascadia Code", "JetBrains Mono", "Consolas", monospace;
  font-size: 0.9em;
}

.markdown-body pre {
  margin: 0.6em 0;
  padding: 10px 12px;
  overflow-x: auto;
  border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
  border-radius: 9px;
  background: color-mix(in srgb, var(--background) 80%, transparent);
}

.markdown-body pre code {
  padding: 0;
  background: transparent;
  color: var(--foreground);
}

.markdown-body blockquote {
  margin: 0.5em 0;
  padding: 0.3em 0.9em;
  border-left: 3px solid var(--accent);
  color: var(--muted-foreground);
  background: color-mix(in srgb, var(--accent) 7%, transparent);
}

.markdown-body a {
  color: var(--accent);
  text-decoration: underline;
  text-underline-offset: 2px;
}

.markdown-body li input[type="checkbox"] {
  margin: 0 6px 0 0;
  accent-color: var(--accent);
}

.markdown-body table {
  margin: 0.6em 0;
  border-collapse: collapse;
  font-size: 0.95em;
}

.markdown-body th,
.markdown-body td {
  padding: 5px 10px;
  border: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
}

.markdown-body th {
  background: color-mix(in srgb, var(--muted-foreground) 10%, transparent);
  font-weight: 600;
}

.markdown-body hr {
  margin: 0.8em 0;
  border: 0;
  border-top: 1px solid var(--border);
}
</style>
