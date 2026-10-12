#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")"
# Nix supplies rust-lld inside the sysroot; rustup installations may do the same.
rust_host=$(rustc -vV | sed -n 's/^host: //p')
export PATH="$(rustc --print sysroot)/lib/rustlib/$rust_host/bin:$PATH"
cargo build --locked --release "$@"
