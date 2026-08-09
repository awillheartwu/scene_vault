<script setup lang="ts">
import AppSidebar from "@/components/layout/AppSidebar.vue";
import AppHeader from "@/components/layout/AppHeader.vue";
import { useRoute } from "vue-router";

const route = useRoute();
</script>

<template>
  <div class="app-shell">
    <AppSidebar />

    <div class="app-content">
      <AppHeader v-if="!route.meta.immersive" />

      <main :class="['app-main', { immersive: route.meta.immersive }]">
        <RouterView />
      </main>
    </div>
  </div>
</template>
