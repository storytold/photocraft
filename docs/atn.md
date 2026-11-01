# Photoshop action files

## Targeted layer links

| Photoshop event | Descriptor | PhotoCraft command |
| --- | --- | --- |
| `linkSelectedLayers` | `null`: layer target (`Lyr `/`Ordn.Trgt`) | `layer.linkLayers {"linked":true}` |
| `unlinkSelectedLayers` | `null`: layer target (`Lyr `/`Ordn.Trgt`) | `layer.linkLayers {"linked":false}` |

These events act on the current layer selection. Link keeps an already linked
selection linked; Unlink keeps an already unlinked selection unlinked. Unlink
also clears a link group left with only one member. Undo restores the edit.

Omitting `linked` retains the native UI toggle. That ambiguous toggle cannot be
saved as either Photoshop event. Explicit layer lists and extra parameters also
cannot be saved as these events. Unsupported targets or extra descriptor keys
are kept as Photoshop steps for lossless saving, rather than partially replayed.

## Other events without settings

| Photoshop event | PhotoCraft command |
| --- | --- |
| `reverse` | `layer.arrange.reverse` |
| `selectLinkedLayers` | `layer.selectLinkedLayers` |
| `disableLayerStyle` | `layer.layerStyle.clear` |
| `placedLayerMakeCopy` | `layer.smartObjects.newSmartObjectViaCopy` |
| `placedLayerConvertToLayers` | `layer.smartObjects.convertToLayers` |
| `placedLayerConvertToEmbedded` | `layer.smartObjects.convertToEmbedded` |
| `placedLayerUpdateAllModified` | `layer.smartObjects.updateAllModifiedContent` |
| `rasterizeAll` | `layer.rasterize.allLayers` |
| `addBlankVideoLayer` | `layer.videoLayers.newBlankVideoLayer` |
| `isolateLayers` | `select.isolateLayers` |
| `textLoadDefaultTypeStyles` | `type.loadDefaultTypeStyles` |
| `textSaveDefaultTypeStyles` | `type.saveDefaultTypeStyles` |
| `recordMeasurements` | `image.analysis.recordMeasurements` |

These events and Link/Unlink accept either an empty descriptor or an explicit
current-layer target. Save retains which form was imported. Other targets or
extra settings stay complete Photoshop steps; native options with no recorded
representation are refused on export.

Playback uses each native command's normal preconditions and current state.
Isolate Layers toggles the panel view. Type defaults are saved in the current
session. Record Measurements uses the current selection/ruler/count and data
point configuration.

## Smart Object file operations

| Photoshop event | Meaningful descriptor keys | PhotoCraft command |
| --- | --- | --- |
| `placedLayerExportContents` | `null`: path | `layer.smartObjects.exportContents` |
| `placedLayerConvertToLinked` | `null`: target layer; `Usng`: path | `layer.smartObjects.convertToLinked` |
| `placedLayerRelinkToFile` | `null`: path | `layer.smartObjects.relinkToFile` |
| `placedLayerReplaceContents` | `null`: path | `layer.smartObjects.replaceContents` |

The path uses the same `Pth `/`txtu` encoding as Open and Place. Missing,
undecodable or unrepresented path options are kept as complete Photoshop steps.
Native steps aimed at explicit layers or carrying extra options cannot be saved
as these targeted events. File access occurs during playback through the native
command; a missing or unusable file produces that command's error.

The reference explicitly permits normalizing the top-level descriptor class to
`null` and omitting recording-session IDs on save. Relink and Replace accept a
recorded integer `LyrI` as a session hint; it does not select a PhotoCraft layer.
Playback operates on the current Smart Object. The hint is omitted on save,
while the event, path and other meaningful keys are retained.
