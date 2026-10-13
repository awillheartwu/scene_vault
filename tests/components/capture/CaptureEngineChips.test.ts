import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import CaptureEngineChips from "@/components/capture/CaptureEngineChips.vue";
import type { CaptureEngineMetadata } from "@/lib/capture-api";

function metadata(overrides: Partial<CaptureEngineMetadata> = {}): CaptureEngineMetadata {
  return {
    captureItemId: "item-1",
    projectId: "project-1",
    engine: "renpy",
    schemaVersion: 1,
    bridgeVersion: "1.1.4",
    engineVersion: "8.1.3.23091805",
    speakerRaw: "Ethan",
    speakerSource: "_last_say_who",
    dialogue: "Nice to meet you.",
    suggestedCharacterId: null,
    virtualWidth: 1920,
    virtualHeight: 1080,
    physicalWidth: 2560,
    physicalHeight: 1440,
    payloadJson: "{}",
    createdAt: "2026-10-12T00:00:00Z",
    updatedAt: "2026-10-12T00:00:00Z",
    ...overrides,
  };
}

describe("CaptureEngineChips", () => {
  it("renders the speaker and both resolutions without dialogue", () => {
    const wrapper = mount(CaptureEngineChips, {
      props: {
        engineMetadata: metadata(),
        imageFacts: {
          source: { width: 3840, height: 2160, format: "png" },
          destination: null,
        },
      },
    });
    const text = wrapper.text();
    expect(text).toContain("说话人 Ethan");
    expect(text).toContain("引擎 1920×1080");
    expect(text).toContain("截图 3840×2160");
    // The dialogue stays in the database but is not part of the chips.
    expect(text).not.toContain("Nice to meet you");
    wrapper.unmount();
  });

  it("hides the information row when all values are unknown", () => {
    const wrapper = mount(CaptureEngineChips, {
      props: {
        engineMetadata: metadata({
          engineVersion: null,
          speakerRaw: null,
          virtualWidth: null,
          virtualHeight: null,
        }),
        imageFacts: null,
      },
    });
    expect(wrapper.find(".engine-chips").exists()).toBe(false);
    wrapper.unmount();
  });

  it("shows an ordinary screenshot's resolution without engine metadata", () => {
    const wrapper = mount(CaptureEngineChips, {
      props: {
        engineMetadata: null,
        imageFacts: { source: { width: 3840, height: 2160, format: "png" }, destination: null },
      },
    });
    expect(wrapper.text()).toBe("截图 3840×2160");
    wrapper.unmount();
  });

  it("renders nothing without any image information", () => {
    const wrapper = mount(CaptureEngineChips, { props: { engineMetadata: null } });
    expect(wrapper.find(".engine-chips").exists()).toBe(false);
    wrapper.unmount();
  });
});
