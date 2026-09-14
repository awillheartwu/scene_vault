import { describe, expect, it } from "vitest";
import { normalizeFaceBox, parseFaceBox } from "./face-box";

describe("parseFaceBox", () => {
  it("reads a stored pixel box", () => {
    expect(parseFaceBox('{"x":10,"y":20,"width":30,"height":40}')).toEqual({ x: 10, y: 20, width: 30, height: 40 });
  });

  it("ignores missing, malformed or degenerate boxes", () => {
    for (const raw of [null, undefined, "", "not json", "{}", '{"x":1,"y":1,"width":0,"height":10}', '{"x":"a","y":1,"width":2,"height":3}', "[1,2,3]"]) {
      expect(parseFaceBox(raw)).toBeNull();
    }
  });
});

describe("normalizeFaceBox", () => {
  it("converts pixels into a normalized region", () => {
    expect(normalizeFaceBox({ x: 250, y: 50, width: 500, height: 100 }, 1000, 1000)).toEqual({ x: 0.25, y: 0.05, width: 0.5, height: 0.1 });
  });

  it("clamps a box that reaches outside the picture", () => {
    expect(normalizeFaceBox({ x: -50, y: 900, width: 200, height: 300 }, 1000, 1000)).toEqual({ x: 0, y: 0.9, width: 0.15, height: 0.1 });
  });

  it("refuses boxes without a picture or without area", () => {
    expect(normalizeFaceBox({ x: 0, y: 0, width: 10, height: 10 }, 0, 0)).toBeNull();
    expect(normalizeFaceBox({ x: 2000, y: 0, width: 10, height: 10 }, 1000, 1000)).toBeNull();
    expect(normalizeFaceBox(null, 1000, 1000)).toBeNull();
  });
});
