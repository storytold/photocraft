# Local Windows pen input patch

Based on the unmodified crates.io winit 0.30.13 source. Cargo patches this dependency
locally rather than editing the machine's shared registry cache.

The Windows WM_POINTER path already reads POINTER_PEN_INFO but used only pressure.
Retain its penFlags and map PEN_FLAG_BARREL (including hover) to right-button
press/release. Tip contact keeps the existing Touch pressure path; barrel contact
must not generate Touch Started, which egui-winit translates to a left press.
Restore CursorMoved after Touch End so an in-range pen keeps its hover position.
Release tracked buttons on pointer leave, reset tracking on focus loss, and use
the supplied screen-pixel position if the driver cannot report device rectangles.
The added mapping is safe Rust and uses winit's existing FFI reads unchanged.

Files changed: platform_impl/windows/{mod.rs,window_state.rs,event_loop.rs}, plus
the new pen_input.rs. Other platform implementations are unchanged.

Reference: https://learn.microsoft.com/en-us/windows/win32/inputmsg/wm-pointerupdate
and https://learn.microsoft.com/en-us/windows/win32/inputmsg/pen-flags-constants

Regression tests: cargo test --manifest-path vendor/winit/Cargo.toml --lib pen_input
