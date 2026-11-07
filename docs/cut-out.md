# Cut Out

Cut Out removes a vertical or horizontal band from the whole image and joins the two sides, the
way screenshot tools such as Snagit let you drop the uninteresting middle of a long capture. It
has no Photoshop counterpart, so it has no menu item: it is the Cut Out tool (C, in the Crop
group) and the `image.cutOut` command.

## Tool

Pick the direction in the options bar (Vertical band removes columns, Horizontal band removes
rows), then drag across the part to remove. The drag shades the band across the whole canvas and
shows its width; releasing removes it. The Edge setting marks the join: Straight leaves nothing,
Sawtooth and Wave draw a seam in the chosen colour on a new top shape layer, "Cut Out Edge", which
can be restyled or deleted like any shape.

## What happens to each layer

- Every pixel surface loses the band and whatever lay past it moves over: pixel layers, cached
  type/shape/smart-object pixels, layer masks, fill caches, smart-filter masks, alpha channels,
  Quick Mask and the selection.
- Type, shape and smart-object layers (and vector masks) wholly past the band move with it and
  stay editable. Ones the band crosses are rasterized first, since live geometry cannot be split.
  Ones wholly inside the band are deleted.
- Guides past the band move with it; guides inside it move to the seam.
- The canvas shrinks by the band. The whole operation, rasterizing and seam included, is one
  history step.

Not handled yet: notes, count and ruler marks, slices, saved paths and artboards keep their
positions, and there is no Snagit-style "fade" edge.

## Automation

```json
{"axis": "vertical", "from": 300, "to": 610, "edge": "sawtooth", "edgeColor": "#ffffff"}
```

`from`/`to` are clamped to the canvas and may be given in either order; the band must be at least
1 px and leave something on one side. The result reports the new `width`/`height`, `removed`
pixels, `rasterized` and `deleted` layer ids and the `edgeLayer` id (or `null`). Bad parameters
are errors and change nothing.
