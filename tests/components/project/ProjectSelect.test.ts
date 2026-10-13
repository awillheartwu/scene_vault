import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import type { Project } from "@/lib/capture-api";
import ProjectSelect from "@/components/project/ProjectSelect.vue";

const project: Project = {
  id: "renpy-project",
  name: "Ren'Py Game",
  description: null,
  coverAssetId: null,
  coverCaptureItemId: null,
  lastSourceDirectory: null,
  lastDestinationDirectory: null,
  destinationDirectory: null,
  createdAt: "2026-08-01T00:00:00Z",
  updatedAt: "2026-08-01T00:00:00Z",
};
const unknownVersionProject = { ...project, id: "unknown-version", name: "Another Game" };
const ordinaryProject = { ...project, id: "ordinary-project", name: "Ordinary Game" };

describe("ProjectSelect", () => {
  it("places project badges before names in the trigger and options and emits a selection", async () => {
    const wrapper = mount(ProjectSelect, {
      attachTo: document.body,
      props: {
        modelValue: project.id,
        projects: [project, unknownVersionProject, ordinaryProject],
        engineVersions: {
          [project.id]: ["8.4.1", "7.8.6", "8.1.3"],
          [unknownVersionProject.id]: [],
        },
        label: "测试项目",
      },
    });
    try {
      await flushPromises();
      const trigger = wrapper.get('[role="combobox"][aria-label="测试项目"]');
      const badge = trigger.get(".renpy-badge");
      expect(badge.text()).toBe("Ren'Py 7/8");
      expect(badge.attributes("title")).toBe("8.4.1、7.8.6、8.1.3");
      expect(badge.element.nextElementSibling).toBe(trigger.get(".project-select-name").element);
      expect(trigger.get(".project-select-name").text()).toBe(project.name);

      await trigger.trigger("keydown", { key: "ArrowDown" });
      await flushPromises();
      expect(wrapper.emitted("open")).toHaveLength(1);
      const options = [...document.body.querySelectorAll<HTMLElement>(".project-select-option")];
      expect(options.map((option) => option.querySelector(".project-select-name")?.textContent)).toEqual([
        project.name, unknownVersionProject.name, ordinaryProject.name,
      ]);
      expect(options[0].querySelector(".renpy-badge")?.textContent).toBe("Ren'Py 7/8");
      expect(options[1].querySelector(".renpy-badge")?.textContent).toBe("Ren'Py");
      expect(options[1].querySelector(".renpy-badge")?.hasAttribute("title")).toBe(false);
      for (const option of options.slice(0, 2)) {
        expect(option.querySelector(".renpy-badge")?.nextElementSibling).toBe(option.querySelector(".project-select-name"));
      }
      expect(options[2].querySelector(".renpy-badge")).toBeNull();

      options[2].dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await flushPromises();
      expect(wrapper.emitted("update:modelValue")?.slice(-1)[0]).toEqual([ordinaryProject.id]);
      await wrapper.setProps({ modelValue: ordinaryProject.id });
      expect(trigger.get(".project-select-name").text()).toBe(ordinaryProject.name);
      expect(trigger.find(".renpy-badge").exists()).toBe(false);

      await wrapper.setProps({ modelValue: unknownVersionProject.id });
      expect(trigger.get(".renpy-badge").text()).toBe("Ren'Py");
      expect(trigger.get(".renpy-badge").attributes("title")).toBeUndefined();
    } finally {
      wrapper.unmount();
    }
  });
});
