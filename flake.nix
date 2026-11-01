{
  description = "PhotoCraft — a native image editor in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Keep aligned with the release workflow's craft-fonts-ref.
    craft-fonts = {
      url = "github:storytold/craft-fonts/abb83316d96aa59c1cf64784289e378fe9fa5695";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      craft-fonts,
    }:
    let
      inherit (nixpkgs) lib;
      # The release scripts build these architectures. Current nixpkgs dropped Intel Darwin.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: lib.genAttrs systems (system: f (import nixpkgs { inherit system; }) system);
      date = self.lastModifiedDate or "19700101000000";
      buildDate = "${builtins.substring 0 4 date}-${builtins.substring 4 2 date}-${builtins.substring 6 2 date}";
    in
    {
      packages = forAllSystems (
        pkgs: _: rec {
          photocraft = pkgs.callPackage ./nix/package.nix { inherit craft-fonts buildDate systems; };
          default = photocraft;
        }
      );
      apps = forAllSystems (
        _: system: {
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/photocraft";
            meta.description = "Launch PhotoCraft";
          };
        }
      );
      devShells = forAllSystems (
        pkgs: system: {
          default = pkgs.mkShell (
            {
              inputsFrom = [ self.packages.${system}.default ];
              packages =
                with pkgs;
                [
                  cargo
                  rustc
                  clippy
                  rustfmt
                  rust-analyzer
                  git
                ]
                ++ self.packages.${system}.default.runtimeTools;
              CRAFT_FONTS_DIR = toString craft-fonts;
              CRAFT_FONTS_REQUIRED = "1";
            }
            // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
              LD_LIBRARY_PATH = lib.makeLibraryPath self.packages.${system}.default.runtimeLibs;
            }
          );
        }
      );
      checks = forAllSystems (
        _: system: {
          package = self.packages.${system}.default;
        }
      );
      formatter = forAllSystems (pkgs: _: pkgs.nixfmt);
    };
}
