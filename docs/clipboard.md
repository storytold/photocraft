# PhotoCraft clipboard

## Copy and paste semantics

With the layer pixels targeted and no pixel selection, `edit.copy` captures the selected
layers in document order. The clipboard retains their independent colour surfaces, editable
raster and vector masks, styles, content, patterns and supported PSD metadata. Group children
are retained. Paste assigns fresh layer IDs recursively and clears retained PSD layer IDs.
The data uses the same copy-on-write surfaces as document snapshots: editing the source, a
pasted layer or a pasted mask does not change the clipboard or another paste.

A pixel selection makes Copy a pixel operation. `edit.copyMerged` always copies the visible
composite. A targeted raster mask, colour channel, alpha channel or Quick Mask copies that
plane; it does not copy the layer or its mask settings. This also works on adjustment-layer
masks without a colour surface. Cut keeps its existing pixel-only semantics. Agents can
request bitmap semantics explicitly with `{"pixels":true}` on Copy or Paste.

Paste into a mask/channel writes the clipboard bitmap into that target. Paste Into / Outside
uses bitmap semantics and creates the selection mask, rather than replacing a copied layer's
editable mask. Paste and Paste in Place otherwise retain the rich layers. Existing placement
rules apply: keep the copied coordinates when they fit, or use the supplied view centre / canvas
centre; Paste in Place keeps the coordinates. Every mask moves by the same offset as the
copied layer, including unlinked masks. Its link setting governs subsequent edits. Native
bitmap export uses the flattened appearance, colour-managed to sRGB.

Layer transfers convert through the existing CMS with Color Settings' intent and black-point
compensation. Mask values are coverage, not colour, so only their sample depth changes.
Enabled, link, density, feather, default pixel and vector path inversion/operation state are
retained. Raster inversion is represented by the samples and default pixel. No document or
native-format fields were added.

Rich data stays inside one application session. An OS bitmap with different dimensions or
bytes replaces it; a text/non-image clipboard replaces a mirrored/imported image with an
empty clipboard. The current bitmap bridge identifies ownership by content signature, so an
external copy of exactly the same bitmap is indistinguishable from our own. It does not
provide layered interchange between processes, a clipboard generation token, or separate
read-error information (`clipboard_get_image` returns an optional image). Native export
errors produce a notice while retaining the internal copy. Vector-path clipboard editing is
still unsupported; copying a layer with a vector mask retains the entire mask.

Clipboard areas are bounded to 64 million pixels and coordinates within ±10 million.
Layer trees are bounded to 64 group levels / 10,000 layers and transferred stored pixels to
512 MiB. Invalid placements, targets, dimensions and truncated OS bitmaps do not create
layers. Clipboard snapshots are not serialized, so there is no clipboard file-format migration.

## Upstream investigation (2026-10-09)

The starting commit was
[`a680ed22`](https://github.com/optimalbytee/photocraft/commit/a680ed22c96f8ca4724c0c48f5cea0ba2686f472),
also the freshly fetched `origin/main` and supplied `upstream/main` ref. Work is isolated on
`codex/mask-clipboard-3227`; other checkouts and branches were not changed.

GitHub's search API was queried across issues and PRs without a state restriction for
clipboard, copy paste, copy/paste, layer mask, masked layer, mask loss, duplicate layer and
mask + copy PRs. Relevant bodies, comments, merge commits and implementation history were
inspected before implementing this change.

| Item | Observed status | Coverage and remaining gap |
|---|---|---|
| [#217](https://github.com/storytold/photocraft/issues/217) | Open | Explicitly requests a whole-layer clipboard with masks, styles and type. |
| [#1123](https://github.com/storytold/photocraft/issues/1123), [#2122](https://github.com/storytold/photocraft/issues/2122) | Open | Mask/channel Copy does not respect its active context; these were still gaps in the base. |
| [#1035](https://github.com/storytold/photocraft/issues/1035), [PR #1078](https://github.com/storytold/photocraft/pull/1078) | Issue closed; PR merged | [fa186c59](https://github.com/storytold/photocraft/commit/fa186c59b913dfc85b8960266b11a44c44201c9a) pastes a bitmap into a targeted mask/channel. Its diff does not add editable masks to the copied payload. Already in the base. |
| [PR #1912](https://github.com/storytold/photocraft/pull/1912) | Merged | [b8087806](https://github.com/storytold/photocraft/commit/b8087806662807af09808b637f7533cb9dfaa45f) drops stale OS images. Its signature-based replacement is retained and tested with rich data. Already in the base. |
| [PR #851](https://github.com/storytold/photocraft/pull/851) | Merged | [20fb1903](https://github.com/storytold/photocraft/commit/20fb1903dddc570eae3c9924e64096335c5312d1) adds mask-preserving drag transfer through `layer.copyToDocument`, not Copy/Paste. Its existing CMS and geometry helpers inform the clipboard implementation. |
| [PR #841](https://github.com/storytold/photocraft/pull/841), [issue #1389](https://github.com/storytold/photocraft/issues/1389) | PR merged; issue closed | [727daf5d](https://github.com/storytold/photocraft/commit/727daf5d6745ba084d1feef02014779354ab4893) permits pixel Copy on smart/type/shape layers. The issue's closing comment recommends drag transfer and describes Copy/Paste as flattened. Current Adobe documentation specifies whole-layer Copy/Paste with masks and effects; selected-pixel Copy remains flattened here. |

No inspected open PR supplies a mask-preserving clipboard implementation. Adobe's
[layer Copy/Paste guide](https://helpx.adobe.com/photoshop/using/create-layers-groups.html)
and [current desktop guide](https://helpx.adobe.com/photoshop/desktop/create-manage-layers/create-layer-compositions/copy-and-paste-layers.html)
describe retaining bitmap/vector masks and effects. This is documentation verification, not
a hands-on Photoshop oracle. The bug reproduced on the fetched base, so an older running
build is not the sole explanation.

## Regression evidence

`edit_cmds/clipboard_tests.rs` initially failed the real Copy → Paste sequence with
`mask: None` despite unchanged colour pixels. The fixture uses black, half-gray and white
coverage with non-default density, feather and linking. It now covers masks and colour data,
appearance, independent edits, repeated paste IDs, undo/redo, group children and PSD metadata,
new clipboard documents, active planes and cross-document mode/depth/placement conversion.
`ui-egui/mask_clipboard_tests.rs` covers keyboard dispatch through the bitmap bridge, external
replacement, malformed dimensions/truncation, adjustment masks and export failure notices.

Copy/Paste is synchronous and opens no confirmation dialog. Rejected commands leave the
document unchanged; a successful paste is one undoable operation.

## Verification (2026-10-10)

The engine's full suite passed with 1,060 unit tests (11 ignored) and all integration/doc tests;
the UI suite passed with 1,361 unit tests (7 ignored) and all integration/doc tests. The new
coverage comprises 13 engine clipboard tests and 5 UI/native-bridge tests. Follow-up checks
exercise matching-format tile sharing as well as editable-mask independence. All-targets
Clippy for both crates, formatting, layering (29 crates), the L0–L6 wasm check, the ignored
adversarial `panic_hunt` and regenerated scorecard consistency pass. No new command IDs or
format/compositor source changes require parity regeneration or corpus tests.

The rebuilt native Windows app was driven through its authenticated control server using
real Ctrl+C/Ctrl+V dispatch and the platform bitmap clipboard. A synthetic RGB/16 document
contains black, half-gray and white mask coverage. Paste retains the mask thumbnail and
appearance; moving the copy and painting its mask leaves the source unchanged. Undo/redo
restores the masked layer, and external text prevents another stale paste. A separate fully
hidden mask also survives the native round trip. Before/paste/mask-edit PNGs were inspected.
Private local scripts, PNGs, inspection JSON and check logs are in
`plan/clipboard-check/` (gitignored); no token or generated fixture is committed.

`cargo xtask perf --quick` completed successfully. It explicitly skipped regression comparison
because the checked-in baseline is macOS Apple M4 Pro and this run is Windows AMD; quick mode
does not apply full-size budgets. This is not evidence that the full performance budgets pass.

The reproducible release example is `cargo run --release -p photocraft-engine --example
clipboard_bench`. It uses a synthetic 6000 × 4000 RGBA8 layer with 0 / 0.5 / 1 mask coverage,
three Copy → Paste in Place → Undo iterations per mode, on the same local Windows AMD x86-64
machine. Timings cover engine dispatch, including the rich copy's composited preview, but
exclude OS bitmap export and UI presentation. These are medians of three iterations:

| Workflow | Copy | Paste in Place |
|---|---:|---:|
| Explicit bitmap-only, final code | 230.06 ms | 5.42 ms |
| Editable layer + mask, initial implementation | 334.61 ms | 1941.15 ms |
| Editable layer + mask, matching tiles kept COW-shared | 339.04 ms | 4.45 ms |

The initial rich paste unnecessarily rewrote every sample through a same-format depth
conversion. The final path converts only surfaces whose sample type differs from the
destination; a regression checks that matching colour and mask tiles remain shared until
edited. The bitmap row is a workflow comparison, not a measurement of the pre-change binary.
This fixture does not measure cross-profile conversion, shifted placement, other sample
depths or complex styles, and does not establish a general clipboard performance budget.
