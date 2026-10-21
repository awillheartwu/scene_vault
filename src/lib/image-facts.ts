import type { ArchiveImageFacts, SourceImageFacts } from "@/lib/capture-api";

/** Human-readable byte size (shared by the project home and file lines). */
export function formatBytes(bytes: number | null | undefined): string {
  if (!bytes) return "0 B";
  if (bytes >= 1024 ** 3) return (bytes / 1024 ** 3).toFixed(2) + " GB";
  if (bytes >= 1024 ** 2) return (bytes / 1024 ** 2).toFixed(1) + " MB";
  if (bytes >= 1024) return (bytes / 1024).toFixed(1) + " KB";
  return bytes + " B";
}

function formatBytesOrNull(bytes: number | null | undefined): string | null {
  return bytes === null || bytes === undefined ? null : formatBytes(bytes);
}

/** `3840×2160`; null when either side is unknown. */
export function formatResolution(
  width: number | null | undefined,
  height: number | null | undefined,
): string | null {
  if (!width || !height) return null;
  return `${width}×${height}`;
}

/** `PNG`, `JPEG`, `WebP`, ... (null when the format is unknown). */
export function formatLabel(format: string | null | undefined): string | null {
  const value = (format ?? "").trim().toLowerCase();
  if (!value) return null;
  if (value === "jpeg" || value === "jpg") return "JPEG";
  if (value === "png") return "PNG";
  if (value === "webp") return "WebP";
  return value.toUpperCase();
}

/** Label for an archive size value (used by the selects and the home line). */
export function archiveSizeLabel(size: string | null | undefined): string {
  switch (size) {
    case "1080p":
    case "1440p":
    case "2160p":
      return size;
    case "native":
      return "按游戏设定尺寸";
    default:
      return "保持原样";
  }
}

/** Label for an archive format value. */
export function archiveFormatLabel(format: string | null | undefined): string {
  switch (format) {
    case "png":
      return "PNG";
    case "jpeg":
      return "JPEG";
    case "webp":
      return "WebP（无损）";
    default:
      return "保持原格式";
  }
}

/** Compact effective settings for new archives on the project home. */
export function archiveSpecSummary(
  size: string | null | undefined,
  format: string | null | undefined,
  quality: number | null | undefined,
): string {
  const sizeLabel = size === "native" ? "游戏尺寸"
    : ["1080p", "1440p", "2160p"].includes(size ?? "") ? size : "原尺寸";
  const formatLabel = format === "webp" ? "WebP 无损"
    : format === "jpeg" ? `JPEG${quality == null ? "" : ` Q${quality}`}`
    : format === "png" ? "PNG" : "原格式";
  return sizeLabel === "原尺寸" && formatLabel === "原格式"
    ? "保持原样" : `${sizeLabel} · ${formatLabel}`;
}

/** Full explanation stays available independently of the compact home copy. */
export function archiveSpecTooltip(
  size: string | null | undefined,
  format: string | null | undefined,
  quality: number | null | undefined,
): string {
  const sizeLabel = archiveSizeLabel(size) === "保持原样" ? "保持原尺寸" : archiveSizeLabel(size);
  const qualityLabel = format === "jpeg" && quality != null ? ` · 质量 ${quality}` : "";
  return `新归档设置：${sizeLabel} · ${archiveFormatLabel(format)}${qualityLabel}。仅用于后续新归档图片，历史图片规格可能不同。`;
}

/** Original file line: `3840×2160 · PNG · 8.6 MB`; null when nothing known. */
export function describeSourceImage(
  facts: SourceImageFacts | null | undefined,
  bytes: number | null | undefined,
): string | null {
  if (!facts) return null;
  const parts = [
    formatResolution(facts.width, facts.height),
    formatLabel(facts.format),
    formatBytesOrNull(bytes),
  ].filter((part): part is string => Boolean(part));
  return parts.length ? parts.join(" · ") : null;
}

export interface ArchiveImageLine {
  text: string;
  tooltip: string | null;
}

/**
 * Archived file line, e.g. `1920×1080 · JPEG q90 · 1.2 MB`. Quality is only
 * shown when this app encoded the file; anything unknown stays unstated
 * (its reason moves into the tooltip).
 */
export function describeArchiveImage(
  facts: ArchiveImageFacts | null | undefined,
): ArchiveImageLine | null {
  if (!facts) return null;
  const format = formatLabel(facts.format);
  let quality: string | null = null;
  if (facts.qualityKind === "quality" && facts.quality != null) {
    quality = `q${facts.quality}`;
  } else if (facts.qualityKind === "lossless") {
    quality = "无损";
  }
  const parts = [
    formatResolution(facts.width, facts.height),
    format ? (quality ? `${format} ${quality}` : format) : null,
    formatBytesOrNull(facts.bytes),
  ].filter((part): part is string => Boolean(part));
  if (!parts.length) return null;
  const notes: string[] = [];
  if (facts.qualityKind === "unknown") notes.push("质量未知");
  if (facts.formatApplied === false) {
    notes.push("格式未转换：源文件无法解码");
  } else if (facts.sizePolicy === "native" && facts.sizePolicyApplied === false) {
    notes.push("未应用：缺少引擎数据");
  } else if (facts.sizePolicyApplied === false) {
    notes.push("未应用：源文件无法按此规格处理");
  }
  return { text: parts.join(" · "), tooltip: notes.length ? notes.join("；") : null };
}

/** Major version from a raw engine version string (`8.1.3…` → `8`). */
export function renpyMajor(version: string | null | undefined): string | null {
  const match = (version ?? "").match(/(\d+)/);
  return match ? match[1] : null;
}

/** `Ren'Py 8`, `Ren'Py 7/8`, or plain `Ren'Py` when no version is known. */
export function renpyBadgeLabel(
  versions: Array<string | null | undefined> | null | undefined,
): string {
  const majors = new Set<string>();
  for (const version of versions ?? []) {
    const major = renpyMajor(version);
    if (major) majors.add(major);
  }
  const sorted = [...majors].sort();
  return sorted.length ? `Ren'Py ${sorted.join("/")}` : "Ren'Py";
}

/** Raw version list for the badge tooltip; null when nothing is known. */
export function renpyBadgeTitle(
  versions: Array<string | null | undefined> | null | undefined,
): string | null {
  const values = (versions ?? [])
    .map((value) => (value ?? "").trim())
    .filter(Boolean);
  return values.length ? values.join("、") : null;
}

/** Splits the comma-joined engine version list from the project overview. */
export function engineVersionList(value: string | null | undefined): string[] {
  return (value ?? "")
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
}
