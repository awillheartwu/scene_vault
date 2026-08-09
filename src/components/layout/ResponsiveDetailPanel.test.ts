import { afterEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import ResponsiveDetailPanel from "./ResponsiveDetailPanel.vue";

function setNarrowViewport() {
  window.matchMedia = vi.fn(
    (query: string): MediaQueryList =>
      ({
        matches: false,
        media: query,
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      }) as MediaQueryList,
  ) as unknown as typeof window.matchMedia;
}

afterEach(() => {
  delete (window as { matchMedia?: typeof window.matchMedia }).matchMedia;
  document.body.innerHTML = "";
});

describe("ResponsiveDetailPanel", () => {
  it("renders inline when matchMedia is unavailable", () => {
    const wrapper = mount(ResponsiveDetailPanel, {
      props: { open: false, title: "截图详情" },
      slots: { default: "详情内容" },
    });

    expect(wrapper.get(".responsive-detail-panel.is-static").text()).toContain("详情内容");
    expect(document.body.querySelector('[role="dialog"]')).toBeNull();
  });

  it("uses a labelled focus-managed drawer in the narrow layout", async () => {
    setNarrowViewport();
    const trigger = document.createElement("button");
    trigger.textContent = "打开详情";
    document.body.appendChild(trigger);
    trigger.focus();
    const wrapper = mount(ResponsiveDetailPanel, {
      attachTo: document.body,
      props: { open: false, title: "截图详情", description: "one.png" },
      slots: { default: "详情内容" },
    });
    await wrapper.setProps({ open: true });
    await flushPromises();

    const dialog = document.body.querySelector('[role="dialog"]');
    expect(dialog?.textContent).toContain("截图详情");
    expect(dialog?.textContent).toContain("one.png");
    expect(dialog?.textContent).toContain("详情内容");

    (dialog!.querySelector('[aria-label="关闭详情"]') as HTMLButtonElement).click();
    await flushPromises();
    const updates = wrapper.emitted("update:open") ?? [];
    expect(updates[updates.length - 1]).toEqual([false]);
    await wrapper.setProps({ open: false });
    await flushPromises();
    expect(document.activeElement).toBe(trigger);
    wrapper.unmount();
  });
});
