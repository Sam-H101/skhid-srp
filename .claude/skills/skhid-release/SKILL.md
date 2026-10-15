---
name: skhid-release
description: Release a change of skhid-srp (the Survarium 0.26g server "Skhid") to players — game exe patch, launcher, updater, crash reporter, resources.db regions, new content (items, maps: a patch for clients and a server version for the server), or server code. Covers what goes out in which order, building and checking the players' ZIP, the test server, and the live host. Use when asked to release, ship, deploy or roll out a change of skhid-srp, or to test one before players get it.
---

# Releasing to Skhid 0.26g players

## Private values

Every host, address and local path is a setting, never written here. Their
format is `release.env.example` next to this file; the filled-in copy is
`private/release/release.env` in the main checkout (never published). Load it
first; if it's missing, ask the owner.

```sh
. "$(git rev-parse --path-format=absolute --git-common-dir)/../private/release/release.env"
# One command on the live host (through the jump host, which holds its key):
live() { ssh "$SKHID_JUMP" "ssh $SKHID_LIVE $(printf %q "$1")"; }
# e.g.  live "cd $SKHID_LIVE_DIR && docker compose ps"
#       live "cat > $SKHID_LIVE_DIR/download/x.tmp" < x      (upload)
```

**Every command on the live host needs the owner's go-ahead first.** Writes
players feel (replacing the ZIP, `docker compose build/up`, editing `.env`)
need an explicit yes each time, with the exact command shown. Never print the
live `.env`; read single settings with `grep -nE '^(NAME|NAME2)=' .env`.

The live host's tree, `$SKHID_LIVE_DIR` (a copied source tree, not a git
checkout): `.env` (mode 600, holds the admin token), `docker-compose.yml`
(project `srp26`: containers `srp26-srp26-1`, `srp26-caddy26-1`),
`download/Survarium-0.26g-srp-files.zip` (what `/download` serves),
`download/resources/<sha256>` (resource blobs), `old-downloads/` (every earlier
ZIP, `Survarium-0.26g-srp-files-YYYYMMDD-HHMM-<label>.zip`), `old-logs/`.
Data: the Docker volume `srp26_srp26-data`; never remove it. The test server is
the same layout in `$SKHID_TEST_DIR`, deployed from `main` by the GitHub
workflow "Deploy the test server".

## What can be in a release, and what each needs

| Change | Where it lives | Goes out by | Restart? |
|---|---|---|---|
| Server code (lobby, match, login, web) | `srp-026g/servers`, `crates` | copy source to `$SKHID_LIVE_DIR`, `docker compose build srp26`, `up -d --no-deps srp26` | **yes**: ends every match, drops lobby links. Only with no match running (`curl -s $SKHID_SITE/api/stats` → `"matches":0`) |
| Game exe patch (`patch_026g.py`, asm) | `srp-026g/client-patches` | new ZIP | no |
| Launcher | `client-patches/launcher` | new ZIP; the updater replaces the running launcher | no |
| Updater, 7zr pin, game mirror | `client/updater` | new ZIP; the old updater installs the new one | no |
| Crash reporter | `client/bugtrap` | new ZIP | no |
| `resources.db` regions (Flash menus) | `--resource-region=<name>=<file>` of `make_client_026g.py` | blobs first, then the ZIP | no |
| New items or maps | content packs → a patch (ZIP + blobs) and a server version | see "New content" | yes |
| Launcher page, site texts | `servers/web-server` | server deploy | yes |
| Refuse old games | `.env`: `SRP_MIN_PATCH_REVISION=<n>` (unset or 0: no check) | edit `.env`, `docker compose up -d srp26` | yes |

The ZIP is picked up the moment its file changes (`/api/client-files` rereads
it): no restart. Every launcher start compares that checksum and installs.

## Order

1. **Server before client** when the client needs something the server must
   answer. A new server must keep working with the clients already out there.
2. **Blobs before ZIP**: upload them and fetch each back
   (`curl -sI $SKHID_SITE/resources/<sha>` → 200).
3. **ZIP before gate**: raise `SRP_MIN_PATCH_REVISION` only once the ZIP with
   that revision is live, else nobody can play.
4. Keep originals' blobs on the server while any player may still have a
   region changed (dropping a region from a later ZIP restores from them).
5. Server restarts only with no match running; tell the owner how many players
   are online before asking.

## Maintenance window (any server restart)

Login, lobby, match and web are one process (`srp`, container
`srp26-srp26-1`): a restart ends every match (no results), drops every lobby
link, and loses the queue and squads. Games sign in again by themselves
within seconds; results not shown yet are kept (`srp.db`). The launcher page
and `/download` are briefly down too.

What a running game shows (`docs/notes/transport.md` §6): after 5 s without
the server a "server sync" overlay; it retries with the same session every
5 s, only while its window is active, and a restored session puts it straight
back in the lobby. After five failed tries (~30 s) it signs out to the login
screen. **Keep the downtime well under 30 s**: build the image first, so the
outage is just `up -d`.

- **Where**: the admin page, tab "Maintenance" (`/<SRP_ADMIN_PATH>?tab=maint`,
  served only to `SRP_ADMIN_IPS`, every request with `token=<SRP_ADMIN_TOKEN>`;
  all three in the live `.env`, never printed). It takes `minutes` (1–10080)
  or `cancel=1`. The owner plans it from their browser; confirm it on the
  public launcher page (`curl -s $SKHID_SITE/launcher/places/en`).
- **What players see**: the lobby (`0x9F`) and the launcher page show it
  within seconds; under 60 minutes the game shows the minutes left; at 30 or
  less **Play is locked**; at 0 the notice stays and Play stays off.
- **In memory only** (`SessionStore::maintenance_at`): the restart clears it.
  After a restart that didn't happen, cancel it by hand.
- **Sequence**: plan 45 minutes → wait for `"matches":0` (or the time) →
  `up -d` (image built before) → check that players sign in again. A restart
  that only changes settings (`SRP_MIN_PATCH_REVISION`) uses the same window.

## Build the players' ZIP

Always from a fresh worktree of `origin/main` after the PRs are merged (the
owner's checkout usually has work in progress; never `git stash`).

```sh
git worktree add <scratch>/rel origin/main && cd <scratch>/rel
nix develop ./srp-026g -c bash -c 'client/bugtrap/build.sh && bash client/updater/build.sh'
python3 srp-026g/client-patches/make_client_026g.py "$SKHID_GAME" \
  <scratch>/Survarium-0.26g-srp-files.zip --pack-zip \
  --server=$SKHID_GAME_SERVER --localization=$SKHID_LOCALIZATION \
  --resource-region=main_menu=<target> --resource-region=lobby_menu=<target> \
  --resource-region=login_menu=<target> --resources-out=<scratch>/blobs
```

`bugtrap.dll` and `updater_skhid.exe` build reproducibly
(`client/check-reproducible.sh`), so they're built fresh.

**Reproduce the live regions**, or players' menus get reverted. Read the plan
from the live ZIP (public: `curl -o live.zip $SKHID_SITE/download`, then
`unzip -p live.zip game/binaries/x86/updater_skhid.resources`). Line format:
`region <path> <file size> <offset> <size> <original sha256> <target sha256> <name>`.
Fetch each target as its entry file (`curl -o x $SKHID_SITE/resources/<target sha>`)
and check its sha256. A release that changes a region passes the new entry
file instead and uploads the new blobs from `--resources-out`.

## New content (items, maps): a patch for clients, a server version for the server

One release build, from one list of what goes in, gives two outputs:

| Who | Gets | What it is |
|---|---|---|
| Players | **the patch**: the ZIP + its blobs | a plan that grows `resources.db`: a signed new head (its file table, `[0, ~10.6 MB)`) and a tail appended at its end, plus the lobby menu with the new icons |
| The server | **the server version**: `server-game-data.db` (a few MB) | the `gameplay/` files srp reads, extracted from stock + the patch, with the patch's provenance inside |

The full new `resources.db` (stock + patch, 6.3 GB) is only an intermediate:
computed in memory to make and check both outputs, written to disk only for a
local test with a real game. Nobody receives it; the live host never stores it.

1. **Decide the items**: the content packs and versions that go in (from
   `$SKHID_PACKS`). A pack comes from an item builder in resource-porter
   (`src/<item>_026.rs`, `examples/build-<item>-026`) through
   `vostok-port pack from-bundle`; stats live in the builder, balance is the
   owner's call. Every pack that should stay live goes into every release:
   a release is relative to stock, not to the last one.
2. **The patch** (in `$SKHID_PORTER`, inside `nix develop <skhid-srp>/srp-026g`
   for `openssl`):
   ```sh
   vostok-port pack release --base-db "$SKHID_STOCK_DB" --pack <pack> … \
     --exclude flash_movies/lobby_menu.swf --output-dir <out> --work-dir <work> \
     --sign-command "python3 scripts/sign-release-head.py --key $SKHID_SIGNING_KEY \
       --signer <skhid-srp>/srp-026g/client-patches/sign_resources_026g.py"
   vostok-port pack lobby-region --swf <live lobby_menu target> \
     --original <stock lobby_menu.swf> --pack <pack> … --output <out>/lobby_menu-region.bin
   ```
   → `resources-head.bin` (signed), `resources-tail.bin`, `grow.json`,
   `release.json`, and the new `lobby_menu` target (stock size and CRC). Then
   the ZIP as above, with `--resource-grow=<out>/grow.json` and
   `--resource-region=lobby_menu=<out>/lobby_menu-region.bin`.
3. **The server version**, `server-game-data.db`, built next to the ZIP from
   stock + the ZIP's plan. srp loads it and refuses to start when it doesn't
   match the plan of the ZIP it serves. Items need no server code; only
   special rules do (e.g. a shop exception by item name in `prices.rs`).
4. **Test**: locally (the full db written, a real game, a local srp on the
   server version), then the test server (a ZIP built with
   `--server=$SKHID_TEST_GAME_SERVER`).
5. **Out, in one maintenance window**: blobs → server version → ZIP → server
   restart with `SRP_MIN_PATCH_REVISION` raised to the new exe revision.

What keeps it safe (one supported version; never silently broken):
- **Signing**: the exe (revision 12 on) starts only with a `resources.db`
  whose file table is signed with the release signing key; a stock-signed or
  unsigned table stops the game at start ("Resources archive is corrupted").
  The private half is `$SKHID_SIGNING_KEY`, never in a repo, bundle or log;
  the public half is `client-patches/resources_db_key_026g.der`, built into the
  exe. Losing the key means a new key and a new exe revision for everyone.
  `make_client` refuses a ZIP whose exe needs a signed db without
  `--resource-grow`, or with a head not signed by the release key.
- **The updater** treats a plan with a tail as required: it backs up the head,
  appends the tail, writes the head and checks the whole file; on any failure
  it restores the file, keeps Play off with a message, and repairs on the next
  start (`updater_skhid.resources.status`, `updater_skhid.log`).
  `play_skhid.bat/.sh` run `updater_skhid.exe ready` first.
- **The gate**: older games are refused at sign-in (the game's "outdated
  version") and their restored sessions at reconnect, so they never get the
  price list, which names every item sold; a game crashes on an item its
  `resources.db` lacks.
- **Every content release raises the revision**, or a game still running the
  previous db reconnects as current and crashes on the new items.
- **Rollback**: before the gate is raised, put the old ZIP back (the updater
  cuts the tail and restores the head). Once players own a new item, removing
  it from the server is not a rollback (unknown item ids): fix forward.

## Check before it goes out

- The build is deterministic: the same inputs give the same ZIP. Diff it
  against the live ZIP entry by entry (sha256 of each): **only the files the
  change should touch may differ**. Anything else is unreleased client work
  on `main` riding along: tell the owner.
- `updater_skhid.resources` equals live's unless regions or content changed.
- Tests of what changed: `selftest_026g.py`,
  `launcher/selftest_launcher_026g.py --nasm=nasm`,
  `python -m unittest discover -s srp-026g/client-patches -p 'test_*.py'`,
  `client/updater/tests/test_resource_tail.py` (Wine), `cargo test --workspace`
  in `srp-026g` for server changes.
- A game window (Wine: unset `WAYLAND_DISPLAY`, run the exe directly) only
  with the owner's go-ahead.

## Test server first

- Server changes: merge to `main`, run the workflow "Deploy the test server"
  (a maintainer approves). It deploys `main` only, and restarts freely.
- Client changes: a ZIP built with `--server=$SKHID_TEST_GAME_SERVER` (a ZIP
  asks the site it was built for), at
  `$SKHID_TEST_DIR/download/Survarium-0.26g-srp-files.zip`, mode 644. Test with
  a game copy that has an older test ZIP installed, so the self-update runs.

## Put it live

1. Show the owner the diff summary and get a yes.
2. Blobs (if any) → `download/resources/`, mode 644; fetch each back over HTTPS.
   For new content, the server version goes up now too.
3. Archive the current ZIP:
   `cp download/Survarium-0.26g-srp-files.zip old-downloads/Survarium-0.26g-srp-files-<UTC YYYYMMDD-HHMM>-<label>.zip`
4. Upload to `download/Survarium-0.26g-srp-files.zip.tmp`, check its sha256 on
   the host, `chmod 644`, then `mv` over the live name (atomic: the server
   never sees a half-written ZIP).
5. `curl -s $SKHID_SITE/api/client-files` shows the new sha256 and size. Note
   the UTC time.
6. Server deploy, when part of the release: copy the changed `srp-026g` source
   into `$SKHID_LIVE_DIR` keeping `.env`, `download/`, `old-*`;
   `docker compose build srp26`; the maintenance window; with no match
   running `docker compose up -d --no-deps srp26`; check
   `docker compose ps`, `docker compose logs --tail=50 srp26`, `/api/stats`.
7. Record it in a PR: `CHANGELOG-SKHID-026G.md` (players' wording, the UTC
   time of step 5, "update required" when gated) and the download's sha256 and
   size where `client-patches/README.md` lists it.

Rollback: `mv` the archived ZIP back (the updater installs any ZIP whose
checksum differs, older ones too).
