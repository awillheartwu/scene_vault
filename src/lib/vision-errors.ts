/**
 * Chinese wording for every error the Python engine can report, so an English
 * code never reaches the interface. Codes come from the engine protocol
 * (python/src/scene_vault_ai/errors.py); the host prefixes them with
 * "vision engine error: <code>: <message>".
 */
export const VISION_ERROR_MESSAGES: Record<string, string> = {
  processing_failed: "AI 引擎处理失败，可以稍后重试",
  invalid_request: "请求内容无效，可能是版本不匹配，请重启应用后再试",
  invalid_json: "AI 引擎返回的内容无法解析，请重启应用后再试",
  invalid_payload: "请求参数不完整，请重启应用后再试",
  unsupported_protocol_version: "AI 引擎协议版本不匹配，请更新到同一版本的安装包",
  unsupported_action: "AI 引擎不支持该操作，请更新到同一版本的安装包",
  capability_unavailable: "AI 引擎缺少所需能力：模型或依赖未安装完整",
  input_not_found: "原图文件不存在或已被移动",
  input_read_failed: "原图无法读取：可能被其他程序占用或权限不足",
  resource_not_found: "模型或资源文件缺失，请在「设置 → 视觉引擎」中重新配置",
  image_decode_failed: "截图无法解码：可能仍在写入或文件已损坏",
  face_detection_failed: "人脸检测失败，可以重试或重启 AI 引擎",
  roi_no_face: "选区内没有检测到人脸，请把框对准要选的那张脸",
  roi_multiple_faces: "选区内检测到多张脸，请把框缩小到只包围一张脸",
  output_write_failed: "结果文件写入失败，请检查输出目录权限与磁盘空间",
};

/** Host-side messages that carry no engine code but do reach the interface. */
const VISION_MESSAGE_PREFIXES: [string, string][] = [
  [
    "vision engine is not configured",
    "尚未配置 AI 引擎：请到「设置 → 视觉引擎」配置 Python 与模型，或安装带 AI 的安装包",
  ],
  ["Python request timed out", "AI 引擎响应超时，请稍后重试；持续超时可以重启应用"],
  ["cannot start Python", "无法启动 Python 进程，请检查「设置 → 视觉引擎」里的解释器路径"],
  ["Python process failed", "AI 引擎进程异常退出，请重试或重启应用"],
];

/**
 * Host-side failures around undo-classification and labeling. These keep their
 * machine-readable English text in the service (the log and the failure classes
 * read it), so the wording shown to the user is mapped here instead.
 */
const HOST_MESSAGE_PREFIXES: [string, string][] = [
  ["capture changed since reset preview", "图片在预览之后发生了变化，请关闭窗口重新发起撤销"],
  ["reset file changed since preview", "目标文件在预览之后被改动，请关闭窗口重新发起撤销"],
  ["reset target changed before deletion", "目标文件在删除前被改动，已停止以避免误删"],
  ["capture is unclassified or busy", "图片未分类或正在处理中，请等处理结束后重试"],
  ["capture claim unavailable", "图片正被其他操作占用，请稍后重试"],
  ["capture is owned by another operation", "图片正被其他操作占用，请稍后重试"],
  ["reset claim lost before final commit", "撤销未能生效（认领已过期），图片保持原分类"],
  ["reset cleanup did not remove the file", "文件未能删除，可能被其他程序占用"],
  ["UNC deletion requires explicit permanent-delete authorization", "NAS/网络目标需要显式确认永久删除"],
  ["original source is missing", "原图不存在，可能已被移动或删除"],
  ["original source content identity is invalid", "原图内容与记录不一致，可能已被替换"],
  ["reset target aliases a protected source", "该文件与某张原图是同一个文件，已跳过"],
  ["does not support recoverable deletion", "该位置不支持回收站删除，可以取消「同时删除归档」或手动清理"],
  ["recycle operation was cancelled", "回收站操作被取消，图片保持原分类"],
];

const VISION_PREFIX = "vision engine error: ";
const CONFLICT_PREFIX = "conflict: ";

/**
 * Formats any host error for the interface: engine codes become Chinese, a few
 * known engine lifecycle messages become Chinese, everything else passes
 * through unchanged so unexpected failures stay diagnosable.
 */
export function describeError(error: unknown): string {
  const raw = (error instanceof Error ? error.message : String(error)).trim();
  const inner = raw.startsWith(VISION_PREFIX) ? raw.slice(VISION_PREFIX.length).trim() : raw;
  const separator = inner.indexOf(": ");
  if (separator > 0) {
    const known = VISION_ERROR_MESSAGES[inner.slice(0, separator)];
    if (known) return known;
  }
  for (const [prefix, message] of VISION_MESSAGE_PREFIXES) {
    if (inner.startsWith(prefix)) return message;
  }
  const host = inner.startsWith(CONFLICT_PREFIX) ? inner.slice(CONFLICT_PREFIX.length) : inner;
  for (const [prefix, message] of HOST_MESSAGE_PREFIXES) {
    if (host.startsWith(prefix)) return message;
  }
  return raw;
}
