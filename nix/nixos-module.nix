self:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.photocraft;
in
{
  options.programs.photocraft = {
    enable = lib.mkEnableOption "PhotoCraft image editor";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.photocraft or (pkgs.extend self.overlays.default).photocraft;
      defaultText = lib.literalExpression "pkgs.photocraft";
      description = "The PhotoCraft package to install.";
    };
  };

  config = lib.mkIf cfg.enable {
    nixpkgs.overlays = [ self.overlays.default ];
    environment.systemPackages = [ cfg.package ];
  };
}
