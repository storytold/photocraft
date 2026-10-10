{
  description = "PhotoCraft: an open-source, native image editor";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Fonts embedded in release builds (crates/text/build.rs). Keep this commit equal to
    # CRAFT_FONTS_REF in .github/workflows/release.yml and the craft-fonts ref in ci.yml (the
    # craft-fonts-pin check fails otherwise), then run `nix flake update craft-fonts`.
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

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = f: lib.genAttrs systems (system: f system nixpkgs.legacyPackages.${system});

      # What CI sets for release builds, taken from this flake's source: the commit (with a
      # -dirty suffix for uncommitted changes) and its date.
      date = self.lastModifiedDate;
      buildArgs = {
        craftFonts = craft-fonts;
        buildSha = self.rev or self.dirtyRev or null;
        buildDate = "${lib.substring 0 4 date}-${lib.substring 4 2 date}-${lib.substring 6 2 date}";
      };
    in
    {
      overlays.default = final: _prev: {
        photocraft = final.callPackage ./nix/package.nix buildArgs;
      };

      packages = forAllSystems (
        system: pkgs: {
          photocraft = pkgs.callPackage ./nix/package.nix buildArgs;
          # Dev profile: debug assertions, overflow checks, full debug info, not stripped.
          photocraft-debug = self.packages.${system}.photocraft.override { buildType = "debug"; };
          default = self.packages.${system}.photocraft;
        }
      );

      apps = forAllSystems (
        system: _pkgs:
        let
          photocraft = self.packages.${system}.photocraft;
        in
        {
          default = {
            type = "app";
            program = lib.getExe photocraft;
            meta.description = "PhotoCraft image editor";
          };
          photocraft-cli = {
            type = "app";
            program = lib.getExe' photocraft "photocraft-cli";
            meta.description = "Headless PhotoCraft: convert, inspect, run commands, batch, MCP server";
          };
        }
      );

      devShells = forAllSystems (
        system: pkgs: {
          default = pkgs.callPackage ./nix/devshell.nix {
            inherit (self.packages.${system}) photocraft;
            craftFonts = craft-fonts;
          };
        }
      );

      checks = forAllSystems (
        system: pkgs: {
          inherit (self.packages.${system}) photocraft;
          devShell = self.devShells.${system}.default;

          craft-fonts-pin = pkgs.runCommand "craft-fonts-pin" { } ''
            for f in release ci; do
              grep -q ${craft-fonts.rev} ${./.github/workflows}/$f.yml || {
                echo "error: the craft-fonts flake input (${craft-fonts.rev}) is not the commit" \
                  ".github/workflows/$f.yml pins; update flake.nix and run 'nix flake update craft-fonts'" >&2
                exit 1
              }
            done
            touch $out
          '';

          nixfmt = pkgs.runCommand "nixfmt-check" { nativeBuildInputs = [ pkgs.nixfmt ]; } ''
            nixfmt --check ${./flake.nix} ${./nix/package.nix} ${./nix/devshell.nix}
            touch $out
          '';
        }
      );

      formatter = forAllSystems (_system: pkgs: pkgs.nixfmt-tree);
    };
}
