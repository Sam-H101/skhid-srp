#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
# Nix supplies rust-lld inside the sysroot; rustup installations may do the same.
rust_host=$(rustc -vV | sed -n 's/^host: //p')
export PATH="$(rustc --print sysroot)/lib/rustlib/$rust_host/bin:$PATH"
cargo build --locked --release "$@"
