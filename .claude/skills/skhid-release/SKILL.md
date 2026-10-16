---
name: skhid-release
description: Make a release of skhid-srp (the Survarium 0.26g server "Skhid") — a new players' download (exe, launcher, updater, crash reporter), new game content as a resources.db patch, server code, or any mix — with release/release.sh, which plans the maintenance, prepares everything during the countdown and swaps it in at once. Covers the questions to ask first, the revision bump, the dry run, what to watch, rollback, and the step scripts in release/. Use when asked to make, ship or roll out a release of skhid-srp.
---

# Making a Skhid 0.26g release

## Private values

Every host, address and local path is a setting. Their format is
`release.env.example` next to this file; the filled-in copy is
`private/release/release.env` in the main checkout (never published). The
scripts load it themselves (`SKHID_RELEASE_ENV=<file>` names another); if
it's missing, ask the owner. Any PC can run a release: what is particular
to it is in its own `release.env`.

The hosts, by role: the **jump host** (`$SKHID_JUMP`, it holds the live
host's key and runs the patches server), the **live host** (`$SKHID_LIVE`,
the srp26 compose project in `$SKHID_LIVE_DIR`, the site `$SKHID_SITE`),
the **test server** (`$SKHID_TEST`, `$SKHID_TEST_SITE`). Never print the live
`.env`; read a single setting with `grep -nE '^(NAME)=' .env`. One release at
a time (more than one person can reach the hosts; tell the others).

## When the owner says "make a release"

Ask, in this order:

1. **Does it include new content?** If yes: **the path to the patch
   database**, `resources.db.patch_<rev>` (a standalone archive in the
   game's format, named for the new revision; how it was made doesn't
   matter). A folder of files at the paths the game reads them from
   (`gameplay/items/...`) is packed into one by
   `release/build-patch <folder> <rev>`.
2. **Is the revision bumped on main?** Every release has a new patch
   revision, content or not: it is how the server refuses games without the
   new files. If `REVISION` in `srp-026g/client-patches/patch_026g.py` on
   `origin/main` is not above the live exe's, the bump PR first: `REVISION`
   +1, its row in the client-patches README revision table, the result
   hashes; merged with the owner's yes. The patch takes this number.
3. **Any server-code change?** `release.sh` compares main's `srp-026g`
   build inputs with the live tree and builds the image when they differ;
   the owner may know better (`--server-code yes|no`).

Then:

```sh
release/release.sh --dry-run [--resources <path>/resources.db.patch_<rev>]
```

Show the owner its output (it runs every read-only check and prints every
other command; it plans no maintenance and writes nothing). When they say
go, the same without `--dry-run`, from a terminal: it asks y/N twice.

## release/release.sh

```
release/release.sh [--resources <resources.db.patch_<rev>>] [--test]
                   [--server-code auto|yes|no] [--minutes <n>]
                   [--label <word>] [--dry-run] [--yes --i-know]
```

Matches last up to 15 minutes, so it plans the maintenance
`$SKHID_MAINT_MINUTES` (default 20) minutes ahead, prepares and stages
everything while the countdown runs, and at its end swaps it all in at once
and restarts srp26 (the downtime is the restart).

1. **Preflight**, nothing players see. It says exactly what's missing: ssh
   to the jump host, through it to the live host, the patches server; nix;
   `$SKHID_GAME`; `$SKHID_STOCK_DB` with the stock sha256 (cached by size
   and time after the first time); free disk here and there; srp-game-data
   builds; main's `REVISION` above the live exe's; the admin page answers
   the host it is asked from; no maintenance planned already; nothing
   staged left from an earlier run. With `--resources`: the name is
   `resources.db.patch_<main's REVISION>`, srp-game-data reads it over the
   stock db, and the patches server doesn't have it yet. Last, which
   server-code files differ.
2. **y/N**, then **maintenance**: a POST to the admin page
   (`tab=maint`, `minutes`) from `$SKHID_ADMIN_FROM` (default the jump
   host, whose address must be in the live `SRP_ADMIN_IPS`; behind Caddy
   with `SRP_WEB_TRUST_PROXY=on`). The admin path and token are read from
   the live `.env` into variables and handed to curl as its config on
   stdin: never printed, in no command line. It checks the countdown on
   `$SKHID_SITE/launcher/places/en` and prints the planned time.
3. **During the countdown**: `upload-patch` (with `--resources`),
   `build-server-db`, `build-zip` (of the commit checked in step 1), the
   checks of `deploy` on both files; both uploaded to the live host as
   `download/<name>.tmp`, sha256-checked there, `chmod 644`, not in place.
   With server code: the running image tagged
   `srp-026g:before-<stamp>-<label>`, main's source unpacked in
   `.release-src.tmp/` there and `docker compose build srp26` run on it (the
   running container is not touched). It prints the time left.
4. **At the planned time**, once the launcher page says the server is down
   and `$SKHID_SITE/api/stats` says `"matches":0` (it waits up to
   `$SKHID_MATCH_WAIT_MINUTES`, default 15, more): it shows what changes
   (files, hashes, `SRP_MIN_PATCH_REVISION`, `/patches` before and after,
   the ZIP's changed entries), **y/N**, then archives the current ZIP and
   server data (and source, with server code) in `old-downloads/` (UTC
   stamp + label), moves the staged files into place, sets
   `SRP_MIN_PATCH_REVISION=<rev>` (that line only) and runs
   `docker compose up -d --no-deps --force-recreate srp26`.
5. **Checks**: the container running (healthy), the log's `game data:`
   line, `/patches` exactly the server data's list, `/api/client-files`
   the new ZIP's sha256 and size; then it prints the rollback.

A stop before step 4 (a failure, a no, Ctrl-C) removes the staged files,
puts the image tag back and cancels the maintenance: the live server is as
it was (a published patch stays; patches are never deleted, so the next run
goes without `--resources`). A stop in steps 4–5 says what was done and
prints the rollback; it does not undo by itself.

`--yes --i-know` (both or neither) answers both questions; only for a run
the owner watches anyway. `--test` releases to the test server instead (its
ZIP from `build-zip --test`; its code comes from the workflow "Deploy the
test server", run that first if server code changed).

### What to watch

- The preflight: every line `ok`.
- After the first yes: the launcher countdown (players see it now).
- The ZIP's entries against the live ZIP (build-zip's list, shown again at
  the second question): `survarium_skhid.exe` always differs (the
  revision); `updater_skhid.exe` only if the updater changed (players get
  it at their next launcher start); anything else must be explained by a
  merged change. Unexplained: answer no (that cancels cleanly).
- After the swap: the checks, then a game updates, downloads the patches,
  signs in; an old game is refused. Ask before starting any game window
  (Wine: unset `WAYLAND_DISPLAY`, run the exe directly).

## Rollback

`release.sh` prints it, ready to run on the live host: the archived ZIP and
server data back into `download/`, `SRP_MIN_PATCH_REVISION` back (or its
line removed), with server code the archived source unpacked and the
`before-` image tagged `srp-026g:latest` again, then
`docker compose up -d --no-deps --force-recreate srp26`. The updater deletes
local patches the server's list no longer has. Published patches stay on
the patches server. Once players own new items from a patch, removing it is
not a rollback (the game crashes on unknown items): fix forward.

## The steps it runs

Each is a script of its own (`--help`; it asks y/N before every write and
shows the command, unless release.sh, already given the go-ahead, runs it):

| Script | Does | Writes |
|---|---|---|
| `release/build-patch <folder> <rev> [--out <file>]` | packs a folder into `resources.db.patch_<rev>` (read back and compared) | a local file |
| `release/upload-patch [--dry-run] <file>` | publishes one patch: `.tmp`, sha256 there, `chmod 644`, `mv -n`, then GET must be 200 with the sha256; never replaces one | the patches server |
| `release/build-server-db [--out <file>]` | `server-game-data.db` from the stock db and every published patch (each GET 200, the server's sha256) | a local file |
| `release/build-zip [--test] [--ref <commit>] [--work <dir>]` | the players' ZIP from a fresh worktree of origin/main (never this checkout, never `git stash`); refuses an unbumped revision; entries against the live ZIP | a worktree, the ZIP |
| `release/deploy --zip --server-data --revision [--label] [--dry-run]` | the swap on its own, in a window planned by hand: archive, `.tmp`, sha256, `mv`, the setting, restart, the checks | the live host |
| `release/setup-patches-server [--check]` | once: the patches folder and Caddy's site block for it on the jump host | the jump host's Caddy |

`release/test.sh` runs them all, release.sh included, against stand-ins on
this machine (`nix shell nixpkgs#python3 nixpkgs#shellcheck -c
release/test.sh`); it sends nothing anywhere.

## How the game gets its content

- The game mounts the stock `game/resources.db`, then every
  `game/resources.db.patch_<n>` that exists, n = 1…255 ascending (gaps are
  normal). A later patch overrides files of earlier ones and adds new files;
  a patch can't delete. `n` is the **revision** the patch was released in.
- The updater asks the game server for `GET /patches`
  (`{"patch_14": {"sha256": "…", "url": "…"}, …}`), downloads the missing
  ones, checks every patch's sha256, deletes local patches not in the list,
  and keeps PLAY off until all match.
- The **patches server** is plain file storage: `$SKHID_PATCHES_URL`
  (files in `$SKHID_PATCHES_DIR` on `$SKHID_PATCHES_SSH`). Patch files there
  are never deleted or changed.
- The **game server** runs on `server-game-data.db`: the files it reads,
  built by `srp-game-data build` from the stock `resources.db` plus every
  patch with its URL, on the release PC (the live host never stores the
  stock db). It serves `/patches` from what that file records, so it can
  only advertise the patches it runs on.
- Login, lobby, match and web are one process (container `srp26-srp26-1`):
  a restart ends every match and drops every lobby link; games reconnect by
  themselves. A planned maintenance lives in its memory: a restart ends it.

## Record

A PR with `CHANGELOG-SKHID-026G.md` (players' wording, the UTC time, "update
required") and the hashes: ZIP, server data, each new patch, its revision
(release.sh keeps the ZIP and the server data in its working folder and
prints their hashes).
