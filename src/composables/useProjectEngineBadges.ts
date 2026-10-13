import { onBeforeUnmount, ref } from "vue";
import { captureApi } from "@/lib/capture-api";
import { engineVersionList } from "@/lib/image-facts";

export function useProjectEngineBadges() {
  // An empty version list still identifies a Ren'Py project.
  const engineVersions = ref<Record<string, string[]>>({});
  let request = 0;

  async function refreshEngineBadges() {
    const token = ++request;
    try {
      const projects = await captureApi.listProjectOverviews();
      if (token !== request) return;
      engineVersions.value = Object.fromEntries(
        projects.filter((project) => project.engine === "renpy")
          .map((project) => [project.projectId, engineVersionList(project.engineVersions)]),
      );
    } catch {
      // Preserve the last known badges when this optional read fails.
    }
  }

  onBeforeUnmount(() => { request += 1; });
  return { engineVersions, refreshEngineBadges };
}
