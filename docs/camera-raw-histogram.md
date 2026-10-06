# Camera Raw histogram and scopes

A clean-room implementation of Camera Raw's SDR histogram interaction and vectorscope,
based on Adobe's public documentation. It does not claim numerical identity with Adobe's
develop pipeline. Automation fields are in [control-protocol.md](control-protocol.md).

| Behaviour | Implementation |
|---|---|
| RGB histogram | Continuous ribbons with additive overlap colours; display-only five-bin binomial smoothing over exact raw counts; edge spikes pinned and excluded from the auto-scale |
| Updates, Before/After | Counted from the same completed float proxy as the preview texture |
| Five tonal zones | Hover tooltip, horizontal drag and double-click reset of the existing Blacks / Shadows / Exposure / Highlights / Whites parameters |
| Clipping warnings | Exact per-channel endpoints (≤0 / ≥1); coloured triangles, U / O, blue / red preview overlays |
| Alt/Option diagnostics | Per-channel shadow or highlight clipping while Alt-dragging a Light tone slider or a histogram zone |
| Pixel hover | Tone band and RGB badge on the histogram; ICC-managed crosshair on the vectorscope |
| Readouts | RGB or Lab (document ICC profile → D50 Lab); up to nine movable probes (S, Alt-click removes) |
| Vectorscope | Hidden on opening, enabled from the context menu; 128×64 hue/saturation bins of the whole image or the document selection; red at 3 o'clock and skin-tone line options |
| Floating scopes | Resizable panel; drag it back onto the dock to restore |
| Preferences | Display options and panel geometry persist in `dialogs["filter.cameraRaw.scope"]`; probes and vectorscope visibility do not |

The settings panel is grouped as Light, Color, Effects, Curve, Color Mixer, Color Grading and
Detail, with the existing filter parameters and processing. The point Curve shares its editing
model with Image › Adjustments › Curves and layer Properties (`point_curve.rs`): button-down hit
testing, grab offsets, ordered points, endpoint protection, drag-off removal and re-entry,
keyboard nudges and deletion, at most 16 points. Camera Raw commits one `filter.cameraRaw` edit on
OK; scope interaction never edits the document. Double-clicking a Camera Raw smart filter reopens
this dialog with its stored settings and the pixels below it; OK updates that filter in place.

## Layering and bounds

Analysis lives in L2 `photocraft-algo` (`histogram`, `vectorscope`), colour transforms in L0
`photocraft-cms`, view state in `state.rs`, interaction and painting in `camera_raw_scope_ui.rs`
and `rgb_histogram.rs`. No engine commands or document fields were added.

- The preview and histogram use the existing proxy, capped at 900 px on the longest side. Each
  pixel with positive finite alpha counts once; transparent and non-finite pixels are excluded,
  finite values outside 0–1 count in the edge bins and the overflow counters.
- The vectorscope converts at most 8192 pixels, on a 2-D grid, through CMS. It is cached per
  preview revision, Before/After and selected-region view.
- Pointer hover samples one proxy pixel and does at most one ICC conversion; it never recounts
  the histogram or rebuilds the vectorscope.
- A feathered selection blends the preview with its proxy coverage, as the engine does on OK.
  Thin details and masks are approximate at proxy size.
- Point curves in new settings (dialog, control channel, `filter.cameraRaw`) are validated:
  empty, or 2–16 finite points in 0–255 with inputs at least one level apart. Stored Smart
  Filters are not validated on load, so curves saved by earlier versions keep rendering.

`camera_raw_bench` times the CPU path on a 36 MP layer (P31: develop + histogram; P36: all SDR
scopes, probes, hover and tessellation). GPU upload and presentation are not included.

## Not covered

- HDR histograms, output and display: no suitable HDR hardware was available for validation.
  Float input and counters outside 0–1 do not imply HDR support.
- Persisting Camera Raw as a PSD Smart Filter (see the file-compatibility scorecard).
- Camera Raw's own mask authoring; scopes use the document selection.

References: [Adobe, Make color and tonal adjustments](https://helpx.adobe.com/camera-raw/desktop/using/make-color-tonal-adjustments-camera.html),
[Julieanne Kost, Camera Raw shortcuts and interactions](https://jkost.com/blog/2022/10/225-shortcuts-tips-and-tricks-for-adobe-camera-raw.html).
Only public behaviour was consulted; no Adobe code, shaders, profiles or assets are included.
