#!/usr/bin/env bash
exec nix shell --impure --expr 'with import <nixpkgs>{}; [ (python3.withPackages(p:[p.pillow p.fonttools p.brotli])) librsvg ]' -c python3 -I "$@"
