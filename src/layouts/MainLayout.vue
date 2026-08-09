<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import { useRoute } from "vue-router";
import AppSidebar from "@/components/layout/AppSidebar.vue";

const route = useRoute();
const main = ref<HTMLElement | null>(null);

watch(
  () => route.fullPath,
  async () => {
    await nextTick();
    main.value?.focus({ preventScroll: true });
  },
);
</script>

<template>
  <div class="app-shell">
    <a class="skip-link" href="#main-content">跳到主要内容</a>
    <AppSidebar />

    <div class="app-content">
      <main
        id="main-content"
        ref="main"
        tabindex="-1"
        :class="['app-main', { immersive: route.meta.immersive }]"
      >
        <RouterView />
      </main>
    </div>
  </div>
</template>
