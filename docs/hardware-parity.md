# Hardware parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-11-07 · **Change:** minor (native Wayland pen input through tablet-v2, #79; hardware-verified on KDE Plasma 6.8 with a Wacom CTL-472 (pressure, clicks, drag, context menu, mouse/pen switching, UI interaction); Windows: flipping the pen to its eraser end selects the Eraser with Windows Ink on, #2285, not yet confirmed on a pen with an eraser end; optional WinTab backend on Windows: pressure and the eraser end; tilt on devices with orientation axes; rotation only with a pen and driver that report twist, with Windows Ink off) · **Target:** Adobe Photoshop 2026

Each hardware feature Photoshop 2026 (27.11.0) uses, per platform, against PhotoCraft. Photoshop
side: its bundle (Metal and OpenCL paths, "Precise Color Management for HDR Display" and EDR
strings, Touch Bar resources, pen pressure/tilt strings) and Adobe's GPU FAQ. PhotoCraft side:
`crates/tablet`, `crates/ui-egui/src/stylus.rs`, `apps/photocraft/src/gpu_startup.rs`,
`engine/src/display_color.rs`, `apps/photocraft/src/monitor_profile.rs`.

**Summary (estimated):** ~50% ready. GPU compositing and colour-managed display are solid; pen
input is uneven by platform (the default Windows Ink path lacks tilt and rotation; native
Wayland is new, hardware-verified on KDE Plasma 6.8 with a Wacom CTL-472 for pressure, clicks,
drag, context menu, mouse/pen switching and UI interaction; eraser/tilt/rotation are synthetic
only, as this pen reports pressure and distance; the opt-in WinTab path on Windows supplies tilt
only on devices that report orientation axes, and rotation only when the pen and driver report
twist) and there is no HDR
output. Remaining: 80–150 h (gap G12). Human: tablets (Wacom, XP-Pen, Huion) on each OS and an
HDR display to verify.

## GPU

| Feature | Photoshop | PhotoCraft | Status |
|---|---|---|---|
| GPU canvas, zoom/pan, rotate view | Metal (macOS), DirectX/OpenCL (Windows) | wgpu 30: Metal, Vulkan, DX12, GL; backend selectable | done |
| GPU compositing of layers, blend modes, effects | partial (Photoshop composites mostly on CPU) | wgpu compositor, ≤ 1/255 vs CPU oracle; Multichannel and > texture-limit regions fall back to CPU | done (ahead in places) |
| GPU compositor startup | available | Pipeline preparation runs in the background on native hardware; the CPU composites until ready. Windows WARP probe: 0.329 ms p95, 6.860 ms maximum device-work stall during two FXC builds; hardware measurements remain pending (#9157, #9364, #6419) | partial |
| GPU filters (Blur Gallery, Liquify, Smart Sharpen, Camera Raw) | yes | CPU (rayon) for filters; GPU for compose and effects. Camera Raw's preview runs its per-pixel stages (white balance to grain) in one wgpu pass, about 2-3 ms vs 11-28 ms on the CPU for the 858×572 proxy, depending on the machine; spatial stages, Calibration and OK still develop on the CPU (#6916) | partial |
| Documents larger than the GPU texture limit | yes (tiled) | CPU fallback; tiling in progress (#49) | partial |
| CPU-only fallback when no GPU | yes, reduced | `cpu` canvas preference; fails to start without an adapter on some Linux systems (#2519, #2412) | partial |
| Choosing the GPU on a multi-GPU machine | Preferences › Performance › Use Graphics Processor | Linux only: Preferences › Performance › Advanced › GPU adapter and `PHOTOCRAFT_GPU_ADAPTER` (#1885); ignored on Windows and macOS | partial |
| Old / integrated GPUs | yes | black line on Intel HD (#2426), OpenCore Macs fail (#1859), Rgba32Float panic (#2020) | partial |

## Pen tablets

| Feature | Photoshop | macOS | Windows | Linux X11 | Linux Wayland | Web |
|---|---|---|---|---|---|---|
| Pressure | yes | yes (AppKit monitor) | yes (WM_POINTER), jumpy (#2046) | yes (XInput2) | yes (tablet-v2; Xwayland only on compositors without it); hardware-verified on KDE 6.8 with a Wacom CTL-472 (2026-11-07: pressure, cursor, clicks, drag, context menu, mouse/pen switching, UI); GNOME 50 (#2622) not yet | yes |
| Tilt | yes | yes | **no** (WinTab: with orientation axes, opt-in) | yes | yes (read; synthetic test; hardware untested) | yes |
| Barrel rotation | yes | yes (not applied to tip angle, #1301) | no (WinTab: with a twist-reporting pen and driver, opt-in) | partial | partial (read, synthetic; hardware untested) | yes (twist) |
| Eraser end | yes | yes | yes with Windows Ink, not yet confirmed on hardware (#2285); WinTab: yes (opt-in) | partial | yes (eraser tool type; synthetic; hardware untested) | partial |
| WinTab driver mode | yes | – | yes (opt-in; Windows Ink off) | – | – | – |
| Pressure curve | Windows Ink / driver | global curve in Preferences (#2195) | same | same | same | same |

Windows: the opt-in WinTab backend (`tools.tabletInput = "winTab"`, with Windows Ink turned off
in the tablet utility) supplies pressure and the eraser end, tilt only on devices that report
orientation axes, and rotation only when the pen and driver report twist (pressure-only devices
get neutral tilt and rotation). The default Windows Ink path still lacks tilt and
rotation; its eraser end comes from the pointer hooks (#2285).

A compositor that offers tablet-v2 can still be forced through Xwayland with
`PHOTOCRAFT_XWAYLAND_PEN=1`, unless `PHOTOCRAFT_NATIVE_WAYLAND=1` is set. The tablet probe also
fails when `WAYLAND_SOCKET` is set (PhotoCraft cannot open its own probe connection), so such a
compositor takes the fallback even when it offers tablet-v2. The fallback needs a detected pen,
a nonempty `DISPLAY` and the X11 libraries; without them the window stays on native Wayland.

## Displays and colour

| Feature | Photoshop | PhotoCraft | Status |
|---|---|---|---|
| Colour-managed canvas (document → monitor profile) | yes | yes, folded into the GPU 3D LUT | done |
| Monitor profile detection | per display, live | macOS per display at launch; others sRGB or a chosen .icc | partial |
| HDR / EDR output for 32-bit and HDR images | yes ("Precise Color Management for HDR Display") | no; 8-bit sRGB-encoded canvas texture | missing |
| High-DPI, physical 100% | yes | yes (#1943) | done |
| Multiple monitors: documents in separate OS windows | yes | yes (`extra_windows`) | done |
| Panels in separate OS windows | yes | in-app floating only (UI-217-5) | missing |

## Input devices

| Feature | Photoshop | PhotoCraft | Status |
|---|---|---|---|
| Trackpad pinch zoom, two-finger pan | yes | yes; pinch scrolls instead in 0.6.0 (#2463) | partial |
| Trackpad rotate (Rotate View) | yes | yes (`rotate_view.rs`) | done |
| Touch Bar | yes (Intel Macs) | no | missing (low value) |
| Touch screens (Windows) | yes | basic pointer only | partial |
| 3D mouse, MIDI, Surface Dial | Surface Dial on Windows | no | missing (low value) |
| Scanners (WIA / TWAIN) | WIA menu (Windows) | menu item live; no driver integration | partial |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-06 | minor | Windows eraser end: the pointer hooks read `PEN_FLAG_INVERTED` and the Eraser switch follows it (Windows Ink on; #2285) |
| 2026-11-05 | minor | Optional WinTab driver mode on Windows: pressure and eraser; tilt with orientation axes; rotation with a twist-reporting pen, with Windows Ink off |
| 2026-10-28 | minor | Build GPU compositor pipelines in the background; benchmark baseline pending (#9157) |
| 2026-10-10 | major | First version |
| 2026-10-17 | minor | GPU filters: Camera Raw's preview, per-pixel stages (#6916 step 1) |
| 2026-10-19 | minor | Graphics Adapter choice on Linux (#1885) |
| 2026-10-22 | minor | Native Wayland pen input through tablet-v2 (#79) |
| 2026-11-07 | minor | KDE 6.8 tablet-v2 hardware-verified with a Wacom CTL-472: pressure, cursor, clicks, drag, context menu, mouse/pen switching and UI interaction; eraser/tilt/rotation covered by synthetic tests (the pen reports pressure and distance only); GNOME 50 remains open |
