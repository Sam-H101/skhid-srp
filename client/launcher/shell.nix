{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  packages = with pkgs; [ cargo rustc rustfmt pkg-config ];
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
    wayland libxkbcommon libglvnd
    xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr
  ]) + ":/run/opengl-driver/lib";
}
