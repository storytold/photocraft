# Parser hardening

Parser safety is broader than “does not crash.” A parser must also bound memory, CPU time, recursion, output expansion, and the number of objects it creates.

## Implemented controls

### PSD/PSB

`craft-psd` enforces maximum document dimensions and channel counts, bounds decoded channel data, limits ActionDescriptor nesting and pattern edges, and uses checked arithmetic for key decoded-size calculations. Tests cover malformed headers, truncation, property-generated inputs, and dedicated fuzz targets.

PhotoCraft's import preflight (`photocraft-io` `psd_budget`) charges every layer, mask, merged and pattern surface before any pixel is decoded, and also the shapes the import renders from their paths (shapes without stored pixels, and soft shapes), which can cover the whole declared canvas from a record of a few bytes. Those renders work in bounded pieces and allocate fallibly, and the compositor sizes a stored shape's fill/stroke split before drawing it (past a fixed draw limit, 2 GiB natively, the stored pixels composite as they are). Smart-filter caches (`FEid` sources, filter masks and the layers mixed through them) are charged against a separate import- and export-wide cap before they are decoded; past it they are left out with a warning. Mixing visits only the tiles the surfaces hold, never the empty grid between far-apart ones.

### Raster codecs

`photocraft-codecs` has configurable `Limits` for width, height, pixel count, and decoded allocation. The default policy is applied across enabled decoders and tested with oversized PNG, PNM, JPEG, and TIFF headers plus random/mutated inputs.

### Native format

`photocraft-format` bounds manifest, blob, and total decompressed bytes; admits the declared canvas (by default at most 300000 pixels per side, the largest canvas `file.new`, Canvas Size and PSB import create, so whatever saves reopens; `LoadOptions::max_canvas_side` / `max_canvas_pixels` let a caller lower it) before decoding anything for it; validates ZIP structure and decompressed size; verifies CRCs and content hashes; rejects unsupported versions; and property-tests random bundles/manifests.

## Review checklist

For every attacker-controlled count, offset, dimension, or length:

1. Parse into a type that can represent the file field without truncation.
2. Validate semantic limits before allocation.
3. Use `checked_add`, `checked_mul`, or a deliberately safe saturating comparison.
4. Verify a range lies inside the input before slicing.
5. Charge decoded bytes, objects, and recursion against a budget.
6. Return a format-specific error; never panic or loop indefinitely.
7. Test zero, maximum, maximum-plus-one, truncation, and inconsistent declarations.

## Future hardening

- define aggregate parse budgets shared across nested PSD resources and embedded objects;
- add time/work counters for highly fragmented but size-bounded files;
- exercise stack depth beyond ActionDescriptors, including recursive document relationships;
- add format-specific decompression ratio and object-count regression cases;
- benchmark worst-case accepted inputs on constrained machines;
- retain minimized fuzz artifacts with a non-sensitive provenance note.
