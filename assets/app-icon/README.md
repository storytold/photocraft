# PhotoCraft app icon

A **nine-tailed kitsune with a brow diamond**: an engraving portrait, seated, looking back at the
viewer, in the Crafting Apps' owl-template framing (full-bleed field, head and body filling the
tile, tails running off the edges).

## Palette

Exactly three colours:

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#0b0b0c` | line work and contours |
| Paper | `#efe9dc` | the figure |
| PhotoCraft blue (app colour) | `#2f7bf5` | the full-bleed field |

## Geometry

A 512-unit tile (`viewBox="0 0 512 512"`), rounded square with `rx=112`, no border. The macOS
renders pad it onto Apple's 824/1024 icon grid; the Windows and Linux renders use the whole tile,
as the other Crafting Apps do.

## Provenance

The owner's original drawing, made in ArtCraft (2880 px, engraving style, keyed to the palette),
vectorised. License: see `LICENSE.txt`.

## Files

- `photocraft.svg`: canonical master (traced at 2048 px).
- `photocraft-small.svg`: lighter trace (traced at 1024 px); also the hicolor scalable icon.
- `photocraft-1024.png`: 1024 px render on the macOS grid.
- `photocraft.icns`: macOS bundle icon (`CFBundleIconFile`).
- `photocraft.ico`: Windows icon, 16–256 px, embedded in the `.exe` by `apps/photocraft/build.rs`.
- `photocraft-document.svg`: Windows PSD file icon master: a page with a folded corner filled into
  a rounded flap, a blue (`#2f7bf5`) ring and the ArtCraft mark, a deep (`#001424`) field and
  outlined `PSD` lettering; no font dependency.
- `photocraft-document.ico`: Windows PSD file icon (16–256 px), embedded as resource 2.
- `photocraft-pcraft-document.svg`: Windows `.pcraft` file icon master: the same page, palette
  and mark with a `PCRAFT` label; no font dependency.
- `photocraft-pcraft-document.ico`: Windows `.pcraft` file icon (16–256 px), embedded as
  resource 3.
- `hicolor/<size>/apps/ai.storyteller.photocraft.png` (16–512) and `hicolor/scalable/...svg`:
  Linux icon theme.

The app also sets the window icon and Wayland app ID at runtime (`apps/photocraft/src/app_icon.rs`,
`main.rs`). On Windows the taskbar shows the icon of the Start Menu shortcut that launches the
exe, so the MSI's `<Icon Id>` keeps the `.exe` extension (ICE50); `app_icon.rs` documents the
details and tests the `.ico` sizes and the WiX icon references.

Windows `.psd`/`.psb` files use resource 2 (`"photocraft.exe",-2`) when PhotoCraft is chosen for
them, and `.pcraft` files use resource 3: PhotoCraft's own format gets the same page with a
`PCRAFT` label, so a document the app wrote is distinguishable from the document it edits. The
app tile and the image ProgId are untouched. `ProgId@Icon` in WiX 5 takes a
File, not an MSI Icon entry, so the shell icon is written as the ProgId's `DefaultIcon` instead. The
document icon adapts design 01, "red outline page", from the redesigns JustCallMeAD posted in
storytold/pdfcraft#308: the page geometry, the fold and the mark placement are theirs, the fold is
filled into a rounded flap as on the shipped PdfCraft icon, the ring and the mark are PhotoCraft blue,
the field is deep ink and the three letters are outlined type. The ArtCraft mark on it is the
project's, used as a mark, not licensed for reuse; a fork that distributes a modified version must
remove or replace it ([LICENSE-document.txt](LICENSE-document.txt)). The label is sized to match the
PdfCraft PDF icon (186 × 70 at 512, against 203 × 76), so the two read as one family. The
`.pcraft` label is 250 × 46 — 61 % of the page width, 66 % of the PSD cap height — on the same
centre, which is how six letters sit in the band without reaching the frame.

Every PNG and the `.ico` are rendered from the master; `packaging/icons.sh` is for the app tile and
does not build this one. The document `.ico` is packed with `xtask`, which takes the output path and
then the input PNGs, the way `icons.sh` packs the app tile:

```sh
for s in 16 20 24 32 40 48 64 128 256; do
  resvg -w "$s" -h "$s" assets/app-icon/photocraft-document.svg "/tmp/psd-$s.png"
done
cargo run -p xtask -- ico assets/app-icon/photocraft-document.ico /tmp/psd-{16,20,24,32,40,48,64,128,256}.png
```

## Regenerate

Replace `photocraft.svg` (and `photocraft-small.svg`), then run `packaging/icons.sh`. It needs
`resvg`, plus `iconutil` on macOS; the app `.ico` is packed by `cargo run -p xtask -- ico <out.ico> <in.png>...`; the
document icon keeps its own command above.

## Nightly channel

The nightly derivatives keep the foreground artwork, alpha and Mac padding unchanged. Only the
background contribution changes from the stable blue to the selected deep background. There is
no letter badge or crescent; the app and Launcher also identify the channel in text.

`photocraft-nightly.png` and its nine-frame ICO use the existing own256 master. The separate
`photocraft-nightly-1024.png` and eleven-slot ICNS use the actual padded Mac1024 master, not an
upscale. Input/output hashes, the palette-fit recipe and container sizes are recorded in
[`nightly-provenance.json`](nightly-provenance.json). Stable source files are not recoloured.
The old N-marked private Windows isolation package is separate and is not rebuilt for this art.
