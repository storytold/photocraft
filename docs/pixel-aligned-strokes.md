# Pixel Align Stroke

`shape.create`, `shape.edit`, `shape.info`, `path.toShape` and `shape.presets.place` expose
`pixelAlignStroke`. It defaults to `false`. The Shape Tools options bar and
Properties edit the selected shape; the tool preference also controls new shapes,
Custom Shape placement and Pen shape creation. Tool presets keep the option;
Reset Tool restores its default for Shape Tools and all three Pen tools.

Make Shape forwards the tool preference for Work and saved paths. An owned Pen
draft uses the preference captured when drawing began. `path.toShape` accepts
only a boolean when the option is present, defaults to false when omitted, and
preserves the original path geometry; conversion and alignment undo together.

When `shape.create` appends with `addTo`, an explicit `pixelAlignStroke` updates the
target layer; omitting it preserves that layer's setting. The Pen checkbox shows
the existing Combine target's setting without overwriting the New Layer default.
During a draft it edits the captured setting, applied together with the combined
or resumed path in one undo step. New Layer retains the tool preference.

Alignment is a rendering option. Stored knots, control handles, live parameters,
stroke width, dash lengths and dash offset are unchanged. Turning it off restores
the legacy renderer. Native projects serialize the option only when enabled.
PSD shapes map it to the `vstk` descriptor's `strokeStyleStrokeAdjust` boolean
([Adobe Generator documentation](https://github.com/adobe-photoshop/generator-core/wiki/JSON-event-format/6a7f39780beb75b0f9988779975201f44e6978f2));
a missing field defaults to false. Changing only this flag preserves unknown
stroke descriptor entries. This mapping does not claim pixel-for-pixel Photoshop
rendering parity.

## Rendering policy

- Integer-bounded, centered 1 px rectangles retain the existing interior-row
  policy, including all four sides, corners and fractional dash offsets.
- Other centered 1 px strokes align isolated, straight horizontal/vertical runs
  whose centerline lies at an integer document coordinate. Coverage from the two
  half-covered rows/columns is combined into the positive-side pixel. On eligible
  straight runs, dashed cap coverage is selected at pixel centers; exact half-pixel
  boundary ties round toward increasing document coordinates. Integer-length
  dashes therefore have equal opaque pixel counts even with fractional dash gaps
  or offsets. Round and square caps retain their extension: a centered 1px
  stroke with a 9px dash body occupies 10 opaque pixels with these caps, versus
  9 pixels with butt caps. Round-cap selection uses the stroker's area-exact
  semicircle coverage rather than the butt-cap threshold. No semitransparent
  cap pixels remain on these runs. Stored dash
  lengths and phase are unchanged; their rasterized boundaries move by at most
  half a pixel. Fractional dash lengths can occupy adjacent integer pixel counts,
  and gaps shorter than a pixel can disappear at document resolution.
  If adjacent area samples are indistinguishable (for example `[1, 1]` dashes
  with a half-pixel offset), a small positive translation of the compiled outline
  resolves only the coverage tie. Dashing is not recomputed for that probe.
- The same path-based rule handles Rounded Rectangle, Polygon, Triangle, Star,
  Custom Shape and Pen paths. Pure curves and diagonals keep their original
  antialiasing, including Ellipse. Flattened curve chords are never mistaken for
  straight source segments.
- Line Tool and ArrowLine bodies are filled vector geometry. An axis-aligned live
  1 px line uses a half-pixel translation of a render-only copy of its fill path.
  Arrowheads move with the shaft and retain antialiasing and proportions. Paint
  coordinates and the stored vector geometry stay unchanged.
  Drag previews apply the same fill alignment in document space before the view's
  zoom, rotation and translation, then discard live metadata to avoid a second
  alignment in screen space. Path-only previews remain unpainted.
- Alignment is evaluated in document pixels after geometry transforms. Fractional
  centerlines and rotated diagonals retain their original coverage. Tiled/banded
  rendering samples both neighboring rows so tile boundaries do not alter results.

Retained PSD paths get an axis-specific allowance of half an 8.24 fixed-point
quantum of the document dimensions (at most about 0.009 px on a valid PSB canvas).
Rectangle clipping and normal coverage account for that error at render time,
while the original path still determines dash lengths and phase. Native paths
retain the strict floating-point epsilon; genuinely fractional centerlines beyond
the allowance retain legacy rendering. Appearance edits preserve the allowance
while the original PSD path block is retained.

## Deliberate limits

The general path rule retains legacy coverage within two pixels of source
segment endpoints/joins, on runs shorter than five pixels, at intersecting or
nearby geometry, on paths with more than 256 knots, and when stroke tessellation
exceeds 8192 points. It does not force sharp
corners on arbitrary paths. Integer rectangles use their existing corner rule.

Widths other than 1 px and inside/outside stroke alignment keep legacy rendering.
Fractional-coordinate strokes are not rounded. Curves, including ellipses, are
not snapped or thresholded. They remain antialiased.

Aligned shape-tool previews rasterize in document pixels before the view transform,
so zoom cannot make a fractional-width stroke eligible. Integer grid zooms use
nearest filtering; rotated or fractional views filter the document coverage.
The preview texture is capped at 2048 pixels per dimension. Larger visible bounds
are reduced by box-filtering the original aligned document pixels, using small
native tiles and premultiplied colour averages. No alignment is disabled at the
size threshold, and stored geometry and dash phase remain unchanged. Very large
previews complete progressively: each draw pass processes at most 4,194,304 source
pixels, requests another frame if needed, and releases the working sums when
complete or when the drag ends. While dragging such a large shape, its painted preview can be partial
until the remaining tiles have been processed; the vector feedback outline stays
visible. This bounds both allocation and per-frame raster work, including on WASM.

Filled Line/ArrowLine alignment requires live parameters matching the current
path, and is disabled for density/feather masks. A point edit or transform that
drops live line metadata leaves the filled path on the legacy renderer. A Line
Tool shape with an additional visible outline retains legacy fill placement, so
the fill and outline remain registered. Imported PSD path quantization can still
prevent an exact live/path match for these filled bars; the ordinary stroke's
quantization allowance does not relax this live-geometry check.

Splitting a shape for interior effects or clipped layers preserves the original
fill-alignment eligibility, so removing the outline for the fill pass cannot
introduce a half-pixel shift. A fully transparent solid outline does not block
alignment of an otherwise eligible live bar; gradient and pattern outlines keep
the conservative fallback.

Tests cover live-line alignment, rectangle dash phases and visible corners,
Pen dashes and tile boundaries, curved corners, polygons/triangles, curve and
star antialiasing, crossing custom subpaths, transforms, arrowheads, stale live
metadata, command validation/undo/redo, preset placement, tool presets/reset,
selected-shape UI,
native round-trip/defaults, and PSD flags/unknown entries. Additional regressions
cover complete-stroke clipboard snapshots, simultaneous width/alignment edits,
Make Shape conversion, canvas-aligned gradient previews, and live-line fill/outline
registration through interior effects at 8-, 16-, and 32-bit depths.
