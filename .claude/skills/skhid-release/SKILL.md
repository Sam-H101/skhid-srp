---
name: skhid-release
description: Make a release of skhid-srp (the Survarium 0.26g server "Skhid") — a new players' download (exe, launcher, updater, crash reporter), new game content as a resources.db patch, server code, or any mix. Covers the questions to ask first, the revision bump, building and checking the ZIP, publishing a patch, the server's game data, the maintenance window and rollback. Use when asked to make, ship or roll out a release of skhid-srp.
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
   made doesn't matter). It will be published as
   `resources.db.patch_<new revision>`.
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
git worktree add <scratch>/rel origin/main && cd <scratch>/rel
nix develop ./srp-026g -c bash -c 'client/bugtrap/build.sh && bash client/updater/build.sh'
python3 srp-026g/client-patches/make_client_026g.py "$SKHID_GAME" \
  <scratch>/Survarium-0.26g-srp-files.zip --pack-zip \
  --server="$SKHID_GAME_SERVER" --localization="$SKHID_LOCALIZATION"
```

**Does the updater (or anything else) change?** `bugtrap.dll`,
`updater_skhid.exe` and the launcher build reproducibly
(`client/check-reproducible.sh`), so compare each entry of the new ZIP with
the live one (`curl -o live.zip $SKHID_SITE/download`, sha256 per entry):

- `survarium_skhid.exe` always differs (the revision bump);
- `updater_skhid.exe` differs only if the updater changed — then players get
  the new updater: it installs itself at their next launcher start, before
  anything else the release needs from it. If it's the same file, nothing
  changes for them there;
- anything else that differs must be explained by a merged change; an
  unexplained difference is unreleased work on `main` riding along: stop and
  tell the releaser.

## 4. Publish the content patch (if any)

```sh
P=resources.db.patch_<rev>
cp <the patch database> "$SKHID_PATCHES_LOCAL/$P"
scp "$SKHID_PATCHES_LOCAL/$P" "$SKHID_PATCHES_SSH:$SKHID_PATCHES_DIR/$P.tmp"
ssh "$SKHID_PATCHES_SSH" "cd $SKHID_PATCHES_DIR && test ! -e $P && chmod 644 $P.tmp && mv $P.tmp $P"
curl -s "$SKHID_PATCHES_URL/$P" | sha256sum      # must equal the local file's
```

`$SKHID_PATCHES_LOCAL` keeps a copy of every published patch: the server's
data is built from all of them. Never overwrite or delete a published patch;
a fix is a new patch at the next revision. (To be scripted:
`release/upload-patches`.)

## 5. Build the server's game data

On the release machine (it has the stock db; the live host never stores it):

```sh
srp-game-data build --db "$SKHID_STOCK_DB" \
  $(for f in "$SKHID_PATCHES_LOCAL"/resources.db.patch_*; do
      printf -- '--patch %s=%s/%s ' "$f" "$SKHID_PATCHES_URL" "$(basename "$f")"; done) \
  --out <scratch>/server-game-data.db
srp-game-data info <scratch>/server-game-data.db   # the patches /patches will offer
```

`srp-game-data` is built with
`nix develop ./srp-026g -c bash -c 'cd srp-026g && cargo build --release -p resources-db --bin srp-game-data'`.
(To be scripted: `release/build-server-db`.) Build it for every release, even
one without new content: it records the patch list the server serves.

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
3. Upload, each to a `.tmp` name, check its sha256 on the host, `chmod 644`,
   then `mv` into place:
   - `server-game-data.db` → `$SKHID_LIVE_DIR/download/server-game-data.db`;
   - the ZIP → `$SKHID_LIVE_DIR/download/Survarium-0.26g-srp-files.zip`
     (archive the current one first in `old-downloads/` as
     `Survarium-0.26g-srp-files-<UTC YYYYMMDD-HHMM>-<label>.zip`, and the
     current server data next to it).
4. `.env`: `SRP_MIN_PATCH_REVISION=<new revision>` (games older than it are
   refused at sign-in and on reconnect).
5. Server code changed? Copy the changed `srp-026g` source into
   `$SKHID_LIVE_DIR` (keep `.env`, `download/`, `old-*`) and
   `docker compose build srp26` — before step 1 if possible.
6. `docker compose up -d --no-deps srp26`.

## 8. Check

- `docker compose ps`, `docker compose logs --tail=50 srp26` (the game data
  line names the patches), `curl -s $SKHID_SITE/patches` (the new patch, its
  sha256 and URL), `curl -s $SKHID_SITE/api/client-files` (the new ZIP's
  sha256 and size).
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
