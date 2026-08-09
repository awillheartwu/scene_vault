<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { ArrowRight, LoaderCircle, Merge, X } from "@lucide/vue";
import {
  captureApi,
  type Character,
  type CharacterSummary,
} from "@/lib/capture-api";
import { toast } from "@/lib/toast";

const props = defineProps<{
  source: CharacterSummary;
  characters: CharacterSummary[];
}>();
const emit = defineEmits<{
  (event: "close"): void;
  (event: "merged", character: Character): void;
}>();

const targetId = ref("");
const busy = ref(false);
const error = ref("");
const targetSelect = ref<HTMLSelectElement | null>(null);

const targets = computed(() =>
  props.characters.filter((character) => character.id !== props.source.id),
);
const target = computed(
  () => targets.value.find((character) => character.id === targetId.value) ?? null,
);

onMounted(() => {
  void nextTick(() => targetSelect.value?.focus());
});

function close() {
  if (!busy.value) emit("close");
}

async function submit() {
  if (!target.value || busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    const merged = await captureApi.mergeCharacters({
      sourceCharacterId: props.source.id,
      targetCharacterId: target.value.id,
    });
    toast.success(`已将「${props.source.name}」合并到「${merged.name}」`);
    emit("merged", merged);
  } catch (caught) {
    error.value = caught instanceof Error ? caught.message : String(caught);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <Teleport to="body">
    <div class="import-overlay" @click.self="close">
      <form
        class="import-dialog merge-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="merge-character-title"
        @submit.prevent="submit"
        @keydown.esc.prevent="close"
      >
        <div class="merge-title">
          <h3 id="merge-character-title">
            <span class="merge-icon"><Merge :size="16" /></span>
            合并角色
          </h3>
          <button
            type="button"
            class="dialog-close"
            aria-label="关闭合并角色弹窗"
            :disabled="busy"
            @click="close"
          >
            <X :size="16" />
          </button>
        </div>

        <p class="merge-description">
          选择要保留的角色。来源角色会被删除，此操作不可撤销。
        </p>

        <label class="merge-target-field">
          保留角色
          <select ref="targetSelect" v-model="targetId" :disabled="busy" required>
            <option value="" disabled>请选择合并目标</option>
            <option v-for="character in targets" :key="character.id" :value="character.id">
              {{ character.name }} · {{ character.captureCount }} 张截图
            </option>
          </select>
        </label>

        <div class="merge-direction" aria-live="polite">
          <div class="merge-character source">
            <span>将删除</span>
            <strong>{{ source.name }}</strong>
            <small>{{ source.captureCount }} 张截图 · {{ source.sampleCount }} 条样本</small>
          </div>
          <ArrowRight :size="20" aria-hidden="true" />
          <div class="merge-character target" :class="{ empty: !target }">
            <span>将保留</span>
            <strong>{{ target?.name ?? "尚未选择" }}</strong>
            <small v-if="target">
              {{ target.captureCount }} 张截图 · {{ target.sampleCount }} 条样本
            </small>
            <small v-else>选择目标后查看合并方向</small>
          </div>
        </div>

        <ul class="merge-effects">
          <li>截图、AI 建议、素材关系和人脸样本迁移到保留角色，并自动去重。</li>
          <li>来源名称和别名加入保留角色的别名；保留角色名称不变。</li>
          <li>保留角色已有代表头像优先；没有时才继承来源角色头像。</li>
          <li>磁盘上的源截图和已有归档文件不会删除或重命名。</li>
        </ul>

        <p v-if="error" class="merge-error" role="alert">{{ error }}</p>

        <div class="merge-actions">
          <button type="button" class="secondary-action" :disabled="busy" @click="close">
            取消
          </button>
          <button type="submit" class="danger-action" :disabled="busy || !target">
            <LoaderCircle v-if="busy" class="animate-spin" :size="16" />
            合并并删除「{{ source.name }}」
          </button>
        </div>
      </form>
    </div>
  </Teleport>
</template>

<style scoped>
.merge-dialog {
  width: min(560px, calc(100vw - 48px));
  gap: 14px;
}

.merge-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.merge-title h3 {
  display: flex;
  align-items: center;
  gap: 9px;
  margin: 0;
  font-size: 15px;
}

.merge-icon {
  display: grid;
  width: 28px;
  height: 28px;
  place-items: center;
  border-radius: 8px;
  background: color-mix(in srgb, var(--destructive) 12%, transparent);
  color: var(--destructive);
}

.dialog-close {
  display: grid;
  width: 36px;
  height: 36px;
  place-items: center;
  border: 0;
  border-radius: 9px;
  background: transparent;
  color: var(--muted-foreground);
}

.dialog-close:hover:not(:disabled) {
  background: var(--muted);
  color: var(--foreground);
}

.merge-description {
  margin: 0;
  color: var(--muted-foreground);
  font-size: 12px;
  line-height: 1.6;
}

.merge-target-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  color: var(--foreground);
  font-size: 12px;
  font-weight: 600;
}

.merge-target-field select {
  height: 40px;
  padding: 0 12px;
  border: 1px solid var(--input);
  border-radius: 10px;
  background: var(--background);
  color: var(--foreground);
  font-size: 13px;
}

.merge-target-field select:focus {
  border-color: var(--ring);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ring) 18%, transparent);
  outline: 0;
}

.merge-direction {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
  align-items: center;
  gap: 12px;
}

.merge-direction > svg {
  color: var(--muted-foreground);
}

.merge-character {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 4px;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--background);
}

.merge-character.source {
  border-color: color-mix(in srgb, var(--destructive) 36%, var(--border));
}

.merge-character.target:not(.empty) {
  border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
}

.merge-character span,
.merge-character small {
  overflow: hidden;
  color: var(--muted-foreground);
  font-size: 10.5px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.merge-character strong {
  overflow: hidden;
  font-size: 13px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.merge-effects {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 0;
  padding: 12px 14px 12px 30px;
  border: 1px solid var(--border);
  border-radius: 10px;
  background: color-mix(in srgb, var(--secondary) 55%, transparent);
  color: var(--muted-foreground);
  font-size: 11.5px;
  line-height: 1.5;
}

.merge-error {
  margin: 0;
  color: var(--destructive);
  font-size: 12px;
}

.merge-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding-top: 14px;
  border-top: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
}

.danger-action {
  display: inline-flex;
  min-height: 38px;
  align-items: center;
  justify-content: center;
  gap: 7px;
  padding: 0 14px;
  border: 1px solid transparent;
  border-radius: 9px;
  background: var(--destructive);
  color: #fff;
  font-size: 12px;
  font-weight: 600;
}

.danger-action:hover:not(:disabled) {
  filter: brightness(1.08);
}

.danger-action:disabled,
.dialog-close:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

@media (max-width: 620px) {
  .merge-direction {
    grid-template-columns: 1fr;
  }

  .merge-direction > svg {
    margin: -2px auto;
    transform: rotate(90deg);
  }
}
</style>
