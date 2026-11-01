import { describe, expect, it } from "vitest";
import { assertPhotoshopImpact } from "../src/reference.js";

describe("Photoshop reference validity", () => {
  it("rejects a nonidentity reference with only alpha impact", () => {
    expect(() => assertPhotoshopImpact({ sameSize: true, exact: false, rgbPixels: 0, alphaPixels: 2 }, { identity: false })).toThrow(/no visible RGB change/);
  });
  it("requires stable alpha for every Hue/Saturation case", () => {
    expect(() => assertPhotoshopImpact({ sameSize: true, exact: false, rgbPixels: 1, alphaPixels: 1 }, { identity: false })).toThrow(/unexpectedly changed alpha/);
  });
});
