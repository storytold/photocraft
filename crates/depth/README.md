# photocraft-depth

Monocular depth estimation: Depth Anything V2 Small (ViT-S/14) on the CPU through
[candle](https://github.com/huggingface/candle). A standalone L0 crate (no workspace dependencies).
Native only: on wasm32 the types exist and `Model::load` returns `DepthError::Unsupported`.

```rust
let model = photocraft_depth::Model::load(&dinov2_bytes, &head_bytes)?;
let map = model.estimate(&rgb_0_to_1, w, h, &|progress| cancelled())?; // DepthMap
```

- `Model::estimate` takes interleaved RGB (0..1) and returns a `DepthMap` at model resolution
  (longer side 518, both sides multiples of 14), min-max normalised to 0..1, **larger = nearer**.
  candle's Depth Anything V2 head is fixed to a square 518 x 518 input, so the image is resized,
  padded to square by repeating its last row and column, and the output cropped back.
- Weights are not shipped. `WEIGHTS` pins the two fp16 safetensors files (URL at a fixed Hugging
  Face revision, size, sha256); the app downloads them on request. They load from memory
  (`VarBuilder::from_buffered_safetensors`), as f32 on the CPU. No memory map, no `unsafe`.
- Weights are Apache-2.0 (`LICENSE-depth-anything-v2.txt`, `ATTRIBUTION.md` > Downloaded on request).
- Tests: `cargo test -p photocraft-depth` runs the resize and normalisation tests. The full-network
  tests are `#[ignore]` (zero weights; real weights when `PHOTOCRAFT_DEPTH_WEIGHTS_DIR` holds the
  two files).

## Dependency licence audit

candle is pinned to `=0.9.2` with default features off: it is the newest release whose
`candle-core` does not depend on `tokenizers` (0.11.1 does), and it has no `hf-hub` dependency
either. `cargo tree -p photocraft-depth -e normal` lists only MIT, Apache-2.0, BSD-2-Clause,
Unlicense and Zlib (all in OR-expressions with MIT or Apache-2.0, or allowlisted) plus two
identifiers outside the house-rules allowlist. Both are already in the workspace's shipped
dependency graph, so candle adds no new licence identifier:

| Crate | Licence | Comes from | Already in the workspace via |
| --- | --- | --- | --- |
| `yoke`, `yoke-derive`, `zerofrom`, `zerofrom-derive` | Unicode-3.0 | `candle-core` | `icu_*` (ui-egui, craft-i18n, parley), `hayro-interpret` (craft-pdf) |
| `unicode-ident` | (MIT OR Apache-2.0) AND Unicode-3.0 | proc-macros | naga / wgpu and most proc-macro crates |
| `foldhash` | Zlib | `safetensors` > `hashbrown` | `hashbrown` 0.15/0.17 (fontique, egui) |

Everything else is dual-licensed so an allowlisted branch exists (`bytemuck`, `zerocopy`:
Zlib/BSD-2-Clause OR Apache-2.0 OR MIT; `aho-corasick`, `memchr`, `byteorder`: Unlicense OR MIT).
Adding Unicode-3.0 and Zlib to the allowlist is proposed in storytold/craft-contrib. Re-run the
audit when bumping candle.
