---
name: skhid-release
description: Make a release of skhid-srp (the Survarium 0.26g server "Skhid") — a new players' download (exe, launcher, updater, crash reporter), new game content as a resources.db patch, server code, or any mix. Covers the questions to ask first, the revision bump, building and checking the ZIP, publishing a patch, the server's game data, the maintenance window and rollback, with the scripts in release/. Use when asked to make, ship or roll out a release of skhid-srp.
---

# Making a Skhid 0.26g release

## Private values

Every host, address and local path is a setting. Their format is
`release.env.example` next to this file; the filled-in copy is
`private/release/release.env` in the main checkout (never published). Load it
in every shell first; if it's missing, ask the owner.

```sh
. "$(git rev-parse --path-format=absolute --git-common-dir)/../private/release/release.env"
# One command on the live host (through the jump host, which holds its key):
live() { ssh "$SKHID_JUMP" "ssh $SKHID_LIVE $(printf %q "$1")"; }
```

**Every command on the live host needs the releaser's go-ahead**, with the
exact command shown; writes players feel (the ZIP, `docker compose`, `.env`)
need a yes each time. Never print the live `.env`; read single settings with
`grep -nE '^(NAME)=' .env`. One release at a time (more than one person can
reach the hosts; the releaser tells the others).

## The scripts

`release/` does each step below; every script has `--help`, loads
`release.env` itself (or the file `SKHID_RELEASE_ENV` names), prints no
secret, and asks y/N before every write on the live host or the patches
server, showing the command (a no stops it). Run them from a checkout of
the branch that has them; they need bash, ssh, curl, jq, nix (python3 and
srp-game-data come from it when missing).

| Script | Step | Writes |
|---|---|---|
| `release/build-patch <folder> <revision> [--out <file>]` | 1, a folder of files into `resources.db.patch_<revision>` | a local file |
| `release/upload-patch [--dry-run] <resources.db.patch_<n>>` | 4 | the patches server (never overwrites) |
| `release/build-server-db [--out <file>]` | 5 | a local file |
| `release/build-zip [--test] [--ref <commit>] [--work <dir>]` | 3 | a worktree and the ZIP, locally |
| `release/deploy --zip <zip> --server-data <db> --revision <n> [--label <word>] [--dry-run]` | 7.4 and 8 | the live host |
| `release/setup-patches-server [--check]` | once, the patches server | the jump host's Caddy |

`release/test.sh` runs them all against stand-ins on this machine (`nix shell
nixpkgs#python3 nixpkgs#shellcheck -c release/test.sh`); it sends nothing
anywhere.

## How the game gets its content

- The game mounts the stock `game/resources.db`, then every
  `game/resources.db.patch_<n>` that exists, n = 1…255 ascending (gaps are
  normal). A later patch overrides files of earlier ones and adds new files;
  a patch can't delete. `n` is the **revision** the patch was released in.
- The updater asks the game server for `GET /patches`
  (`{"patch_14": {"sha256": "…", "url": "…"}, …}`), downloads the missing
  ones from their URLs, checks every patch's sha256, deletes local patches not
  in the list, and keeps PLAY off until all match.
- The **patches server** is plain file storage: `$SKHID_PATCHES_URL`
  (files in `$SKHID_PATCHES_DIR` on `$SKHID_PATCHES_SSH`). Patch files there
  are never deleted or changed.
- The **game server** runs on `server-game-data.db`: the files it reads, built
  by `srp-game-data build` from the stock `resources.db` plus every patch with
  its URL. It serves `/patches` from what that file records, so it can only
  advertise the patches it runs on.

## 1. Ask first

1. **Does this release include new content?** If yes, ask for **the path to
   the patch database** (a standalone archive in the game's format; how it was
   made doesn't matter), or **a patch folder** (its files at the paths the
   game reads them from, e.g. `gameplay/items/...`), which
   `release/build-patch <folder> <new revision>` packs (and checks by reading
   it back). It will be published as `resources.db.patch_<new revision>`.
2. What else changed since the live release: the exe, launcher, updater,
   crash reporter, server code? (`git log` from the live release's commit on
   `main`; the previous changelog entry names it.)
3. Is the releaser ready for a maintenance window (any server restart is
   one)? A ZIP-only release needs none.

## 2. Bump the revision

Every release gets a new patch revision, even when nothing else in the exe
changes: the revision is how the server refuses games that lack the new
content. A small PR: `REVISION` in `srp-026g/client-patches/patch_026g.py`
+1, its row in the README revision table, and the result hashes. Merge it
(owner's yes) before building. The new content patch, if any, takes this
number.

## 3. Build the players' ZIP

From a fresh worktree of `origin/main` (never the releaser's checkout, never
`git stash`); needs the original game in `$SKHID_GAME`:

```sh
release/build-zip --work <scratch>/zip          # --test: for the test server
```

It fetches `origin/main` and makes the worktree, downloads the live ZIP
(`$SKHID_SITE/download`) and refuses when main's `REVISION` is not above
the live exe's (bump it first, step 2), builds bugtrap and the updater
(`nix develop ./srp-026g -c bash -c 'client/bugtrap/build.sh && bash
client/updater/build.sh'`), runs `make_client_026g.py "$SKHID_GAME" <zip>
--pack-zip --server=$SKHID_GAME_SERVER --localization=$SKHID_LOCALIZATION`,
then prints each entry as same / CHANGED / ADDED / REMOVED against the live
ZIP and says whether `updater_skhid.exe` changed. Remove the worktree after
(it prints the command).

**Does the updater (or anything else) change?** `bugtrap.dll`,
`updater_skhid.exe` and the launcher build reproducibly
(`client/check-reproducible.sh`), so the entry-by-entry comparison means:

- `survarium_skhid.exe` always differs (the revision bump);
- `updater_skhid.exe` differs only if the updater changed — then players get
  the new updater: it installs itself at their next launcher start, before
  anything else the release needs from it. If it's the same file, nothing
  changes for them there;
- anything else that differs must be explained by a merged change; an
  unexplained difference is unreleased work on `main` riding along: stop and
  tell the releaser.

## 4. Upload the new patch (if any)

Only the new patch goes up; the patches server is the one copy of every
published patch.

```sh
release/upload-patch resources.db.patch_<rev>   # --dry-run first shows the commands
```

It refuses a name the game doesn't mount and a patch already there (it says
whether with the same bytes), uploads to `<name>.tmp`, checks its sha256
there, `chmod 644`, `mv -n` into place, then `GET $SKHID_PATCHES_URL/<name>`
must answer 200 with the local sha256. Never overwrite or delete a
published patch; a fix is a new patch at the next revision.

The patches server itself (Caddy on the jump host, a site block for
`$SKHID_PATCHES_URL` serving `$SKHID_PATCHES_DIR`, after the other site,
which stays untouched) is set up once by `release/setup-patches-server`;
`--check` only reads and says "nothing to do" when it is all there.

## 5. Build the server's game data

On the release machine (it has the stock db; the live host never stores it),
for every release, content or not (it records the patch list the server
serves). From the patches server:

```sh
release/build-server-db --out <scratch>/server-game-data.db
```

1. it lists `resources.db.patch_*` in `$SKHID_PATCHES_DIR` (ssh);
2. downloads each from `$SKHID_PATCHES_URL/<name>`: every one must return
   200 with the sha256 the file has there;
3. runs `srp-game-data build --db "$SKHID_STOCK_DB" --patch-url
   "$SKHID_PATCHES_URL" --patch <each> --out <file>`, then `srp-game-data
   info <file>`, and prints what `/patches` will offer.

`srp-game-data` is built with
`nix develop ./srp-026g -c bash -c 'cd srp-026g && cargo build --release -p resources-db --bin srp-game-data'`
(the scripts do it; `SRP_GAME_DATA_BIN` names another).

## 6. Test

Locally first (a game folder with the stock db + the patches, the new exe,
a local srp on the new `server-game-data.db`), then the test server (a ZIP
built with `--server=$SKHID_TEST_GAME_SERVER`, deployed from `main` by the
workflow "Deploy the test server"). Ask before starting any game window
(Wine: unset `WAYLAND_DISPLAY`, run the exe directly).

## 7. The maintenance window

Login, lobby, match and web are one process (`srp`, container
`srp26-srp26-1`): a restart ends every match and drops every lobby link;
games reconnect by themselves. **Keep the downtime well under 30 s**: build
the server image before the window, so the outage is just `up -d`.

1. The releaser plans maintenance on the admin page (tab "Maintenance",
   `/<SRP_ADMIN_PATH>?tab=maint`, from their own browser; 45 minutes): players
   see a countdown, Play locks at 30 minutes. Confirm on
   `curl -s $SKHID_SITE/launcher/places/en`.
2. Wait for `"matches":0` on `curl -s $SKHID_SITE/api/stats`.
3. Server code changed? Copy the changed `srp-026g` source into
   `$SKHID_LIVE_DIR` (keep `.env`, `download/`, `old-*`) and
   `docker compose build srp26` — before step 1 if possible.
4. `release/deploy --zip <zip> --server-data <db> --revision <n>` (run it
   with `--dry-run` before the window: it prints every command and runs
   none on the live host). It checks first that the ZIP's exe says revision
   `<n>` and that the server data offers no later patch, then asks before
   each write. What it does, by hand:
   - archive the current ZIP and server data in `old-downloads/` as
     `Survarium-0.26g-srp-files-<UTC YYYYMMDD-HHMM>-<label>.zip` and
     `server-game-data-<UTC YYYYMMDD-HHMM>-<label>.db`;
   - upload each to a `.tmp` name, check its sha256 on the host,
     `chmod 644`, then `mv` into place: `server-game-data.db` →
     `$SKHID_LIVE_DIR/download/server-game-data.db`, the ZIP →
     `$SKHID_LIVE_DIR/download/Survarium-0.26g-srp-files.zip`;
   - `.env`: `SRP_MIN_PATCH_REVISION=<new revision>` (games older than it
     are refused at sign-in and on reconnect); that one line only;
   - `docker compose up -d --no-deps srp26`;
   - the checks of step 8 on `/patches` and `/api/client-files`.

## 8. Check

- `docker compose ps`, `docker compose logs --tail=50 srp26` (the game data
  line names the patches), `curl -s $SKHID_SITE/patches` (the new patch, its
  sha256 and URL), `curl -s $SKHID_SITE/api/client-files` (the new ZIP's
  sha256 and size). `release/deploy` does these after the restart: `/patches`
  must be exactly what the server data offers.
- A game updates (launcher), downloads the patch, starts and signs in; an old
  game is refused.

## 9. Record

A PR with `CHANGELOG-SKHID-026G.md` (players' wording, the UTC time, "update
required") and the hashes: ZIP, server data, each new patch, its revision.

## Rollback

Put back the archived ZIP and server data and restart (`up -d`); set
`SRP_MIN_PATCH_REVISION` back. The updater deletes local patches that the
server's list no longer has. Published patch files stay on the patches
server. Once players own new items from a patch, removing that patch from the
server is not a rollback (the game crashes on unknown items): fix forward.
