import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  ProjectEngineIntegration,
  ProjectEngineIntegrationView,
  RenpyScreenshotPolicyView,
} from "@/lib/capture-api";

vi.mock("@/lib/capture-api", () => ({
  pathFileName: (path: string) => path.split(/[\\/]/).pop() || path,
}));

import RenpyScreenshotSettings from "@/components/capture/RenpyScreenshotSettings.vue";

function policy(overrides: Partial<RenpyScreenshotPolicyView> = {}): RenpyScreenshotPolicyView {
  return {
    projectAutoHideUi: null, globalAutoHideUi: false, resolvedAutoHideUi: false,
    ...overrides,
  };
}

function integration(
  overrides: Partial<ProjectEngineIntegration> = {},
  resolvedAutoHideUi = false,
): ProjectEngineIntegrationView {
  const engineRoot = overrides.engineRoot ?? "D:\\Games\\Demo";
  return {
    integration: {
      id: "install-1", projectId: "project-1", engine: "renpy", engineRoot,
      gameDirectory: engineRoot + "\\game", pythonMajor: 3, engineVersion: "8.4.1",
      bridgeVersion: "1.1.6", autoHideUi: false, autoHideUiOverride: null, manifestJson: "[]",
      installedAt: "", updatedAt: "", ...overrides,
    },
    bridgeFilePresent: true, compiledFilePresent: true,
    bundledBridgeVersion: "1.1.6", resolvedAutoHideUi, warning: null,
  };
}

const wrappers: ReturnType<typeof mount>[] = [];
afterEach(() => {
  wrappers.splice(0).forEach((wrapper) => wrapper.unmount());
  document.body.innerHTML = "";
});

function mountSettings(integrations = [integration()], screenshotPolicy = policy()) {
  const wrapper = mount(RenpyScreenshotSettings, {
    props: {
      open: true, integrations, policy: screenshotPolicy, loading: false,
      loadError: null, working: false, saving: false, warning: null,
    },
    attachTo: document.body,
  });
  wrappers.push(wrapper);
  return wrapper;
}

function projectSelect() {
  return document.body.querySelector<HTMLSelectElement>('[aria-label="项目截图设置"]')!;
}

function gameSelects() {
  return [...document.body.querySelectorAll<HTMLSelectElement>(".renpy-screenshot-list select")];
}

function choose(select: HTMLSelectElement, value: string) {
  select.value = value;
  select.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("RenpyScreenshotSettings", () => {
  it("shows all three choices, defaults to inheritance and keeps engine versions and shortcut guidance", async () => {
    const installed = integration();
    mountSettings([installed]);
    await flushPromises();

    expect(projectSelect().value).toBe("inherit");
    expect(gameSelects()[0].value).toBe("inherit");
    expect([...projectSelect().options].map((option) => option.text)).toEqual(["跟随全局（关闭）", "启用", "关闭"]);
    expect([...gameSelects()[0].options].map((option) => option.text)).toEqual(["跟随项目（关闭）", "启用", "关闭"]);
    const dialog = document.body.querySelector(".renpy-screenshot-settings")!;
    const name = dialog.querySelector(".screenshot-game-name")!;
    expect(name.textContent?.trim()).toBe("Demo");
    expect(name.getAttribute("title")).toBe(installed.integration.engineRoot);
    expect(dialog.textContent).toContain("Ren'Py 8.4.1");
    expect(dialog.textContent).toContain("全局默认关闭");
    expect(dialog.textContent).toContain("原截图键（通常为 S）");
    expect(dialog.textContent).toContain("标准剧情 UI");
    expect(dialog.textContent).toContain("请先按 H 隐藏 UI，再按 S 截图");
    expect(dialog.textContent).toContain("插件更新后需重启游戏一次");
    expect(dialog.textContent).toContain("已有选择保留，可按项目或游戏覆盖");
  });

  it("uses the returned global/project/effective values rather than the install-time fallback", async () => {
    const wrapper = mountSettings([
      integration({ autoHideUi: false }, true),
      integration({ id: "install-2", autoHideUi: true, autoHideUiOverride: false }),
    ], policy({ globalAutoHideUi: true, resolvedAutoHideUi: true }));
    await flushPromises();

    expect(projectSelect().options[0].text).toBe("跟随全局（启用）");
    expect(gameSelects().map((select) => select.value)).toEqual(["inherit", "off"]);
    expect(gameSelects()[0].options[0].text).toBe("跟随项目（启用）");
    expect([...document.body.querySelectorAll(".screenshot-game-state")].map((state) => state.textContent)).toEqual([
      "当前有效：启用", "当前有效：关闭",
    ]);

    await wrapper.setProps({ policy: policy({ projectAutoHideUi: false, globalAutoHideUi: true }) });
    expect(projectSelect().value).toBe("off");
    expect(projectSelect().options[0].text).toBe("跟随全局（启用）");
    expect(gameSelects()[0].options[0].text).toBe("跟随项目（关闭）");
  });

  it.each([
    ["inherit", null], ["on", true], ["off", false],
  ] as const)("emits %s as the project choice and retains the persisted select until saved", async (choice, expected) => {
    const wrapper = mountSettings([], policy({ projectAutoHideUi: true, resolvedAutoHideUi: true }));
    await flushPromises();
    choose(projectSelect(), choice);
    expect(wrapper.emitted("project-change")).toEqual([[expected]]);
    expect(projectSelect().value).toBe("on");

    await wrapper.setProps({ policy: policy({ projectAutoHideUi: expected, resolvedAutoHideUi: expected ?? false }) });
    expect(projectSelect().value).toBe(choice);
    expect(document.body.textContent).toContain("仍可保存项目默认");
  });

  it.each([
    ["inherit", null], ["on", true], ["off", false],
  ] as const)("targets the chosen game for %s and preserves its old value while saving", async (choice, expected) => {
    const first = integration({ autoHideUiOverride: true }, true);
    const second = integration({ id: "install-2", engineRoot: "E:\\Games\\Other\\", engineVersion: "7.8.6" });
    const wrapper = mountSettings([first, second]);
    await flushPromises();
    const inputs = gameSelects();
    choose(inputs[1], choice);
    expect(wrapper.emitted("installation-change")).toEqual([[second, expected]]);
    expect(inputs.map((select) => select.value)).toEqual(["on", "inherit"]);
    expect(document.body.querySelectorAll(".screenshot-game-name")[1].textContent?.trim()).toBe("Other");
    expect(document.body.textContent).toContain("Ren'Py 7.8.6");

    await wrapper.setProps({
      integrations: [first, { ...second, integration: { ...second.integration, autoHideUiOverride: expected } }],
    });
    expect(inputs[1].value).toBe(choice);
  });

  it.each(["loading", "working", "saving", "loadError", "policy"] as const)(
    "blocks project and game changes when %s prevents editing",
    async (reason) => {
      const wrapper = mountSettings();
      await flushPromises();
      if (reason === "policy") await wrapper.setProps({ policy: null });
      else if (reason === "loadError") await wrapper.setProps({ loadError: "读取失败" });
      else await wrapper.setProps({ [reason]: true });
      expect(projectSelect().disabled).toBe(true);
      expect(gameSelects()[0].disabled).toBe(true);
      choose(projectSelect(), "on");
      choose(gameSelects()[0], "on");
      expect(projectSelect().value).toBe("inherit");
      expect(gameSelects()[0].value).toBe("inherit");
      expect(wrapper.emitted("project-change")).toBeUndefined();
      expect(wrapper.emitted("installation-change")).toBeUndefined();
    },
  );

  it("offers a retry for a load failure and keeps persistence warnings visible", async () => {
    const installed = { ...integration(), warning: "已有文件需要确认" };
    const wrapper = mountSettings([installed]);
    await wrapper.setProps({ loadError: "数据库读取失败", warning: "截图设置未保存，请重试" });
    await flushPromises();
    document.body.querySelector<HTMLButtonElement>(".screenshot-settings-error button")!.click();
    expect(wrapper.emitted("retry")).toEqual([[]]);
    expect(document.body.textContent).toContain("截图设置加载失败");
    expect(document.body.textContent).toContain("截图设置未保存，请重试");
    expect(document.body.textContent).toContain("已有文件需要确认");
    await wrapper.setProps({ working: true, saving: true });
    expect(document.body.textContent).toContain("正在保存…");
    expect(document.body.querySelector<HTMLButtonElement>(".screenshot-settings-error button")!.disabled).toBe(true);
  });

  it("prompts stale plugins to update and restart once before using inherited settings", async () => {
    mountSettings([integration({ bridgeVersion: "1.1.5" })]);
    await flushPromises();
    const text = document.body.textContent;
    expect(text).toContain("插件需要更新（1.1.5 → 1.1.6）");
    expect(text).toContain("重新安装并重启游戏一次");
    expect(text).toContain("更新后的插件才会在截图时应用此设置");
  });
});
