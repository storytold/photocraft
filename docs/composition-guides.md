# Composition guides

View → Composition Guides, beside Extras and Show, adds full-canvas composition aids independently of the selected
painting, moving or inspection tool. This is a scoped implementation of [#5193](https://github.com/storytold/photocraft/issues/5193): Rule of Thirds,
Golden Ratio and Golden Spiral. Additional patterns such as S-curves, L-shapes and pyramids
remain follow-up work.

Guides start off, with Rule of Thirds selected. Show Composition Guides toggles visibility;
choosing a pattern also shows it and enables View → Extras. Extras hides guides temporarily
without changing their enabled setting. Rotate Spiral 90° and the horizontal/vertical flip
commands provide eight spiral orientations; they are available when Golden Spiral is selected.
Orientation commands preserve Show and Extras visibility. The chosen orientation is retained when switching patterns. Like Crop's spiral, the normalized
golden-rectangle curve stretches to the canvas proportions.

Guides follow the document canvas through zoom, pan, rotation, horizontal view flipping,
pixel-aspect correction, display scaling and canvas/window resizing. For size-changing filter
previews, guides use the document dimensions and update when the edit is applied. Artboard documents show
guides within each visible top-level artboard, independently of the selected layer. Pattern
Preview keeps one set on the original canvas. Guides use contrasting outline strokes and
never receive pointer input, change pixels, enter document history or appear in exports.

While Crop or Perspective Crop is selected, full-canvas composition guides are suspended,
including when Crop's own overlay is Auto or Never, or its default frame is still untouched.
Crop retains its current Always default: an idle untouched frame has no guide, while its turning
and straightening previews keep their existing guides. A retained Crop frame continues to suspend
full-canvas guides during temporary pen erasure or a tool switch mid-drag, while it still owns
Enter/Escape. Clearing that frame by committing, cancelling or switching documents restores guides
when another tool is selected. A dormant Perspective Crop quad after an explicit tool switch does
not own keys or suspend guides. Crop's
pattern, visibility, orientation, O and Shift+O behavior are unchanged; its overlay exclusively
uses the crop frame. This prevents two independently aligned overlays from appearing together.

The choice is an application view preference shared across documents and document windows,
remembered in the existing `viewExtras.compositionGuides` preference bucket. Older preferences
start off; malformed fields fall back separately to disabled, thirds and orientation 0.

## Automation

`ui.inspect` reports `view.composition_guides` as `{enabled, kind, orientation}`.
`ui.set {"compositionGuides":{"enabled":true,"kind":"goldenSpiral","orientation":5}}`
merges the given fields. `kind` is `thirds`, `goldenRatio` or `goldenSpiral`; `enabled` is boolean;
`orientation` is an integer 0–7: low two bits are clockwise quarter turns in normalized canvas
coordinates, bit 2 reflects horizontally after the turn. Orientation affects only the spiral.
Non-square canvases stretch these normalized coordinates to their bounds, as Crop does.
Unknown fields, wrong types and out-of-range values reject the entire `ui.set` request before
any mutation. An explicit `enabled: true` also reveals Extras; empty and orientation-only
patches preserve its visibility. Valid settings can be chosen without a document; nothing is drawn until one opens.

The menu/command-palette/`ui.menu.invoke`/`engine.execute` command IDs share the prefix
`view.compositionGuides.`: `show`, `thirds`, `goldenRatio`, `goldenSpiral`, `rotate`,
`flipHorizontal`, `flipVertical`. `show` accepts optional `{on: boolean}` (omitting it toggles);
the others accept an empty object. Null parameters mean an empty object. These are shell view commands without default shortcuts.
