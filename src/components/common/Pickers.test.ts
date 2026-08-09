import { describe, expect, it, vi } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import type { AnnotationSettings } from "@/lib/capture-api";
import CornerFallbackPicker from "./CornerFallbackPicker.vue";
import FaceTextPositionPicker from "./FaceTextPositionPicker.vue";

function baseAnnotation(over: Partial<AnnotationSettings> = {}): AnnotationSettings {
  return {
    textColor: null,
    strokeColor: null,
    strokeWidth: null,
    padding: null,
    faceBoxExpansion: null,
    faceTextPosition: null,
    fallbackPosition: null,
    textOffsetX: null,
    textOffsetY: null,
    fontSize: null,
    ...over,
  };
}

function mockStageRect(wrapper: VueWrapper) {
  const element = wrapper.find(".picker-stage").element;
  vi.spyOn(element, "getBoundingClientRect").mockReturnValue({
    width: 260,
    height: 236,
    left: 0,
    top: 0,
    right: 260,
    bottom: 236,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  } as DOMRect);
}

function pointer(element: Element | Window, type: string, clientX: number, clientY: number) {
  const event = new MouseEvent(type, {
    clientX,
    clientY,
    bubbles: true,
    cancelable: true,
    button: 0,
  });
  Object.defineProperty(event, "pointerId", { value: 1 });
  element.dispatchEvent(event);
}

function lastEmitted(wrapper: VueWrapper) {
  const emitted = wrapper.emitted("update:modelValue")!;
  return emitted[emitted.length - 1][0] as AnnotationSettings;
}

describe("FaceTextPositionPicker", () => {
  it("renders four stars and defaults to above", () => {
    const wrapper = mount(FaceTextPositionPicker, {
      props: { modelValue: baseAnnotation() },
    });

    const buttons = wrapper.findAll(".pos-btn");
    expect(buttons).toHaveLength(4);
    expect(buttons[0].attributes("aria-pressed")).toBe("true");
    expect(wrapper.find(".marker").exists()).toBe(true);
    expect(wrapper.text()).toContain("首选");
    expect(wrapper.text()).toContain("上方 → 右侧 → 下方 → 左侧");
  });

  it("visualizes the configured face box expansion", () => {
    const wrapper = mount(FaceTextPositionPicker, {
      props: { modelValue: baseAnnotation({ faceBoxExpansion: 64 }) },
    });

    expect(wrapper.get(".picker-stage").attributes("style")).toContain("--face-expansion");
    expect(wrapper.text()).toContain("参考框外扩 64 px");
  });

  it("selects a discrete direction on star click", async () => {
    const wrapper = mount(FaceTextPositionPicker, {
      props: { modelValue: baseAnnotation() },
    });
    mockStageRect(wrapper);

    await wrapper.findAll(".pos-btn")[3].trigger("click");

    const last = lastEmitted(wrapper);
    expect(last.faceTextPosition).toBe("left");
    expect(last.textOffsetX).toBeNull();
    expect(last.textOffsetY).toBeNull();
  });

  it("drags the marker to a custom position", () => {
    const wrapper = mount(FaceTextPositionPicker, {
      props: { modelValue: baseAnnotation() },
    });
    mockStageRect(wrapper);
    const stage = wrapper.find(".picker-stage").element;

    pointer(stage, "pointerdown", 60, 60);
    pointer(window, "pointermove", 100, 60);
    pointer(window, "pointerup", 100, 60);

    const last = lastEmitted(wrapper);
    expect(last.faceTextPosition).toBe("custom");
    expect(last.textOffsetX).toBeCloseTo(-0.33, 1);
    expect(last.textOffsetY).toBeCloseTo(-0.7, 1);
  });

  it("snaps back to a discrete direction when dropped on a star", () => {
    const wrapper = mount(FaceTextPositionPicker, {
      props: { modelValue: baseAnnotation() },
    });
    mockStageRect(wrapper);
    const stage = wrapper.find(".picker-stage").element;

    // Above star center: (130, 18.88)
    pointer(stage, "pointerdown", 60, 60);
    pointer(window, "pointermove", 130, 19);
    pointer(window, "pointerup", 130, 19);

    const last = lastEmitted(wrapper);
    expect(last.faceTextPosition).toBe("above");
    expect(last.textOffsetX).toBeNull();
  });

  it("starts a drag from a star button", () => {
    const wrapper = mount(FaceTextPositionPicker, {
      props: { modelValue: baseAnnotation() },
    });
    mockStageRect(wrapper);
    const star = wrapper.findAll(".pos-btn")[0].element;

    pointer(star, "pointerdown", 130, 19);
    pointer(window, "pointermove", 130, 40);
    pointer(window, "pointerup", 130, 40);

    const last = lastEmitted(wrapper);
    expect(last.faceTextPosition).toBe("custom");
    expect(last.textOffsetY).toBeCloseTo(-0.94, 1);
  });

  it("places a custom marker by clicking blank stage space", () => {
    const wrapper = mount(FaceTextPositionPicker, {
      props: { modelValue: baseAnnotation() },
    });
    mockStageRect(wrapper);
    const stage = wrapper.find(".picker-stage").element;

    pointer(stage, "pointerdown", 200, 170);
    pointer(window, "pointerup", 200, 170);

    const last = lastEmitted(wrapper);
    expect(last.faceTextPosition).toBe("custom");
    expect(last.textOffsetX).toBeCloseTo(0.77, 1);
  });

  it("nudges the marker with arrow keys from a discrete start", async () => {
    const wrapper = mount(FaceTextPositionPicker, {
      props: { modelValue: baseAnnotation() },
    });
    await wrapper.find(".marker").trigger("keydown", { key: "ArrowLeft" });

    const last = lastEmitted(wrapper);
    expect(last.faceTextPosition).toBe("custom");
    expect(last.textOffsetX).toBeCloseTo(-0.1, 2);
    expect(last.textOffsetY).toBeCloseTo(-1.2, 1);
  });
});

describe("CornerFallbackPicker", () => {
  it("renders four corners and defaults to top_left", () => {
    const wrapper = mount(CornerFallbackPicker, {
      props: { modelValue: null },
    });

    const buttons = wrapper.findAll(".corner-btn");
    expect(buttons).toHaveLength(4);
    expect(buttons[0].attributes("aria-pressed")).toBe("true");
    expect(wrapper.text()).toContain("左上");
  });

  it("emits the clicked corner", async () => {
    const wrapper = mount(CornerFallbackPicker, {
      props: { modelValue: "top_left" },
    });

    await wrapper.findAll(".corner-btn")[3].trigger("click");
    expect(wrapper.emitted("update:modelValue")![0]).toEqual(["bottom_right"]);
  });
});
