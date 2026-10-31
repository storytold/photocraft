### Transaction functions from the command registry

Registry commands: 907; after collapsing parameter-only families: 895.

| Type | Count | Rule |
|---|--:|---|
| EI | 824 | every state-changing command not matched below |
| EQ | 43 | last segment info/list/get/inspect/printers/browse, `*Info`, `list*` |
| EO | 28 | export, print, saveACopy, saveContents, presets/actions save, package, contactSheet, renderVideo, imageAssets |

Collapsed: trailing `More`, `slotN`, `bits8/16/32` variants count once.

| Command prefix | EI | EQ | EO |
|---|--:|--:|--:|
| actions | 14 | 2 | 1 |
| brush | 11 | 2 | 1 |
| channel | 15 | 1 | 0 |
| cloneSource | 4 | 1 | 0 |
| color | 1 | 0 | 0 |
| command | 0 | 1 | 0 |
| count | 7 | 0 | 0 |
| document | 6 | 4 | 1 |
| edit | 76 | 1 | 2 |
| file | 32 | 3 | 17 |
| filter | 122 | 0 | 0 |
| gradient | 10 | 2 | 0 |
| history | 9 | 0 | 0 |
| image | 61 | 2 | 0 |
| jobs | 1 | 1 | 0 |
| layer | 210 | 1 | 3 |
| layerComp | 12 | 1 | 0 |
| measurementLog | 1 | 1 | 1 |
| notes | 3 | 1 | 0 |
| paint | 26 | 0 | 0 |
| path | 19 | 2 | 0 |
| pattern | 7 | 2 | 1 |
| plugin | 4 | 1 | 0 |
| prefs | 2 | 1 | 0 |
| select | 35 | 0 | 0 |
| session | 0 | 1 | 0 |
| shape | 7 | 2 | 0 |
| slice | 6 | 1 | 0 |
| style | 4 | 1 | 0 |
| swatches | 10 | 1 | 1 |
| timeline | 6 | 1 | 0 |
| tool | 4 | 1 | 0 |
| tools | 9 | 0 | 0 |
| type | 61 | 3 | 0 |
| variables | 0 | 1 | 0 |
| view | 29 | 1 | 0 |

EQ commands: actions.get, actions.list, brush.get, brush.presets.list, channel.list, cloneSource.list, command.list, document.inspect, document.presets.get, document.presets.list, document.recent.list, edit.profileInfo, file.fileInfo, file.printers, file.scripts.browse, gradient.fill.get, gradient.presets.list, image.adjustments.colorLookup.list, image.analysis.info, jobs.list, layer.vectorMask.info, layerComp.list, measurementLog.list, notes.list, path.info, path.list, pattern.list, pattern.presets.list, plugin.list, prefs.get, session.inspect, shape.info, shape.presets.list, slice.list, style.presets.list, swatches.list, timeline.info, tool.presets.list, type.characterStyle.list, type.info, type.paragraphStyle.list, variables.list, view.colorSamplers.list

EO commands: actions.save, brush.presets.save, document.presets.save, edit.preferences.export, edit.presets.exportImportPresets, file.automate.contactSheetII, file.export.artboardsToFiles, file.export.artboardsToPdf, file.export.colorLookupTables, file.export.dataSetsAsFiles, file.export.exportPreferences, file.export.layerCompsToFiles, file.export.layersToFiles, file.export.pathsToIllustrator, file.export.quickExport, file.export.renderVideo, file.export.saveForWebLegacy, file.generate.imageAssets, file.package, file.print, file.printOneCopy, file.saveACopy, layer.exportAs, layer.smartObjects.exportContents, layer.smartObjects.saveContents, measurementLog.export, pattern.export, swatches.export
