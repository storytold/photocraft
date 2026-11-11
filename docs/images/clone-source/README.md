# Clone source preview captures

These screenshots use an original synthetic red circle on white, rendered offscreen with the
PhotoCraft `snapshot` example. UI and artwork: PhotoCraft contributors, MIT OR Apache-2.0;
see the repository license files. No external artwork or fonts are included here.

- `opacity-flow-before.png`: the original hover renderer, with full-strength preview despite
  the Clone Stamp's 50% opacity and 40% flow. The source sampler is held fixed to isolate the
  preview-strength change.
- `opacity-flow-after.png`: the same capture with opacity and flow applied. The preview is
  approximately 20% strength, matching a single hard-tip stamp on an empty destination layer.
- `cross-document.png`: a source picked in a separate document, previewed on an empty destination
  layer with the same opacity/flow settings.
- `source-edge.png`: a source picked at its image's top-left corner. Missing source pixels produce
  straight top and left clipping edges; the remaining preview has the brush's curved edge.

For the opacity/flow comparison, pass the contents of `capture.json` to the snapshot example's
`--script` argument, with `--size 1000x760 --scale 1 --hover-at 434,493` and the desired `--out` path.
The before capture uses the original `canvas/strokes.rs`; the after capture uses the fix.
The engine source sampler is identical in these two captures.
