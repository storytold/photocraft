# Average filter: transparency reference

The Average filter replaces colour while retaining each pixel's transparency.
Two synthetic cases were executed in Photoshop **26.6.1 on Windows x64** on
2026-10-23. These are observations of RGB8 raster layers, without a selection or
transparency lock; they are not a claim of parity in other colour modes or at
other bit depths.

Each input is a 16 × 16 RGBA PNG. Its left eight columns are red and its right
eight columns are blue. Open it in Photoshop, run Filter › Blur › Average, and
save a PNG copy. Inspect the file's RGBA values, including invisible colours.

| Input left / right RGBA | Output left / right RGBA |
|---|---|
| `(255,0,0,255)` / `(0,0,255,0)` | `(128,0,128,255)` / `(128,0,128,0)` |
| `(255,0,0,64)` / `(0,0,255,192)` | `(128,0,128,64)` / `(128,0,128,192)` |

An opaque control produces `(128,0,128,255)` throughout. Thus transparency is
neither a weight in this colour average nor a channel to replace with its mean.

PhotoCraft previously alpha-weighted the RGB mean and replaced alpha with its
mean. On the first case that predicts translucent red throughout, rather than
purple pixels with their original transparency. The fix uses selection coverage
for the existing mean and leaves each pixel's alpha alone. Selection mixing and
the existing colour conversion path are unchanged. In particular, this does not
establish Photoshop parity for feathered selection weighting or CMYK/Lab means.

The engine tests construct their own pixels; no Photoshop binary, proprietary
code, or reference image is included. They cover transparent and partially
transparent pixels, selection boundaries, one history step, undo and redo at
8/16/32-bit storage. The higher-depth tests establish PhotoCraft invariants;
the external reference measurements above are RGB8 only.

Run the focused regressions with:

```sh
cargo test -p photocraft-engine --lib average_preserves
```
