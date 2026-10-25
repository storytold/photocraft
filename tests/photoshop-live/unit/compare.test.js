import { describe, expect, it } from "vitest";
import { compareRgba } from "../src/compare.js";
import { rasterFor, selectedCases } from "../src/cases.js";

describe("strict visible RGB and full-canvas alpha comparator", () => {
  it("finds a one-step visible channel error and an alpha error", () => {
    const a = { width: 2, height: 1, data: Buffer.from([12, 12, 12, 255, 64, 64, 64, 0]) };
    const b = { width: 2, height: 1, data: Buffer.from([13, 12, 12, 255, 11, 22, 33, 1]) };
    expect(compareRgba(a, b)).toMatchObject({ rgbPixels: 2, alphaPixels: 1, maxRgbDifference: 53, maxAlphaDifference: 1, exact: false });
  });
  it("ignores only hidden RGB, never hidden alpha", () => {
    const a = { width: 1, height: 1, data: Buffer.from([0, 0, 0, 0]) };
    const b = { width: 1, height: 1, data: Buffer.from([255, 255, 255, 0]) };
    expect(compareRgba(a, b)).toMatchObject({ rgbPixels: 0, alphaPixels: 0, exact: true });
  });
  it("makes all 256 gray values and diagnostic RGB swatches", () => {
    const gray = rasterFor("gray"), swatches = rasterFor("swatches");
    expect([...gray.data.subarray(0, 4)]).toEqual([0, 0, 0, 255]);
    expect([...gray.data.subarray(255 * 4, 256 * 4)]).toEqual([255, 255, 255, 255]);
    expect([...swatches.data.subarray(0, 4)]).toEqual([1, 34, 65, 255]);
  });
  it("rejects misspelled single-case selection", () => {
    expect(() => selectedCases({ PHOTOCRAFT_HS_CASES: "missing" })).toThrow(/Unknown/);
  });
  it("covers all six hue sectors including endpoint ties", () => {
    const ramp = rasterFor("sectors");
    expect([ramp.width, ramp.height]).toEqual([256, 6]);
    expect([...ramp.data.subarray(4, 8)]).toEqual([255, 1, 0, 255]);
    expect([...ramp.data.subarray((3 * 256 + 255) * 4, (3 * 256 + 256) * 4)]).toEqual([0, 255, 255, 255]);
  });
  it("runs the reported issue by default and opts into diagnostic cases explicitly", () => {
    expect(selectedCases({}).map(c => c.id)).toEqual(["issue-original"]);
    expect(selectedCases({ PHOTOCRAFT_HS_CASES: "swatches-hue-only" }).map(c => c.id)).toEqual(["swatches-hue-only"]);
  });
  it("rejects truncated RGBA data rather than reporting an exact match", () => {
    const full = { width: 1, height: 1, data: Buffer.from([1, 2, 3, 255]) };
    const short = { width: 1, height: 1, data: Buffer.from([1, 2, 3]) };
    expect(() => compareRgba(full, short)).toThrow(/Invalid actual RGBA/);
    expect(() => compareRgba({ ...full, width: 0 }, full)).toThrow(/Invalid expected RGBA/);
  });
});
