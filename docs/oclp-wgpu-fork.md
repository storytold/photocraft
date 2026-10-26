# Proposal: use the shared OCLP wgpu compatibility fork

PhotoCraft uses the same pinned wgpu 30.0.1 backport proposed for
[ArtCraft Launcher](https://github.com/storytold/craft-launcher/pull/917).
It replaces the Intel-wide indirect-validation bypass with compiler workarounds
in wgpu while preserving PhotoCraft's other GPU startup and recovery behavior.

## Dependency and ownership

- Prototype repository: <https://github.com/ITJamie/wgpu-fork>
- Branch: `codex/metal-msl-intel-v30.0.1`
- Immutable commit: `21c08e23135ba27ed1c6b68177ecc20c0d076dd9`
- Based on upstream tag `v30.0.1`, preserving the released API used by egui.
- Backports: <https://github.com/gfx-rs/wgpu/pull/10725> and
  <https://github.com/gfx-rs/wgpu/pull/10731>.

The fork probes supported Metal Shading Language versions, uses pipeline
workgroup limits when the Intel compiler rejects the shader attribute, and
emits runtime-sized storage arrays as pointers on Intel. Internal indirect-call
validation and shader bounds checks remain available and enabled by default.
Explicit wgpu environment settings still apply.

This follows the compiler-fallback approach discussed in
[#4011](https://github.com/storytold/photocraft/issues/4011) and alecanales'
[#5910](https://github.com/storytold/photocraft/pull/5910). It does not include
that PR's separate renderer startup error-handling changes.

A maintainer should decide whether to create `storytold/wgpu` and preserve the
backport branch and commit there. Once available, switch the patch URL in
`Cargo.toml` to the organization fork, keep the reviewed commit pin, and
regenerate `Cargo.lock`. PhotoCraft and ArtCraft Launcher should share that
compatibility dependency. Return to an upstream release after both fixes land
and hardware validation passes.

## Validation

```sh
cargo build --locked -p photocraft --features heif
cargo test --locked -p photocraft --features heif
cargo clippy -p photocraft --features heif --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
```

For an isolated desktop run, set `PHOTOCRAFT_CONFIG_DIR` to a fresh directory.
Enable the authenticated control API as described in
[the control protocol](control-protocol.md), with its read/write roots limited
to synthetic test images. Verify both the window renderer and GPU image canvas,
then open and save the synthetic image and inspect an internal `ui.screenshot`.

On the OCLP test Mac, use `WGPU_BACKEND=metal`,
`WGPU_VALIDATION_INDIRECT_CALL=1`, `MTL_DEBUG_LAYER=0`, and
`MTL_SHADER_VALIDATION=0`. The last two settings disable incompatible external
Metal validation layers for the test process; internal wgpu validation remains
enabled. Use `WGPU_POWER_PREF=low` for the Intel preference and `high` for AMD,
and confirm the selected adapter in logs. Do not disable validation globally
in production code as a substitute for a compiler workaround.

### Recorded PhotoCraft checks

On Apple M4 Max, the native build, affected-package Clippy, all 183 desktop
app tests, and the 31-crate architecture check passed. Focused GPU tests passed
for blend modes and cached composition at U8, U16 and F32 depths, with external
Metal validation enabled. An isolated native run opened and exported a synthetic
PNG with the GPU canvas active; its internal screenshot was visually inspected.
The debug window responded after approximately three seconds.

`cargo xtask test-corpus --changed` skipped because no covered source paths
changed. The first WebAssembly attempt failed because the Homebrew toolchain
lacked the target; the isolated Rust 1.96.1 toolchain passed all 24 crates and
the optional HEIF check. The 13 startup/control integration tests and native
AppKit menu integration harness also passed.

Repeated package runs, including a serial run, failed
`single_instance::tests::a_sandboxed_second_launch_hands_its_paths_over_a_unix_socket`
(182 passed, one failed each); it passed when rerun alone. The initial
183-test run passed. This IPC failure remains untriaged.

PhotoCraft validation on the OCLP Intel/AMD Mac remains pending. Launcher
results below establish shared-dependency coverage, not a PhotoCraft pass.

## Known limits of the shared backport

The prior launcher debug runs on MacBookPro11,5, macOS 15.8.1 through OCLP,
took roughly 35–45 seconds to reach swapchain creation. PhotoCraft startup
and editing performance must be measured independently.

The backport's full wgpu suite on Apple M4 Max ran 1,010 tests: 1,002 passed,
eight failed, and six were skipped. Failures were:

- `subgroup_operations`: an expected-failure mismatch.
- `all_passthrough_shaders_binary`, `all_passthrough_shaders_source`, and
  `metallib_passthrough_shader`: missing DXC or offline Metal tools.
- `binding_array_storage_buffers`, `partial_binding_array_storage_buffers`,
  `validate_capabilities`, and the `Computepass Encoding` benchmark: untriaged
  binding-array/capability failures.

The shared backport is a prototype, not a clean full-suite pass. Other OCLP
hardware and normal cross-platform CI still need validation before adopting it
for releases.
