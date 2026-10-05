<script setup lang="ts">
import { computed } from "vue";
import { UserRound } from "@lucide/vue";
import type { Character } from "@/lib/capture-api";
import { characterAvatarItem } from "@/lib/character-avatar";
import CaptureThumbnail from "@/components/capture/CaptureThumbnail.vue";

const props = defineProps<{ character: Character }>();

const item = computed(() => characterAvatarItem(props.character));
</script>

<template>
  <CaptureThumbnail
    v-if="item"
    :item="item"
    variant="avatar"
    fallback-variant="source"
    :alt="`${character.name} 的头像`"
  />
  <UserRound v-else :size="18" />
</template>
