<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
defineProps<{ embedded?: boolean }>();
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import {
  Check,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Image,
  ImageOff,
  LoaderCircle,
  Lock,
  Pin,
  PinOff,
  Plus,
  Search,
  Sparkles,
  UserRound,
  X,
} from "@lucide/vue";
import {
  captureApi,
  pathMimeType,
  type CaptureClassification,
  type CaptureItem,
  type Character,
  type ClassifyPopupContext,
  type VerificationResult,
} from "@/lib/capture-api";
import CaptureProgress from "@/components/capture/CaptureProgress.vue";

const ctx = ref<ClassifyPopupContext | null>(null);
const currentIndex = ref(0);
const mode = ref<"choose" | "person">("choose");
const selectedCharacterId = ref<string | null>(null);
const search = ref("");
const newName = ref("");
const previewUrl = ref<string | null>(null);
const busy = ref(false);
const errorMessage = ref("");
const autoCloseEmpty = ref(false);
const pinned = ref(false);
const pendingVerification = ref<{
  characterId: string;
  result: VerificationResult;
} | null>(null);
let emptyTimer: ReturnType<typeof setTimeout> | null = null;
let previewRequest = 0;
const unlisteners: UnlistenFn[] = [];

const currentItem = computed(() => ctx.value?.items[currentIndex.value] ?? null);
const suggestedCharacter = computed(() => {
  const item = currentItem.value;
  if (!item || item.reviewStatus !== "pending" || !item.suggestedCharacterId) return null;
  return (
    ctx.value?.characters.find((character) => character.id === item.suggestedCharacterId) ?? null
  );
});

const characters = computed(() => {
  if (!ctx.value) return [];
  const query = search.value.trim().toLocaleLowerCase();
  return query
    ? ctx.value.characters.filter((character) => character.name.toLocaleLowerCase().includes(query))
    : ctx.value.characters;
});

async function load() {
  busy.value = true;
  errorMessage.value = "";
  try {
    const [context, appSettings] = await Promise.all([
      captureApi.classifyPopupContext(),
      captureApi.getAppSettings().catch(() => null),
    ]);
    ctx.value = context;
    autoCloseEmpty.value = appSettings?.autoCloseEmptyPopup ?? false;
    currentIndex.value = 0;
    mode.value = "choose";
    selectedCharacterId.value = null;
    search.value = "";
    if (!ctx.value?.items.length) {
      scheduleAutoClose();
    }
  } catch (error) {
    errorMessage.value = normalizeError(error);
    console.error("[popup] load failed:", error);
  } finally {
    busy.value = false;
  }
}

async function loadPreview() {
  const request = ++previewRequest;
  if (previewUrl.value) URL.revokeObjectURL(previewUrl.value);
  previewUrl.value = null;
  const item = currentItem.value;
  if (!item) return;
  try {
    const bytes = await captureApi.readImage(item.id, "source");
    const nextUrl = URL.createObjectURL(
      new Blob([bytes], { type: pathMimeType(item.sourcePath) }),
    );
    if (request !== previewRequest) {
      URL.revokeObjectURL(nextUrl);
      return;
    }
    previewUrl.value = nextUrl;
  } catch (error) {
    if (request !== previewRequest) return;
    errorMessage.value = normalizeError(error);
  }
}

function scheduleAutoClose() {
  if (!autoCloseEmpty.value) return;
  if (emptyTimer) return;
  emptyTimer = setTimeout(() => {
    emptyTimer = null;
    void close();
  }, 3000);
}

function cancelAutoClose() {
  if (emptyTimer) {
    clearTimeout(emptyTimer);
    emptyTimer = null;
  }
}

function choosePerson() {
  mode.value = "person";
  void refreshCurrentSuggestion();
}

// Mirrors the Capture page: re-run the face-bank comparison with the freshest
// samples when the user is about to label a person capture, so a suggestion
// uses samples enrolled earlier in the same session. Best-effort: the stored
// suggestion remains if the refresh fails.
async function refreshCurrentSuggestion() {
  const item = currentItem.value;
  if (!item || busy.value) return;
  try {
    const updated = await captureApi.suggestForCapture(item.id);
    const ctxValue = ctx.value;
    if (!ctxValue) return;
    const index = ctxValue.items.findIndex((entry) => entry.id === updated.id);
    if (index < 0) return;
    const items = [...ctxValue.items];
    items[index] = updated;
    ctx.value = { ...ctxValue, items };
  } catch {
    // Best-effort refresh; the previously stored suggestion stays visible.
  }
}

function resetSelection() {
  mode.value = "choose";
  selectedCharacterId.value = null;
  search.value = "";
  newName.value = "";
  pendingVerification.value = null;
}

function selectPrevious() {
  if (currentIndex.value <= 0) return;
  currentIndex.value -= 1;
  resetSelection();
}

function selectNext() {
  if (!ctx.value || currentIndex.value >= ctx.value.items.length - 1) return;
  currentIndex.value += 1;
  resetSelection();
}

function removeItem(itemId: string) {
  if (!ctx.value) return;
  const remaining = ctx.value.items.filter((item) => item.id !== itemId);
  ctx.value = { ...ctx.value, items: remaining };
  if (currentIndex.value >= remaining.length) {
    currentIndex.value = Math.max(0, remaining.length - 1);
  }
}

// After the last capture is classified the workbench stays open: the same
// window also hosts the note editor, so closing it would interrupt writing.
// Only when the user opted into "auto close on empty" does the 3s timer run.
function finishAll() {
  resetSelection();
  if (autoCloseEmpty.value) scheduleAutoClose();
}

// Refresh the queue in place when a new screenshot is discovered, without
// resetting the user's current selection or the character input.
async function refreshItems() {
  busy.value = true;
  errorMessage.value = "";
  try {
    const context = await captureApi.classifyPopupContext();
    ctx.value = context;
    if (currentIndex.value >= context.items.length) {
      currentIndex.value = Math.max(0, context.items.length - 1);
    }
    if (context.items.length) {
      cancelAutoClose();
    } else {
      scheduleAutoClose();
    }
  } catch (error) {
    errorMessage.value = normalizeError(error);
  } finally {
    busy.value = false;
  }
}

async function commit(classification: CaptureClassification, characterId: string | null = null) {
  const item = currentItem.value;
  if (!item || busy.value) return;
  busy.value = true;
  errorMessage.value = "";
  try {
    await captureApi.label(item.id, characterId, classification);
    removeItem(item.id);
    if (ctx.value?.items.length) {
      // Keep working through the queue: switch to the next capture instead of
      // closing, and only hide the popup once everything is classified.
      resetSelection();
    } else {
      finishAll();
    }
  } catch (error) {
    errorMessage.value = normalizeError(error);
  } finally {
    busy.value = false;
  }
}

async function submitCharacter(character: Character) {
  const item = currentItem.value;
  if (!item || busy.value) return;
  busy.value = true;
  errorMessage.value = "";
  try {
    const result = await captureApi.verifyCaptureIdentity({
      captureItemId: item.id,
      characterId: character.id,
    });
    if (result.level === "ok" || result.level === "unverified") {
      busy.value = false;
      await commit("person", character.id);
    } else {
      pendingVerification.value = { characterId: character.id, result };
    }
  } catch (error) {
    errorMessage.value = normalizeError(error);
  } finally {
    busy.value = false;
  }
}

async function confirmForcedConfirm() {
  const pending = pendingVerification.value;
  if (!pending || busy.value) return;
  pendingVerification.value = null;
  busy.value = false;
  await commit("person", pending.characterId);
}

function cancelVerification() {
  pendingVerification.value = null;
}

const verificationWarning = computed(() => {
  const pending = pendingVerification.value;
  if (!pending) return null;
  const { result } = pending;
  const score =
    result.score != null ? `${Math.round(result.score * 100)}%` : "—";
  const characterName =
    ctx.value?.characters.find(
      (character) => character.id === pending.characterId,
    )?.name ?? "";
  if (result.level === "strong") {
    const other =
      result.bestOtherCharacterId
        ? (ctx.value?.characters.find(
            (character) => character.id === result.bestOtherCharacterId,
          )?.name ?? "其他角色")
        : null;
    const otherText =
      other && result.bestOtherScore != null
        ? `，更像 ${other}（${Math.round(result.bestOtherScore * 100)}%）`
        : "";
    return `相似度很低（${score}）${otherText}，很可能标错了角色！`;
  }
  return `相似度偏低（${score}），确认这张脸是「${characterName}」吗？`;
});

function acceptSuggestion() {
  const character = suggestedCharacter.value;
  if (character) void submitCharacter(character);
}

async function createAndSubmit() {
  const name = newName.value.trim();
  const projectId = ctx.value?.projectId;
  if (!name || !projectId || busy.value) return;
  busy.value = true;
  errorMessage.value = "";
  try {
    const character = await captureApi.createCharacter(projectId, name);
    newName.value = "";
    // Keep the picker fresh: without this the newly created character would
    // only appear after a full context reload, so the next capture's list
    // would miss it.
    if (ctx.value) {
      ctx.value = {
        ...ctx.value,
        characters: [...ctx.value.characters, character].sort((a, b) =>
          a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
        ),
      };
    }
    // commit() re-enters busy handling; release the guard we set for the
    // create call so the label step is not short-circuited.
    busy.value = false;
    await commit("person", character.id);
  } catch (error) {
    errorMessage.value = normalizeError(error);
  } finally {
    busy.value = false;
  }
}

async function close() {
  try {
    // Hide instead of destroy: a destroyed popup window would leave the
    // manager, so the global shortcut could no longer find it. A hidden
    // window is instantly re-shown on the next shortcut press.
    await getCurrentWindow().hide();
  } catch {
    window.close();
  }
}

async function syncPinState() {
  try {
    pinned.value = await getCurrentWindow().isAlwaysOnTop();
  } catch {
    pinned.value = false;
  }
}

async function togglePin() {
  const next = !pinned.value;
  try {
    await invoke("set_window_always_on_top", {
      windowLabel: getCurrentWindow().label,
      always: next,
    });
    pinned.value = next;
  } catch (error) {
    errorMessage.value = normalizeError(error);
  }
}

function onKeydown(event: KeyboardEvent) {
  if (event.target instanceof Element && event.target.matches("input")) return;
  if (event.target instanceof HTMLTextAreaElement) return;
  if (event.key === "Escape") {
    if (mode.value === "person") {
      mode.value = "choose";
      selectedCharacterId.value = null;
    } else {
      void close();
    }
  } else if (event.key === "Enter" && mode.value === "person") {
    const character =
      characters.value.find((value) => value.id === selectedCharacterId.value) ??
      characters.value[0];
    if (character) void submitCharacter(character);
  } else if (/^[1-5]$/.test(event.key) && mode.value === "person") {
    const character = characters.value[Number(event.key) - 1];
    if (character) selectedCharacterId.value = character.id;
  } else if (event.key === "ArrowLeft") {
    selectPrevious();
  } else if (event.key === "ArrowRight") {
    selectNext();
  }
}

function normalizeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

watch(
  () => currentItem.value?.id,
  () => void loadPreview(),
);

onMounted(async () => {
  // The shared bundle sets body { min-width: 1080px } for the main window;
  // the popup is only ~460px wide, so neutralize it here or content gets
  // pushed off to the right.
  document.body.style.minWidth = "0";
  void syncPinState();
  window.addEventListener("keydown", onKeydown);
  try {
    unlisteners.push(
      await listen("classify:refresh", () => {
        void load();
      }),
      await listen<CaptureItem>("capture:item-updated", (event) => {
        const updated = event.payload;
        if (!updated?.id || !ctx.value) return;
        const index = ctx.value.items.findIndex((item) => item.id === updated.id);
        if (index < 0) return;
        // Feature/suggestion updates keep the capture in the queue: refresh
        // it in place so a fresh recommendation shows up. Only a capture
        // that was actually classified (or reclassified) elsewhere leaves
        // the queue.
        if (updated.status === "awaiting_label" && updated.classification === "unclassified") {
          const items = [...ctx.value.items];
          items[index] = updated;
          ctx.value = { ...ctx.value, items };
          return;
        }
        removeItem(updated.id);
        if (!ctx.value?.items.length) {
          finishAll();
        } else {
          resetSelection();
          void loadPreview();
        }
      }),
      await listen("capture:item-created", () => {
        // A new screenshot arrived (Rust discovery event, push-based): make it
        // appear in the open popup immediately.
        void refreshItems();
      }),
      await listen("capture:face-bank-rebuilt", () => {
        // Suggestions were refreshed for many captures; reload the queue so
        // pending recommendations show up without reopening the popup.
        void refreshItems();
      }),
    );
  } catch {
    // Browser previews do not expose the Tauri event bridge.
    console.error("[popup] event bridge unavailable");
  }
  await load();
});

onBeforeUnmount(() => {
  previewRequest += 1;
  if (emptyTimer) {
    clearTimeout(emptyTimer);
    emptyTimer = null;
  }
  document.body.style.minWidth = "";
  window.removeEventListener("keydown", onKeydown);
  unlisteners.forEach((unlisten) => unlisten());
  if (previewUrl.value) URL.revokeObjectURL(previewUrl.value);
});
</script>

<template>
  <section class="popup-page" :class="{ embedded }">
    <header v-if="!embedded" class="popup-header" data-tauri-drag-region="deep">
      <div class="popup-title" data-tauri-drag-region="deep">
        <span class="popup-dot" />
        <strong>{{ ctx?.projectName ?? "Scene Vault" }}</strong>
        <span v-if="currentItem" class="popup-sub">
          {{ ctx?.items.length ? `${currentIndex + 1}/${ctx.items.length}` : "待分类截图" }}
        </span>
      </div>
      <button
        type="button"
        class="popup-pin"
        :class="{ pinned }"
        :aria-label="pinned ? '取消置顶' : '置顶'"
        :title="pinned ? '取消置顶' : '置顶'"
        @click="togglePin"
      >
        <Pin v-if="pinned" :size="16" /><PinOff v-else :size="16" />
      </button>
      <button type="button" class="popup-close" aria-label="关闭" @click="close"><X :size="16" /></button>
    </header>

    <div v-if="errorMessage" class="popup-error" role="alert">{{ errorMessage }}</div>

    <div v-if="busy && !currentItem" class="popup-loading">
      <LoaderCircle class="animate-spin" :size="22" />加载中…
    </div>

    <template v-else-if="currentItem">
      <div :class="['popup-preview', { compact: mode === 'person' }]">
        <img v-if="previewUrl" :src="previewUrl" alt="待分类截图预览" />
        <div v-else class="popup-preview-empty"><ImageOff :size="28" /><span>无法加载预览</span></div>
      </div>
      <CaptureProgress persistent />

      <div v-if="ctx && ctx.items.length > 1" class="popup-nav">
        <button type="button" class="popup-nav-button" :disabled="currentIndex === 0" aria-label="上一张" @click="selectPrevious">
          <ChevronLeft :size="16" />
        </button>
        <span class="popup-nav-count">{{ currentIndex + 1 }} / {{ ctx.items.length }}</span>
        <button
          type="button"
          class="popup-nav-button"
          :disabled="currentIndex >= ctx.items.length - 1"
          aria-label="下一张"
          @click="selectNext"
        >
          <ChevronRight :size="16" />
        </button>
        <span class="popup-nav-hint">← → 切换</span>
      </div>

      <div v-if="mode === 'choose'" class="popup-body">
        <button
          v-if="suggestedCharacter"
          type="button"
          class="popup-suggestion"
          :disabled="busy"
          @click="acceptSuggestion"
        >
          <Sparkles :size="17" />
          <span>
            <strong>推荐：{{ suggestedCharacter.name }}</strong>
            <small>相似度 {{ Math.round((currentItem?.recognitionConfidence ?? 0) * 100) }}%</small>
          </span>
          <Check :size="16" />
        </button>
        <p
          v-if="currentItem?.faceCount && currentItem.faceCount > 1"
          class="multi-face-note"
        >
          ⚠ 检测到 {{ currentItem.faceCount }} 张脸，建议基于主脸
        </p>
        <p class="popup-hint">这张截图属于哪一类？</p>
        <div class="popup-actions">
          <button type="button" class="popup-action person" @click="choosePerson">
            <UserRound :size="18" /><span><strong>人物</strong><small>选择角色并标注</small></span>
          </button>
          <button type="button" class="popup-action" :disabled="busy" @click="commit('scene')">
            <Image :size="18" /><span><strong>游戏截图</strong><small>原图直存</small></span>
          </button>
          <button type="button" class="popup-action private" :disabled="busy" @click="commit('private')">
            <Lock :size="18" /><span><strong>收藏</strong><small>默认隐藏</small></span>
          </button>
        </div>
        <button type="button" class="popup-cancel" @click="close">暂不处理（Esc）</button>
      </div>

      <div v-else class="popup-body popup-body--fill">
        <div class="popup-person-head">
          <p class="popup-hint">选择人物（点击即提交）</p>
          <button type="button" class="popup-back" @click="mode = 'choose'">‹ 返回</button>
        </div>
        <button
          v-if="suggestedCharacter"
          type="button"
          class="popup-suggestion"
          :disabled="busy"
          @click="acceptSuggestion"
        >
          <Sparkles :size="17" />
          <span>
            <strong>推荐：{{ suggestedCharacter.name }}</strong>
            <small>相似度 {{ Math.round((currentItem?.recognitionConfidence ?? 0) * 100) }}%</small>
          </span>
          <Check :size="16" />
        </button>
        <div v-if="pendingVerification" class="popup-verification" role="alert">
          <p>{{ verificationWarning }}</p>
          <div class="popup-verification-actions">
            <button type="button" class="popup-verification-confirm" :disabled="busy" @click="confirmForcedConfirm">仍确认</button>
            <button type="button" :disabled="busy" @click="cancelVerification">返回</button>
          </div>
        </div>
        <label class="popup-search">
          <Search :size="15" />
          <input v-model="search" placeholder="搜索角色" />
        </label>
        <div class="popup-characters">
          <button
            v-for="(character, index) in characters"
            :key="character.id"
            type="button"
            class="popup-character"
            :class="{ selected: selectedCharacterId === character.id }"
            :disabled="busy"
            @click="submitCharacter(character)"
          >
            <span class="popup-avatar"><UserRound :size="15" /></span>
            <span class="popup-character-name">{{ character.name }}</span>
            <kbd v-if="index < 5">{{ index + 1 }}</kbd>
            <Check v-if="selectedCharacterId === character.id" :size="15" />
          </button>
          <p v-if="!characters.length" class="popup-empty">还没有角色，先创建一个：</p>
        </div>
        <form class="popup-create" @submit.prevent="createAndSubmit">
          <input v-model="newName" placeholder="新建角色名称" />
          <button type="submit" :disabled="busy || !newName.trim()"><Plus :size="15" />创建并提交</button>
        </form>
        <p class="popup-tip">Enter 提交选中 · Esc 返回 / 关闭</p>
      </div>
    </template>

    <div v-else class="popup-empty-state">
      <CheckCircle2 :size="30" />
      <strong>当前没有待分类截图</strong>
      <span>
        新截图会自动出现
        <template v-if="autoCloseEmpty">· 3 秒后自动关闭</template>
      </span>
    </div>
  </section>
</template>

<style scoped>
.popup-page {
  display: flex;
  width: 100%;
  height: 100vh;
  flex-direction: column;
  background: var(--bg-gradient);
  color: var(--foreground);
  user-select: none;
}

.popup-page.embedded {
  height: 100%;
}

.popup-header {
  display: flex;
  height: 48px;
  flex: none;
  align-items: center;
  justify-content: space-between;
  padding: 0 10px 0 14px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 72%, transparent);
  background: color-mix(in srgb, var(--sidebar) 72%, transparent);
  backdrop-filter: var(--panel-blur);
}

.popup-title {
  display: flex;
  flex: 1;
  min-width: 0;
  align-items: center;
  gap: 9px;
  font-size: 12.5px;
}

.popup-title strong {
  overflow: hidden;
  font-weight: 700;
  letter-spacing: 0.04em;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.popup-dot {
  width: 8px;
  height: 8px;
  flex: none;
  border-radius: 50%;
  background: var(--ok);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ok) 18%, transparent);
}

.popup-sub {
  flex: none;
  padding: 2px 8px;
  border-radius: 99px;
  background: color-mix(in srgb, var(--warn) 20%, var(--card));
  color: var(--warn);
  font-size: 11px;
  font-weight: 600;
}

.popup-close,
.popup-pin {
  display: grid;
  width: 40px;
  height: 40px;
  margin-left: 4px;
  place-items: center;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--muted-foreground);
  cursor: pointer;
  transition:
    background-color var(--motion-fast),
    border-color var(--motion-fast),
    color var(--motion-fast);
}

.popup-close:hover {
  background: color-mix(in srgb, var(--destructive) 14%, transparent);
  color: var(--destructive);
}

.popup-close:focus-visible,
.popup-pin:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.popup-pin:hover {
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--accent);
}

.popup-pin.pinned {
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--accent);
}

.popup-error {
  margin: 10px 14px 0;
  padding: 8px 11px;
  border: 1px solid color-mix(in srgb, var(--destructive) 45%, var(--border));
  border-radius: 10px;
  background: color-mix(in srgb, var(--destructive) 12%, var(--card));
  color: var(--destructive);
  font-size: 11.5px;
  line-height: 1.5;
}

.popup-loading,
.popup-empty-state {
  display: flex;
  flex: 1;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 9px;
  color: var(--muted-foreground);
  font-size: 12px;
}

.popup-empty-state svg {
  color: var(--ok);
}

.popup-empty-state strong {
  color: var(--foreground);
  font-size: 14px;
}

.popup-preview {
  display: grid;
  flex: 1 1 0;
  min-height: 180px;
  margin: 12px 14px 0;
  overflow: hidden;
  place-items: center;
  border: 1px solid color-mix(in srgb, var(--border) 78%, transparent);
  border-radius: 12px;
  background: #0d1114;
  box-shadow: var(--card-shadow), var(--inner-highlight);
}

.popup-preview.compact {
  flex: 0 0 190px;
  min-height: 0;
}

.popup-preview img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
}

.popup-preview-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  color: var(--muted-foreground);
  font-size: 11.5px;
}

.popup-nav {
  display: flex;
  height: 44px;
  flex: none;
  align-items: center;
  justify-content: center;
  gap: 8px;
  margin: 8px 14px 0;
}

.popup-nav-button {
  display: grid;
  width: 40px;
  height: 40px;
  place-items: center;
  border: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
  border-radius: 10px;
  background: color-mix(in srgb, var(--card) 78%, transparent);
  color: var(--foreground);
  box-shadow: var(--inner-highlight);
  cursor: pointer;
  transition: border-color 0.18s, color 0.18s;
}

.popup-nav-button:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.popup-nav-button:hover:not(:disabled) {
  border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  color: var(--accent);
}

.popup-nav-button:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.popup-nav-count {
  min-width: 46px;
  color: var(--muted-foreground);
  font-size: 11.5px;
  font-variant-numeric: tabular-nums;
  text-align: center;
}

.popup-nav-hint {
  margin-left: 2px;
  color: var(--muted-foreground);
  font-size: 11px;
}

.popup-body {
  display: flex;
  min-height: 0;
  flex: 0 0 auto;
  flex-direction: column;
  gap: 10px;
  padding: 12px 14px;
}

.popup-body--fill {
  flex: 1 1 0;
}

.popup-hint {
  margin: 0;
  color: var(--muted-foreground);
  font-size: 12px;
  font-weight: 600;
}

.multi-face-note {
  margin: 0 0 10px;
  color: var(--muted-foreground);
  font-size: 12px;
  line-height: 1.5;
}

.popup-suggestion {
  display: flex;
  width: 100%;
  min-height: 50px;
  align-items: center;
  gap: 10px;
  margin-bottom: 10px;
  padding: 0 14px;
  border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--border));
  border-radius: 10px;
  background: color-mix(in srgb, var(--accent) 13%, var(--card));
  color: var(--foreground);
  box-shadow: var(--card-shadow), var(--inner-highlight);
  cursor: pointer;
  transition: border-color 0.18s, transform 0.15s, box-shadow 0.18s;
}

.popup-suggestion:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.popup-suggestion:hover:not(:disabled) {
  border-color: var(--accent);
  transform: translateY(-1px);
  box-shadow: var(--card-shadow-hover);
}

.popup-suggestion > svg:first-child {
  flex: none;
  color: var(--accent);
}

.popup-suggestion > svg:last-child {
  flex: none;
  margin-left: auto;
  color: var(--accent);
}

.popup-suggestion span {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
}

.popup-suggestion strong {
  font-size: 13px;
}

.popup-suggestion small {
  color: var(--muted-foreground);
  font-size: 11px;
}

.popup-verification {
  display: flex;
  flex-direction: column;
  gap: 9px;
  margin-bottom: 10px;
  padding: 11px 12px;
  border: 1px solid color-mix(in srgb, var(--warn) 55%, var(--border));
  border-radius: 10px;
  background: color-mix(in srgb, var(--warn) 14%, var(--card));
}

.popup-verification p {
  margin: 0;
  color: var(--foreground);
  font-size: 12px;
  line-height: 1.5;
}

.popup-verification-actions {
  display: flex;
  gap: 8px;
}

.popup-verification-actions button {
  height: 40px;
  padding: 0 14px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--secondary);
  color: var(--foreground);
  font-size: 11.5px;
  cursor: pointer;
}

.popup-verification-actions button:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.popup-verification-actions .popup-verification-confirm {
  border-color: var(--warn);
  background: color-mix(in srgb, var(--warn) 22%, var(--card));
  color: var(--warn);
  font-weight: 700;
}

.popup-actions {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.popup-action {
  display: flex;
  min-height: 52px;
  align-items: center;
  gap: 12px;
  padding: 0 14px;
  border: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
  border-radius: 10px;
  background: var(--card);
  color: var(--foreground);
  box-shadow: var(--card-shadow), var(--inner-highlight);
  cursor: pointer;
  transition: border-color 0.18s, transform 0.15s, box-shadow 0.18s;
}

.popup-action:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.popup-action:hover:not(:disabled) {
  border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  transform: translateY(-1px);
  box-shadow: var(--card-shadow-hover);
}

.popup-action.person svg {
  color: var(--accent);
}

.popup-action.private svg {
  color: var(--warn);
}

.popup-action svg {
  flex: none;
  color: var(--info);
}

.popup-action span {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
}

.popup-action strong {
  font-size: 13px;
}

.popup-action small {
  color: var(--muted-foreground);
  font-size: 11px;
}

.popup-cancel {
  min-height: 40px;
  margin-top: auto;
  padding: 0 12px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--muted-foreground);
  font-size: 11.5px;
  cursor: pointer;
}

.popup-cancel:hover {
  background: color-mix(in srgb, var(--secondary) 55%, transparent);
  color: var(--foreground);
}

.popup-cancel:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.popup-person-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.popup-back {
  min-height: 40px;
  padding: 0 12px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--accent);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.popup-back:hover {
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}

.popup-back:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.popup-search {
  display: flex;
  height: 40px;
  flex: none;
  align-items: center;
  gap: 8px;
  padding: 0 11px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: color-mix(in srgb, var(--background) 78%, transparent);
  color: var(--muted-foreground);
  transition: border-color var(--motion-fast), box-shadow var(--motion-fast);
}

.popup-search:focus-within {
  border-color: color-mix(in srgb, var(--accent) 65%, var(--border));
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ring) 22%, transparent);
}

.popup-search input {
  min-width: 0;
  flex: 1;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--foreground);
  font-size: 12px;
}

.popup-characters {
  display: flex;
  min-height: 0;
  flex: 1;
  flex-direction: column;
  gap: 6px;
  overflow-y: auto;
  overflow-x: hidden;
}

.popup-character {
  display: grid;
  min-height: 44px;
  grid-template-columns: 30px minmax(0, 1fr) auto auto;
  align-items: center;
  gap: 9px;
  padding: 5px 10px;
  border: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
  border-radius: 10px;
  background: var(--secondary);
  color: var(--foreground);
  text-align: left;
  cursor: pointer;
  transition: border-color 0.18s, transform 0.15s;
}

.popup-character:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.popup-character:hover:not(:disabled) {
  border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  transform: translateY(-1px);
}

.popup-character.selected {
  border-color: var(--accent);
  box-shadow: inset 3px 0 var(--accent);
}

.popup-avatar {
  display: grid;
  width: 28px;
  height: 28px;
  place-items: center;
  border-radius: 8px;
  background: var(--muted);
  color: var(--muted-foreground);
}

.popup-character-name {
  overflow: hidden;
  font-size: 12px;
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.popup-character kbd {
  padding: 1px 6px;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: var(--background);
  color: var(--muted-foreground);
  font-size: 11px;
  font-family: inherit;
}

.popup-character svg {
  color: var(--accent);
}

.popup-empty {
  margin: 6px 0;
  color: var(--muted-foreground);
  font-size: 11.5px;
  text-align: center;
}

.popup-create {
  display: flex;
  flex: none;
  gap: 8px;
}

.popup-create input {
  min-width: 0;
  flex: 1;
  height: 40px;
  padding: 0 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--card);
  color: var(--foreground);
  font-size: 12px;
  transition: border-color var(--motion-fast), box-shadow var(--motion-fast);
}

.popup-create input:focus-visible {
  border-color: color-mix(in srgb, var(--accent) 65%, var(--border));
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ring) 22%, transparent);
  outline: none;
}

.popup-create button {
  display: inline-flex;
  height: 40px;
  align-items: center;
  gap: 5px;
  padding: 0 14px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: var(--accent-gradient);
  color: var(--accent-foreground);
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
}

.popup-create button:focus-visible {
  outline: 2px solid var(--ring);
  outline-offset: 2px;
}

.popup-tip {
  margin: 0;
  color: var(--muted-foreground);
  font-size: 11px;
  text-align: center;
}
</style>
