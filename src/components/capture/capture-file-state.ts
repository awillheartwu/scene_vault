import type {
  CaptureDestinationFileState,
  CaptureItem,
  CaptureSourceFileState,
} from "@/lib/capture-api";

export type CaptureFileVariant = "source" | "annotated" | "avatar" | "destination";

export interface CaptureFileIssue {
  kind: "source" | "target";
  state: "missing" | "replaced" | "unavailable";
  label: string;
}

type CaptureFileStateItem = Pick<
  CaptureItem,
  | "sourcePath"
  | "annotatedPath"
  | "avatarPath"
  | "destinationPath"
  | "destinationAvatarPath"
  | "sourceFileState"
  | "destinationFileState"
  | "destinationAvatarFileState"
>;

export function sourceFileIssueLabel(
  state?: CaptureSourceFileState,
): string | null {
  if (state === "missing") return "原图缺失";
  if (state === "replaced") return "原图已被替换";
  return null;
}

export function destinationFileIssueLabel(
  state?: CaptureDestinationFileState,
): string | null {
  if (state === "missing") return "归档图缺失";
  if (state === "unavailable") return "归档位置不可访问";
  return null;
}

export function avatarFileIssueLabel(
  state?: CaptureDestinationFileState,
): string | null {
  if (state === "missing") return "头像归档缺失";
  if (state === "unavailable") return "头像归档不可访问";
  return null;
}

/** File issues worth surfacing as chips; at most one source and one target issue. */
export function captureFileIssues(
  item: CaptureFileStateItem | null | undefined,
): CaptureFileIssue[] {
  if (!item) return [];
  const issues: CaptureFileIssue[] = [];
  const source = sourceFileIssueLabel(item.sourceFileState);
  if (
    source &&
    (item.sourceFileState === "missing" || item.sourceFileState === "replaced")
  ) {
    issues.push({
      kind: "source",
      state: item.sourceFileState,
      label: source,
    });
  }
  const destination = destinationFileIssueLabel(item.destinationFileState);
  const avatar = avatarFileIssueLabel(item.destinationAvatarFileState);
  if (
    destination &&
    item.destinationPath &&
    (item.destinationFileState === "missing" ||
      item.destinationFileState === "unavailable")
  ) {
    issues.push({
      kind: "target",
      state: item.destinationFileState,
      label: destination,
    });
  } else if (
    avatar &&
    item.destinationAvatarPath &&
    (item.destinationAvatarFileState === "missing" ||
      item.destinationAvatarFileState === "unavailable")
  ) {
    issues.push({
      kind: "target",
      state: item.destinationAvatarFileState,
      label: avatar,
    });
  }
  return issues;
}

/** Reason a read or reveal for this variant is pointless, or null when it may proceed. */
export function captureVariantReadReason(
  item: CaptureFileStateItem,
  variant: CaptureFileVariant,
): string | null {
  if (variant === "source") {
    if (!item.sourcePath) return "没有原图路径";
    return sourceFileIssueLabel(item.sourceFileState);
  }
  if (variant === "annotated") {
    return item.annotatedPath ? null : "没有标注图";
  }
  if (variant === "avatar") {
    // The avatar may be readable server-side by item id even when the summary
    // card only carries the capture id; only an explicit missing/unreachable
    // archive state (without a local generated file) justifies a block.
    if (item.avatarPath) return null;
    return avatarFileIssueLabel(item.destinationAvatarFileState);
  }
  if (variant === "destination") {
    if (!item.destinationPath) return "没有归档图";
    return destinationFileIssueLabel(item.destinationFileState);
  }
  return null;
}
