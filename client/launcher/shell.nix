{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  packages = with pkgs; [ cargo rustc rustfmt pkg-config ];
  buildInputs = with pkgs; [ gtk3 webkitgtk_4_1 libsoup_3 ];
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.libglvnd ] + ":/run/opengl-driver/lib";
}
