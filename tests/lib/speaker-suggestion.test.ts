import { describe, expect, it } from "vitest";
import type { Character } from "@/lib/capture-api";
import {
  characterMatchesSpeaker,
  findCharacterBySpeaker,
  isLikelySpeakerName,
  normalizeSpeakerName,
} from "@/lib/speaker-suggestion";

function character(id: string, name: string, aliases: string[] = []): Character {
  return {
    id,
    projectId: "p",
    name,
    aliasesJson: JSON.stringify(aliases),
    avatarAssetId: null,
    createdAt: "",
    updatedAt: "",
  };
}

describe("speaker-suggestion", () => {
  it("normalizes text tags, case and whitespace like the backend", () => {
    expect(normalizeSpeakerName(" {color=#fff}Ashley{/color} ")).toBe("ashley");
  });

  it("matches names and aliases", () => {
    const roster = [character("1", "Ashley", ["Ash", "小艾"])];
    expect(findCharacterBySpeaker(roster, "ash")?.id).toBe("1");
    expect(findCharacterBySpeaker(roster, "小艾")?.id).toBe("1");
    expect(findCharacterBySpeaker(roster, "nobody")).toBeNull();
  });

  it("stops reusing a character that was renamed away from the speaker", () => {
    const renamed = character("1", "Sarah", []);
    expect(characterMatchesSpeaker(renamed, "Alice")).toBe(false);
    expect(findCharacterBySpeaker([renamed], "Alice")).toBeNull();
  });

  it("filters junk speakers from create suggestions", () => {
    expect(isLikelySpeakerName("Alicia")).toBe(true);
    expect(isLikelySpeakerName("旁白")).toBe(false);
    expect(isLikelySpeakerName("???")).toBe(false);
    expect(isLikelySpeakerName("  ")).toBe(false);
  });
});
