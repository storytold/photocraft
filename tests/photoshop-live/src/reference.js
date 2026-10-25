import assert from "node:assert/strict";

/**
 * Reject a Photoshop reference that did not exercise the requested adjustment.
 * Identity must leave every channel untouched; other cases must change visible
 * RGB while preserving alpha because Hue/Saturation does not change coverage.
 * @param {ReturnType<typeof import('./compare.js').compareRgba>} impact
 * @param {{identity?: boolean}} c
 */
export function assertPhotoshopImpact(impact, c) {
  assert.equal(impact.sameSize, true, "SETUP_UNVERIFIED: Photoshop impact canvas size changed");
  if (c.identity) {
    assert.equal(impact.exact, true, "SETUP_UNVERIFIED: identity adjustment changed Photoshop pixels");
  } else {
    assert(impact.rgbPixels > 0, "SETUP_UNVERIFIED: Photoshop adjustment caused no visible RGB change");
  }
  assert.equal(impact.alphaPixels, 0, "SETUP_UNVERIFIED: Photoshop adjustment unexpectedly changed alpha");
}
