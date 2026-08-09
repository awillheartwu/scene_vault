import { createRouter, createWebHashHistory } from "vue-router";

export const router = createRouter({
  history: createWebHashHistory(),

  routes: [
    {
      path: "/",
      component: () => import("@/pages/Home.vue"),
    },
    {
      path: "/capture",
      component: () => import("@/pages/Capture.vue"),
      meta: { immersive: true },
    },
    {
      path: "/history",
      component: () => import("@/pages/History.vue"),
      meta: { immersive: true },
    },
    {
      path: "/workbench",
      component: () => import("@/pages/Workbench.vue"),
      meta: { immersive: true },
    },
    {
      path: "/settings",
      component: () => import("@/pages/Settings.vue"),
    },
  ],
});
