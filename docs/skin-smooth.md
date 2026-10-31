# Skin Smooth integration

Skin Smooth 0.6.0 is an ABI v1 WASM filter with an optional trusted native accelerator and a dedicated desktop UI. The companion source is `plugins/skin-smooth` beside the host checkout; the selection formula is shared from `crates/algo/src/skin_smooth_selection.rs`. Keep that sibling layout when building.

## Loading and acceleration

The manifest declares `colors` parameters and `accelerator: "skinsmooth.v2"`. ABI v1 alone does not establish compatibility: a host without the parameter type rejects the module before any CPU fallback. A compatible host without this accelerator uses WASM. This is a host-provided implementation, not GPU code or host imports supplied by WASM. See the [plug-in contract](plugins.md#native-accelerators-in-the-smooth-fork).

Version 2 adds positive and negative colour ranges. Defaults, presets, samples, colour interpretation, alpha, band edges and selection blending form the contract. The host resolves `presetValues` before both paths; built-in preset IDs with contradictory slider values are covered by GPU/WASM tests. Older v1 modules do not use the v2 backend.

GPU pipelines and buffers are created lazily and reused. Software adapters, CPU preference, unsupported input and device failures use WASM. A failed band restores its original input before retry. Cancellation or failure must not commit partial results. Preview and formal runs carry the same scoped accelerator onto their workers. GPU output still passes through CPU composition and display upload.

## Samples and Mask

Samples are normalized source RGB, read before effects or the Mask overlay. Positive and negative groups each accept at most 16 colours; repeated picks do not increase strength. Positive weights use their maximum, negative weights their maximum, then multiply by one minus the exclusion. With no positive samples, automatic skin selection remains the base. This is a colour range, not a portrait detector or spatial brush.

Samples survive preset switches and Reset, but clear on OK, Cancel or source invalidation. Named presets omit them; formal command parameters retain them for replay. The display toggle is not a formal filter or preset parameter.

Mask weight is skin weight × document selection coverage × source alpha. With Skin Only off, skin weight is one. It describes the input's potential contribution, not final composite visibility or effect strength. Preview and Show Mask are independent; OK applies smoothing, never the Mask.

Mask/sampling support unmasked, unclipped RGB raster layers in Composite view at 8/16/32-bit SDR. Unsupported targets, including channels, Quick Mask, masks and Smart Objects, show an explanation. The filter itself has its separately validated scope. The engine checks the effective document profile with `photocraft-cms::Profile::same_colors` before filtering (including previews and Smart Filter cache hits); sampling and Mask use the same check. Untagged images use the same CMS document-profile policy as the compositor; the filter does not assign a profile. Invalid ICC data and non-sRGB spaces return an error. Assign sRGB only when the pixels are already sRGB; otherwise use Convert to Profile first. No implicit colour conversion is performed. The low-level plug-in ABI still requires its caller to supply sRGB pixels because ABI v1 carries no profile.

## Preview and invalidation

Filter previews run on one background worker with cancellation of stale requests. Ordinary plug-in previews use the visible region plus context and an appropriate proxy; OK handles the full image/selection. Reduced previews remain approximate. Smart-filter stack previews keep their stack processing rules.

Mask has a separate worker and texture, limited to at most 1M visible/proxy pixels and does not run smoothing. Its request identity covers the source revision/target, dialog, samples, tolerance, Skin Only, viewport and scale. Closed dialogs and invalid sources cancel work; stale results cannot paint over another request. It does not write pixels, selection or history.

## Verification

Relevant sources: [GPU adapter](../crates/ui-egui/src/skin_smooth_gpu.rs), [compute implementation](../crates/gpu/src/skin_smooth.rs), [plug-in band processing](../crates/plugins/src/apply.rs), [preview worker](../crates/ui-egui/src/dialogs/filter_preview_worker.rs) and [Mask UI](../crates/ui-egui/src/dialogs/skin_smooth_ui.rs).

Regression coverage includes depths, alpha/transparent colour, selection, offsets, seams, preset overrides, cancellation and fallback. Real-photo appearance, low-end UI latency and Alt+Tab still require manual acceptance; synthetic events or kernel parity do not prove them. The historical focus-loss report is distinct from the removed implicit Enter confirmation.

Historical recovery archives and their original JSON evidence remain in the development workspace and are excluded from this review bundle. They do not establish validation for later source revisions.
