# Invert: an example PhotoCraft plug-in

A complete WebAssembly filter plug-in in about 70 lines of `no_std` Rust: no allocator, no imports,
under 1 KiB built. It inverts the colour channels and leaves alpha alone, matching
Image › Adjustments › Invert. The ABI is documented in [`docs/plugins.md`](../../../docs/plugins.md).

## Build

```sh
rustup toolchain install 1.95.0 --profile minimal
rustup target add --toolchain 1.95.0 wasm32-unknown-unknown
./build.sh
```

`build.sh` runs `cargo +1.95.0 build --release --target wasm32-unknown-unknown` and copies the module to
`crates/plugins/tests/fixtures/invert.wasm`, the test fixture the host's tests use. This crate is not
a member of the PhotoCraft workspace.

## Reproduce the test fixture

The checked-in fixture was built with `rustc 1.95.0 (59807616e 2026-04-14)` for
`wasm32-unknown-unknown`, using the release profile in this crate's `Cargo.toml`.
From the repository root, after installing the toolchain and target above:

```sh
./examples/plugins/invert-rs/build.sh --check
sha256sum crates/plugins/tests/fixtures/invert.wasm
cargo test -p photocraft-plugins --test it abi::
```

`--check` builds the module and uses `cmp` to compare it byte-for-byte with the committed
fixture without overwriting it. The fixture is 942 bytes; its SHA-256 is
`f9b6244fd9a0945f7327c2bdcccaf2e469424ba302649f3e852df6eaaa468dd7`.
Run `build.sh` without arguments to refresh it after a source change.

The ABI regression test calls this compiled fixture directly through wasmi, since the normal
host supplies valid buffers. It checks a buffer one sample short, one sample long, a byte
length not divisible by four, overflowing pixel and sample counts, alpha with one channel,
and zero channels. Rejected inputs must leave the buffer untouched. Valid RGBA inversion
and an empty image are positive controls.

## Install

```sh
# Into a running app (control channel) or the CLI, as a command:
plugin.install {"path": "examples/plugins/invert-rs/target/wasm32-unknown-unknown/release/photocraft_plugin_invert.wasm"}
plugin.run {"id": "org.photocraft.example.invert"}
```

Or copy the `.wasm` into the folder set in Edit › Preferences › Plug-ins (with *Use Additional
Plug-ins Folder* on); it then appears under Filter › Plug-ins.
