# External editor integration

Photo managers, scripts and other tools often hand an image to an editor and take it back when
the user is done: a DAM's "Edit in", git's `core.editor`, `code --wait`, a build step that opens a
file for retouching. PhotoCraft supports that workflow directly:

| Need | How |
|---|---|
| Save writes the file that was handed over | File › Save writes back to an opened TIFF (with its layers), JPEG, PNG or WebP, keeping bit depth, ICC profile, EXIF and XMP ([Raster formats](../formats/raster-formats.md)) |
| Know when it was saved or closed | `events.subscribe` on the [control protocol](control-protocol.md) pushes `document.opened`, `document.saved` and `document.closed` |
| Block until the user is done | `photocraft --wait <file>…` |
| Feature-detect | `app.capabilities` lists `features` (`saveInPlace`, `events`, `wait`, …) and the `version` |

## `--wait`

```sh
photocraft --wait photo-edit.tif && echo "saved" || echo "closed without saving"
```

The process opens the files and exits once every one of them has been closed:

- exit status **0** when any of them was saved at least once (File › Save, Save As over it, or a
  confirmed flatten into a JPEG, PNG or WebP);
- exit status **1** when they were all closed without saving, or could not be opened;
- **2** for a usage error (`--wait` without files).

When the last waited file closes and no other document is open, the window closes by itself. If
other documents are open the app stays, and the process exits when the user quits it, with the
status the waited files earned. Quitting before closing them works too: the status is 0 if any of
them had been saved.

As a git editor for an image-producing step, or from a photo manager's external-editor setting:

```sh
git config core.editor "photocraft --wait"      # for tools that pass an image path
```
