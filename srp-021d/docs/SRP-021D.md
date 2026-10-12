# srp-021d: the server for client 0.21d

> **Version scope: everything on this page is for client 0.21d** (build 2010).
> The 0.100b server is in SRP-0100B.md. Another client version needs its own
> protocol research: login, lobby and match formats all changed between
> 0.100b and 0.21d.

This folder is what ran on skhid.link from 2026-10-10 to 2026-10-12 19:48 UTC. The server is stopped since (its data is kept), its host is the 0.26g test server, and the name skhid.link leads to the 0.26g server. Client 0.21d is build 2010 (April
2014), the last build with a PDB.

## The parts

One process, `srp` (`servers/srp`), runs everything. Each server is a library
crate.

| Crate | Does |
|---|---|
| `servers/login-server` | TLS sign-in (TCP 1234). First sign-in of a name registers it; argon2id passwords; 5 wrong in a row lock the name for 60 s; ban checks; sign-in and registration rate limits; maintenance and closed-registration messages. UDP 1234 presence pings |
| `servers/browser-server` | HTTP `/hello` on 80: tells the client the lobby address. One request per connection (fixes a wedge after restarts) |
| `servers/lobby-server` | Lobby (TCP 1235): pushes profiles, inventory, shop and prices, money, skills, quests, premium, loyalty, rating, supplies; equip, buy and sell, skills, rerolls; matchmaking and squads (`lobby_server/`). Chat on the same port (`messaging_server/`): one general chat for every client language |
| `servers/match-server` | Matches (UDP 1236): reliable transport (`transport.rs`), lockstep input relay with hash voting, snapshots and resyncs (`game.rs`), match setup and results (`server.rs`). See MATCH-SYNC-021D.md |
| `servers/web-server` | The web page (8088 behind caddy): download, silver for gold, the admin page (balances, bans) on `SRP_ADMIN_PATH` for `SRP_ADMIN_IPS` |
| `crates/session` | Accounts, sessions, matchmaking (`matchmaking.rs`), squads, bans, limits, persistence (`persist/sqlite.rs`: `srp.db`, backed up at every start) |
| `crates/survarium` | Game data and wire formats: item dictionary, prices, `v021d/` (lobby messages, 0x51 match start, results, drops, quests, rating, premium, loyalty, supplies, skills) |
| `crates/binary-config-parser` | Reads the game's binary configs (`ConfigValue`) |
| `crates/vostok` | Shared protocol plumbing (TCP client with `TCP_USER_TIMEOUT` 30 s, packets, config) |

Game data the code embeds, in `resources/binary-config-examples/`:
`db_static_dictionaries_021d`, `test_player_profiles_021d.options`,
`drops_021d.options`, `quests_021d.options`, `db_skills_tree_021d`.

## How a player gets into a match

1. Login (TLS) → browser `/hello` → lobby. The lobby pushes everything unasked.
2. Play → the matchmaker queues the player. The gather window (60 s) starts
   when a second player queues who did not come in the same squad (a squad
   alone waits for somebody who is not of it, as a player alone waits; since
   2026-10-12), and the match starts when it ends. It starts
   sooner if 10 or more are waiting (`DEFAULT_FILL_PLAYERS`, 5 v 5) and nobody
   has joined for 10 s. At most 20 players. Squads stay together and teams are
   balanced. Match ids count from 0x123 after each restart.
3. The seat goes out as `0x33` (match id, team, address) on **whatever lobby
   connection the session has**. Before 2026-10-11 20:03 it went only to the
   connection that queued, which left players on "Match-making complete".
4. The client connects to 1236. The server sends 0x51 (match options plus
   every player's profile; the roster is fixed for the match), then a world
   snapshot to start from. See MATCH-SYNC-021D.md.
5. At the end, the results come from the snapshots the majority agrees on, so
   one client can't forge a win. Experience, money, reputation, drops and
   quests follow.

Rewards: experience = score / 10, +10 for finishing, +20 for a win, × `SRP_EXPERIENCE_RATE`
(default 3); money 5 × experience; reputation goes to the faction of the
gear; drops come from `drops.options` (progress = score × 10); daily quests
come from `quests.options`.

## Settings (`SRP_*` environment variables)

Addresses: `SRP_BIND_HOST`, `SRP_PUBLIC_HOST`, `SRP_MATCH_PUBLIC_HOST`, per-server `*_BIND`/`*_PORT`, `SRP_LOCAL_PEERS`.
Data: `SRP_DATA_DIR`, `SRP_DB` (`off` = memory only), `SRP_SNAPSHOT_DIR`, `SRP_LOG_FILES`.
Matches: `SRP_MATCH_GATHER_SECONDS`, `SRP_MATCH_FILL_PLAYERS`, `SRP_MATCH_QUIET_SECONDS`, `SRP_MATCH_MODES`.
Economy: `SRP_EXPERIENCE_RATE`.
Abuse: `SRP_SIGN_INS_PER_MINUTE`, `SRP_REGISTRATIONS_PER_DAY`, `SRP_REGISTRATION` (open or closed).
Web: `SRP_WEB_*`, `SRP_ADMIN_TOKEN`, `SRP_ADMIN_PATH`, `SRP_ADMIN_IPS`, `SRP_DOWNLOAD_FILE`, `SRP_DOWNLOAD_URL`, `SRP_DOWNLOAD_SHA256`, `SRP_OTHER_SERVER_URL` (a plain `https://` address: a link to the 0.26g server's page in every page's header; live since 2026-10-12), `SRP_GAME_URL` (where the original game is had: the home page's first step).

## Build and test on Windows

Building `login-server` and `lobby-server` (and the workspace) needs OpenSSL:

```bash
export OPENSSL_DIR="C:/Program Files/OpenSSL-Win64-3" OPENSSL_STATIC=true OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64-3/lib/VC/x64/MD"
```

```bash
cd srp-021d && cargo test -p match-server -p lobby-server -p survarium -p session
```

On 2026-10-12 the whole workspace (`cargo test --workspace`) stood at 317
passing, 3 ignored.
`match-server` and `survarium` build without OpenSSL. Format only the files
you touched (`rustfmt --edition 2024 <file>`): `cargo fmt` would reformat
unrelated files.

Local run: `cargo run -p srp` in `srp-021d`.

## The client patch

`scripts/client-patch-021d/patch.py <survarium.exe> <survarium_srp.exe>
<snapshot_upload.bin> <battery_spawn.bin>` builds the patched client. Each
site is checked against the original bytes first. It patches:
- the single-instance mutex check, so several clients can run on one PC;
- **snapshot upload (option B)**: code in the padding after `.text` plus two
  hooks, so a client sends its world when the server asks (0x59). The stock
  client cannot start a match without a snapshot from the server, and nothing
  else in it can produce one. See `resources/notes/v021d/option_b.md`;
- mode-3 batteries: a new `match_time_vi_spawn_rule::tick`.

Players get the patched exe in `Survarium-0.21d-srp-files.zip` (4.2 MB:
the four files that go over the original game; since 2026-10-12 the site
hands out only these, with a link to the original game on the web archive).
