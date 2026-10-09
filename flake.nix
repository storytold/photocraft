{
  description = "PhotoCraft: an open-source, clean-room reimplementation of Adobe Photoshop in pure Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      crane,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachSystem
      [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ]
      (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          inherit (pkgs) lib stdenv;
          isLinux = stdenv.hostPlatform.isLinux;

          # ---------------------------------------------------------------
          # Toolchain
          #
          # The workspace needs a recent stable Rust (docs/development.md says
          # 1.95+, CI uses the latest stable). rust-overlay gives us that,
          # pinned by flake.lock. Bump with `nix flake update rust-overlay`.
          # ---------------------------------------------------------------
          mkToolchain =
            {
              extensions ? [ ],
              targets ? [ ],
            }:
            pkgs.rust-bin.stable.latest.minimal.override {
              extensions = [
                "clippy"
                "rustfmt"
              ]
              ++ extensions;
              inherit targets;
            };

          craneLib = (crane.mkLib pkgs).overrideToolchain (_: mkToolchain { });

          # Richer toolchain for the dev shell only (keeps CI closures small).
          craneLibDev = (crane.mkLib pkgs).overrideToolchain (
            _:
            mkToolchain {
              extensions = [
                "rust-src"
                "rust-analyzer"
              ];
              targets = [ "wasm32-unknown-unknown" ]; # apps/photocraft-web
            }
          );

          # ---------------------------------------------------------------
          # Source
          #
          # Everything except files that can't influence the build, so editing
          # CI, the book, or this flake doesn't invalidate the build. We do not
          # use crane's cleanCargoSource because the workspace embeds non-Rust
          # files (shaders, fonts, ICC profiles, translations, ...).
          # The *dependency* derivation is keyed only on Cargo.toml/Cargo.lock
          # (crane's dummy source), so this filter never causes dep rebuilds.
          # ---------------------------------------------------------------
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.difference (lib.fileset.fromSource (lib.sources.cleanSource ./.)) (
              lib.fileset.unions (
                map lib.fileset.maybeMissing [
                  ./.github
                  ./book
                  ./contributors
                  ./docs/nix.md
                  ./flake.nix
                  ./flake.lock
                  ./Dockerfile
                  ./.dockerignore
                ]
              )
            );
          };

          # ---------------------------------------------------------------
          # System libraries
          #
          # winit/wgpu/egui dlopen most of these at runtime, so they are also
          # put on the library path of the wrapped binary.
          # gtk3 is for native file dialogs (rfd).
          # ---------------------------------------------------------------
          runtimeLibs = lib.optionals isLinux (
            with pkgs;
            [
              libxkbcommon
              wayland
              libx11
              libxcursor
              libxi
              libxrandr
              libxcb
              vulkan-loader
              libGL
              fontconfig
              freetype
            ]
          );

          buildInputs = lib.optionals isLinux (
            runtimeLibs
            ++ (with pkgs; [
              gtk3
              alsa-lib
            ])
          );

          nativeBuildInputs = [ pkgs.pkg-config ];

          # Constant pname/version: the dependency derivation must not change
          # between commits, otherwise the dependency cache would never hit.
          commonArgs = {
            inherit src buildInputs nativeBuildInputs;
            pname = "photocraft-workspace";
            version = "0.0.0";
            strictDeps = true;
            cargoExtraArgs = "--locked -p photocraft -p photocraft-cli";
          };

          # Third-party crates only. Rebuilt only when Cargo.lock / manifests
          # (or the toolchain / system libs) change.
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          # ---------------------------------------------------------------
          # Build provenance (crates/engine/src/build_info.rs). Deliberately
          # NOT part of commonArgs, otherwise every commit would invalidate
          # cargoArtifacts. Empty means "dev build", same as the Dockerfile.
          # ---------------------------------------------------------------
          lastModified = self.lastModifiedDate or "19700101000000";
          buildDate = "${builtins.substring 0 4 lastModified}-${builtins.substring 4 2 lastModified}-${builtins.substring 6 2 lastModified}";
          provenance = {
            PHOTOCRAFT_BUILD_SHA = self.rev or "";
            PHOTOCRAFT_BUILD_DATE = if self ? rev then buildDate else "";
          };
          version = "unstable-${buildDate}";

          meta = {
            description = "Open-source, clean-room reimplementation of Adobe Photoshop in pure Rust";
            homepage = "https://getartcraft.com/apps/photocraft";
            license = with lib.licenses; [
              mit
              asl20
            ];
            platforms = lib.platforms.linux ++ lib.platforms.darwin;
          };

          desktopItem = pkgs.makeDesktopItem {
            name = "ai.storyteller.photocraft";
            desktopName = "PhotoCraft";
            genericName = "Image Editor";
            comment = "Layered image editor with real PSD support";
            exec = "photocraft %F";
            icon = "ai.storyteller.photocraft";
            terminal = false;
            categories = [
              "Graphics"
              "2DGraphics"
              "RasterGraphics"
            ];
            mimeTypes = [
              "image/vnd.adobe.photoshop"
              "image/png"
              "image/jpeg"
              "image/tiff"
              "image/webp"
            ];
            startupWMClass = "ai.storyteller.photocraft";
          };

          # Desktop app (egui/wgpu).
          photocraft = craneLib.buildPackage (
            commonArgs
            // provenance
            // {
              inherit cargoArtifacts version;
              pname = "photocraft";
              cargoExtraArgs = "--locked -p photocraft";
              doCheck = false; # tests run in `checks.photocraft-nextest`

              nativeBuildInputs =
                nativeBuildInputs
                ++ lib.optionals isLinux [
                  pkgs.makeWrapper
                  pkgs.copyDesktopItems
                ];
              desktopItems = lib.optionals isLinux [ desktopItem ];

              postInstall = lib.optionalString isLinux ''
                if [ -d ${src}/assets/app-icon/hicolor ]; then
                  mkdir -p $out/share/icons
                  # --no-preserve=mode: the source is read-only (Nix store), and
                  # crane's reference-stripping hook needs to write into $out.
                  cp -r --no-preserve=mode,ownership ${src}/assets/app-icon/hicolor $out/share/icons/
                fi
              '';

              # wgpu/winit dlopen Vulkan, GL, Wayland and X11 at runtime, so
              # they are not DT_NEEDED and stdenv's rpath shrinking would drop
              # them. This must therefore run in postFixup (after shrinking):
              #  1. add them to the binary's RUNPATH (used by winit/wgpu's dlopen),
              #  2. wrap it so libraries loaded by those libraries (Vulkan ICDs,
              #     EGL vendors) can be found too, and so GTK file dialogs (rfd)
              #     find their GSettings schemas.
              postFixup = lib.optionalString isLinux ''
                patchelf --add-rpath ${lib.makeLibraryPath runtimeLibs} $out/bin/photocraft
                wrapProgram $out/bin/photocraft \
                  --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs} \
                  --prefix XDG_DATA_DIRS : ${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name} \
                  --prefix XDG_DATA_DIRS : ${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}
              '';

              meta = meta // {
                mainProgram = "photocraft";
              };
            }
          );

          # Headless CLI / MCP server.
          photocraft-cli = craneLib.buildPackage (
            commonArgs
            // provenance
            // {
              inherit cargoArtifacts version;
              pname = "photocraft-cli";
              cargoExtraArgs = "--locked -p photocraft-cli";
              doCheck = false;

              postFixup = lib.optionalString isLinux ''
                patchelf --add-rpath ${
                  lib.makeLibraryPath [
                    pkgs.vulkan-loader
                    pkgs.libGL
                  ]
                } $out/bin/photocraft-cli
              '';

              meta = meta // {
                description = "Headless CLI and MCP server for PhotoCraft";
                mainProgram = "photocraft-cli";
              };
            }
          );

          # ---------------------------------------------------------------
          # Checks (fmt / clippy / tests) run over the whole workspace, which
          # has a different feature set than the two shipped binaries, so they
          # get their own dependency derivation.
          # ---------------------------------------------------------------
          workspaceArgs = commonArgs // {
            pname = "photocraft-workspace-all";
            cargoExtraArgs = "--locked --workspace";
          };
          cargoArtifactsAll = craneLib.buildDepsOnly workspaceArgs;
        in
        {
          packages = {
            default = photocraft;
            inherit photocraft photocraft-cli;
            # Exposed so CI can pin it as a GC root and cache it.
            photocraft-deps = cargoArtifacts;
          };

          apps = rec {
            default = photocraft;
            photocraft = flake-utils.lib.mkApp {
              drv = self.packages.${system}.photocraft;
              exePath = "/bin/photocraft";
            };
            photocraft-cli = flake-utils.lib.mkApp {
              drv = self.packages.${system}.photocraft-cli;
              exePath = "/bin/photocraft-cli";
            };
          };

          checks = {
            inherit photocraft photocraft-cli;

            photocraft-fmt = craneLib.cargoFmt {
              inherit src;
              pname = "photocraft-workspace";
              version = "0.0.0";
            };

            photocraft-clippy = craneLib.cargoClippy (
              workspaceArgs
              // {
                cargoArtifacts = cargoArtifactsAll;
                cargoClippyExtraArgs = "--all-targets -- --deny warnings";
              }
            );

            photocraft-nextest = craneLib.cargoNextest (
              workspaceArgs
              // {
                cargoArtifacts = cargoArtifactsAll;
                cargoNextestExtraArgs = "--no-fail-fast";
                # Tests that need a GPU, the corpora (`corpus` feature) or
                # craft-fonts are skipped/gated upstream; force the CPU path
                # for anything else since the sandbox has no adapter.
                env.PHOTOCRAFT_CPU_CANVAS = "1";
                preBuild = ''
                  export HOME=$TMPDIR
                '';
              }
            );
          };

          devShells.default = craneLibDev.devShell (
            {
              inherit buildInputs nativeBuildInputs;
              packages = with pkgs; [
                cargo-nextest
                trunk
                nixfmt
              ];
            }
            // lib.optionalAttrs isLinux {
              LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;
            }
          );

          formatter = pkgs.nixfmt;
        }
      );
}
