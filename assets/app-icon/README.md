# PhotoCraft app icon

The canonical logo is a vector recreation of the project owner's supplied orange filmstrip
mark: a rounded golden frame with four dark sprocket holes per side, an orange inset, and dark
“Pc” lettering. The master is geometric SVG artwork, with no embedded raster image or font.
The corners outside the tile are transparent. The dark colour appears only in the sprocket
holes and lettering.

The macOS render pads the rounded 512-unit tile onto the platform icon grid. Windows and
Linux render the tile tightly with a slightly smaller corner radius, so the mark fills small
shell icons without clipping the frame or sprocket holes.

## Files

- `photocraft.svg`: full-detail vector master on a 512-unit square.
- `photocraft-small.svg`: flat-colour version of the tighter icon for tiny/scalable icons.
- `photocraft-1024.png`, `photocraft.icns`, `photocraft.ico`, and `hicolor/`: generated
  macOS, Windows, Linux and runtime icons.
- `PhotoCraft.icon`: Apple Icon Composer document. Its background fill and three unmasked SVG
  foreground groups let macOS apply system lighting, edge highlights and appearance variants.
  The SVGs declare a 1024-point intrinsic size so Icon Composer fills its canvas; the dark
  perforations and letters retain their ink colour. `packaging/macos/generate-icon-layers.py`
  derives the SVGs from the canonical master.

`apps/photocraft/src/app_icon.rs` loads the generated PNG for unpackaged runs. A macOS app bundle
uses the compiled `Assets.car` from the Icon Composer document and its generated `.icns` fallback;
the runtime leaves the bundle icon alone. Windows and Linux use the `.ico` and hicolor assets.

## Regenerate

Run `packaging/icons.sh` after changing the SVG. It uses `resvg` or `rsvg-convert`, `iconutil`
on macOS, and `cargo xtask ico` to pack the Windows icon.
The macOS release build additionally needs Xcode 26+ `actool` to compile `PhotoCraft.icon`.
