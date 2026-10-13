{
  description = "Skhid / Survarium 0.26g native launcher with internal Wine runtime";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; config.allowUnfreePredicate = p: nixpkgs.lib.getName p == "skhid"; };
      launcher = pkgs.callPackage ./nix/launcher.nix { };
      game = pkgs.lib.makeOverridable (import ./nix/game.nix { inherit pkgs launcher; }) { };
      module = target: { config, lib, pkgs, ... }:
        let cfg = config.programs.skhid;
            package = cfg.package.override { inherit (cfg) game locale site; };
        in {
          options.programs.skhid = {
            enable = lib.mkEnableOption "Survarium 0.26g with the native Skhid launcher";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              description = "Game wrapper package; game, locale and site are applied with override.";
            };
            game = lib.mkOption {
              type = lib.types.nullOr lib.types.path;
              default = null;
              description = "Original 0.26g game folder or archive. Null fetches the pinned archive.org copy. A fetchurl derivation works here, as in the HOMM modules.";
            };
            locale = lib.mkOption {
              type = lib.types.enum [ "en" "ru" "ua" "de" "pl" ];
              default = "en";
              description = "Initial launcher and game language; subsequent choices are remembered per user.";
            };
            site = lib.mkOption {
              type = lib.types.str;
              default = "https://skhid.link";
              description = "HTTPS site providing the server page and Skhid updates.";
            };
          };
          config = lib.mkIf cfg.enable (if target == "nixos" then {
            environment.systemPackages = [ package ];
            hardware.graphics.enable = true;
          } else { home.packages = [ package ]; });
        };
    in {
      packages.${system} = { inherit launcher; skhid = game; default = game; };
      apps.${system}.default = { type = "app"; program = "${game}/bin/skhid"; };
      nixosModules.default = module "nixos";
      homeManagerModules.default = module "home-manager";
      devShells.${system}.default = import ./client/launcher/shell.nix { inherit pkgs; };
    };
}
