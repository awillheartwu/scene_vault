import type { CaptureItem, Character } from "@/lib/capture-api";

/**
 * Minimal capture stub for the shared thumbnail reader, mirroring the
 * workbench's thumbnail item helper: the reader only needs the capture id and
 * falls back through the avatar-to-source variants on its own. Its shared
 * capture:item-updated subscription refreshes this id-only stub on completion;
 * character.updatedAt need not change when the existing avatar is reprocessed.
 */
export function characterAvatarItem(
  character: Pick<Character, "avatarCaptureItemId" | "latestCaptureItemId">,
): CaptureItem | null {
  const id = character.avatarCaptureItemId ?? character.latestCaptureItemId;
  return id ? ({ id, sourcePath: "avatar.png" } as CaptureItem) : null;
}
