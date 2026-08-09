<script setup lang="ts">
import { Camera, FolderKanban, History, Moon, Settings, Sun, UserRound, Vault } from "@lucide/vue";
import { useAppStore } from "@/stores/app";

const appStore = useAppStore();

const menus = [
  {
    name: "项目",
    path: "/",
    icon: FolderKanban,
  },
  {
    name: "捕获",
    path: "/capture",
    icon: Camera,
  },
  {
    name: "工作台",
    path: "/workbench",
    icon: UserRound,
  },
  {
    name: "历史",
    path: "/history",
    icon: History,
  },
  {
    name: "设置",
    path: "/settings",
    icon: Settings,
  },
];
</script>

<template>
  <aside class="app-sidebar">
    <div class="sidebar-brand">
      <span class="brand-mark"><Vault :size="18" /></span>
      <span class="brand-copy">SCENE<br /><strong>VAULT</strong></span>
    </div>

    <nav aria-label="主导航">
      <RouterLink
        v-for="item in menus"
        :key="item.path"
        :to="item.path"
        class="sidebar-link"
        active-class="active"
        :aria-label="item.name"
        :data-label="item.name"
        :title="item.name"
      >
        <component :is="item.icon" :size="20" aria-hidden="true" />
        <span class="sidebar-label">{{ item.name }}</span>
      </RouterLink>
    </nav>
    <div class="sidebar-footer">
      <span class="status-dot active" /><span class="sidebar-footer-label">本地优先</span>
      <button
        type="button"
        class="theme-toggle"
        :aria-label="appStore.theme === 'dark' ? '切换到浅色主题' : '切换到深色主题'"
        :title="appStore.theme === 'dark' ? '切换到浅色' : '切换到深色'"
        @click="appStore.toggleTheme()"
      >
        <Moon v-if="appStore.theme === 'dark'" :size="14" />
        <Sun v-else :size="14" />
      </button>
    </div>
  </aside>
</template>
