export const cases = [
  { id: "issue-original", kind: "issue", hue: -158, saturation: 20, lightness: 20, clipped: true },
  { id: "swatches-combined", kind: "swatches", hue: -158, saturation: 20, lightness: 20, clipped: true, exploratory: true },
  { id: "swatches-hue-only", kind: "swatches", hue: -158, saturation: 0, lightness: 0, clipped: true, exploratory: true },
  { id: "swatches-saturation-only", kind: "swatches", hue: 0, saturation: 20, lightness: 0, clipped: true, exploratory: true },
  { id: "swatches-lightness-only", kind: "swatches", hue: 0, saturation: 0, lightness: 20, clipped: true, exploratory: true },
  { id: "swatches-post-lightness-hs", kind: "post-lightness", hue: -158, saturation: 20, lightness: 0, clipped: true, exploratory: true },
  { id: "swatches-hue-lightness", kind: "swatches", hue: -158, saturation: 0, lightness: 20, clipped: true, exploratory: true },
  { id: "swatches-saturation-lightness", kind: "swatches", hue: 0, saturation: 20, lightness: 20, clipped: true, exploratory: true },
  { id: "swatches-hue-minus90", kind: "swatches", hue: -90, saturation: 20, lightness: 20, clipped: true, exploratory: true },
  { id: "swatches-hue-plus40", kind: "swatches", hue: 40, saturation: 20, lightness: 20, clipped: true, exploratory: true },
  { id: "swatches-hue-plus120", kind: "swatches", hue: 120, saturation: 20, lightness: 20, clipped: true, exploratory: true },
  { id: "swatches-hue-minus157", kind: "swatches", hue: -157, saturation: 20, lightness: 20, clipped: true, exploratory: true },
  { id: "sectors-hue-minus157", kind: "sectors", hue: -157, saturation: 20, lightness: 20, clipped: true, exploratory: true },
  { id: "sectors-hue-plus40", kind: "sectors", hue: 40, saturation: 20, lightness: 20, clipped: true, exploratory: true },
  { id: "gray-256-combined", kind: "gray", hue: -158, saturation: 20, lightness: 20, clipped: true, exploratory: true },
  { id: "gray-256-identity", kind: "gray", hue: 0, saturation: 0, lightness: 0, clipped: true, identity: true, exploratory: true },
  { id: "image-hue-only", kind: "source-image", hue: -158, saturation: 0, lightness: 0, clipped: true, exploratory: true },
  { id: "image-saturation-only", kind: "source-image", hue: 0, saturation: 20, lightness: 0, clipped: true, exploratory: true },
];

/**
 * Select the issue fixture by default or the explicit comma-separated case
 * IDs in PHOTOCRAFT_HS_CASES. Reject unknown IDs so a typo cannot silently
 * turn a requested Photoshop check into a skipped case.
 * @param {{PHOTOCRAFT_HS_CASES?: string}} env
 * @returns {typeof cases}
 */
export function selectedCases(env = process.env) {
  const selected = env.PHOTOCRAFT_HS_CASES?.split(",").map(s => s.trim()).filter(Boolean);
  if (!selected?.length) return cases.filter(c => !c.exploratory);
  const unknown = selected.filter(id => !cases.some(c => c.id === id));
  if (unknown.length) throw new Error(`Unknown PHOTOCRAFT_HS_CASES: ${unknown.join(", ")}`);
  return cases.filter(c => selected.includes(c.id));
}

export const swatches = [
  [1, 34, 65], [1, 83, 133], [74, 141, 183], [170, 184, 193],
  [0, 0, 0], [1, 1, 1], [64, 64, 64], [128, 128, 128], [255, 255, 255],
  [255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0], [0, 255, 255], [255, 0, 255],
];

export const postLightnessSwatches = [
  [52, 78, 103], [52, 117, 157], [110, 164, 197], [187, 198, 205],
];

/**
 * Build opaque RGBA8 pixels for synthetic Photoshop source documents. The
 * swatches cover diagnostic ties, sectors cover six hue intervals, and gray
 * covers every channel value; unsupported kinds throw.
 * @param {"swatches"|"post-lightness"|"sectors"|"gray"} kind
 * @returns {{width: number, height: number, data: Buffer}}
 */
export function rasterFor(kind) {
  if (kind === "swatches" || kind === "post-lightness") {
    const samples = kind === "swatches" ? swatches : postLightnessSwatches;
    const data = Buffer.from(samples.flatMap(rgb => [...rgb, 255]));
    return { width: samples.length, height: 1, data };
  }
  if (kind === "sectors") {
    const data = Buffer.alloc(256 * 6 * 4);
    for (let v = 0; v < 256; v++) {
      const colors = [[255, v, 0], [v, 255, 0], [0, 255, v], [0, v, 255], [v, 0, 255], [255, 0, v]];
      for (let sector = 0; sector < colors.length; sector++) {
        const p = (sector * 256 + v) * 4;
        for (let channel = 0; channel < 3; channel++) data[p + channel] = colors[sector][channel];
        data[p + 3] = 255;
      }
    }
    return { width: 256, height: 6, data };
  }
  if (kind !== "gray") throw new Error(`Unsupported synthetic raster: ${kind}`);
  const data = Buffer.from(Array.from({ length: 256 }, (_, v) => [v, v, v, 255]).flat());
  return { width: 256, height: 1, data };
}
