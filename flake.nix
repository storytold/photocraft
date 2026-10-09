{
  description = "PhotoCraft: an open-source, native image editor written in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;

      nixpkgsFor = forAllSystems (system:
        import nixpkgs {
          inherit system;
          overlays = [ self.overlays.default ];
        }
      );
    in
    {
      overlays = {
        default = final: prev: {
          photocraft = final.callPackage ./nix/package.nix { };
        };
        photocraft = self.overlays.default;
      };

      packages = forAllSystems (system: {
        default = self.packages.${system}.photocraft;
        photocraft = nixpkgsFor.${system}.photocraft;
      });

      apps = forAllSystems (system: {
        default = self.apps.${system}.photocraft;
        photocraft = {
          type = "app";
          program = "${self.packages.${system}.photocraft}/bin/photocraft";
        };
        photocraft-cli = {
          type = "app";
          program = "${self.packages.${system}.photocraft}/bin/photocraft-cli";
        };
      });

      devShells = forAllSystems (system:
        let
          pkgs = nixpkgsFor.${system};
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ pkgs.photocraft ];
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer
            ];
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };
        }
      );

      nixosModules = {
        default = import ./nix/nixos-module.nix self;
        photocraft = self.nixosModules.default;
      };

      homeManagerModules = {
        default = import ./nix/home-manager-module.nix self;
        photocraft = self.homeManagerModules.default;
      };
      homeModules = self.homeManagerModules;
    };
}
