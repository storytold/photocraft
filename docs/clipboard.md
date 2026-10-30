# Editable masks on the layer clipboard

Updated 2026-10-30.

With no pixel selection, Copy takes a pixel layer with a raster or vector mask
through the existing whole-layer clipboard path, just like smart objects, type,
shapes and groups. Multiple selected layers also use that path. The existing
layer-selection helper preserves stack order and excludes children already
included through a selected parent.

Paste and Paste in Place reuse the existing layer duplication, colour/profile/
depth conversion and single history step. Raster/vector masks, hidden colour,
mask settings and layer content stay editable. Pasted layers and group children
receive fresh IDs. Matching-format tiles retain the existing copy-on-write
sharing. The placement offset applied during paste moves even unlinked masks
with the copied layer; subsequent ordinary movement follows each mask's link flag.

The clipboard also carries a composited bitmap that respects the masks for
native bitmap export and bitmap destinations. Whole-layer placement uses the
underlying content bounds, so a fully hidden mask can still be copied without
losing its hidden colour or changing the paste position.

Copying a masked raster layer checks an estimated compositor working set before
rendering. The estimate includes the destination tiles and temporary buffers;
over-budget copies return an error without replacing the clipboard. This is a
guard for large workloads, not a guarantee against every allocation failure.

A source pixel selection, Cut and Copy Merged keep bitmap semantics. Targeted
colour/alpha channel Copy continues through the existing channel path. Paste
Into/Outside use the clipboard bitmap and their destination selection mask;
`edit.paste {pixels: true}` requests bitmap-only paste explicitly.

The native clipboard bridge hashes the complete exported bitmap and its dimensions
when distinguishing an external image from the internal layer payload. Editable
layers remain within one application session; an externally rewritten but
byte-identical bitmap cannot be distinguished without platform clipboard
ownership information. Copying vector paths, cross-process layered exchange,
bitmap-mode layer conversion and clipboard-to-new-document layer preservation
remain outside this extension.

The OS bitmap export has its own estimated memory budget. If it cannot export,
PhotoCraft keeps the internal copy and reports that the system clipboard was not
updated. An image already in the system clipboard may still take precedence on a
later Paste when the image-only bridge cannot identify it as an older image.

Adobe documents copying [one or multiple selected layers](https://helpx.adobe.com/photoshop/desktop/create-manage-layers/create-layer-compositions/copy-and-paste-layers.html)
and retaining [bitmap/vector masks and effects](https://helpx.adobe.com/photoshop/using/create-layers-groups.html).
