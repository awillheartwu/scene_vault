import type { Character } from "@/lib/capture-api";

/** Same normalization the backend uses for speaker-to-character matching. */
export function normalizeSpeakerName(value: string | null | undefined): string {
  if (!value) return "";
  let out = "";
  let inTag = false;
  for (const ch of value) {
    if (ch === "{") inTag = true;
    else if (ch === "}") inTag = false;
    else if (!inTag) out += ch;
  }
  return out.trim().toLocaleLowerCase();
}

function parseAliases(aliasesJson: string): string[] {
  try {
    const values: unknown = JSON.parse(aliasesJson);
    if (Array.isArray(values)) {
      return values.filter((value): value is string => typeof value === "string");
    }
  } catch {
    // Fall through to the loose separator parsing below.
  }
  return aliasesJson
    .split(/[,、;]/)
    .map((value) => value.trim().replace(/^["\']|["\']$/g, ""))
    .filter(Boolean);
}

/**
 * Whether one character still answers to the speaker name right now (its own
 * name or one of its aliases). Used to decide if a stored suggestion may
 * still be reused: a renamed character no longer matches and must not be
 * recommended for its old speaker.
 */
export function characterMatchesSpeaker(
  character: Character,
  speaker: string | null | undefined,
): boolean {
  const needle = normalizeSpeakerName(speaker);
  if (!needle) return false;
  if (normalizeSpeakerName(character.name) === needle) return true;
  return parseAliases(character.aliasesJson).some(
    (alias) => normalizeSpeakerName(alias) === needle,
  );
}

/**
 * Finds the character a speaker name refers to (name or alias, ignoring
 * case, text tags and surrounding whitespace). Mirrors the backend matcher so
 * the UI can re-check live matches after a character was created or renamed
 * following the capture.
 */
export function findCharacterBySpeaker(
  characters: Character[],
  speaker: string | null | undefined,
): Character | null {
  if (!normalizeSpeakerName(speaker)) return null;
  for (const character of characters) {
    if (characterMatchesSpeaker(character, speaker)) return character;
  }
  return null;
}

const NON_NAME_SPEAKERS = new Set([
  "narrator",
  "narration",
  "旁白",
  "旁白君",
  "unknown",
  "system",
  "me",
  "you",
  "none",
  "null",
  "?",
  "???",
  "？？？",
]);

/** Heuristic gate so junk speakers never produce a create-character prompt. */
export function isLikelySpeakerName(value: string | null | undefined): boolean {
  const normalized = normalizeSpeakerName(value);
  if (!normalized || NON_NAME_SPEAKERS.has(normalized)) return false;
  if (normalized.length > 40) return false;
  return /\p{L}/u.test(normalized);
}
