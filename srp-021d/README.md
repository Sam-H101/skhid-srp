# srp-021d: the server for Survarium 0.21d

The server side for the 0.21d client (build 2010, April 2014, the last build
that shipped with a PDB). It was hosted on skhid.link from 2026-10-10 to 2026-10-12 and is not hosted
now: that name leads to the 0.26g server since.

One process, `srp`, is every server a client talks to:

```
survarium.exe -client=<host>:1234
      |
      1. login    TCP 1234   TLS sign-in; the first sign-in of a name registers it
      2. browser  HTTP 80    tells the client where the lobby is
      3. lobby    TCP 1235   account, inventory, shop, skills, quests, squads, the
      |                      queue; chat on the same port
      4. match    UDP 1236   lockstep: the server relays inputs and compares world
                             hashes; the world itself comes from a patched client
      web page    HTTP 8088  home, download, silver for gold, the admin page
```

A 0.21d match is **lockstep**: every client simulates the game, the server
relays their inputs, has them vote on world hashes and resyncs a client that
falls out of step. It does not simulate the game, so a client must be able to
send its world: that is what the client patch adds (`client-patches/`).

Accounts, inventories and sessions are kept in SQLite (`srp.db`), with
argon2id password hashes; results are decided from the snapshots the majority
of clients agree on.

## Build, run, test

A recent Rust toolchain and OpenSSL (`flake.nix` gives a dev shell with both).

```bash
cargo run -p srp
cargo test --workspace       # 320 tests
```

On Windows, point cargo at OpenSSL first:

```bash
export OPENSSL_DIR="C:/Program Files/OpenSSL-Win64-3" OPENSSL_STATIC=true OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64-3/lib/VC/x64/MD"
```

By default everything listens on 127.0.0.1 and the data goes to `./data`.
Every setting is an `SRP_*` environment variable with a default
(`docs/SRP-021D.md`, "Settings").

The game: the original 0.21d with the patched exe and the certificate of
`client-patches/`, started from `game\binaries\x86` with

```
survarium.exe -localization=english -client=127.0.0.1:1234 -minimum_resources_memory_size=768
```

A match needs two players who did not come as one squad.

## Where things are

| Path | What |
|---|---|
| `docs/SRP-021D.md` | The server: its parts, how a player gets into a match, the settings |
| `docs/MATCH-SYNC-021D.md` | How a lockstep match stays in step, why clients fall out and how they are resynced |
| `docs/CLIENT-FACTS-021D.md` | Verified facts about the 0.21d client |
| `docs/OPEN-ISSUES.md` | What is broken, unexplained or not done |
| `docs/DEPLOY.md` | Hosting in Docker: the files, the settings, updating |
| `docs/notes/` | The protocol and client research, message by message (`README.md` first) |
| `client-patches/` | The patch tools for the original exe, the login certificate, and what each patch does |
| `Dockerfile`, `docker-compose.yml`, `deploy/env-021d.example` | The Docker files |
| `servers/`, `crates/`, `scripts/` | The code; each server is a library crate, `servers/srp` joins them |
| `certs/` | The login server's certificate and key |
| `resources/binary-config-examples/` | Game data the code embeds |
