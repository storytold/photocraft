# Building and running with Nix

The repository ships a [flake](https://nix.dev/concepts/flakes) that builds the
desktop app and the headless CLI, runs the checks, and provides a development
shell. It uses [crane](https://github.com/ipetkov/crane) and
[rust-overlay](https://github.com/oxalica/rust-overlay), so third-party
dependencies are built once and reused until `Cargo.lock` changes.

## Requirements

[Nix](https://nixos.org/download) with the `nix-command` and `flakes`
experimental features enabled:

```
# ~/.config/nix/nix.conf  (or /etc/nix/nix.conf)
experimental-features = nix-command flakes
```

Linux (x86_64, aarch64) is the primary target. The flake also lists
x86_64-darwin and aarch64-darwin, but those are not exercised in CI.

## Run without cloning

```
nix run github:storytold/photocraft -- path/to/image.psd          # desktop app
nix run github:storytold/photocraft#photocraft-cli -- --help      # headless CLI
nix run github:storytold/photocraft#photocraft-cli -- mcp         # MCP server
```

## Build from a checkout

```
git clone https://github.com/storytold/photocraft
cd photocraft

nix build .#photocraft          # ./result/bin/photocraft
nix build .#photocraft-cli      # ./result/bin/photocraft-cli
nix run .# -- image.psd         # same as `nix run .#photocraft`
```

The first build compiles every dependency from source and takes a while. Later
builds only recompile the workspace crates unless `Cargo.lock`, the flake
inputs or the system libraries change. There is no public binary cache; CI
caches its own store between runs, but that cache is not available to you.

| Output                           | What it is                                         |
| -------------------------------- | -------------------------------------------------- |
| `packages.<system>.photocraft`     | Desktop app, with a `.desktop` entry and icons     |
| `packages.<system>.photocraft-cli` | Headless CLI and MCP server                        |
| `packages.<system>.photocraft-deps`| Dependency artifacts only (used for caching in CI) |
| `apps.<system>.*`                | `nix run` entry points for the two binaries        |
| `checks.<system>.*`              | Build, `rustfmt`, `clippy` and `nextest`           |
| `devShells.<system>.default`     | Development shell                                  |

## Development shell

```
nix develop
cargo run --release -p photocraft -- image.psd
cargo test --workspace
cargo xtask ci
```

The shell provides the pinned Rust toolchain (with `rust-src`, `rust-analyzer`,
`clippy`, `rustfmt` and the `wasm32-unknown-unknown` target), `cargo-nextest`,
`trunk`, `nixfmt`, and all the system libraries the app loads at runtime, with
`LD_LIBRARY_PATH` set so `cargo run` finds Vulkan, GL, Wayland and X11.

If you use [direnv](https://direnv.net), put `use flake` in `.envrc`.

The web build (`apps/photocraft-web`) is not packaged by the flake. The shell
gives you `trunk` and the wasm target for the steps in
[development.md](development.md#web-build); `trunk` downloads its own
`wasm-bindgen` and `wasm-opt`, which may not run unmodified on NixOS.

## Checks

```
nix flake check            # build + fmt + clippy + tests, in the Nix sandbox
nix build .#checks.x86_64-linux.photocraft-clippy
nix fmt                    # format flake.nix with nixfmt
```

The tests run without a GPU in the sandbox. Tests behind the `corpus` feature
and the craft-fonts tests are not run there; use `cargo xtask test-corpus` in
the development shell for those (see [development.md](development.md#test-corpora)).

## Installing

Add the flake as an input and install the package.

NixOS:

```nix
{
  inputs.photocraft.url = "github:storytold/photocraft";

  # in your configuration:
  environment.systemPackages = [
    inputs.photocraft.packages.${pkgs.stdenv.hostPlatform.system}.photocraft
  ];
}
```

The same package works in Home Manager's `home.packages`.

## Graphics drivers

PhotoCraft renders with wgpu (Vulkan, falling back to GL).

- **NixOS:** enable `hardware.graphics.enable = true;`.
- **Other Linux distributions:** Nix-built graphics programs can't see your
  system's GPU drivers by default. Run the app through
  [nixGL](https://github.com/nix-community/nixGL), for example
  `nixGL nix run github:storytold/photocraft`, or use `--safe-gpu` to start
  with the CPU renderer.

Wayland drag and drop is a winit limitation, not a Nix one; see the
[README](../README.md#get-started) for the XWayland workaround.

## Build provenance

The Git revision and commit date are passed to the build as
`PHOTOCRAFT_BUILD_SHA` and `PHOTOCRAFT_BUILD_DATE`, so About and `--version`
show them. Builds from a dirty tree report themselves as dev builds.

## Updating

```
nix flake update                 # all inputs
nix flake update rust-overlay    # just the Rust toolchain
```

Rust comes from `rust-overlay`'s latest stable, pinned by `flake.lock`. The
workspace needs Rust 1.95 or newer.

## Troubleshooting

- **`error: experimental Nix feature 'flakes' is disabled`**: see
  [Requirements](#requirements).
- **Window doesn't open / no Vulkan adapter on a non-NixOS system**: see
  [Graphics drivers](#graphics-drivers).
- **A build needs a native library that isn't listed**: add it to `buildInputs`
  (and to `runtimeLibs` if it is loaded with `dlopen`) in `flake.nix`.
