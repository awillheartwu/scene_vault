import { describe, expect, it } from "vitest";
import {
  archiveFormatLabel,
  archiveSizeLabel,
  archiveSpecSummary,
  describeArchiveImage,
  describeSourceImage,
  engineVersionList,
  formatBytes,
  formatLabel,
  formatResolution,
  renpyBadgeLabel,
  renpyBadgeTitle,
} from "@/lib/image-facts";

describe("image facts formatting", () => {
  it("formats resolutions only when both sides are known", () => {
    expect(formatResolution(3840, 2160)).toBe("3840×2160");
    expect(formatResolution(null, 2160)).toBeNull();
    expect(formatResolution(0, 1080)).toBeNull();
    expect(formatResolution(undefined, undefined)).toBeNull();
  });

  it("labels known formats and falls back to upper case", () => {
    expect(formatLabel("jpeg")).toBe("JPEG");
    expect(formatLabel("jpg")).toBe("JPEG");
    expect(formatLabel("png")).toBe("PNG");
    expect(formatLabel("webp")).toBe("WebP");
    expect(formatLabel("bmp")).toBe("BMP");
    expect(formatLabel(null)).toBeNull();
  });

  it("sizes and format labels match the select options", () => {
    expect(archiveSizeLabel("native")).toBe("按游戏设定尺寸");
    expect(archiveSizeLabel("1080p")).toBe("1080p");
    expect(archiveSizeLabel("keep")).toBe("保持原样");
    expect(archiveFormatLabel("webp")).toBe("WebP（无损）");
    expect(archiveFormatLabel("keep")).toBe("保持原格式");
    expect(formatBytes(524288)).toBe("512.0 KB");
  });

  it("summarizes the archive policy for the project home", () => {
    expect(archiveSpecSummary("keep", "keep", 90)).toBe("保持原样");
    expect(archiveSpecSummary("native", "keep", 90)).toBe("游戏尺寸 · 原格式");
    expect(archiveSpecSummary("keep", "jpeg", 92)).toBe("原尺寸 · JPEG Q92");
    expect(archiveSpecSummary("1080p", "jpeg", 92)).toBe("1080p · JPEG Q92");
    expect(archiveSpecSummary("1080p", "jpeg", null)).toBe("1080p · JPEG");
    expect(archiveSpecSummary("1080p", "webp", 92)).toBe("1080p · WebP 无损");
    expect(archiveSpecSummary("native", "png", 92)).toBe("游戏尺寸 · PNG");
    expect(archiveSpecSummary("keep", "inherit", 90)).toBe("保持原样");
  });

  it("describes the original file without inventing quality", () => {
    expect(describeSourceImage({ width: 3840, height: 2160, format: "png" }, 9_000_000)).toBe(
      "3840×2160 · PNG · 8.6 MB",
    );
    expect(describeSourceImage({ width: null, height: null, format: "jpeg" }, null)).toBe("JPEG");
    expect(describeSourceImage(null, 1024)).toBeNull();
  });

  it("describes the archived file with its recorded quality", () => {
    const line = describeArchiveImage({
      width: 1920,
      height: 1080,
      format: "jpeg",
      bytes: 1_200_000,
      qualityKind: "quality",
      quality: 90,
      sizePolicy: "native",
      sizePolicyApplied: true,
      formatApplied: true,
    });
    expect(line?.text).toBe("1920×1080 · JPEG q90 · 1.1 MB");
    expect(line?.tooltip).toBeNull();

    const lossless = describeArchiveImage({
      width: 1920,
      height: 1080,
      format: "webp",
      bytes: 500_000,
      qualityKind: "lossless",
      quality: null,
      sizePolicy: "keep",
      sizePolicyApplied: true,
      formatApplied: true,
    });
    expect(lossless?.text).toBe("1920×1080 · WebP 无损 · 488.3 KB");

    const historicalLossyWebp = describeArchiveImage({
      width: 1920,
      height: 1080,
      format: "webp",
      bytes: 500_000,
      qualityKind: "quality",
      quality: 90,
      sizePolicy: "keep",
      sizePolicyApplied: true,
      formatApplied: true,
    });
    expect(historicalLossyWebp?.text).toBe("1920×1080 · WebP q90 · 488.3 KB");

    const unknown = describeArchiveImage({
      width: 1920,
      height: 1080,
      format: "jpeg",
      bytes: null,
      qualityKind: "unknown",
      quality: null,
      sizePolicy: "native",
      sizePolicyApplied: false,
      formatApplied: true,
    });
    expect(unknown?.text).toBe("1920×1080 · JPEG");
    expect(unknown?.tooltip).toContain("质量未知");
    expect(unknown?.tooltip).toContain("未应用：缺少引擎数据");
  });

  it("builds the Ren'Py badge from version strings", () => {
    expect(renpyBadgeLabel(["8.1.3.23091805"])).toBe("Ren'Py 8");
    expect(renpyBadgeLabel(["7.6.1.23060707"])).toBe("Ren'Py 7");
    expect(renpyBadgeLabel(["8.1.3", "7.6.1"])).toBe("Ren'Py 7/8");
    expect(renpyBadgeLabel([null])).toBe("Ren'Py");
    expect(renpyBadgeTitle(["8.1.3", "7"])).toBe("8.1.3、7");
    expect(renpyBadgeTitle([null])).toBeNull();
  });

  it("splits the overview version list", () => {
    expect(engineVersionList("8.1.3, 7")).toEqual(["8.1.3", "7"]);
    expect(engineVersionList(null)).toEqual([]);
  });
});
