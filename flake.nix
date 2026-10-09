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

    # The Android targets' standard library for the Android build (nix/android.nix). nixpkgs'
    # rustc only ships the host's.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      craft-fonts,
      rust-overlay,
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

      # The Android build runs on x86_64 Linux only: the NDK and build tools nixpkgs packages are
      # x86_64 Linux binaries. Building it accepts the Android SDK licence
      # (https://developer.android.com/studio/terms), which the SDK and NDK are distributed under.
      androidSystems = [ "x86_64-linux" ];
      androidPkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
          config = {
            android_sdk.accept_license = true;
            # The SDK's parts (nixpkgs' androidenv, all with this homepage) are unfree; allow those
            # and nothing else.
            allowUnfreePredicate =
              pkg: lib.hasPrefix "https://developer.android.com/" (pkg.meta.homepage or "");
          };
        };
      androidPackages =
        system:
        let
          pkgs = androidPkgsFor system;
          android = pkgs.callPackage ./nix/android.nix buildArgs;
        in
        lib.optionalAttrs (lib.elem system androidSystems) {
          # Phones and tablets (arm64-v8a).
          photocraft-android = android;
          # The Android emulator on an x86_64 host.
          photocraft-android-x86_64 = android.override { abis = [ "x86_64" ]; };
          photocraft-android-sign = pkgs.callPackage ./nix/android-sign.nix { photocraft-android = android; };
        };
    in
    {
      overlays.default = final: _prev: {
        photocraft = final.callPackage ./nix/package.nix buildArgs;
      };

      packages = forAllSystems (
        system: pkgs:
        {
          photocraft = pkgs.callPackage ./nix/package.nix buildArgs;
          # Dev profile: debug assertions, overflow checks, full debug info, not stripped.
          photocraft-debug = self.packages.${system}.photocraft.override { buildType = "debug"; };
          default = self.packages.${system}.photocraft;
        }
        // androidPackages system
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
        // lib.optionalAttrs (lib.elem system androidSystems) {
          photocraft-android-sign = {
            type = "app";
            program = lib.getExe self.packages.${system}.photocraft-android-sign;
            meta.description = "Sign the Android APK with the debug key; --install also installs it with adb";
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
            nixfmt --check ${./flake.nix} ${./nix/package.nix} ${./nix/devshell.nix} ${./nix/android.nix} ${./nix/android-sign.nix}
            touch $out
          '';
        }
      );

      formatter = forAllSystems (_system: pkgs: pkgs.nixfmt-tree);
    };
}
