import { mount, flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  isExcludedContextTarget,
  useContextMenu,
  type ContextMenuController,
  type ContextMenuItem,
} from "@/composables/useContextMenu";
import ContextMenu from "./ContextMenu.vue";

function contextEvent(target: Element, currentTarget: Element): MouseEvent {
  const event = new MouseEvent("contextmenu", {
    bubbles: true,
    cancelable: true,
    clientX: 120,
    clientY: 90,
  });
  Object.defineProperty(event, "target", { value: target });
  Object.defineProperty(event, "currentTarget", { value: currentTarget });
  return event;
}

describe("isExcludedContextTarget", () => {
  function setupDom() {
    document.body.innerHTML = `
      <div class="root">
        <button class="root-button">item root button</button>
        <button class="allowed-button" data-context-allow>allowed</button>
        <button class="plain-button">plain button</button>
        <input class="field" />
        <select class="select"><option>a</option></select>
        <textarea class="textarea"></textarea>
        <div class="drag" data-tauri-drag-region="deep">drag</div>
        <div class="area">plain area</div>
      </div>
    `;
    return {
      root: document.querySelector(".root") as HTMLElement,
      rootButton: document.querySelector(".root-button") as HTMLElement,
      allowedButton: document.querySelector(".allowed-button") as HTMLElement,
      plainButton: document.querySelector(".plain-button") as HTMLElement,
      field: document.querySelector(".field") as HTMLElement,
      select: document.querySelector(".select") as HTMLElement,
      textarea: document.querySelector(".textarea") as HTMLElement,
      drag: document.querySelector(".drag") as HTMLElement,
      area: document.querySelector(".area") as HTMLElement,
    };
  }

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("allows plain areas", () => {
    const dom = setupDom();
    expect(isExcludedContextTarget(contextEvent(dom.area, dom.root))).toBe(false);
  });

  it("allows a button that is the item root itself", () => {
    const dom = setupDom();
    expect(isExcludedContextTarget(contextEvent(dom.rootButton, dom.rootButton))).toBe(false);
  });

  it("allows buttons marked with data-context-allow inside the item", () => {
    const dom = setupDom();
    expect(isExcludedContextTarget(contextEvent(dom.allowedButton, dom.root))).toBe(false);
  });

  it("excludes plain buttons inside the item", () => {
    const dom = setupDom();
    expect(isExcludedContextTarget(contextEvent(dom.plainButton, dom.root))).toBe(true);
  });

  it("excludes inputs, selects, textareas and drag regions", () => {
    const dom = setupDom();
    expect(isExcludedContextTarget(contextEvent(dom.field, dom.root))).toBe(true);
    expect(isExcludedContextTarget(contextEvent(dom.select, dom.root))).toBe(true);
    expect(isExcludedContextTarget(contextEvent(dom.textarea, dom.root))).toBe(true);
    expect(isExcludedContextTarget(contextEvent(dom.drag, dom.root))).toBe(true);
  });

  it("excludes non-element targets", () => {
    const event = new MouseEvent("contextmenu");
    expect(isExcludedContextTarget(event)).toBe(true);
  });
});

describe("ContextMenu", () => {
  let menu: ContextMenuController;
  let wrapper: ReturnType<typeof mount>;
  const actions = {
    first: vi.fn(),
    danger: vi.fn(),
    disabled: vi.fn(),
  };

  function items(): ContextMenuItem[] {
    return [
      { id: "first", label: "第一项", action: actions.first },
      { id: "danger", label: "危险项", danger: true, action: actions.danger },
      { id: "disabled", label: "禁用项", disabled: true, action: actions.disabled },
    ];
  }

  async function openAt(target = document.body) {
    menu.open(contextEvent(target, target), items());
    await flushPromises();
  }

  function menuRoot(): HTMLElement | null {
    return document.querySelector('[role="menu"]');
  }

  function menuItems(): HTMLElement[] {
    return Array.from(document.querySelectorAll('[role="menuitem"]')) as HTMLElement[];
  }

  beforeEach(() => {
    vi.clearAllMocks();
    menu = useContextMenu();
    wrapper = mount(ContextMenu, { props: { menu } });
  });

  afterEach(() => {
    menu.close();
    wrapper.unmount();
    document.body.innerHTML = "";
  });

  it("opens at the cursor position and focuses the first item", async () => {
    await openAt();
    const root = menuRoot();
    expect(root).not.toBeNull();
    expect(root!.style.left).toBe("120px");
    expect(root!.style.top).toBe("90px");
    expect(root!.textContent).toContain("第一项");
    expect(menuItems()[0]).toBe(document.activeElement);
  });

  it("selects an item on click, runs its action and closes", async () => {
    await openAt();
    menuItems()[0].click();
    await flushPromises();
    expect(actions.first).toHaveBeenCalledOnce();
    expect(menuRoot()).toBeNull();
  });

  it("marks disabled items and skips them during keyboard navigation", async () => {
    await openAt();
    const disabled = menuItems()[2];
    expect(disabled.getAttribute("disabled")).not.toBeNull();

    menuRoot()!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(menuItems()[1]).toBe(document.activeElement);
    menuRoot()!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    // Wraps around to the first enabled item, never the disabled one.
    expect(menuItems()[0]).toBe(document.activeElement);
  });

  it("activates the focused item with Enter", async () => {
    await openAt();
    menuRoot()!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    menuRoot()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flushPromises();
    expect(actions.danger).toHaveBeenCalledOnce();
    expect(menuRoot()).toBeNull();
  });

  it("closes on Escape without running an action", async () => {
    await openAt();
    menuRoot()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await flushPromises();
    expect(menuRoot()).toBeNull();
    expect(actions.first).not.toHaveBeenCalled();
  });

  it("closes on pointerdown outside the menu", async () => {
    await openAt();
    document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    await flushPromises();
    expect(menuRoot()).toBeNull();
  });

  it("keeps the menu open when clicking inside it", async () => {
    await openAt();
    menuItems()[0].dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    await flushPromises();
    expect(menuRoot()).not.toBeNull();
  });

  it("replaces items when reopened while open", async () => {
    await openAt();
    menu.open(contextEvent(document.body, document.body), [
      { id: "other", label: "另一项", action: actions.first },
    ]);
    await flushPromises();
    expect(menuRoot()!.textContent).toContain("另一项");
    expect(menuRoot()!.textContent).not.toContain("第一项");
  });
});
