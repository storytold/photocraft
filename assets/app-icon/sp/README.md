# Fork app icon: the "Sp" tile

ShotoPhopEX shows its own "Sp" tile instead of upstream's kitsune. It was a "Ps" tile until
2026-10-29 and a "Pc" tile until 2026-11-01 (it read as "PC"); the repository carries no Adobe
mark. The code points here through three one-line edits (`crates/ui-egui/src/panels/brand.rs`,
`apps/photocraft/src/app_icon.rs`, `apps/photocraft/build.rs`). The tile is the same artwork as
the packaging icon set one folder up. A custom icon picked in Edit › Preferences › App Icon…
replaces it at run time.

## Palette

| Colour | Hex | Used for |
|---|---|---|
| Navy | `#001E36` | the rounded tile |
| Light blue | `#31A8FF` | the "Sp" |

## Geometry

A full-bleed rounded square (corner radius 112/512, as the kitsune tile), "Sp" in a bold sans
(Segoe UI Bold on Windows; Inter Display Bold for the 2026-11-01 render) at 56% of the tile,
centred.

## Files

| File | Used by |
|---|---|
| `sp-128.png` | `brand.rs`: the title-bar mark (mipmapped texture) |
| `sp-256.png` | `app_icon.rs`: the window and taskbar icon on Windows and Linux |
| `sp.ico` | `build.rs`: the `photocraft.exe` resource icon (PNG entries at 16 to 256 px) |

## Regenerating

```powershell
python -I packaging\icon-tile.py . photocraft Sp "#001E36" "#31A8FF"
copy assets\app-icon\hicolor\128x128\apps\ai.storyteller.photocraft.png assets\app-icon\sp\sp-128.png
copy assets\app-icon\hicolor\256x256\apps\ai.storyteller.photocraft.png assets\app-icon\sp\sp-256.png
copy assets\app-icon\photocraft.ico assets\app-icon\sp\sp.ico
```

Delete the `photocraft-macos-512.png` the script also writes; it is not tracked.
`app_icon.rs`'s `exe_icon_has_every_shell_size` test checks the `.ico`.
