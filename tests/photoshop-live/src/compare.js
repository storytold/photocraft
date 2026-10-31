/**
 * Compare RGBA canvases exactly, counting visible RGB and all alpha differences
 * per pixel when sizes match. Hidden RGB is ignored only when both alpha values
 * are zero; malformed buffers throw. Examples are diagnostic only.
 * @param {{width: number, height: number, data: Uint8Array}} expected
 * @param {{width: number, height: number, data: Uint8Array}} actual
 * @param {number} maxExamples Maximum mismatch coordinates to retain.
 */
export function compareRgba(expected, actual, maxExamples = 24) {
  for (const [name, raster] of [["expected", expected], ["actual", actual]]) {
    if (!Number.isSafeInteger(raster.width) || !Number.isSafeInteger(raster.height) || raster.width < 1 || raster.height < 1 ||
        !raster.data || raster.data.length !== raster.width * raster.height * 4)
      throw new Error(`Invalid ${name} RGBA raster dimensions or buffer length`);
  }
  if (expected.width !== actual.width || expected.height !== actual.height)
    return { sameSize: false, expectedSize: [expected.width, expected.height], actualSize: [actual.width, actual.height], rgbPixels: null, alphaPixels: null };
  const width = expected.width, height = expected.height;
  let rgbPixels = 0, alphaPixels = 0, maxRgbDifference = 0, maxAlphaDifference = 0;
  const examples = [];
  for (let i = 0; i < width * height; i++) {
    const p = i * 4, ea = expected.data[p + 3], aa = actual.data[p + 3];
    const alphaDifference = Math.abs(ea - aa);
    if (alphaDifference) { alphaPixels++; maxAlphaDifference = Math.max(maxAlphaDifference, alphaDifference); }
    let rgbDifference = 0;
    if (ea !== 0 || aa !== 0) for (let channel = 0; channel < 3; channel++)
      rgbDifference = Math.max(rgbDifference, Math.abs(expected.data[p + channel] - actual.data[p + channel]));
    if (rgbDifference) { rgbPixels++; maxRgbDifference = Math.max(maxRgbDifference, rgbDifference); }
    if ((rgbDifference || alphaDifference) && examples.length < maxExamples)
      examples.push({ x: i % width, y: Math.floor(i / width), expected: [...expected.data.subarray(p, p + 4)], actual: [...actual.data.subarray(p, p + 4)], rgbDifference, alphaDifference });
  }
  return { sameSize: true, width, height, rgbPixels, alphaPixels, maxRgbDifference, maxAlphaDifference, examples, exact: rgbPixels === 0 && alphaPixels === 0 };
}
