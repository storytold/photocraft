# Actions files (`.atn`)

PhotoCraft loads and saves action sets in the reference application's `.atn` format: Load Actions and
Save Actions in the Actions panel menu, `actions.load` / `actions.save` for agents, and
`photocraft batch --actions set.atn` on the command line. Reading and writing the file lives in
`photocraft-psd` (`crates/psd/src/atn.rs`, version 16 sets, exact round trip); the mapping between the
application's events and PhotoCraft commands lives in `photocraft-engine`
(`crates/engine/src/cmd/atn_cmds/`, table `EVENTS` in `events.rs`).

A step whose event is in the table below loads as the PhotoCraft command it names, plays, and saves back
as the same event. Any other step loads as a kept step (`actions.photoshopStep`), and Save Actions writes
it back unchanged. Playback stops at an enabled kept step with its name, so an action never silently skips
work. Kept steps with no effect on the document (tool selections and the like) are imported switched off,
so playback skips them.

This page has two parts: the events PhotoCraft maps today, and a measured reference for the events it
does not map yet, with everything needed to implement them without the reference application.

## Supported events (127)

| Code | String ID | Label | PhotoCraft command(s) |
|---|---|---|---|
| `AddT` | addTo | Add to Selection | `select.rect` (add rectangle or ellipse) |
| `SbtF` | subtractFrom | Subtract from Selection | `select.rect` (subtract rectangle) |
| `IntW` | intersectWith | Intersect with Selection | `select.rect` (intersect rectangle) |
| `PstI` | pasteInto | Paste Into | `edit.pasteSpecial.pasteInto` |
| `PstO` | pasteOutside | Paste Outside | `edit.pasteSpecial.pasteOutside` |
| `Fade` | fade | Fade | `edit.fade` |
| (string ID only) | linkSelectedLayers | Link Layers | `layer.linkLayers` |
| `Dstr` | distribute | Distribute | `layer.distribute.leftEdges`, `layer.distribute.horizontalCenters`, `layer.distribute.rightEdges`, `layer.distribute.topEdges`, `layer.distribute.verticalCenters`, `layer.distribute.bottomEdges` |
| `RmvL` | removeLayerMask | Remove Layer Mask (older files) | `layer.layerMask.delete` |
| `DfnP` | definePattern | Define Pattern | `edit.definePattern` |
| `MrgL` | mergeLayers | Merge Layers (older files) | `layer.mergeLayers` |
| `AdNs` | addNoise | Add Noise | `filter.noise.addNoise` |
| `Algn` | align | Align | `layer.align.leftEdges`, `layer.align.horizontalCenters`, `layer.align.rightEdges`, `layer.align.topEdges`, `layer.align.verticalCenters`, `layer.align.bottomEdges` |
| `AppI` | applyImageEvent | Apply Image | `image.applyImage` |
| (string ID only) | applyComp | Apply Layer Comp | `layerComp.apply` |
| (string ID only) | applyLocking | Apply Locking | `layer.lockLayers` |
| (string ID only) | assignProfile | Assign Profile | `edit.assignProfile` |
| (string ID only) | autoColor | Auto Color | `image.autoColor` |
| (string ID only) | blackAndWhite | Black & White | `image.adjustments.blackWhite` |
| `Blr ` | blur | Blur | `filter.blur.blur` |
| `BlrM` | blurMore | Blur More | `filter.blur.blurMore` |
| `Brdr` | border | Border | `select.modify.border` |
| (string ID only) | boxblur | Box Blur | `filter.blur.boxBlur` |
| `BrgC` | brightnessEvent | Brightness/Contrast | `image.adjustments.brightnessContrast` |
| `CnvS` | canvasSize | Canvas Size | `image.canvasSize` |
| `ChnM` | channelMixer | Channel Mixer | `image.adjustments.channelMixer` |
| (string ID only) | clearEvent | Clear | `select.editInQuickMaskMode`, `history.clear` |
| `Cls ` | close | Close | `file.close` |
| (string ID only) | collapseAllGroupsEvent | Collapse All Groups | `layer.setExpanded` |
| `ClrR` | colorRange | Color Range | `select.colorRange` |
| `ClrB` | colorBalance | Color Balance | `image.adjustments.colorBalance` |
| `Cntc` | contract | Contract | `select.modify.contract` |
| `CnvM` | convertMode | Convert Mode | `image.mode.rgb`, `image.mode.grayscale`, `image.mode.cmyk`, `image.mode.lab`, `image.mode.multichannel`, `image.mode.bits8`, `image.mode.bits16`, `image.mode.bits32`, `image.mode.bitmap` |
| (string ID only) | convertToProfile | Convert to Profile | `edit.convertToProfile` |
| (string ID only) | newPlacedLayer | Convert to Smart Object | `layer.smartObjects.convertToSmartObject` |
| `copy` | copyEvent | Copy | `edit.copy` |
| (string ID only) | copyEffects | Copy Layer Style | `layer.layerStyle.copyLayerStyle` |
| `CpyM` | copyMerged | Copy Merged | `edit.copyMerged` |
| `GrpL` | groupEvent | Create Clipping Mask | `layer.createClippingMask` |
| `Crop` | crop | Crop | `image.crop` |
| `Crst` | crystallize | Crystallize | `filter.pixelate.crystallize` |
| `Crvs` | curves | Curves | `image.adjustments.curves` |
| `cut ` | cut | Cut | `edit.cut` |
| `Dfrg` | defringe | Defringe | `layer.matting.defringe` |
| `Dlt ` | delete | Delete | `layer.delete`, `edit.clear`, `layer.layerMask.delete`, `layer.smartFilter.deleteFilterMask` |
| `Dstt` | desaturate | Desaturate | `image.adjustments.desaturate` |
| (string ID only) | selectNoLayers | Deselect Layers | `select.deselectLayers` |
| `Dspc` | despeckle | Despeckle | `filter.noise.despeckle` |
| `Dfs ` | diffuse | Diffuse | `filter.stylize.diffuse` |
| `Dplc` | duplicate | Duplicate | `layer.duplicate`, `image.duplicate` |
| `DstS` | dustAndScratches | Dust & Scratches | `filter.noise.dustAndScratches` |
| (string ID only) | placedLayerEditContents | Edit Contents | `layer.smartObjects.editContents` |
| `Embs` | emboss | Emboss | `filter.stylize.emboss` |
| `Eqlz` | equalize | Equalize | `image.adjustments.equalize` |
| `Exch` | exchange | Exchange | `tools.swapColors` |
| `Expn` | expand | Expand | `select.modify.expand` |
| `Expr` | export | Export | `file.export.saveForWebLegacy` |
| `Exps` | exposure | Exposure | `image.adjustments.exposure` |
| `Fthr` | feather | Feather | `select.modify.feather` |
| `Fl  ` | fill | Fill | `edit.fill`, `paint.bucket` |
| `GEfc` | filterGallery | Filter Gallery | `filter.gallery.spatter` |
| `FltI` | flattenImage | Flatten Image | `layer.flattenImage` |
| `Flip` | flip | Flip | `image.imageRotation.flipCanvasHorizontal`, `image.imageRotation.flipCanvasVertical`, `edit.transform.flipHorizontal`, `edit.transform.flipVertical` |
| `GsnB` | gaussianBlur | Gaussian Blur | `filter.blur.gaussianBlur` |
| `GdMp` | gradientMapEvent | Gradient Map | `image.adjustments.gradientMap` |
| `Grow` | grow | Grow | `select.grow` |
| `Hd  ` | hide | Hide | `layer.hideLayers` |
| `HghP` | highPass | High Pass | `filter.other.highPass` |
| `HStr` | hueSaturation | Hue/Saturation | `image.adjustments.hueSaturation` |
| (string ID only) | conditional | If | `actions.conditional` |
| `ImgS` | imageSize | Image Size | `image.imageSize` |
| `Invs` | inverse | Inverse | `select.inverse` |
| `Invr` | invert | Invert | `image.adjustments.invert` |
| `CpTL` | copyToLayer | Layer Via Copy | `layer.new.layerViaCopy` |
| `CtTL` | cutToLayer | Layer Via Cut | `layer.new.layerViaCut` |
| `Lvls` | levels | Levels | `image.adjustments.levels`, `image.autoTone`, `image.autoContrast` |
| `Mk  ` | make | Make | `layer.new.layer`, `layer.new.group`, `layer.groupLayers`, `layer.duplicate`, `layer.new.layerViaCopy`, `layer.new.layerViaCut`, `layer.newFillLayer.solidColor`, `layer.newFillLayer.pattern`, `layer.newAdjustmentLayer.levels`, `layer.newAdjustmentLayer.curves`, `layer.newAdjustmentLayer.hueSaturation`, `layer.newAdjustmentLayer.brightnessContrast`, `layer.newAdjustmentLayer.threshold`, `layer.newAdjustmentLayer.posterize`, `layer.newAdjustmentLayer.invert`, `layer.newAdjustmentLayer.colorBalance`, `layer.newAdjustmentLayer.vibrance`, `layer.newAdjustmentLayer.exposure`, `layer.newAdjustmentLayer.photoFilter`, `layer.newAdjustmentLayer.blackWhite`, `layer.newAdjustmentLayer.channelMixer`, `layer.newAdjustmentLayer.selectiveColor`, `layer.newAdjustmentLayer.gradientMap`, `layer.layerMask.revealAll`, `layer.layerMask.hideAll`, `layer.layerMask.revealSelection`, `layer.layerMask.hideSelection`, `file.new`, `file.newFromClipboard`, `history.newSnapshot` |
| `Mxm ` | maximum | Maximum | `filter.other.maximum` |
| `Mdn ` | median | Median | `filter.noise.median` |
| `Mrg2` | mergeLayersNew | Merge Layers | `layer.mergeLayers` |
| `MrgV` | mergeVisible | Merge Visible | `layer.mergeVisible`, `layer.stampVisible` |
| `Mnm ` | minimum | Minimum | `filter.other.minimum` |
| `MtnB` | motionBlur | Motion Blur | `filter.blur.motionBlur` |
| `move` | move | Move | `layer.translate`, `layer.arrange.bringToFront`, `layer.arrange.sendToBack`, `layer.arrange.bringForward`, `layer.arrange.sendBackward` |
| `Ofst` | offset | Offset | `filter.other.offset` |
| `Opn ` | open | Open | `file.openAs` |
| `past` | paste | Paste | `edit.paste`, `edit.pasteSpecial.pasteInPlace` |
| (string ID only) | pasteEffects | Paste Layer Style | `layer.layerStyle.pasteLayerStyle` |
| (string ID only) | photoFilter | Photo Filter | `image.adjustments.photoFilter` |
| `Plc ` | placeEvent | Place | `file.placeEmbedded`, `file.placeLinked` |
| `Ply ` | play | Play | `actions.play` |
| `Pstr` | posterize | Posterize | `image.adjustments.posterize` |
| `Prge` | purge | Purge | `edit.purge.all`, `edit.purge.clipboard`, `edit.purge.histories`, `edit.purge.undo` |
| `RdlB` | radialBlur | Radial Blur | `filter.blur.radialBlur` |
| (string ID only) | rasterizeLayer | Rasterize | `layer.rasterize.layer` |
| `Ungr` | ungroup | Release Clipping Mask | `layer.releaseClippingMask` |
| `Rset` | reset | Reset | `tools.defaultColors` |
| `RvlA` | revealAll | Reveal All | `image.revealAll` |
| `Rvrt` | revert | Revert | `file.revert` |
| `Rple` | ripple | Ripple | `filter.distort.ripple` |
| `Rtte` | rotateEventEnum | Rotate | `image.imageRotation.90cw`, `image.imageRotation.90ccw`, `image.imageRotation.180`, `image.rotation.arbitrary`, `edit.transform.rotate90Cw`, `edit.transform.rotate90Ccw`, `edit.transform.rotate180` |
| `save` | save | Save | `file.save`, `file.saveAs`, `file.saveACopy` |
| `slct` | select | Select | `layer.select`, `layer.selectAbove`, `layer.selectBelow`, `layer.addAboveToSelection`, `layer.addBelowToSelection`, `layer.selectTop`, `layer.selectBottom`, `channel.target`, `edit.undo`, `edit.redo`, `view.fitOnScreen`, `view.actualPixels`, `view.zoomIn`, `view.zoomOut`, `document.activate` |
| (string ID only) | selectAllLayers | Select All Layers | `select.allLayers` |
| (string ID only) | refineSelectionEdge | Select and Mask | `select.refineEdge` |
| (string ID only) | smartBrushWorkspace | Select and Mask | `select.refineEdge` |
| (string ID only) | autoCutout | Select Subject | `select.subject` |
| `SlcC` | selectiveColor | Selective Color | `image.adjustments.selectiveColor` |
| `setd` | set | Set | `select.all`, `select.deselect`, `select.reselect`, `select.loadSelection`, `select.rect`, `select.lasso`, `select.editInQuickMaskMode`, `tools.setColors`, `layer.setProps`, `layer.renameLayer`, `layer.setLabelColor`, `layer.setAdjustment`, `layer.fill.pattern.set`, `layer.new.layerFromBackground`, `layer.layerStyle.blendingOptions`, `layer.smartFilter.blendingOptions` |
| (string ID only) | adaptCorrect | Shadows/Highlights | `image.adjustments.shadowsHighlights` |
| `Shrp` | sharpen | Sharpen | `filter.sharpen.sharpen` |
| `ShrE` | sharpenEdges | Sharpen Edges | `filter.sharpen.sharpenEdges` |
| `ShrM` | sharpenMore | Sharpen More | `filter.sharpen.sharpenMore` |
| `Shw ` | show | Show | `layer.showLayers` |
| `Smlr` | similar | Similar | `select.similar` |
| `SmrB` | smartBlur | Smart Blur | `filter.blur.smartBlur` |
| `Smth` | smoothness | Smooth | `select.modify.smooth` |
| `Stop` | stop | Stop | `actions.stopMessage` |
| `Strk` | stroke | Stroke | `edit.stroke` |
| (string ID only) | surfaceBlur | Surface Blur | `filter.blur.surfaceBlur` |
| `Thrs` | threshold | Threshold | `image.adjustments.threshold` |
| `Trnf` | transform | Transform | `edit.transform`, `select.transformSelection` |
| (string ID only) | trim | Trim | `image.trim` |
| (string ID only) | ungroupLayersEvent | Ungroup Layers | `layer.ungroupLayers` |
| `UnsM` | unsharpMask | Unsharp Mask | `filter.sharpen.unsharpMask` |
| (string ID only) | updatePlacedLayer | Update Smart Objects | `layer.smartObjects.updateModifiedContent` |
| `Vbrn` | vibrance | Vibrance | `image.adjustments.vibrance` |

### Group F: Selection combinations, Paste Into/Outside, Fade, Link, Distribute, Define Pattern and older merge/mask events

Steps recorded in an ordinary editing session. The values below were measured in a document set up for each step (a selection, the clipboard, linked or several selected layers). Align, Copy/Paste Layer Style and Rename are already mapped under their current events and are not repeated here.

#### Add to Selection (rectangle)

- Event: `AddT` / `addTo`
- PhotoCraft command: `select.rect {"mode":"add"}`
- Shift-drag with the Rectangular Marquee.
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Chnl' channel prop 'fsel' selection |  |
| `T   ` | to | object |  | obj 'Rctn' rectangle |  |

Nested values as recorded (example values):
```
    'Top ' top : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Left' left : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Btom' bottom : UNITDOUBLE = 90 '#Pxl' pixelsUnit
    'Rght' right : UNITDOUBLE = 90 '#Pxl' pixelsUnit
```

#### Add to Selection (ellipse)

- Event: `AddT` / `addTo`
- PhotoCraft command: `select.rect {"mode":"add","ellipse":true}`
- Same event with an `Elps` target and anti-aliasing.
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Chnl' channel prop 'fsel' selection |  |
| `T   ` | to | object |  | obj 'Elps' ellipse |  |
| `AntA` | antiAlias | boolean |  | true |  |

Nested values as recorded (example values):
```
    'Top ' top : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Left' left : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Btom' bottom : UNITDOUBLE = 90 '#Pxl' pixelsUnit
    'Rght' right : UNITDOUBLE = 90 '#Pxl' pixelsUnit
```

#### Subtract from Selection

- Event: `SbtF` / `subtractFrom`
- PhotoCraft command: `select.rect {"mode":"subtract"}`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Chnl' channel prop 'fsel' selection |  |
| `T   ` | to | object |  | obj 'Rctn' rectangle |  |

Nested values as recorded (example values):
```
    'Top ' top : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Left' left : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Btom' bottom : UNITDOUBLE = 60 '#Pxl' pixelsUnit
    'Rght' right : UNITDOUBLE = 60 '#Pxl' pixelsUnit
```

#### Intersect with Selection

- Event: `IntW` / `intersectWith`
- PhotoCraft command: `select.rect {"mode":"intersect"}`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Chnl' channel prop 'fsel' selection |  |
| `T   ` | to | object |  | obj 'Rctn' rectangle |  |

Nested values as recorded (example values):
```
    'Top ' top : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Left' left : UNITDOUBLE = 30 '#Pxl' pixelsUnit
    'Btom' bottom : UNITDOUBLE = 120 '#Pxl' pixelsUnit
    'Rght' right : UNITDOUBLE = 120 '#Pxl' pixelsUnit
```

#### Paste Into

- Event: `PstI` / `pasteInto`
- PhotoCraft command: `edit.pasteSpecial.pasteInto`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `AntA` | antiAlias | enumerated |  | `Anno` antiAliasNone | not measured yet |
| `As  ` | as | class |  | class 'Pxel' pixel |  |

#### Paste Outside

- Event: `PstO` / `pasteOutside`
- PhotoCraft command: `edit.pasteSpecial.pasteOutside`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `AntA` | antiAlias | enumerated |  | `Anno` antiAliasNone | not measured yet |
| `As  ` | as | class |  | class 'Pxel' pixel |  |

#### Fade

- Event: `Fade` / `fade`
- PhotoCraft command: `edit.fade`
- Recorded right after the step it fades.
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `Opct` | opacity | unitdouble | `#Prc` | 50 | 0 .. 100 (out-of-range values are clamped) |
| `Md  ` | mode | enumerated |  | `Mltp` multiply | not measured yet |

#### Link Layers

- Event: `` / `linkSelectedLayers`
- PhotoCraft command: `layer.linkLayers`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Lyr ' layer enum 'Ordn' ordinal.'Trgt' targetEnum |  |

#### Distribute

- Event: `Dstr` / `distribute`
- PhotoCraft command: `layer.distribute.leftEdges (one command per edge)`
- `Usng` selects the edge or centre; one enum value per Distribute menu item.
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `null` | null | reference |  | ref 'Lyr ' layer enum 'Ordn' ordinal.'Trgt' targetEnum |  |
| `Usng` | using | enumerated |  | `AdLf` ADSLefts | not measured yet |

#### Remove Layer Mask (older files)

- Event: `RmvL` / `removeLayerMask`
- PhotoCraft command: `layer.layerMask.delete`
- The reference application asks "Apply mask before deleting?" when this step plays without the apply choice; see the table for the key it records.
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

No parameters: the step carries an empty descriptor.

#### Define Pattern

- Event: `DfnP` / `definePattern`
- PhotoCraft command: `edit.definePattern`

No parameters: the step carries an empty descriptor.

#### Merge Layers (older files)

- Event: `MrgL` / `mergeLayers`
- PhotoCraft command: `layer.mergeLayers`

No parameters: the step carries an empty descriptor.

The prepared-document values in this group are required inputs, rather than factory defaults. Unknown settings stay kept steps; the descriptor name and class (the reference application records the localized step name and the event code there) do not matter. Paste uses the current clipboard and selection; Fade uses the preceding edit. Link Layers uses the existing selected-layer toggle. Native exports retain their current Delete Mask and Merge Layers events; imported older events retain their original IDs.


## Recognized steps kept disabled

These recognized events have no playable command. They load as kept steps switched
off, and Save Actions preserves their descriptors. They are excluded from the
supported-event count and the measured unmapped reference.

| String ID | Label |
|---|---|
| animationFramesFromLayers | Make Frames from Layers |

## Event reference: events not mapped yet

Each entry below is an event the reference application records that PhotoCraft does not map yet: its
4-character code and string ID, the PhotoCraft command it should map to, and every key the step carries,
in the order it is recorded, with type, unit, factory default and allowed values. When an event is
implemented, move it to the table above in the same change.

### How these values were measured

- **Keys, order, types, units, defaults:** each event was run in the reference application without a
  dialog on an RGB 8-bit document (with a selection, the clipboard or several layers where the step
  needs them), and the descriptor it returned was read back. That descriptor is what an action records.
  Settings were reset first, so the defaults are factory defaults. Steps that need a prepared document
  (a selection, the clipboard, several layers) were run with example values; their tables say so and show
  those values instead of defaults.
- **Ranges:** every number was set far above and below its range. The application clamps instead of
  refusing, so the values it returned are the exact limits. "Out-of-range values are clamped" in a table
  means exactly that; implement the same clamping on import.
- **Enum values:** every known enum value was tried; only values the application accepted and returned
  unchanged are listed. A value outside the list should keep the step as a kept step.
- **Names:** keys are stored by their 4-character code. The string ID column is a label; where one code
  has two names in the application, the one that fits the setting is shown.
- **Filter Gallery effects:** the "PhotoCraft today" column compares each value with PhotoCraft's own
  effect parameters (`crates/algo/src/artistic.rs`). All ranges and defaults already match; 26 parameters
  have a different name in PhotoCraft, given in that column.

### What an implementation needs to do

1. **Import:** `actions.load` maps the step to the command and params listed, using every key in the
   table; missing keys take the default from the table.
2. **Out-of-range values:** clamped to the allowed range, as the reference application does. Enum values
   outside the allowed list keep the step as a kept step, never a silent default.
3. **Export:** `actions.save` writes the same event with all keys in the order of the table, defaults
   included, with the units shown.
4. **Round trip:** load, then save, gives identical descriptors, for a step with defaults and for one
   with every value changed.
5. **Tests:** add the event to `every_mapped_command_exports_and_imports_back` and test the params that
   reach the command.

### Group A: The 47 Filter Gallery effects

Each Filter Gallery effect is recorded as its own event (one per effect), carrying a `GEfk` key with the effect id plus the effect's settings. Today they all load as kept steps that stop at playback. PhotoCraft already has every effect (`crates/algo/src/artistic.rs`) behind `filter.filterGallery`; this issue only maps them. Where our own range or default differs from the measured one, fix ours to match (marked in each table).

#### Accented Edges

- Event: `AccE` / `accentedEdges`
- PhotoCraft command: `filter.filterGallery effect "accentedEdges"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `EdgW` | edgeWidth | integer |  | 2 | 1 .. 14 (out-of-range values are clamped) | same |
| `EdgB` | edgeBrightness | integer |  | 38 | 0 .. 50 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 5 | 1 .. 15 (out-of-range values are clamped) | same |

#### Angled Strokes

- Event: `AngS` / `angledStrokes`
- PhotoCraft command: `filter.filterGallery effect "angledStrokes"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `DrcB` | directionBalance | integer |  | 50 | 0 .. 100 (out-of-range values are clamped) | same |
| `StrL` | strokeLength | integer |  | 15 | 3 .. 50 (out-of-range values are clamped) | same |
| `Shrp` | sharpness | integer |  | 3 | 0 .. 10 (out-of-range values are clamped) | maps to our `sharpness` (same range and default) |

#### Bas Relief

- Event: `BsRl` / `basRelief`
- PhotoCraft command: `filter.filterGallery effect "basRelief"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Dtl ` | detail | integer |  | 13 | 1 .. 15 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 3 | 1 .. 15 (out-of-range values are clamped) | same |
| `LghD` | lightDirection | enumerated |  | `LDBt` lightDirBottom | `LDBL` lightDirBottomLeft, `LDBR` lightDirBottomRight, `LDBt` lightDirBottom, `LDLf` lightDirLeft, `LDRg` lightDirRight, `LDTL` lightDirTopLeft, `LDTR` lightDirTopRight, `LDTp` lightDirTop |  |

#### Chalk Charcoal

- Event: `ChlC` / `chalkCharcoal`
- PhotoCraft command: `filter.filterGallery effect "chalkCharcoal"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ChrA` | charcoalArea | integer |  | 6 | 0 .. 20 (out-of-range values are clamped) | same |
| `ChlA` | chalkArea | integer |  | 6 | 0 .. 20 (out-of-range values are clamped) | same |
| `StrP` | strokePressure | integer |  | 1 | 0 .. 5 (out-of-range values are clamped) | same |

#### Charcoal

- Event: `Chrc` / `charcoal`
- PhotoCraft command: `filter.filterGallery effect "charcoal"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ChAm` | charcoalAmount | integer |  | 1 | 1 .. 7 (out-of-range values are clamped) | maps to our `charcoalThickness` (same range and default) |
| `Dtl ` | detail | integer |  | 5 | 0 .. 5 (out-of-range values are clamped) | same |
| `LgDr` | lightDark | integer |  | 50 | 0 .. 100 (out-of-range values are clamped) | maps to our `lightDarkBalance` (same range and default) |

#### Chrome

- Event: `Chrm` / `chrome`
- PhotoCraft command: `filter.filterGallery effect "chrome"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Dtl ` | detail | integer |  | 4 | 0 .. 10 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 7 | 0 .. 10 (out-of-range values are clamped) | same |

#### Colored Pencil

- Event: `ClrP` / `coloredPencil`
- PhotoCraft command: `filter.filterGallery effect "coloredPencil"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Pncl` | pencilWidth | integer |  | 4 | 1 .. 24 (out-of-range values are clamped) | maps to our `pencilWidth` (same range and default) |
| `StrP` | strokePressure | integer |  | 8 | 0 .. 15 (out-of-range values are clamped) | same |
| `PprB` | paperBrightness | integer |  | 25 | 0 .. 50 (out-of-range values are clamped) | same |

#### Conte Crayon

- Event: `CntC` / `conteCrayon`
- PhotoCraft command: `filter.filterGallery effect "conteCrayon"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `FrgL` | foregroundLevel | integer |  | 11 | 1 .. 15 (out-of-range values are clamped) | same |
| `BckL` | backgroundLevel | integer |  | 7 | 1 .. 15 (out-of-range values are clamped) | maps to our `backgroundLevel` (same range and default) |
| `TxtT` | textureType | enumerated |  | `TxCa` texTypeCanvas | `TxBr` texTypeBrick, `TxBu` texTypeBurlap, `TxCa` texTypeCanvas, `TxSt` texTypeSandstone |  |
| `Scln` | scaling | integer |  | 100 | 50 .. 200 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 4 | 0 .. 50 (out-of-range values are clamped) | same |
| `LghD` | lightDirection | enumerated |  | `LDTp` lightDirTop | `LDBL` lightDirBottomLeft, `LDBR` lightDirBottomRight, `LDBt` lightDirBottom, `LDLf` lightDirLeft, `LDRg` lightDirRight, `LDTL` lightDirTopLeft, `LDTR` lightDirTopRight, `LDTp` lightDirTop |  |
| `InvT` | invertTexture | boolean |  | false |  |  |

#### Craquelure

- Event: `Crql` / `craquelure`
- PhotoCraft command: `filter.filterGallery effect "craquelure"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `CrcS` | crackSpacing | integer |  | 15 | 2 .. 100 (out-of-range values are clamped) | same |
| `CrcD` | crackDepth | integer |  | 6 | 0 .. 10 (out-of-range values are clamped) | same |
| `CrcB` | crackBrightness | integer |  | 9 | 0 .. 10 (out-of-range values are clamped) | same |

#### Crosshatch

- Event: `Crsh` / `crosshatch`
- PhotoCraft command: `filter.filterGallery effect "crosshatch"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 9 | 3 .. 50 (out-of-range values are clamped) | same |
| `Shrp` | sharpness | integer |  | 6 | 0 .. 20 (out-of-range values are clamped) | maps to our `sharpness` (same range and default) |
| `Strg` | strength | integer |  | 1 | 1 .. 3 (out-of-range values are clamped) | maps to our `strength` (same range and default) |

#### Cutout

- Event: `Ct  ` / `cutout`
- PhotoCraft command: `filter.filterGallery effect "cutout"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `NmbL` | numberOfLayers | integer |  | 4 | 2 .. 8 (out-of-range values are clamped) | maps to our `numberOfLevels` (same range and default) |
| `EdgS` | edgeSimplicity | integer |  | 4 | 0 .. 10 (out-of-range values are clamped) | same |
| `EdgF` | edgeFidelity | integer |  | 2 | 1 .. 3 (out-of-range values are clamped) | same |

#### Dark Strokes

- Event: `DrkS` / `darkStrokes`
- PhotoCraft command: `filter.filterGallery effect "darkStrokes"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Blnc` | balance | integer |  | 5 | 0 .. 10 (out-of-range values are clamped) | same |
| `BlcI` | blackIntensity | integer |  | 6 | 0 .. 10 (out-of-range values are clamped) | same |
| `WhtI` | whiteIntensity | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |

#### Diffuse Glow

- Event: `DfsG` / `diffuseGlow`
- PhotoCraft command: `filter.filterGallery effect "diffuseGlow"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Grns` | graininess | integer |  | 6 | 0 .. 10 (out-of-range values are clamped) | same |
| `GlwA` | glowAmount | integer |  | 10 | 0 .. 20 (out-of-range values are clamped) | same |
| `ClrA` | clearAmount | integer |  | 15 | 0 .. 20 (out-of-range values are clamped) | same |

#### Dry Brush

- Event: `DryB` / `dryBrush`
- PhotoCraft command: `filter.filterGallery effect "dryBrush"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `BrsS` | brushSize | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |
| `BrsD` | brushDetail | integer |  | 8 | 0 .. 10 (out-of-range values are clamped) | same |
| `Txtr` | texture | integer |  | 1 | 1 .. 3 (out-of-range values are clamped) | same |

#### Film Grain

- Event: `FlmG` / `filmGrain`
- PhotoCraft command: `filter.filterGallery effect "filmGrain"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Grn ` | grain | integer |  | 4 | 0 .. 20 (out-of-range values are clamped) | same |
| `HghA` | highlightArea | integer |  | 0 | 0 .. 20 (out-of-range values are clamped) | same |
| `Intn` | intensity | integer |  | 10 | 0 .. 10 (out-of-range values are clamped) | same |

#### Fresco

- Event: `Frsc` / `fresco`
- PhotoCraft command: `filter.filterGallery effect "fresco"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `BrsS` | brushSize | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |
| `BrsD` | brushDetail | integer |  | 8 | 0 .. 10 (out-of-range values are clamped) | same |
| `Txtr` | texture | integer |  | 1 | 1 .. 3 (out-of-range values are clamped) | same |

#### Glass

- Event: `Gls ` / `glass`
- PhotoCraft command: `filter.filterGallery effect "glass"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Dstr` | distortion | integer |  | 5 | 0 .. 20 (out-of-range values are clamped) | maps to our `distortion` (same range and default) |
| `Smth` | smoothness | integer |  | 3 | 1 .. 15 (out-of-range values are clamped) | same |
| `TxtT` | textureType | enumerated |  | `TxFr` texTypeFrosted | `TxBr` texTypeBrick, `TxBu` texTypeBurlap, `TxCa` texTypeCanvas, `TxSt` texTypeSandstone |  |
| `Scln` | scaling | integer |  | 100 | 50 .. 200 (out-of-range values are clamped) | same |
| `InvT` | invertTexture | boolean |  | false |  |  |

#### Glowing Edges

- Event: `GlwE` / `glowingEdges`
- PhotoCraft command: `filter.filterGallery effect "glowingEdges"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `EdgW` | edgeWidth | integer |  | 2 | 1 .. 14 (out-of-range values are clamped) | same |
| `EdgB` | edgeBrightness | integer |  | 6 | 0 .. 20 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 5 | 1 .. 15 (out-of-range values are clamped) | same |

#### Graphic Pen

- Event: `GraP` / `graphicPen`
- PhotoCraft command: `filter.filterGallery effect "graphicPen"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 15 | 1 .. 15 (out-of-range values are clamped) | same |
| `LgDr` | lightDark | integer |  | 50 | 0 .. 100 (out-of-range values are clamped) | maps to our `lightDarkBalance` (same range and default) |
| `SDir` | strokeDirection | enumerated |  | `SDRD` strokeDirRightDiag | `SDHz` strokeDirHorizontal, `SDLD` strokeDirLeftDiag, `SDRD` strokeDirRightDiag, `SDVt` strokeDirVertical |  |

#### Halftone Screen

- Event: `HlfS` / `halftoneScreen`
- PhotoCraft command: `filter.filterGallery effect "halftonePattern"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `HlSz` | halftoneSize | integer |  | 1 | 1 .. 12 (out-of-range values are clamped) | maps to our `size` (same range and default) |
| `Cntr` | contrast | integer |  | 5 | 0 .. 50 (out-of-range values are clamped) | maps to our `contrast` (same range and default) |
| `ScrT` | screenType | enumerated |  | `ScrD` screenDot | `ScrC` screenCircle, `ScrD` screenDot, `ScrL` screenLine |  |

#### Ink Outlines

- Event: `InkO` / `inkOutlines`
- PhotoCraft command: `filter.filterGallery effect "inkOutlines"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 4 | 1 .. 50 (out-of-range values are clamped) | same |
| `DrkI` | darkIntensity | integer |  | 20 | 0 .. 50 (out-of-range values are clamped) | same |
| `LghI` | lightIntensity | integer |  | 10 | 0 .. 50 (out-of-range values are clamped) | same |

#### Mosaic  Tiles

- Event: `MscT` (no string ID)
- PhotoCraft command: `filter.filterGallery effect "mosaicTiles"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `TlSz` | tileSize | integer |  | 12 | 2 .. 100 (out-of-range values are clamped) | same |
| `GrtW` | groutWidth | integer |  | 3 | 1 .. 15 (out-of-range values are clamped) | same |
| `LghG` | lightenGrout | integer |  | 9 | 0 .. 10 (out-of-range values are clamped) | same |

#### Neon Glow

- Event: `NGlw` / `neonGlow`
- PhotoCraft command: `filter.filterGallery effect "neonGlow"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Sz  ` | size | integer |  | 5 | -24 .. 24 (out-of-range values are clamped) | maps to our `glowSize` (same range and default) |
| `Brgh` | brightness | integer |  | 15 | 0 .. 50 (out-of-range values are clamped) | maps to our `glowBrightness` (same range and default) |
| `Clr ` | color | object |  | obj 'RGBC' RGBColor |  |  |

Nested values as recorded (defaults):
```
    'Rd  ' red : DOUBLETYPE = 0
    'Grn ' green : DOUBLETYPE = 0
    'Bl  ' blue : DOUBLETYPE = 255
```

#### Note Paper

- Event: `NtPr` / `notePaper`
- PhotoCraft command: `filter.filterGallery effect "notePaper"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ImgB` | imageBalance | integer |  | 25 | 0 .. 50 (out-of-range values are clamped) | same |
| `Grns` | graininess | integer |  | 10 | 0 .. 20 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 11 | 0 .. 25 (out-of-range values are clamped) | same |

#### Ocean Ripple

- Event: `OcnR` / `oceanRipple`
- PhotoCraft command: `filter.filterGallery effect "oceanRipple"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `RplS` | rippleSize | integer |  | 9 | 1 .. 15 (out-of-range values are clamped) | same |
| `RplM` | rippleMagnitude | integer |  | 9 | 0 .. 20 (out-of-range values are clamped) | same |

#### Paint Daubs

- Event: `PntD` / `paintDaubs`
- PhotoCraft command: `filter.filterGallery effect "paintDaubs"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Sz  ` | size | integer |  | 8 | 1 .. 50 (out-of-range values are clamped) | maps to our `brushSize` (same range and default) |
| `Shrp` | sharpness | integer |  | 7 | 0 .. 40 (out-of-range values are clamped) | maps to our `sharpness` (same range and default) |
| `BrsT` | brushType | enumerated |  | `BrSm` brushSimple | `BrDR` brushDarkRough, `BrSm` brushSimple, `BrSp` brushSparkle, `BrbW` brushWideBlurry, `BrsL` brushLightRough, `BrsW` brushWideSharp |  |

#### Palette Knife

- Event: `PltK` / `paletteKnife`
- PhotoCraft command: `filter.filterGallery effect "paletteKnife"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrS` | strokeSize | integer |  | 25 | 1 .. 50 (out-of-range values are clamped) | same |
| `StDt` | strokeDetail | integer |  | 3 | 1 .. 3 (out-of-range values are clamped) | same |
| `Sftn` | softness | integer |  | 0 | 0 .. 10 (out-of-range values are clamped) | same |

#### Patchwork

- Event: `Ptch` / `patchwork`
- PhotoCraft command: `filter.filterGallery effect "patchwork"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `SqrS` | squareSize | integer |  | 4 | 0 .. 10 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 8 | 0 .. 25 (out-of-range values are clamped) | same |

#### Photocopy

- Event: `Phtc` / `photocopy`
- PhotoCraft command: `filter.filterGallery effect "photocopy"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Dtl ` | detail | integer |  | 7 | 1 .. 24 (out-of-range values are clamped) | same |
| `Drkn` | darkness | integer |  | 8 | 1 .. 50 (out-of-range values are clamped) | maps to our `darkness` (same range and default) |

#### Plaster

- Event: `Plst` / `plaster`
- PhotoCraft command: `filter.filterGallery effect "plaster"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ImgB` | imageBalance | integer |  | 20 | 0 .. 50 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 2 | 1 .. 15 (out-of-range values are clamped) | same |
| `LghP` | lightPosition | enumerated |  | `LPTp` lightPosTop | `LPBL` lightPosBottomLeft, `LPBr` lightPosBottomRight, `LPBt` lightPosBottom, `LPLf` lightPosLeft, `LPRg` lightPosRight, `LPTL` lightPosTopLeft, `LPTR` lightPosTopRight, `LPTp` lightPosTop |  |

#### Plastic Wrap

- Event: `PlsW` / `plasticWrap`
- PhotoCraft command: `filter.filterGallery effect "plasticWrap"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `HghS` | highlightStrength | integer |  | 15 | 0 .. 20 (out-of-range values are clamped) | same |
| `Dtl ` | detail | integer |  | 9 | 1 .. 15 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 7 | 1 .. 15 (out-of-range values are clamped) | same |

#### Poster Edges

- Event: `PstE` / `posterEdges`
- PhotoCraft command: `filter.filterGallery effect "posterEdges"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `EdgT` | edgeThickness | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |
| `EdgI` | edgeIntensity | integer |  | 1 | 0 .. 10 (out-of-range values are clamped) | same |
| `Pstr` | posterization | integer |  | 2 | 0 .. 6 (out-of-range values are clamped) | same |

#### Reticulation

- Event: `Rtcl` / `reticulation`
- PhotoCraft command: `filter.filterGallery effect "reticulation"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Dnst` | density | integer |  | 12 | 0 .. 50 (out-of-range values are clamped) | same |
| `BlcL` | blackLevel | integer |  | 40 | 0 .. 50 (out-of-range values are clamped) | maps to our `foregroundLevel` (same range and default) |
| `WhtL` | whiteLevel | integer |  | 5 | 0 .. 50 (out-of-range values are clamped) | maps to our `backgroundLevel` (same range and default) |

#### Rough Pastels

- Event: `RghP` / `roughPastels`
- PhotoCraft command: `filter.filterGallery effect "roughPastels"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 6 | 0 .. 40 (out-of-range values are clamped) | same |
| `StDt` | strokeDetail | integer |  | 4 | 1 .. 20 (out-of-range values are clamped) | same |
| `TxtT` | textureType | enumerated |  | `TxCa` texTypeCanvas | `TxBr` texTypeBrick, `TxBu` texTypeBurlap, `TxCa` texTypeCanvas, `TxSt` texTypeSandstone |  |
| `Scln` | scaling | integer |  | 100 | 50 .. 200 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 20 | 0 .. 50 (out-of-range values are clamped) | same |
| `LghD` | lightDirection | enumerated |  | `LDBt` lightDirBottom | `LDBL` lightDirBottomLeft, `LDBR` lightDirBottomRight, `LDBt` lightDirBottom, `LDLf` lightDirLeft, `LDRg` lightDirRight, `LDTL` lightDirTopLeft, `LDTR` lightDirTopRight, `LDTp` lightDirTop |  |
| `InvT` | invertTexture | boolean |  | false |  |  |

#### Smudge Stick

- Event: `SmdS` / `smudgeStick`
- PhotoCraft command: `filter.filterGallery effect "smudgeStick"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |
| `HghA` | highlightArea | integer |  | 0 | 0 .. 20 (out-of-range values are clamped) | same |
| `Intn` | intensity | integer |  | 10 | 0 .. 10 (out-of-range values are clamped) | same |

#### Spatter

- Event: `Spt ` / `spatter`
- PhotoCraft command: `filter.filterGallery effect "spatter"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `SprR` | sprayRadius | integer |  | 10 | 0 .. 25 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 5 | 1 .. 15 (out-of-range values are clamped) | same |

#### Sponge

- Event: `Spng` / `sponge`
- PhotoCraft command: `filter.filterGallery effect "sponge"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `BrsS` | brushSize | integer |  | 2 | 0 .. 10 (out-of-range values are clamped) | same |
| `Dfnt` | definition | integer |  | 12 | 0 .. 25 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 5 | 1 .. 15 (out-of-range values are clamped) | same |

#### Sprayed Strokes

- Event: `SprS` / `sprayedStrokes`
- PhotoCraft command: `filter.filterGallery effect "sprayedStrokes"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrL` | strokeLength | integer |  | 12 | 0 .. 20 (out-of-range values are clamped) | same |
| `SprR` | sprayRadius | integer |  | 7 | 0 .. 25 (out-of-range values are clamped) | same |
| `SDir` | strokeDirection | enumerated |  | `SDRD` strokeDirRightDiag | `SDHz` strokeDirHorizontal, `SDLD` strokeDirLeftDiag, `SDRD` strokeDirRightDiag, `SDVt` strokeDirVertical |  |

#### Stained Glass

- Event: `StnG` / `stainedGlass`
- PhotoCraft command: `filter.filterGallery effect "stainedGlass"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ClSz` | cellSize | integer |  | 10 | 2 .. 50 (out-of-range values are clamped) | same |
| `BrdT` | borderThickness | integer |  | 4 | 1 .. 20 (out-of-range values are clamped) | same |
| `LghI` | lightIntensity | integer |  | 3 | 0 .. 10 (out-of-range values are clamped) | same |

#### Stamp

- Event: `Stmp` / `stamp`
- PhotoCraft command: `filter.filterGallery effect "stamp"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `LgDr` | lightDark | integer |  | 25 | 0 .. 50 (out-of-range values are clamped) | maps to our `lightDarkBalance` (same range and default) |
| `Smth` | smoothness | integer |  | 5 | 1 .. 50 (out-of-range values are clamped) | same |

#### Sumie

- Event: `Smie` / `sumie`
- PhotoCraft command: `filter.filterGallery effect "sumiE"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `StrW` | strokeWidth | integer |  | 10 | 3 .. 15 (out-of-range values are clamped) | same |
| `StrP` | strokePressure | integer |  | 2 | 0 .. 15 (out-of-range values are clamped) | same |
| `Cntr` | contrast | integer |  | 16 | 0 .. 40 (out-of-range values are clamped) | maps to our `contrast` (same range and default) |

#### Texturizer

- Event: `Txtz` / `texturizer`
- PhotoCraft command: `filter.filterGallery effect "texturizer"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `TxtT` | textureType | enumerated |  | `TxCa` texTypeCanvas | `TxBr` texTypeBrick, `TxBu` texTypeBurlap, `TxCa` texTypeCanvas, `TxSt` texTypeSandstone |  |
| `Scln` | scaling | integer |  | 100 | 50 .. 200 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 4 | 0 .. 50 (out-of-range values are clamped) | same |
| `LghD` | lightDirection | enumerated |  | `LDTp` lightDirTop | `LDBL` lightDirBottomLeft, `LDBR` lightDirBottomRight, `LDBt` lightDirBottom, `LDLf` lightDirLeft, `LDRg` lightDirRight, `LDTL` lightDirTopLeft, `LDTR` lightDirTopRight, `LDTp` lightDirTop |  |
| `InvT` | invertTexture | boolean |  | false |  |  |

#### Torn Edges

- Event: `TrnE` / `tornEdges`
- PhotoCraft command: `filter.filterGallery effect "tornEdges"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `ImgB` | imageBalance | integer |  | 25 | 0 .. 50 (out-of-range values are clamped) | same |
| `Smth` | smoothness | integer |  | 11 | 1 .. 15 (out-of-range values are clamped) | same |
| `Cntr` | contrast | integer |  | 17 | 1 .. 25 (out-of-range values are clamped) | maps to our `contrast` (same range and default) |

#### Underpainting

- Event: `Undr` / `underpainting`
- PhotoCraft command: `filter.filterGallery effect "underpainting"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `BrsS` | brushSize | integer |  | 6 | 0 .. 40 (out-of-range values are clamped) | same |
| `TxtC` | textureCoverage | integer |  | 16 | 0 .. 40 (out-of-range values are clamped) | maps to our `textureCoverage` (same range and default) |
| `TxtT` | textureType | enumerated |  | `TxCa` texTypeCanvas | `TxBr` texTypeBrick, `TxBu` texTypeBurlap, `TxCa` texTypeCanvas, `TxSt` texTypeSandstone |  |
| `Scln` | scaling | integer |  | 100 | 50 .. 200 (out-of-range values are clamped) | same |
| `Rlf ` | relief | integer |  | 4 | 0 .. 50 (out-of-range values are clamped) | same |
| `LghD` | lightDirection | enumerated |  | `LDTp` lightDirTop | `LDBL` lightDirBottomLeft, `LDBR` lightDirBottomRight, `LDBt` lightDirBottom, `LDLf` lightDirLeft, `LDRg` lightDirRight, `LDTL` lightDirTopLeft, `LDTR` lightDirTopRight, `LDTp` lightDirTop |  |
| `InvT` | invertTexture | boolean |  | false |  |  |

#### Water Paper

- Event: `WtrP` / `waterPaper`
- PhotoCraft command: `filter.filterGallery effect "waterPaper"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `FbrL` | fiberLength | integer |  | 15 | 3 .. 50 (out-of-range values are clamped) | same |
| `Brgh` | brightness | integer |  | 60 | 0 .. 100 (out-of-range values are clamped) | same |
| `Cntr` | contrast | integer |  | 80 | 0 .. 100 (out-of-range values are clamped) | maps to our `contrast` (same range and default) |

#### Watercolor

- Event: `Wtrc` / `watercolor`
- PhotoCraft command: `filter.filterGallery effect "watercolor"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `BrsD` | brushDetail | integer |  | 9 | 1 .. 14 (out-of-range values are clamped) | same |
| `ShdI` | shadingIntensity | integer |  | 1 | 0 .. 10 (out-of-range values are clamped) | maps to our `shadowIntensity` (same range and default) |
| `Txtr` | texture | integer |  | 1 | 1 .. 3 (out-of-range values are clamped) | same |

#### Grain

- Event: `Grn ` / `grain`
- PhotoCraft command: `filter.filterGallery effect "grain"`

| Key | String ID | Type | Unit | Default | Allowed | PhotoCraft today |
|---|---|---|---|---|---|---|
| `Intn` | intensity | integer |  | 40 | 0 .. 100 (out-of-range values are clamped) | same |
| `Cntr` | contrast | integer |  | 50 | 0 .. 100 (out-of-range values are clamped) | maps to our `contrast` (same range and default) |
| `Grnt` | grainType | enumerated |  | `GrnR` grainRegular | `GrCn` grainContrasty, `GrSf` grainSoft, `GrSp` grainSpeckle, `GrSr` grainSprinkles, `GrSt` grainStippled, `GrnC` grainClumped, `GrnE` grainEnlarged, `GrnH` grainHorizontal, `GrnR` grainRegular, `GrnV` grainVertical |  |

### Group B: The Render filters Clouds, Difference Clouds, Fibers and Lens Flare

All four commands exist in PhotoCraft; only the `.atn` mapping is missing. Lighting Effects is covered with the events that need a recorded step (its lights are only recorded through the dialog).

#### Clouds

- Event: `Clds` / `clouds`
- PhotoCraft command: `filter.render.clouds`
- Uses the current foreground and background colours; the step itself carries no colours.

No parameters: the step carries an empty descriptor.

#### Difference Clouds

- Event: `DfrC` / `differenceClouds`
- PhotoCraft command: `filter.render.differenceClouds`
- Same as Clouds, blended with Difference.

No parameters: the step carries an empty descriptor.

#### Fibers

- Event: `Fbrs` (no string ID)
- PhotoCraft command: `filter.render.fibers`
- `randomSeed` makes playback reproduce the recorded result; keep it.

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Vrnc` | (none) | integer |  | 16 | not clamped in the step (values are returned unchanged); limits pending, see "Events that need a step recorded through their dialog" |
| `Strg` | strengthPlugin | integer |  | 4 | not clamped in the step (values are returned unchanged); limits pending, see "Events that need a step recorded through their dialog" |
| `RndS` | randomSeed | integer |  | random | any 32-bit integer; a new random value per recording |

#### Lens Flare

- Event: `LnsF` / `lensFlare`
- PhotoCraft command: `filter.render.lensFlare`
- Lens types: `Zm  ` 50-300 mm Zoom, `Nkn ` 35 mm Prime, `Nkn1` 105 mm Prime, `PnVs` Movie Prime.

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Brgh` | brightness | integer |  | 100 | 10 .. 300 (out-of-range values are clamped) |
| `FlrC` | flareCenter | object |  | obj 'Pnt ' point | `Hrzn` / `Vrtc` doubles, 0.4 / 0.4 by default; not clamped: (5, -5) is kept as given and plays |
| `Lns ` | lens | enumerated |  | `Zm  ` zoom | `Nkn ` nikon, `Nkn1` nikon105, `PnVs` panaVision, `Zm  ` zoom |

Nested values as recorded (defaults):
```
    'Hrzn' horizontal : DOUBLETYPE = 0.40000000596046
    'Vrtc' vertical : DOUBLETYPE = 0.40000000596046
```

### Group C: The Distort filters (Pinch, Polar Coordinates, Shear, Spherize, Twirl, Wave, ZigZag)

All commands exist in PhotoCraft (`filter.distort.*`); only the `.atn` mapping is missing. Displace needs a displacement map file and is listed separately at the end.

#### Pinch

- Event: `Pnch` / `pinch`
- PhotoCraft command: `filter.distort.pinch`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Amnt` | amount | integer |  | 50 | -100 .. 100 (out-of-range values are clamped) |

#### Polar Coordinates

- Event: `Plr ` / `polar`
- PhotoCraft command: `filter.distort.polarCoordinates`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Cnvr` | convert | enumerated |  | `RctP` rectToPolar | `PlrR` polarToRect, `RctP` rectToPolar |

#### Shear

- Event: `Shr ` / `shear`
- PhotoCraft command: `filter.distort.shear`
- `shearPoints` is a list of curve points; keep their order and count.

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `ShrP` | shearPoints | list |  | list[2] | list of `Pnt ` points (`Hrzn`, `Vrtc`); default two points (0, 1) and (0, 128); limits pending |
| `UndA` | undefinedArea | enumerated |  | `WrpA` wrapAround | `RptE` repeatEdgePixels, `WrpA` wrapAround |
| `ShrS` | shearStart | integer |  | 0 | limits pending, see "Events that need a step recorded through their dialog" |
| `ShrE` | shearEnd | integer |  | 1 | limits pending, see "Events that need a step recorded through their dialog" |

Nested values as recorded (defaults):
```
    [0] OBJECTTYPE = obj 'Pnt ' point
        'Hrzn' horizontal : DOUBLETYPE = 0
        'Vrtc' vertical : DOUBLETYPE = 1
    [1] OBJECTTYPE = obj 'Pnt ' point
        'Hrzn' horizontal : DOUBLETYPE = 0
        'Vrtc' vertical : DOUBLETYPE = 128
```

#### Spherize

- Event: `Sphr` / `spherize`
- PhotoCraft command: `filter.distort.spherize`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Amnt` | amount | integer |  | 100 | -100 .. 100 (out-of-range values are clamped) |
| `SphM` | spherizeMode | enumerated |  | `Nrml` normal | `HrzO` horizontalOnly, `Nrml` normal, `VrtO` verticalOnly |

#### Twirl

- Event: `Twrl` / `twirl`
- PhotoCraft command: `filter.distort.twirl`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Angl` | angle | integer |  | 50 | -999 .. 999 (out-of-range values are clamped) |

#### Wave

- Event: `Wave` / `wave`
- PhotoCraft command: `filter.distort.wave`
- `randomSeed`: keep it so playback reproduces the recorded result.

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Wvtp` | waveType | enumerated |  | `WvSn` waveSine | `WvSn` waveSine, `WvSq` waveSquare, `WvTr` waveTriangle |
| `NmbG` | numberOfGenerators | integer |  | 5 | 1 .. 999 (out-of-range values are clamped) |
| `WLMn` | wavelengthMin | integer |  | 10 | 1 .. 999 (out-of-range values are clamped) |
| `WLMx` | wavelengthMax | integer |  | 120 | 1 .. 999 (out-of-range values are clamped) |
| `AmMn` | amplitudeMin | integer |  | 5 | 1 .. 999 (out-of-range values are clamped) |
| `AmMx` | amplitudeMax | integer |  | 35 | 1 .. 999 (out-of-range values are clamped) |
| `SclH` | scaleHorizontal | integer |  | 100 | 1 .. 100 (out-of-range values are clamped) |
| `SclV` | scaleVertical | integer |  | 100 | 1 .. 100 (out-of-range values are clamped) |
| `UndA` | undefinedArea | enumerated |  | `RptE` repeatEdgePixels | `RptE` repeatEdgePixels, `WrpA` wrapAround |
| `RndS` | randomSeed | integer |  | random | any 32-bit integer; a new random value per recording |

#### ZigZag

- Event: `ZgZg` / `zigZag`
- PhotoCraft command: `filter.distort.zigZag`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Amnt` | amount | integer |  | 10 | -100 .. 100 (out-of-range values are clamped) |
| `NmbR` | numberOfRipples | integer |  | 5 | 0 .. 20 (out-of-range values are clamped) |
| `ZZTy` | zigZagType | enumerated |  | `PndR` pondRipples | `ArnC` aroundCenter, `OtFr` outFromCenter, `PndR` pondRipples |

#### Displace

Displace reads a displacement map from a file chosen in a dialog; record one step and add its keys (file path form) before implementing.

### Group D: The Stylize and Pixelate filters

All commands exist in PhotoCraft (`filter.stylize.*`, `filter.pixelate.*`); only the `.atn` mapping is missing.

#### Extrude

- Event: `Extr` / `extrude`
- PhotoCraft command: `filter.stylize.extrude`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `ExtS` | extrudeSize | integer |  | 30 | 2 .. 255 (out-of-range values are clamped) |
| `ExtD` | extrudeDepth | integer |  | 30 | 1 .. 255 (out-of-range values are clamped) |
| `ExtF` | extrudeSolidFace | boolean |  | false |  |
| `ExtM` | extrudeMaskIncomplete | boolean |  | false |  |
| `ExtT` | extrudeType | enumerated |  | `Blks` blacks | `Blks` blacks, `Pyrm` pyramids |
| `ExtR` | extrudeRandom | enumerated |  | `Rndm` random | `LvlB` levelBased, `Rndm` random |

#### Find Edges

- Event: `FndE` / `findEdges`
- PhotoCraft command: `filter.stylize.findEdges`

No parameters: the step carries an empty descriptor.

#### Solarize

- Event: `Slrz` / `solarize`
- PhotoCraft command: `filter.stylize.solarize`

No parameters: the step carries an empty descriptor.

#### Trace Contour

- Event: `TrcC` / `traceContour`
- PhotoCraft command: `filter.stylize.traceContour`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Lvl ` | level | integer |  | 128 | applied, but not echoed: the returned step always shows 128. Levels <= 0 and >= 255 give the same image. Dialog range pending. |
| `Edg ` | edge | enumerated |  | `Upr ` upper | `Lwr ` lower, `Upr ` upper (applied; the returned step always shows upper) |

#### Wind

- Event: `Wnd ` / `wind`
- PhotoCraft command: `filter.stylize.wind`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `WndM` | windMethod | enumerated |  | `Wnd ` wind | `Blst` blast, `Stgr` stagger, `Wnd ` wind |
| `Drct` | direction | enumerated |  | `Left` left | `Left` left, `Rght` right |

#### Tiles

- Event: `Tls ` / `tiles`
- PhotoCraft command: `filter.stylize.tiles`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `TlNm` | tileNumber | integer |  | 10 | 1 .. 99 (out-of-range values are clamped) |
| `TlOf` | tileOffset | integer |  | 10 | 1 .. 99 (out-of-range values are clamped) |
| `FlCl` | fillColor | enumerated |  | `FlBc` fillBack | `FlBc` fillBack, `FlFr` fillFore, `FlIn` fillInverse, `FlSm` fillSame |

#### Color Halftone

- Event: `ClrH` / `colorHalftone`
- PhotoCraft command: `filter.pixelate.colorHalftone`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Rds ` | radius | integer |  | 8 | 4 .. 127 (out-of-range values are clamped) |
| `Ang1` | angle1 | integer |  | 108 | -10000 .. 10000 (out-of-range values are clamped) |
| `Ang2` | angle2 | integer |  | 162 | -10000 .. 10000 (out-of-range values are clamped) |
| `Ang3` | angle3 | integer |  | 90 | -10000 .. 10000 (out-of-range values are clamped) |
| `Ang4` | angle4 | integer |  | 45 | -10000 .. 10000 (out-of-range values are clamped) |

#### Facet

- Event: `Fct ` / `facet`
- PhotoCraft command: `filter.pixelate.facet`

No parameters: the step carries an empty descriptor.

#### Fragment

- Event: `Frgm` / `fragment`
- PhotoCraft command: `filter.pixelate.fragment`

No parameters: the step carries an empty descriptor.

#### Mezzotint

- Event: `Mztn` / `mezzotint`
- PhotoCraft command: `filter.pixelate.mezzotint`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `MztT` | mezzotintType | enumerated |  | `FnDt` fineDots | `CrsD` coarseDots, `FnDt` fineDots, `GrnD` grainyDots, `LngL` longLines, `LngS` longStrokes, `MdmD` mediumDots, `MdmL` mediumLines, `MdmS` mediumStrokes, `ShSt` shortStrokes, `ShrL` shortLines |

#### Mosaic

- Event: `Msc ` / `mosaic`
- PhotoCraft command: `filter.pixelate.mosaic`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `ClSz` | cellSize | unitdouble |  | 2 | 2 .. 200 pixels (out-of-range values are clamped). Write it as a unit double in `#Pxl`: the application accepts that and returns it unchanged. Read a value without a unit as pixels (the application returns its own default that way). |

#### Pointillize

- Event: `Pntl` / `pointillize`
- PhotoCraft command: `filter.pixelate.pointillize`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `ClSz` | cellSize | integer |  | 5 | 3 .. 300 (out-of-range values are clamped) |

### Group E: Video, Other and matting filters and the direct Gradient Map / Replace Color adjustments

All commands exist in PhotoCraft; only the `.atn` mapping is missing. Gradient Map and Replace Color here are the direct adjustments (Image > Adjustments), not adjustment layers (those are already mapped).

#### De-Interlace

- Event: `Dntr` / `deInterlace`
- PhotoCraft command: `filter.video.deInterlace`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `IntE` | interlaceEliminateType | enumerated |  | `ElmO` eliminateOddFields | `ElmE` eliminateEvenFields, `ElmO` eliminateOddFields |
| `IntC` | interlaceCreateType | enumerated |  | `CrtI` createInterpolation | `CrtD` createDroplet, `CrtI` createInterpolation |

#### NTSC Colors

- Event: `NTSC` (no string ID)
- PhotoCraft command: `filter.video.ntscColors`

No parameters: the step carries an empty descriptor.

#### HSB/HSL

- Event: `HsbP` (no string ID)
- PhotoCraft command: `filter.other.hsbHsl`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Inpt` | input | enumerated |  | `RGBC` RGBColor | `HSBl` HSBColorEnum, `HSLC` HSLColor, `RGBC` RGBColor |
| `Otpt` | output | enumerated |  | `HSBl` HSBColorEnum | `HSBl` HSBColorEnum, `HSLC` HSLColor, `RGBC` RGBColor |

#### Remove Black Matte

- Event: `RmvB` / `removeBlackMatte`
- PhotoCraft command: `layer.matting.removeBlackMatte`

No parameters: the step carries an empty descriptor.

#### Remove White Matte

- Event: `RmvW` / `removeWhiteMatte`
- PhotoCraft command: `layer.matting.removeWhiteMatte`

No parameters: the step carries an empty descriptor.

#### Gradient Map (direct)

- Event: `GrMp` (no string ID)
- PhotoCraft command: `image.adjustments.gradientMap`
- The `gradient` object is the same gradient form the adjustment layer already maps; reuse that code.

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `` | gradientsInterpolationMethod | enumerated |  |  gradientInterpolationMethodType.'Gcls' classic |  |
| `Grad` | gradient | object |  | obj 'Grdn' gradientClassEvent |  |

Nested values as recorded (defaults):
```
    'Nm  ' name : STRINGTYPE = "Vorder- zu Hintergrundfarbe"
    'GrdF' gradientForm : ENUMERATEDTYPE = 'GrdF' gradientForm.'CstS' customStops
    'Intr' interfaceIconFrameDimmed : DOUBLETYPE = 4096
    'Clrs' colors : LISTTYPE = list[2]
        [0] OBJECTTYPE = obj 'Clrt' colorStop
            'Clr ' color : OBJECTTYPE = obj 'RGBC' RGBColor
                'Rd  ' red : DOUBLETYPE = 0
                'Grn ' green : DOUBLETYPE = 0
                'Bl  ' blue : DOUBLETYPE = 0
            'Type' type : ENUMERATEDTYPE = 'Clry' colorStopType.'UsrS' userStop
            'Lctn' location : INTEGERTYPE = 0
            'Mdpn' midpoint : INTEGERTYPE = 50
        [1] OBJECTTYPE = obj 'Clrt' colorStop
            'Clr ' color : OBJECTTYPE = obj 'RGBC' RGBColor
                'Rd  ' red : DOUBLETYPE = 255
                'Grn ' green : DOUBLETYPE = 255
                'Bl  ' blue : DOUBLETYPE = 255
            'Type' type : ENUMERATEDTYPE = 'Clry' colorStopType.'UsrS' userStop
            'Lctn' location : INTEGERTYPE = 4096
            'Mdpn' midpoint : INTEGERTYPE = 50
    'Trns' transparency : LISTTYPE = list[2]
        [0] OBJECTTYPE = obj 'TrnS' transferSpec
            'Opct' opacity : UNITDOUBLE = 100 '#Prc' percentUnit
            'Lctn' location : INTEGERTYPE = 0
            'Mdpn' midpoint : INTEGERTYPE = 50
        [1] OBJECTTYPE = obj 'TrnS' transferSpec
            'Opct' opacity : UNITDOUBLE = 100 '#Prc' percentUnit
            'Lctn' location : INTEGERTYPE = 4096
            'Mdpn' midpoint : INTEGERTYPE = 50
```

#### Average

- Event: `Avrg` (no string ID)
- PhotoCraft command: `filter.blur.average`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

No parameters: the step carries an empty descriptor.

#### Custom

- Event: `Cstm` / `custom`
- PhotoCraft command: `filter.other.custom`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `Scl ` | scale | integer |  | 1 | ignored without the dialog; see "Events that need a step recorded through their dialog" |
| `Ofst` | offset | integer |  | 0 | ignored without the dialog; see "Events that need a step recorded through their dialog" |
| `Mtrx` | matrix | list |  | none: the 25 cells as set in the dialog | exactly 25 integers (5 x 5, row by row); a 3 x 3 list makes the application fail |

#### Replace Color

- Event: `RplC` / `replaceColor`
- PhotoCraft command: `image.adjustments.replaceColor`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `Fzns` | fuzziness | integer |  | 40 | 0 .. 200 (out-of-range values are clamped) |
| `Mnm ` | minimum | object |  | obj 'LbCl' labColor |  |
| `Mxm ` | maximum | object |  | obj 'LbCl' labColor |  |
| `H   ` | hue | integer |  | 30 | -180 .. 180 (out-of-range values are clamped) |
| `Strt` | saturation | integer |  | 0 | -100 .. 100 (out-of-range values are clamped) |
| `Lght` | lightness | integer |  | 0 | -100 .. 100 (out-of-range values are clamped) |
| `` | colorModel | integer |  | 0 |  |

Nested values as recorded (example values):
```
    'Lmnc' luminance : DOUBLETYPE = 40
    'A   ' a : DOUBLETYPE = 30
    'B   ' b : DOUBLETYPE = 40
    'Lmnc' luminance : DOUBLETYPE = 40
    'A   ' a : DOUBLETYPE = 30
    'B   ' b : DOUBLETYPE = 40
```

### Group G: Split Channels, Merge Spot Channel, Trap and Match Color

Channel and colour steps. Trap was measured in a CMYK document (it only exists there).

#### Split Channels

- Event: `SplC` / `splitChannels`
- PhotoCraft command: `channel.split`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `` | documentIDs | list |  | list[3] |  |

Nested values as recorded (example values):
```
    [0] 182
    [1] 185
    [2] 188
```

#### Merge Spot Channel

- Event: `MSpt` / `mergeSpotChannel`
- PhotoCraft command: `channel.mergeSpot`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

No parameters: the step carries an empty descriptor.

#### Trap

- Event: `Trap` / `trap`
- PhotoCraft command: `image.trap`
- Measured with example values in a prepared document: the Value column shows what was sent and recorded, not a factory default (a step always carries the values the user set).

| Key | String ID | Type | Unit | Example value | Allowed |
|---|---|---|---|---|---|
| `Wdth` | width | unitdouble | `#Pxl` | 2 | up to 32767 (clamped); the minimum was not measured (a huge negative value ran out of memory) |

#### Match Color

- Event: `` / `matchColor`
- PhotoCraft command: `image.adjustments.matchColor`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `Lght` | lightness | integer |  | 100 |  |
| `ClrR` | colorRange | integer |  | 100 |  |
| `Fade` | fade | integer |  | 0 |  |
| `` | noReference | boolean |  | true |  |

### Group I: Newer features (Focus Area, Remove Background, Select Sky, Oil Paint, Smart Sharpen)

These events only have a string ID (no 4-character code). Steps recorded in newer versions use them.

#### Select > Focus Area

- Event: `` / `focusMask`
- PhotoCraft command: `select.focusArea`

No parameters: the step carries an empty descriptor.

#### Remove Background (quick action)

- Event: `` / `removeBackground`
- PhotoCraft command: `layer.removeBackground`

No parameters: the step carries an empty descriptor.

#### Select > Sky

- Event: `` / `selectSky`
- PhotoCraft command: `select.sky`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `` | sampleAllLayers | boolean |  | false |  |

#### Filter > Stylize > Oil Paint

- Event: `` / `oilPaint`
- PhotoCraft command: `filter.stylize.oilPaint`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `` | lightingOn | boolean |  | true |  |
| `` | stylization | double |  | 4 |  |
| `` | cleanliness | double |  | 2.29999995231628 |  |
| `` | brushScale | double |  | 0.80000001192093 |  |
| `` | microBrush | double |  | 10 |  |
| `LghD` | lightDirection | integer |  | -60 |  |
| `` | specularity | double |  | 1.29999995231628 |  |

#### Filter > Sharpen > Smart Sharpen

- Event: `` / `smartSharpen`
- PhotoCraft command: `filter.sharpen.smartSharpen`

| Key | String ID | Type | Unit | Default | Allowed |
|---|---|---|---|---|---|
| `` | presetKind | enumerated |  |  presetKindType. presetKindCustom |  |
| `` | useLegacy | boolean |  | true |  |
| `Amnt` | amount | unitdouble | `#Prc` | 200 |  |
| `Rds ` | radius | unitdouble | `#Pxl` | 1 |  |
| `` | moreAccurate | boolean |  | false |  |
| `blur` | blur | enumerated |  |  blurType. lensBlur |  |

### Events that need a step recorded through their dialog

These carry settings that only exist once their dialog was used, so a dialog-less run returns no
values or ignores the ones it was given. They are listed so nobody measures them again from scratch;
their tables will be added once a recorded step was read.

| Event | Why |
|---|---|
| Lighting Effects | The light list only appears after the dialog. |
| Displace | Reads a displacement map file chosen in the dialog. |
| Fibers (`Vrnc`, `Strg` ranges) | Values outside the dialog's range are returned unchanged, not clamped; the limits come from the dialog. |
| Custom | Without the dialog, scale and offset are ignored, and a matrix that is not 5 x 5 makes the application fail. |
| Shear | Probing the point list made the application fail; the limits come from the dialog. |
| Trace Contour | The step's level and edge are applied, but the returned descriptor always shows the defaults (level 128, upper). Lower (`Lwr `) is valid; levels at or below 0 and at or above 255 give the same image. |
| Gradient tool | The drag needs a full gradient object (see Gradient Map in group E for its form). |
| Batch, Create Droplet, Print | File and printer settings come from their dialogs; Batch's file naming is still unknown (a step written with PhotoCraft's current naming rules is refused). |
| Sky Replacement, Smart Object commands (embed, link, relink, replace, export contents), Convert Stroke to Fill, Rasterize All Layers | Known to the application but need a matching document (a sky, a smart object, a shape with a stroke). |
| Generative Fill (`syntheticFill`) | AI feature: in PhotoCraft this goes through the shared AI layer; until then such steps stay kept steps. |

### Recorded under another event

Some older events are no longer what the application records; PhotoCraft already maps the current one:
Clear (recorded as Delete), Rasterize Type (`rasterizeLayer`), Convert to Profile (`convertToProfile`),
Copy and Paste Layer Style (`copyEffects`, `pasteEffects`), Align (`align`), Rename Layer (Set of the
layer's name), Deselect (Set of the selection to None). Steps recorded with Backlight, Fill Flash, Color
Cast, Variations, Texture Fill and the old Snapshot events come only from very old versions and stay kept
steps.
