import type { FaceRoi } from "@/lib/capture-api";

export interface FaceBox {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Parses a stored face box JSON; malformed or degenerate boxes are ignored. */
export function parseFaceBox(raw: string | null | undefined): FaceBox | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as Partial<FaceBox> | null;
    if (!value || typeof value !== "object") return null;
    const box: FaceBox = {
      x: Number(value.x),
      y: Number(value.y),
      width: Number(value.width),
      height: Number(value.height),
    };
    if (![box.x, box.y, box.width, box.height].every(Number.isFinite)) return null;
    if (box.width <= 0 || box.height <= 0) return null;
    return box;
  } catch {
    return null;
  }
}

/**
 * Converts a pixel face box into the normalized ROI the selector works with,
 * clamped to the image so a stale box can never produce an invalid selection.
 */
export function normalizeFaceBox(
  box: FaceBox | null | undefined,
  width: number,
  height: number,
): FaceRoi | null {
  if (!box || !(width > 0) || !(height > 0)) return null;
  const clamp = (value: number) => Math.min(Math.max(value, 0), 1);
  // Six decimals keep the numbers readable (and stable in tests) without
  // losing anything a picture could show.
  const round = (value: number) => Math.round(value * 1e6) / 1e6;
  const x = round(clamp(box.x / width));
  const y = round(clamp(box.y / height));
  const right = round(clamp((box.x + box.width) / width));
  const bottom = round(clamp((box.y + box.height) / height));
  const roi: FaceRoi = { x, y, width: round(right - x), height: round(bottom - y) };
  return roi.width > 0 && roi.height > 0 ? roi : null;
}
