{
  description = "Experimental Nix flake for PhotoCraft";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rustfmt" "clippy" "rust-src" ];
        };

        commonDeps = with pkgs; [
          pkg-config
          openssl
          libxkbcommon
          xorg.libX11
          xorg.libXcursor
          xorg.libXrandr
          xorg.libXi
          xorg.libxcb
          libGL
          vulkan-loader
          mesa
          wayland
          wayland-protocols
          gtk3
          fontconfig
          freetype
          alsa-lib
          zlib
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rust
            cmake
            ninja
            libclang
          ];
          buildInputs = commonDeps;

          shellHook = ''
            export LIBCLANG_PATH="${pkgs.libclang.lib}/lib"
            export PKG_CONFIG_PATH="${pkgs.openssl.dev}/lib/pkgconfig:$PKG_CONFIG_PATH"
          '';
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "photocraft";
          version = "0.5.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
            cmake
            ninja
            libclang
          ];
          buildInputs = commonDeps;

          doCheck = false;
        };

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/photocraft";
        };
      });
}