# Panel repair screenshots

Before captures supplied by @KotovGGG; after captures rendered locally from the
PhotoCraft regression harness. Crops show panel UI, without documents or personal
photos. The GIFs include only synthetic blank documents. The after images demonstrate the same transitions and are not intended as
pixel-identical theme/localization comparisons with the contributor's build.

After PNGs were refreshed on 2026-11-01 from
`a4077cb15a240549715b16e2075ce6c3ec8e2420`, integrating upstream
`799e12962806dac2d8b1a319bd386042dcbd61e4`. Before captures remain the original
contributor evidence. See [validation scope](../../panel-docking-validation.md).

The 13 GIFs retain the 2026-10-31 replay source at `7005992d2` (runtime
`88da5a914`); they are behavior illustrations, not a rerun of the latest merge.
Fresh acceptance uses the 444 PNG matrix described in the validation notes.

The 13 GIFs cover compact grouping/flyouts and tear-off, tab reordering, side
attachment, document-row placement, attached-row collapse and insertion,
independent horizontal parts, normal editor heights, vertical dividers, outer
bottom resize, and whole-Tools movement and resizing on both axes.

Every sequence uses actual offscreen pointer input. There are no caption overlays
or synthesized UI frames. One fixed crop is used for an entire clip, with longer
holds before a gesture and after the result. Frames are cropped/resized and
quantized to a shared 256-color GIF palette. Playback timing is for readability;
it does not measure input/GPU latency or native frame rate. The pointer ring is
an existing test-only input indicator, not product chrome.

Replay entry points: `current_version_showcase` with `PHOTOCRAFT_PANEL_SHOWCASE`,
and `pr_followup_pointer_replay` with `PHOTOCRAFT_PR_FOLLOWUP_REPLAY`. The latter
records focus rectangles and display durations in `frames.json`; both write an
exact `frame-count.txt`. Raw frames, encoding recipe, crop/timing/source hashes,
and decoded-pixel checks are preserved in the fork's handoff branch under
`plan/panel-repair/pr-gifs-portable-2026-10-31`.

These original screenshot assets are available under MIT OR Apache-2.0; see the
adjacent license files. The software UI and its existing icon assets retain their
original project/third-party licenses (see the repository ATTRIBUTION.md).
