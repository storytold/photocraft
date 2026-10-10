# Panel docking screenshots

Captured from PhotoCraft on Windows with the offscreen snapshot example, a generated blank document and the Pro Medium theme. No personal documents or proprietary editor assets were used.

- `panels-before.png`: default dock layout before rearranging panels.
- `panels-after.png`: floating Layers and History at bounded sizes, and collapsed Color.
- `panels-drop-preview.png`: a Layers tab being dragged toward Color, with the destination line and insertion caret.

Original captures by @Kindleshard, licensed under MIT OR Apache-2.0, like the contribution. Existing PhotoCraft assets depicted retain their original licences. See [LICENSE-MIT](../../../LICENSE-MIT) and [LICENSE-APACHE](../../../LICENSE-APACHE).

## Reproduce

The committed captures used the initial integration base `ec03c602b46400381869e8ea3f8716b718823ffd`, with this contribution applied. The contribution was subsequently rebased onto `4e8aaec643f39d19e82b187fa6f7445cf5eb0b4f`. Run from the repository root. Coordinates are logical screen points; the resulting PNGs are 2160 × 1350 pixels. On the capturing Windows machine, `WGPU_BACKEND=dx12` selected the working offscreen graphics backend.

Default layout:

```sh
cargo run -p photocraft-ui-egui --example snapshot -- --out panels-before.png --size 1440x900 --scale 1.5 --custom-titlebar --script '[["engine.execute",{"command":"file.new","params":{"width":400,"height":300}}],["ui.set",{"theme":"proMedium"}]]'
```

Held drag from Layers to the Color header:

```sh
cargo run -p photocraft-ui-egui --example snapshot -- --out panels-drop-preview.png --size 1440x900 --scale 1.5 --custom-titlebar --drag-from 1180,389 --drag-to 1208,83 --script '[["engine.execute",{"command":"file.new","params":{"width":400,"height":300}}],["ui.set",{"theme":"proMedium"}]]'
```

For the detached capture, use the same document, theme, size and scale. Detach Layers and History, set their content sizes to 310 × 340 and 310 × 220 points, and positions to (180,140) and (550,140). Detach Color at (550,400) with stored size 310 × 340 and collapse it. The serialized `ui.set` layout shape is `dock.arrangement.floating`, with each entry containing `id`, `tabs` (`group` and canonical `tab`), `selected`, `position`, `size`, `open` and `collapsed`. The workspace normalization restores the remaining default tabs into their original groups.
