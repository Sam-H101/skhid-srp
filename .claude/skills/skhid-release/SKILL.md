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

The functions above live only in the shell that loaded `release.env`; load it
again in every new shell. Placeholders in `<…>` are yours to fill:
`<scratch>` a scratch folder outside the repo, `<final>` the final players'
`resources.db` the release wants (see "The final resources.db"), `<out>`/`<work>` the output and
work folders of `vostok-port`, `<label>` a short name of the release.

**Who decides.** More than one person (the owner and a second developer) can
reach the live host. "The releaser" below is the person you are releasing for.
Before a release starts, the releaser tells the others; one release at a time
(two would clash on the `.tmp` uploads, the `old-downloads/` names and the
maintenance timer).

**Every command on the live host needs the releaser's go-ahead first.** Writes
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
| `resources.db` changes (Flash menus, content) | the final `resources.db`, `make_client_026g.py --resources-db=<final>` | blobs first, then the ZIP | no (yes with content) |
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
5. Server restarts only with no match running; tell the releaser how many
   players are online before asking.

## Maintenance window (any server restart)

Login, lobby, match and web are one process (`srp`, container
`srp26-srp26-1`): a restart ends every match (no results), drops every lobby
link, and loses the queue and squads. Games sign in again by themselves
within seconds; results not shown yet are kept (`srp.db`). The launcher page
and `/download` are briefly down too.

What a running game shows (`srp-026g/docs/notes/transport.md` §6): after 5 s without
the server a "server sync" overlay; it retries with the same session every
5 s, only while its window is active, and a restored session puts it straight
back in the lobby. After five failed tries (~30 s) it signs out to the login
screen. **Keep the downtime well under 30 s**: build the image first, so the
outage is just `up -d`.

- **Where**: the admin page, tab "Maintenance" (`/<SRP_ADMIN_PATH>?tab=maint`,
  served only to `SRP_ADMIN_IPS`, every request with `token=<SRP_ADMIN_TOKEN>`;
  all three in the live `.env`, never printed). It takes `minutes` (1–10080)
  or `cancel=1`. The releaser plans it from their browser; confirm it on the
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
releaser's checkout usually has work in progress; never `git stash`).

```sh
git worktree add <scratch>/rel origin/main && cd <scratch>/rel
nix develop ./srp-026g -c bash -c 'client/bugtrap/build.sh && bash client/updater/build.sh'
# make_client runs srp-game-data for the server version and stops without it
# (or name it with --game-data-tool=<path>):
nix develop ./srp-026g -c bash -c 'cd srp-026g && cargo build --release -p resources-db --bin srp-game-data'
python3 srp-026g/client-patches/make_client_026g.py "$SKHID_GAME" \
  <scratch>/Survarium-0.26g-srp-files.zip --pack-zip \
  --server="$SKHID_GAME_SERVER" --localization="$SKHID_LOCALIZATION" \
  --resources-db=<final> --resources-out=<scratch>/blobs
```

`bugtrap.dll` and `updater_skhid.exe` build reproducibly
(`client/check-reproducible.sh`), so they're built fresh.

`$SKHID_GAME` must hold the stock `resources.db` (the sha256 of
`$SKHID_STOCK_DB`): `make_client` makes the server version from the source
game's `resources.db`, so a game with regions or content installed gives the
server wrong data.

**The final resources.db.** `make_client` compares `<final>` with the stock
`resources.db` and makes the plan itself: a changed file in place becomes a
region named after it (`main_menu`, `lobby_menu`, `login_menu`), a new file
table the head region `resources_head` (it must be signed with the release
key), anything past the stock end the tail; anything else is refused.
`<final>` must carry everything players should keep, or their menus get
reverted. Read the live plan (public: `curl -o live.zip $SKHID_SITE/download`,
then `unzip -p live.zip game/binaries/x86/updater_skhid.resources`; line
format `region <path> <file size> <offset> <size> <original sha256> <target sha256> <name>`),
fetch each target (`curl -o x $SKHID_SITE/resources/<target sha>`, check its
sha256), and put it into a copy of stock:
`srp-game-data put --db <final> --path flash_movies/<menu>.swf --file <target>`.
With content, `vostok-port pack release --replace …` does this (see "New
content"). Upload the new blobs from `--resources-out`.

## New content (items, maps): a patch for clients, a server version for the server

> **Needs pull requests 84, 85 and 88 (`srp-game-data`, the server version,
> `--resources-db`)** and resource-porter #3 (`pack release --output-db`).
> Until they are merged, this section and "The server version of every ZIP"
> describe what is to come.

One release build, from one list of what goes in, gives two outputs:

| Who | Gets | What it is |
|---|---|---|
| Players | **the patch**: the ZIP + its blobs | a plan that grows `resources.db`: a signed new head (its file table, `[0, ~10.6 MB)`) and a tail appended at its end, plus the menus (the lobby with the new icons) |
| The server | **the server version**: `server-game-data.db` (a few MB) | the `gameplay/` files srp reads, extracted from the final db, with the patch's provenance inside |

The final `resources.db` (6.3 GB) is built on the release machine (on the SSD;
it needs ~7 GB free) and both outputs come from it. Nobody receives it; the
live host never stores it.

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
     --replace flash_movies/main_menu.swf=<live target> \
     --replace flash_movies/login_menu.swf=<live target> \
     --replace flash_movies/lobby_menu.swf=<live target> \
     --sign-command "python3 scripts/sign-release-head.py --key $SKHID_SIGNING_KEY \
       --signer <skhid-srp>/srp-026g/client-patches/sign_resources_026g.py" \
     --work-dir <work> --output-db <final>
   ```
   → `<final>` (the packs' icons drawn on the live lobby) and `<final>.json`,
   the record of what went in. Then the ZIP as above, with
   `--resources-db=<final>`.
3. **The server version**, `server-game-data.db`, built by `make_client`
   next to the ZIP from `<final>` (`srp-game-data extract --final`).
   srp loads it and refuses to start when it doesn't match the plan of the ZIP
   it serves. Items need no server code; only special rules do (e.g. a shop
   exception by item name in `prices.rs`).
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
  `make_client` refuses a ZIP whose exe needs a signed db without a
  `--resources-db` whose table the release key signed.
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

## The server version of every ZIP

srp reads its game data from `SRP_GAME_DATA` (default
`download/server-game-data.db`) and starts only when that file was made for
the plan (`updater_skhid.resources`) of the ZIP it serves, so **every ZIP has
its server version, content or not**. `make_client` writes it next to the ZIP
from `--resources-db`, which every players' ZIP has. By hand:
`srp-game-data extract --final <final> --zip <ZIP> --out server-game-data.db`. The check is of the plan, not
the ZIP's checksum: a ZIP that changes only the exe, launcher or updater keeps
the plan and works with the server version already live.

It is a release output like the ZIP: never committed. It goes up with the
blobs, is archived with the ZIP, and its sha256 is recorded with the ZIP's.
`srp-game-data check --data <file> --zip <ZIP>` says whether a pair matches.
The test server serves its own ZIP, so it needs its own server version.

**First deploy of a server with pull request 85**: the live host has no
server version yet, and srp will not start without one. Before that restart,
make it for the ZIP that is live now (download it; its blobs are in
`$SKHID_SITE/resources/`), check it with `srp-game-data check`, and upload it
to `download/server-game-data.db`, mode 644.

## Check before it goes out

- The build is deterministic: the same inputs give the same ZIP. Diff it
  against the live ZIP entry by entry (sha256 of each): **only the files the
  change should touch may differ**. Anything else is unreleased client work
  on `main` riding along: tell the releaser.
- `updater_skhid.resources` equals live's unless regions or content changed.
- Tests of what changed: `selftest_026g.py`,
  `launcher/selftest_launcher_026g.py --nasm=nasm`,
  `python -m unittest discover -s srp-026g/client-patches -p 'test_*.py'`,
  `client/updater/tests/test_resource_tail.py` (Wine), `cargo test --workspace`
  in `srp-026g` for server changes.
- A game window (Wine: unset `WAYLAND_DISPLAY`, run the exe directly) only
  with the releaser's go-ahead.

## Test server first

- Server changes: merge to `main`, run the workflow "Deploy the test server"
  (a maintainer approves). It deploys `main` only, and restarts freely.
- Client changes: a ZIP built with `--server=$SKHID_TEST_GAME_SERVER` (a ZIP
  asks the site it was built for), at
  `$SKHID_TEST_DIR/download/Survarium-0.26g-srp-files.zip`, mode 644. Test with
  a game copy that has an older test ZIP installed, so the self-update runs.

## Put it live

1. Show the releaser the diff summary and get a yes.
2. Blobs (if any) → `download/resources/`, mode 644; fetch each back over HTTPS.
3. Archive the current pair:
   `cp download/Survarium-0.26g-srp-files.zip old-downloads/Survarium-0.26g-srp-files-<UTC YYYYMMDD-HHMM>-<label>.zip`
   and `cp download/server-game-data.db old-downloads/server-game-data-<same stamp>-<label>.db`.
4. Upload to `download/Survarium-0.26g-srp-files.zip.tmp`, check its sha256 on
   the host, `chmod 644`. When the plan changes (regions, content), upload the
   new server version as `download/server-game-data.db.tmp` too and check it.
5. Swap:
   - **Plan unchanged**: `mv` the ZIP over the live name (atomic: the server
     never sees a half-written ZIP). No restart.
   - **Plan changed**: in the maintenance window, back to back, `mv` the
     server version, `mv` the ZIP, `docker compose up -d`. Never leave the new
     ZIP live with the old server version: srp keeps running on the data it
     read, but any restart in between (a crash, the restart policy) refuses to
     start and the server stays down.

   `curl -s $SKHID_SITE/api/client-files` shows the new sha256 and size. Note
   the UTC time.
6. Server deploy, when part of the release: copy the changed `srp-026g` source
   into `$SKHID_LIVE_DIR` keeping `.env`, `download/`, `old-*`;
   `docker compose build srp26`; the maintenance window; with no match
   running `docker compose up -d --no-deps srp26`; check
   `docker compose ps`, `docker compose logs --tail=50 srp26`, `/api/stats`.
7. Record it in a PR: `CHANGELOG-SKHID-026G.md` (players' wording, the UTC
   time of step 5, "update required" when gated) and the download's sha256 and
   size, with its server version's sha256, where `client-patches/README.md`
   lists the downloads.

Rollback: `mv` the archived ZIP back (the updater installs any ZIP whose
checksum differs, older ones too), and its archived server version with it.
When their plans differ, that is the same swap as step 5 (both files, then a
restart, in a window).
