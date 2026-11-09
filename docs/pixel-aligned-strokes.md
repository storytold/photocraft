# Align Edges

Shape tools expose **Align Edges** in the options bar. The corresponding scripted
setting is `useAlignedRendering` on `shape.create`, `shape.edit`, `shape.info`,
`path.toShape` and `shape.presets.place`. It defaults to false for new shapes;
appending with `addTo` preserves the target setting when omitted. Tool presets
retain the preference; Reset Tool restores false.

Alignment affects both fill and stroke. Straight horizontal and vertical source
edges at fractional coordinates snap to document pixel boundaries on a render-only
copy. Axis-aligned curve extrema snap too, including ellipse boundaries. Handles
move with their anchors, preserving adjoining curve tangents.
Curves and diagonals retain antialiasing; flattened curve chords are never snapped.
Stored paths, live parameters, stroke widths and dash settings are unchanged.
Turning the option off restores the original coverage. Existing one-pixel stroke
and dash-cap coverage rules remain in use after edge alignment. Straight outline
edges of wider strokes also snap; curved source outlines retain antialiasing.

Native projects retain the setting. PSD `strokeStyleStrokeAdjust` is an independent
stroke-style property: it neither enables Align Edges on import nor changes when
Align Edges is toggled. Existing values survive stroke appearance edits. PSD/PSB
stores Align Edges in the layer's `sn2P` (Using Aligned Rendering) block: four
big-endian bytes, nonzero for enabled, as specified by
[Adobe's PSD format documentation](https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/).
An absent block defaults to false. Unchanged valid blocks stay byte-identical.
Fill/stroke splitting retains the layer setting for ordinary fills as well as
live lines, so layer effects and clipped content use the same aligned coverage.

Aligned previews rasterize in document pixels before the view transform. Large
previews use bounded work per frame and a temporary coarse fallback until native
coverage is ready. Cache identities include the setting and geometry provenance.
