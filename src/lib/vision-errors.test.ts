import { describe, expect, it } from "vitest";
import { VISION_ERROR_MESSAGES, describeError } from "./vision-errors";

describe("describeError", () => {
  it("translates every engine error code", () => {
    for (const [code, message] of Object.entries(VISION_ERROR_MESSAGES)) {
      const raw = `vision engine error: ${code}: some english detail`;
      expect(describeError(raw)).toBe(message);
      expect(describeError(new Error(raw))).toBe(message);
    }
  });

  it("translates the framing failure that used to leak English", () => {
    expect(
      describeError(
        "vision engine error: roi_multiple_faces: multiple face centers were detected inside faceRoi",
      ),
    ).toContain("选区内检测到多张脸");
    expect(describeError("vision engine error: roi_no_face: no face center was detected inside faceRoi")).toContain(
      "选区内没有检测到人脸",
    );
  });

  it("translates engine lifecycle failures that carry no code", () => {
    expect(describeError("vision engine error: vision engine is not configured; configure Python")).toContain(
      "尚未配置 AI 引擎",
    );
    expect(describeError("vision engine error: Python request timed out")).toContain("响应超时");
  });

  it("passes through anything it does not know", () => {
    expect(describeError("conflict: 图片正在撤销分类或等待撤销重试，请先完成该操作")).toBe(
      "conflict: 图片正在撤销分类或等待撤销重试，请先完成该操作",
    );
    expect(describeError("vision engine error: cannot encode face box: boom")).toBe(
      "vision engine error: cannot encode face box: boom",
    );
    expect(describeError(new TypeError("x is not a function"))).toBe("x is not a function");
  });

  it("tells the user when a reset needs a fresh preview instead of a retry", () => {
    expect(describeError("conflict: capture changed since reset preview; create a new preview")).toContain(
      "请关闭窗口重新发起撤销",
    );
    expect(describeError("conflict: reset file changed since preview: D:/archive/a.png")).toContain(
      "请关闭窗口重新发起撤销",
    );
    expect(describeError("conflict: capture is unclassified or busy")).toContain("等处理结束后重试");
    expect(describeError("conflict: original source content identity is invalid")).toContain("可能已被替换");
    expect(describeError("conflict: reset cleanup did not remove the file")).toContain("可能被其他程序占用");
  });
});
