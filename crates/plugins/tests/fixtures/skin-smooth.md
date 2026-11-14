# Skin Smooth fixture

Version 0.6.0, original Skin Smooth contributors (Codex-assisted), MIT OR Apache-2.0.

Rebuilt from the companion `plugins/skin-smooth` source in the review bundle, beside the integrated `photocraft` checkout. The source shares `crates/algo/src/skin_smooth_selection.rs` from that checkout. From the companion directory: `cargo test --locked` then `sh build.sh`; copy `dist/skin-smooth.wasm` here. No Node or Electron build/test dependency is required.

This delivery was built with rustc 1.97.1, Cargo 1.97.1, wasm32-unknown-unknown std dated 2026-07-16, locked serde_json 1.0.151 and release settings in Cargo.toml. The native verifier's separate lockfile applies only to host verification.

SHA-256: `2552cdba3a81427d3422b3e484c749077f4a71db54fd0e935ba221f926b6c3ae`. Different toolchains or source paths may produce different binaries; reproduce with the pinned toolchain and record the resulting hash rather than assuming byte identity.

Original source licenses are adjacent (`skin-smooth-LICENSE-MIT`, `skin-smooth-LICENSE-APACHE`); compiled dependency and Rust standard library notices are in `skin-smooth-LICENSES/`. The source archive carries these notices as well. The independent synthetic prototype fixture is documented under the companion `tests/fixtures/`; it contains no photographs.

The filter crate and its std-only export build tool enforce `forbid(unsafe_code)`. Safe owned buffers replace raw-memory slices; the build tool rewrites only export names while preserving ABI v1. Source placement remains a maintainer review decision; no unsafe policy exception is needed or assumed for authored ABI code. See the companion `src/abi.rs` and `tools/export-abi.rs`.
