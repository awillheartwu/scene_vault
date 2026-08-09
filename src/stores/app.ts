import { defineStore } from "pinia";
import { ref, watch } from "vue";
import { applyTheme, broadcastTheme, resolveInitialTheme } from "@/lib/theme";

export const useAppStore = defineStore("app", () => {
  const sidebarCollapsed = ref(false);

  const theme = ref<"light" | "dark">(resolveInitialTheme());

  function toggleSidebar() {
    sidebarCollapsed.value = !sidebarCollapsed.value;
  }

  function setTheme(value: "light" | "dark") {
    theme.value = value;
  }

  function toggleTheme() {
    setTheme(theme.value === "dark" ? "light" : "dark");
  }

  watch(
    theme,
    (value) => {
      localStorage.setItem("scene-vault-theme", value);
      applyTheme(value);
      void broadcastTheme(value);
    },
    { immediate: true },
  );

  return {
    sidebarCollapsed,
    theme,
    toggleSidebar,
    setTheme,
    toggleTheme,
  };
});
