# Align Edges

A shape layer's **Align Edges** (`ShapeLayer::use_aligned_rendering`, PSD block `sn2P`, scripting
key `useAlignedRendering`) fits the whole shape to whole document pixels when it is rendered, as the
reference application does. The stored path is never changed: an anchor at 10.5 stays at 10.5, and
turning the flag off renders the original soft edges again.

## The rule

Measured in the reference application (a black shape on white):

1. Take the bounds of what the shape paints: the fill area, or the stroke's outer edge when it has
   a visible stroke (a centred stroke reaches half its width beyond the path, an outside stroke its
   whole width, an inside stroke nothing).
2. Round the left and top edge to the nearest whole pixel (a half rounds up: 10.5 becomes 11) and
   the width and height to whole pixels (20.5 becomes 21).
3. Translate and scale the whole path so its painted bounds fill that box. The fill and the stroke
   take this one transform; the stroke keeps its width.

So curves stay antialiased (an ellipse moves, it is not bent), diagonals just move with the shape,
and the inner edges of a shape with several subpaths are not snapped one by one. A few results:

| Shape | Reference application, Align Edges on |
|---|---|
| Rectangle 10.5..30.5, fill | covers columns 11..30 (off: columns 10 and 30 half covered) |
| Rectangle 10.25..30.75 / 10.4..30.6 / 10.75..30.25 | covers 10..30 / 10..29 / 11..30 |
| 1 px centred stroke on rectangle 10..30 | the stroke paints columns 10 and 30 |
| 2 px stroke on rectangle 10.25..30.25 | columns 9-10 and 29-30 |
| Fill + 1 px stroke at 10.5 | unchanged: its outer edge 10..31 is already whole |
| Pen triangle (10.5,10.5) (30.5,10.5) (10.5,30.5) | moves by half a pixel on both axes |

Not measured, and chosen here: a shape thinner than half a pixel keeps its size (rounding would
make it vanish); an inverted path (it paints everywhere) is not moved; a 1 px Line paints its band
19.5..20.5 on row 20 (the reference's Line tool could not be driven by script).

The rule lives in `photocraft_vector::align_edges` and is applied in `CompiledShape`, which every
renderer uses: layer caches, the compositor (also its fill/stroke split under layer effects, which
starts from `align_edges::aligned_shape` so both parts move together), previews and extents.

## The checkbox

The shape tools (not the Star tool, which the reference application lacks) and the Pen (in Shape mode) have one **Align Edges** checkbox at the options bar's
right end, on by default and after Reset Tool, greyed in Path mode. It does both: the drag lands on
whole pixels (with it off, the Rectangle's, Triangle's and Polygon's drags keep their fractions; the other
tools' drags always land on whole pixels), and the new shape layer gets Align Edges. With a shape layer selected the checkbox
shows and edits that layer's flag, as Fill and Stroke do; the Properties panel has it too. Tool
presets store it as `useAlignedRendering` (older settings and presets as `align_edges`).

## Files

- PSD: `sn2P` is a four-byte value, nonzero for on. It is separate from the stroke's Stroke Adjust.
  The reference application writes it as 1 when on and leaves it out when off (checked by saving
  both), so a PSD shape layer without it renders without Align Edges.
- `.pcraft`: `use_aligned_rendering`, written only when on. Files from before it existed have no
  such field and keep rendering as they did (off).
- Scripting: `shape.create`, `shape.edit`, `path.toShape` and `shape.presets.place` take
  `useAlignedRendering`; `shape.info` reports it. Without the param a new layer is off (scripts
  keep their old results); `addTo` keeps the target layer's value. The UI always passes the
  checkbox's value.
