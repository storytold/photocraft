# `nix develop`: the toolchain the package builds with, plus what the workflows in AGENTS.md and
# docs/development.md use, plus debuggers and profilers.
{
  lib,
  stdenv,
  mkShell,
  photocraft,
  rustPlatform,

  # Rust: the package's toolchain. rustc includes the wasm32-unknown-unknown std, so
  # `cargo xtask wasm` works without rustup.
  cargo,
  clippy,
  rustfmt,
  rust-analyzer,
  cargo-expand,
  cargo-bloat,

  # Web build (apps/photocraft-web). trunk fetches the wasm-bindgen-cli matching Cargo.lock.
  trunk,
  binaryen,

  # Debuggers and profilers
  gdb,
  lldb,
  rr,
  valgrind,
  heaptrack,
  samply,
  perf,
  cargo-flamegraph,
  strace,
  ltrace,

  # Graphics and windowing diagnostics
  renderdoc,
  vulkan-tools,
  vulkan-validation-layers,
  mesa-demos,
  wayland-utils,
  xdpyinfo,
  xvfb-run,

  # Driving the app (docs/control-protocol.md), xtask corpus, packaging lint
  curl,
  git,
  jq,
  netcat-openbsd,
  shellcheck,
  desktop-file-utils,
  appstream,

  # The craft-fonts checkout release builds embed; CRAFT_FONTS_DIR defaults to it.
  craftFonts ? null,
}:

let
  inherit (stdenv.hostPlatform) isLinux;
  # Not every tool builds everywhere (renderdoc: x86_64 Linux; netcat-openbsd: Linux).
  available = lib.filter (lib.meta.availableOn stdenv.hostPlatform);
in
mkShell {
  inputsFrom = [ photocraft ];

  packages = available (
    [
      # Ahead of the package's cargo, which runs every command through `cargo auditable`.
      cargo
      clippy
      rustfmt
      rust-analyzer
      cargo-expand
      cargo-bloat
      trunk
      binaryen
      lldb
      samply
      curl
      git
      jq
      netcat-openbsd # macOS has its own nc
      shellcheck
    ]
    ++ lib.optionals isLinux [
      gdb
      rr
      valgrind
      heaptrack
      perf
      cargo-flamegraph
      strace
      ltrace
      renderdoc
      vulkan-tools
      vulkan-validation-layers
      mesa-demos
      wayland-utils
      xdpyinfo
      xvfb-run
      desktop-file-utils
      appstream
    ]
  );

  env = {
    RUST_SRC_PATH = "${rustPlatform.rustLibSrc}";
    RUST_BACKTRACE = "1";
  }
  // lib.optionalAttrs isLinux {
    # winit and wgpu dlopen these; the package gets them through its RUNPATH instead.
    LD_LIBRARY_PATH = lib.makeLibraryPath photocraft.runtimeLibraries;
    # Lets debug builds (where wgpu turns validation on) and VK_INSTANCE_LAYERS find
    # VK_LAYER_KHRONOS_validation.
    VK_ADD_LAYER_PATH = "${vulkan-validation-layers}/share/vulkan/explicit_layer.d";
  };

  # Like CI and releases, build with the pinned craft-fonts unless CRAFT_FONTS_DIR is already
  # set (an empty value opts out; crates/text/build.rs treats it as unset).
  shellHook = lib.optionalString (craftFonts != null) ''
    export CRAFT_FONTS_DIR="''${CRAFT_FONTS_DIR-${craftFonts}}"
  '';
}
