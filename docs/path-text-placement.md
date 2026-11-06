# Path-text placement

Path Selection and Direct Selection can adjust the active type layer's text range independently of its curve. Drag the beginning or ending marker along the curve to change that boundary; drag the text to move its paragraph anchor and both boundaries while they fit. At an open endpoint the range shortens so a range already reaching the endpoint remains movable. Crossing the curve flips the side and direction. Escape discards the current gesture; release commits one undo step. Character-panel Baseline Shift moves text across the curve without changing direction.

The on-canvas preview uses the native text renderer. Its cached document leaves document history unchanged while dragging, and repaint damage is limited to the old and new text areas. The ending marker shows a plus when the path range hides drawable glyphs.

## Data and automation

`type.setPathPlacement` updates an existing path-text layer. Parameters are `layer` (optional, defaults to the active layer), `start` (optional pixels), `end` (optional pixels, or `null` for legacy placement), and `flip` (optional boolean). Omitted values are preserved, so a request can update only one property. `end: null` clears the end limit and restores legacy placement. An explicit end must be greater than or equal to start; invalid ordering is rejected. Position and full-layer locks prevent placement changes.

Offsets are measured in the text layer's local space, along its current direction of travel. An explicit end is unwrapped: equal start/end means no capacity, and an end one path length after the start means a full closed lap. The renderer limits closed paths to one lap. Native `.pcraft` files preserve the range; older files without an end retain legacy placement until an actual edit.

## Photoshop reference and remaining verification

Adobe documents the Path/Direct Selection gestures and directional cursors in [Flip or move text along a path](https://helpx.adobe.com/photoshop/desktop/text-typography/text-on-paths-shapes/flip-or-move-text-along-a-path.html). The [Photoshop reference](https://helpx.adobe.com/pdf/cs6/photoshop_reference.pdf) distinguishes flipping from baseline shift. These documents guide the implementation; direct visual comparison against Photoshop is still required before claiming exact parity.

Editable PSD end-limit mapping is unresolved. PSD export warns that the end limit is lost; alignment, placement, and overflow can therefore change when reopened. The existing path/start/flip export mapping is retained. Native persistence tests do not establish Photoshop interchange fidelity.

## Manual comparison protocol

Use the same font, size, content, baseline shift, and alignment in PhotoCraft and Photoshop.

1. Create an open curve and a circle, then add path text. Test left, center, and right alignment.
2. Select each path-selection tool in turn. Check beginning/end markers, directional cursors, and text-body hover.
3. Drag each marker along the path. Shorten the interval until text overflows; check the ending marker and hidden glyphs.
4. Drag the text along the curve and across it. Check the occupied path interval, direction, and baseline side before and after a flip.
5. Cross a closed-path seam in both directions. Check for jumps and duplicate text. Repeat after zoom, view rotation/mirroring, and a skewed layer transform.
6. Repeat the first drag on an older native file with centered/right-aligned text and no explicit end. Check for an unexpected placement jump.
7. Cancel with Escape, undo/redo completed drags, switch documents during a gesture, and try a position-locked layer.
8. Save/reopen the native file. Export to PSD and verify the warning; end-limit parity remains unsupported.

Automated regression tests cover the model, bounded layout, caret projection, marker/body gestures, flipping, history, locks, transformed marker targets, preview caching, repaint damage, and native persistence. Remote CI results and this manual comparison must be assessed separately.
