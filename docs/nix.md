# Nix development and packaging

Enable Nix's `nix-command` and `flakes` experimental features. From this checkout:

```sh
nix develop
cargo build -p photocraft -p photocraft-cli --features heif
cargo run --release -p photocraft
```

The default shell supplies Rust, Cargo, Clippy, rustfmt, rust-analyzer, Git,
pkg-config, the native compiler/SDK, and platform build dependencies. Rust comes
from the locked Nixpkgs revision (at least the workspace's Rust 1.95 minimum);
there is no separate rustup toolchain. The shell targets native development;
the WebAssembly toolchain and release signing tools are separate.

```sh
nix build                     # result/bin/photocraft and photocraft-cli
nix run                       # desktop application
nix run . -- --version
./result/bin/photocraft-cli --version
nix flake check               # evaluation, package build, CLI unit and install checks
```

Before committing new Nix files, add them to Git's index so local Git flakes can
see them. To validate untracked changes without staging, use `nix build path:.`,
`nix run path:.`, `nix develop path:.`, and `nix flake check path:.`.

The outputs cover `x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`, matching
the native release build configuration. Intel macOS is excluded because the
pinned Nixpkgs no longer supports it. Evaluation on a Mac does not validate a
Linux build or NixOS desktop integration.

The package builds both binaries from `Cargo.lock`, including its pinned Git
dependencies, with HEIF enabled as in the release scripts. `craft-fonts` is a
locked source input matching the release workflow's font revision. Fonts, UI
icons, dictionaries, translations, and ICC profiles are embedded at build time;
their licenses are installed under `share/doc/photocraft`. No sibling checkout
is required. Update inputs deliberately with `nix flake update`; keep the
`craft-fonts` URL aligned with `.github/workflows/release.yml`.

On macOS, Nixpkgs supplies the Apple SDK and frameworks. The package installs
`result/Applications/PhotoCraft.app` using the existing bundle metadata and icon.
`nix run` launches its executable directly. Its minimum macOS version follows
the pinned Nixpkgs SDK/toolchain rather than the official DMG's deployment target.
The bundle is not Developer ID signed
or notarized; this is separate from the official DMG release process.

On Linux, the package reuses the release desktop entry, MIME definitions, icons,
and AppStream metadata. Wrappers provide the X11, Wayland, xkbcommon, Vulkan,
EGL, and fontconfig libraries loaded at runtime, plus `xdg-open`. The shell
exposes the same libraries for `cargo run`. File dialogs and portal printing
need a running session bus, `xdg-desktop-portal`, and a desktop-appropriate portal
backend. Printing directly to CUPS needs the host's CUPS service.

NixOS also needs working graphics drivers, normally through
`hardware.graphics.enable = true;`. The package uses Nixpkgs' driver runpath
hook; it does not bundle a host GPU driver. On other Linux distributions, Nix
GPU libraries may need host-driver integration such as
[nixGL](https://github.com/nix-community/nixGL).

## WSLg graphics

On WSL2 with WSLg, the Nix Mesa driver may need access to the Windows-provided
D3D12 libraries. The following configuration was manually verified with an
NVIDIA GeForce RTX 4060 Ti and the locked Nixpkgs revision:

```sh
LD_LIBRARY_PATH="/usr/lib/wsl/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" GALLIUM_DRIVER=d3d12 WGPU_BACKEND=gl nix run --override-input nixpkgs github:NixOS/nixpkgs/151fa4e8ddfdd8dd25d945ad94ed54a13de9f6e4 github:nix-community/nixGL#nixGLIntel -- nix run .
```

Run this from the checkout. Despite its name, `nixGLIntel` supplies the Mesa
wrapper used here with D3D12. Keep its Nixpkgs revision aligned with `flake.lock`.
If PhotoCraft previously saved CPU rendering as a preference, select GPU
rendering and restart, or test with a fresh settings directory by prefixing the
command with `PHOTOCRAFT_CONFIG_DIR="$(mktemp -d /tmp/photocraft-gpu-test.XXXXXX)"`.
In Help > System Info, verify that the adapter names the hardware GPU and
`Image compositor` reports `GPU`; `llvmpipe` indicates software rendering.
These settings are specific to WSLg and must not be applied to native NixOS.
Successful WSLg execution does not validate NixOS desktop integration.

## NixOS installation

For declarative NixOS installation, add this repository as a flake input and use
`inputs.photocraft.packages.${pkgs.stdenv.hostPlatform.system}.default` in
`environment.systemPackages`. Desktop and portal configuration remain host
responsibilities.

## Linux/NixOS validation

Run on each Linux architecture being qualified:

```sh
nix flake check
nix build
nix run . -- --version
./result/bin/photocraft-cli --version
nix develop --command cargo build -p photocraft -p photocraft-cli --features heif
nix develop --command cargo xtask layers
nix run . -- --new-instance
```

In the GUI, verify startup and rendering under both Wayland and X11/XWayland,
Open/Save dialogs through the portal, clipboard image transfer, text with CJK
glyphs, opening a HEIC file, and saving/reopening a `.pcraft` document. Install
the package in the NixOS configuration and verify its launcher entry, icon,
MIME associations, and file-manager opening. Check printing when CUPS or a print
portal is configured. A successful `--version` check alone does not exercise
graphics, dialogs, or these desktop integrations.
