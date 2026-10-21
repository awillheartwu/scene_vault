import type { ProjectArchiveUsage, ProjectOverviewSummary } from "@/lib/capture-api";
import { formatBytes } from "@/lib/image-facts";

const stateLabels: Record<ProjectArchiveUsage["state"], string> = {
  ready: "归档",
  unconfigured: "未设置归档",
  unavailable: "未连接",
  missing: "目录不存在",
  partial: "统计不完整",
  error: "无法读取",
};

/** Shared copy for the card summary and the list's separate archive columns. */
export function projectArchivePresentation(
  project: ProjectOverviewSummary,
  usage?: ProjectArchiveUsage,
) {
  const state = usage?.state ?? "loading";
  const label = usage ? stateLabels[usage.state] : "归档统计中…";
  const mainImageCount = usage?.state === "ready" && usage.mainImageCount !== null
    ? usage.mainImageCount.toLocaleString("zh-CN") : "—";
  const totalSize = usage?.state === "ready" && usage.totalBytes !== null
    ? formatBytes(usage.totalBytes) : "—";
  const lines = [
    "主图张数含人物图、游戏截图和收藏，不含头像。",
    "总大小包含所有正式归档图片与头像。",
    `截图来源：${project.sourceCount} 个`,
  ];
  if (!usage) {
    lines.unshift("正在统计归档目录…");
  } else {
    if (usage.state !== "ready") lines.unshift(label);
    if (usage.state === "partial") {
      lines.push("未取得完整结果，因此不显示数量和大小");
    } else if (usage.state === "ready") {
      lines.push(`主图：${mainImageCount} 张`);
      lines.push(usage.avatarImageCount === null
        ? "头像：数量未知"
        : `头像：${usage.avatarImageCount.toLocaleString("zh-CN")} 张`);
      lines.push(usage.totalBytes === null
        ? "总大小：未知"
        : `总大小：${totalSize}（${usage.totalBytes.toLocaleString("zh-CN")} 字节）`);
    }
    const scannedAt = new Date(usage.scannedAt);
    lines.push(`统计时间：${Number.isNaN(scannedAt.getTime())
      ? usage.scannedAt
      : scannedAt.toLocaleString("zh-CN", { hour12: false })}`);
    if (usage.message) lines.push(usage.message);
  }
  return { state, label, mainImageCount, totalSize, tooltip: lines.join("\n") };
}
