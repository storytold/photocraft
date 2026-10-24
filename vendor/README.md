# Vendored dependencies

## winit 0.30.13

Candidate Windows 11 mixed-DPI fix for [PhotoCraft #3810](https://github.com/storytold/photocraft/issues/3810).
This draft has not been compiled, tested, or reproduced in PhotoCraft.

Baseline: https://static.crates.io/crates/winit/winit-0.30.13.crate
SHA-256: `a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d`.
Published VCS revision: `e9809ef54b18499bb4f2cac945719ecc2a61061b`.
The baseline import preserves every published file except `docs/res/*.svg` and
`examples/data/*.png`: documentation/example images are unnecessary for the app.
Consequently winit's own image-dependent examples are not buildable from this copy.
Upstream code is Apache-2.0; its license is retained at `winit/LICENSE`.

The separate behavioral commit adapts [winit #4341](https://github.com/rust-windowing/winit/pull/4341)
(commits `e239d03dff7d5a00b1eeffc49e381859b670741a` and
`9d791c97239d07cfeda5d978126634759bf2639f`) following merged
[PdfCraft #544](https://github.com/storytold/pdfcraft/pull/544), merge revision
`24ba8385f89b3772a2eb4cad8787515527d374c8` (behavioral commit
`64bd636b8dfc5111607f506331e9d5bbc5b38a0c`). Local changes are marked
`PhotoCraft patch:` in two Windows source files. The existing OS-build helper stays
in `dark_mode.rs` and becomes crate-visible rather than being moved as on upstream.

On Windows 11 (build 22000+) WM_DPICHANGED applies the OS suggested rectangle.
Older builds keep the existing adjustment unchanged. An unavailable build number
also follows the suggested rectangle, matching the reference's condition.
There are no Linux behavior, application chrome, UI scale, or app geometry changes.
The Cargo patch retains version 0.30.13; only winit's registry source/checksum are
removed from the application lockfile.

Known caveat: [winit #4721](https://github.com/rust-windowing/winit/issues/4721)
reports that explicit ScaleFactorChanged size-writer overrides are ignored by this
Windows 11 branch. Published egui-winit 0.36.2 handles the scale factor without
using the writer. Revisit this caveat when upgrading the adapter or adding an override.

[PhotoCraft #3724](https://github.com/storytold/photocraft/pull/3724) independently
vendors winit for Linux DPI. If both land, reconcile the single vendor directory
and Cargo patch while preserving each platform's reviewed changes; this copy does
not incorporate that unmerged Linux behavior.

Maintenance: `apps/photocraft/tests/winit_windows_dpi_patch.rs` guards patch presence
and lockfile provenance, not runtime behavior. It has not been run for this draft.
Remove this vendor copy and Cargo patch when a compatible released winit includes
the fix (and review #4721), or when eframe moves to a suitable newer winit.
Re-vendoring must retain the Windows branch or deliberately retire the guard.
