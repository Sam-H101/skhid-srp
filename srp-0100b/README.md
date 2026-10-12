# srp-0100b: the server for Survarium 0.100b

The server side for the 0.100b client (early 2013). This is the SRP project's
server ([srp-survarium/srp](https://github.com/srp-survarium/srp), where its
own README, plan and development notes are) with the game-mode work described
in `docs/SRP-0100B.md` on top: match lifecycle, batteries and team spawns
from the map's data, real combat, traps, late join, lobby prices and rewards.

Unlike 0.21d and 0.26g, a 0.100b match is **server-authoritative**: the server
runs combat, spawns, batteries and traps itself.

Not hosted anywhere; it runs locally.

## Build, run, test

It builds on Linux and Windows with a recent Rust toolchain and OpenSSL
(`flake.nix` gives a dev shell with both: `nix develop`).

```bash
cargo run --bin srp          # login 1234/tcp, browser 80/tcp, lobby 1235/tcp, match 1236/udp
cargo test --workspace       # 322 tests
```

On Windows, point cargo at OpenSSL first:

```bash
export OPENSSL_DIR="C:/Program Files/OpenSSL-Win64-3" OPENSSL_STATIC=true OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64-3/lib/VC/x64/MD"
```

Addresses are settings (`SRP_BIND_HOST`, `SRP_PUBLIC_HOST`, per-server
`SRP_<SERVER>_{BIND,PUBLIC_HOST,PORT}`; `crates/vostok/src/config.rs`). By
default everything listens on 127.0.0.1.

The game: the original 0.100b client with the login certificate of
`client-patches/` in place, started with

```
survarium.exe -no_splash_screen -client=127.0.0.1:1234
```

## Where things are

| Path | What |
|---|---|
| `docs/SRP-0100B.md` | What the server has, what is not done, the gotchas |
| `docs/notes/` | Research: the weapon state machine and its timing, aim sway, traps, late join, prices, rewards, server notices, sessions and friends |
| `docs/test-report-2026-10-09.md` | A test report of that day |
| `client-patches/` | The login certificate (needed) and optional client patches, each described |
| `servers/`, `crates/`, `scripts/` | The code: `servers/srp` runs every server in one process |
| `resources/` | Game data the code embeds, and the SRP project's IDA, WinDbg and patch scripts |

Scripted test clients for this server are in the repository's
`tools/re-kit/v0100b/e2e/`.
