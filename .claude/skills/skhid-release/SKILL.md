---
name: skhid-release
description: Make a release of skhid-srp (the Survarium 0.26g server "Skhid") — a new players' download (exe, launcher, updater, crash reporter), new game content as a resources.db patch, server code, or any mix — with release/release.sh, which plans the maintenance, prepares everything during the countdown, swaps it in at once, keeps a folder per release on the host and rolls back by itself when the new server does not come up. Covers the questions to ask first, the revision bump, the offered-patch list, the dry run, the test environment, what to watch, rollback, and the step scripts in release/. Use when asked to make, ship, test or roll out a release of skhid-srp.
---

# Making a Skhid 0.26g release

## Private values

Every host, address and local path is a setting. Their format is
`release.env.example` next to this file; the filled-in copy is
`private/release/release.env` in the main checkout (never published). The
scripts load it themselves (`SKHID_RELEASE_ENV=<file>` names another); if
it's missing, ask the owner. Any PC can run a release: what is particular
to it is in its own `release.env`.

The hosts, by role: the **jump host** (`$SKHID_JUMP`: it holds the live
host's key, runs the patches servers, asks the admin page, holds the
release lock and the copies of srp.db), the **live host** (`$SKHID_LIVE`,
the srp26 compose project in `$SKHID_LIVE_DIR`, the site `$SKHID_SITE`),
the **test environment** (`$SKHID_TEST` in `$SKHID_TEST_DIR`, compose
project `srptest`, its site `$SKHID_TEST_SITE`, its own patches server
`$SKHID_TEST_PATCHES_*`; today it lives on the jump host itself). Never print
a `.env`; read a single setting with `grep -nE '^(NAME)=' .env`. One release
at a time per target: `release.sh` and `deploy` take a lock on the jump
host and refuse while another run holds it.

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
   `origin/main` is not above the live exe's, the bump PR first. The patch
   takes this number. From revision 12 on the exe expects the menus of
   `resources.db.patch_12`: a release of revision 12 or later is refused
   unless patch_12 is offered (in the list, or the `--resources` file).
3. **Any server-code change?** `release.sh` compares main's `srp-026g`
   build inputs with the target's tree and builds the image when they
   differ; the owner may know better (`--server-code yes|no`).

Then, first on the test environment, then live:

```sh
release/release.sh --test --dry-run [--resources <path>/resources.db.patch_<rev>]
release/release.sh --test            [--resources ...]   # then play it there
release/release.sh --dry-run         [--resources ...]
release/release.sh                   [--resources ...]
```

Show the owner each dry run's output (every read-only check runs; every
other command is printed; nothing is planned or written). When they say go,
the same without `--dry-run`, from a terminal: it asks y/N twice.

## What is offered: release/patches.tsv

The patches players get are an explicit, committed list:
`release/patches.tsv` (live) and `release/patches-test.tsv` (test), one
`<name> <sha256> <size> <urls>` row per patch, tabs between, in revision
order. `<urls>` is the patch's URLs, one space apart, in the order the
players' updater tries them: the patches server's (`$SKHID_PATCHES_URL/<name>`,
always first), then mirrors; 1 to 8, none twice. The server data is built
of exactly those rows and `/patches` offers exactly those; a file on the
patches server that is not listed is offered to no one.
`release/upload-patch` (and `release.sh --resources`) adds the row when it
publishes: **commit the list with the release's record**. A row's patch is
never changed (a published patch is never replaced; a fix is the next
revision); only its mirrors may be. The preflight checks every row against
the patches server (size, sha256) and every URL of it (below).

**Mirrors.** `release/add-mirror [--test] <resources.db.patch_<n>> <url>`
downloads the file from `<url>` as the updater does (up to 5 redirects,
http and https only), checks it is not a web page and has the row's size
and sha256, and appends the URL to the row; commit the list. Players get a
mirror with the next release (the server data names the URLs; srp's
`/patches` offers them). A mirror is removed by hand (the first URL stays).
For **Google Drive**, share the file with "anyone with the link" and add
`https://drive.usercontent.google.com/download?id=<id>&export=download&confirm=t`
(the id of the link `.../file/d/<id>/view`): the `drive.google.com/uc`
form answers a large file with a "cannot scan for viruses" HTML page,
which the updater and the scripts refuse. The updater tries the URLs in
order and moves on at any failure (no connection, another status than 200,
a web page, cut short, another sha256); PLAY stays off only when every URL
failed. There is no download resume: a broken download is fetched again
whole.

The checks of every URL (release.sh's preflight, build-server-db for the
mirrors, add-mirror) download it whole once and keep the result per (URL,
sha256) in `~/.cache/skhid-release/patch-urls.checked` (`$XDG_CACHE_HOME`
if set): later runs ask such a URL for its first bytes only (200 or 206,
not a web page, the row's size when the answer gives the whole file's
size, the archive's `little-endian` header). A mirror whose file was
replaced since by one of the same size is not seen so (the updater's
sha256 check still refuses it, and moves on to the next URL). Delete a
line, or the file, to have a URL downloaded whole again.

## release/release.sh

```
release/release.sh [--resources <resources.db.patch_<rev>>] [--test]
                   [--server-code auto|yes|no]
                   [--minutes <n> | --no-maintenance] [--match-wait <minutes>]
                   [--label <word>] [--ref <branch>] [--dry-run] [--yes --i-know]
```

Matches last up to 15 minutes, so it plans the maintenance
`$SKHID_MAINT_MINUTES` (default 20) minutes ahead, prepares and stages
everything while the countdown runs, and at its end swaps it all in at once
(the downtime is the restart).

1. **Preflight**, nothing players see. It says exactly what's missing: ssh
   to the jump host, through it to the live host, the patches server; the
   release lock free; nix; `$SKHID_GAME`; `$SKHID_STOCK_DB` with the stock
   sha256 (cached by size and time); free disk here and there;
   srp-game-data builds; main's `REVISION` above the target exe's; the
   admin page answers the jump host (not with `--no-maintenance`); no
   maintenance planned already; no
   release folder left unfinished; `download/` on the file system of the
   target's folder; `download/resources/` keeps the original menus' blobs
   (main_menu `0ff118a8…`, lobby_menu `ab6f22d8…`, login_menu `805ea5f8…`:
   the updater's one-time restore to the stock resources.db fetches them);
   srp26's container and its image ID, which must be what `srp-026g:latest`
   names (else a restart would run an image no release made); the list's
   rows on the patches server, and every URL of every row giving the file
   (as the updater fetches it, not a web page, the size and sha256; whole
   the first time, then from the cache above); patch_12 offered from
   revision 12 on; with
   `--resources`: the name is `resources.db.patch_<main's REVISION>`,
   srp-game-data reads it over the stock db, it is neither published nor
   listed yet. Last, which server-code files differ, and which settings
   main's compose file newly passes that the target's `.env` sets (names
   only: their values take effect at the recreate).
2. **The lock, y/N, then maintenance**: a POST to the admin page
   (`tab=maint`, `minutes`) from the jump host, whose address must be in
   the target's `SRP_ADMIN_IPS` (behind Caddy with
   `SRP_WEB_TRUST_PROXY=on`). The admin path and token are read from the
   target's `.env` into variables and handed to curl as its config on
   stdin: never printed, in no command line. It checks the countdown on
   the launcher page and prints the planned time.
3. **During the countdown**: `upload-patch` (with `--resources`; it adds
   the row), `build-server-db` (of the list), `build-zip` (of the commit
   checked in step 1); the checks of `deploy` on both files; then the
   **release folder** on the target (below) with both files staged in it
   (sha256 checked there, `chmod 644`) and `changed.txt`. With server
   code: the running image tagged `srp-026g:before-rev<rev>-<stamp>` by
   its ID, main's source unpacked in the folder's `src/` and
   `docker compose build srp26` run on it (the running container is not
   touched). The local builds run at nice 10.
4. **At the planned time**, once the launcher page says the server is down
   and `/api/stats` says `"matches":0` (it waits up to `--match-wait`,
   default `$SKHID_MATCH_WAIT_MINUTES`, else 15, more): it shows what changes,
   **y/N** (no answer in `$SKHID_SWAP_ANSWER_MINUTES`, default 10, is a
   no), then: notes srp's newest backup of srp.db, stops srp26, keeps the
   old files in the folder, puts the folder's rollback in, puts the server
   data in place (a hard link renamed over the old file: atomic), sets
   `SRP_MIN_PATCH_REVISION=<rev>` (that line only; it must be at least the
   highest patch, or srp refuses to start) and runs
   `docker compose up -d --no-deps --force-recreate srp26` (after any
   `.env` change the container is made anew, never just restarted).
5. **Checks, then the ZIP**: the container running, healthy, not
   restarting; the log's `game data:` line; `/patches` exactly the list's.
   Then srp.db: at its start srp copies it with `VACUUM INTO` to
   `backups/srp-<unix>.db` before it opens it, so that copy is the state
   just before the release; it is copied (`docker exec ... cat`) into the
   folder and, from live, to the jump host (`$SKHID_BACKUP_DIR`, default
   `/srv/skhid-backups/srp-db`: folder 700, files 600, a line in
   `SHA256SUMS` there), its sha256 compared on both ends. Only then the
   ZIP goes in place and `/api/client-files` must name it.

A **failed check** (srp refusing to start, `/patches` wrong, no backup of
srp.db, the ZIP not handed out) prints srp's last log lines and the lines
where it says why — `srp: the game's files: <data> lacks what srp's code
names`, `srp: SRP_MIN_PATCH_REVISION is …`, `srp: cannot load the 0.26g
accounts: … lacks …` — and runs the folder's rollback at once, then checks
srp26 is up and `/patches` is as before.

A **stop before step 4** (a failure, a no, Ctrl-C) removes the staged
files (the folder stays, state `stopped`, with its release.log), puts the
image tag back and cancels the maintenance: the server is as it was (a
published patch stays with its row; the next run goes without
`--resources`). A stop in the swap before anything moved starts srp26
again as it was.

`--yes --i-know` (both or neither) answers both questions; only for a run
the owner watches anyway.

### A quick release: --no-maintenance

When the owner doesn't want a countdown (a small fix, few players on),
`--no-maintenance` skips it: no maintenance is planned and the admin page is
never asked (the preflight doesn't check it, nor `SRP_ADMIN_*` and
`SRP_WEB_TRUST_PROXY`). After the lock and the first y/N, step 3 (publish
the patch, build the server data and the ZIP, stage them, build the image)
runs at once; then it waits for `"matches":0` on `$TARGET_SITE/api/stats`
(`--match-wait`, default 120 min here; past it, a clean stop: nothing
swapped, the staged files removed, state `stopped`), shows what changes and
asks the second y/N (no answer in `$SKHID_SWAP_ANSWER_MINUTES` is a no). Right
before it stops srp26 it asks `/api/stats` once more: a match begun during
the question is waited for (within the same limit), and the stop follows a
`"matches":0` by one ssh. The swap, the checks and the rollback are the same.

What players see: those in a running match lose nothing (the swap waits for
`matches:0`); those in the lobby get a ~30 s "server sync" while srp26 is
made anew and reconnect by themselves; the launcher shows no countdown.
The progress lines show the time since the go-ahead instead of the time to
the planned end. `--minutes` and `--no-maintenance` are refused together.

### The release folder

Every release keeps a folder on its target:
`$TARGET_DIR/release-rev<rev>-<UTC YYYYMMDD-HHMM>[-<label>]/` (live:
`$SKHID_LIVE_DIR`, test: `$SKHID_TEST_DIR`). It holds:

| File | What |
|---|---|
| `state` | `staging`, `swapping`, `released`, `stopped`, `rolled-back` or `rollback-refused` |
| `release.log` | what the run printed (no secret is ever printed) |
| `Survarium-0.26g-srp-files.zip`, `server-game-data.db` | the new files, staged here; the swap hard-links them into `download/` |
| `new.sha`, `old.sha` | sha256sum of the new files, and of `download/`'s before |
| `old-Survarium-0.26g-srp-files.zip`, `old-server-game-data.db` | `download/`'s files before the swap (hard links) |
| `srp-<unix>.db`, `srp-db.name` | srp.db as it was before the release (srp's own start backup) |
| `image.old`, `image.new`, `min.old` | the image IDs before and after, `SRP_MIN_PATCH_REVISION` before |
| `changed.txt` | the server source files that changed, the ZIP's changed entries |
| `src.tar`, `old-src.tgz` | with server code: main's source, the tree's files it replaced |
| `rollback.sh`, `rollback.env` | the rollback (`release/host-rollback.sh`) and what it puts back |

A folder left in `staging` or `swapping` (a run that died) blocks the next
preflight: look at its `release.log`, finish or roll it back, then write
`stopped` in its `state`.

### What to watch

- The preflight: every line `ok`.
- After the first yes: the launcher countdown (players see it now; none
  with `--no-maintenance`).
- The ZIP's entries against the current ZIP (build-zip's list, shown again
  at the second question and in `changed.txt`): `survarium_skhid.exe`
  always differs (the revision); `updater_skhid.exe` only if the updater
  changed (players get it at their next launcher start); anything else
  must be explained by a merged change. Unexplained: answer no (that
  cancels cleanly).
- After the swap: the checks, the srp.db copy on both ends; then a game
  updates, downloads the patches, signs in; an old game is refused. Ask
  before starting any game window (Wine: unset `WAYLAND_DISPLAY`, run the
  exe directly).

## Rollback

```sh
release/rollback [--test]                                # the release folders and their state
release/rollback [--test] [--keep-db] release-rev<rev>-<stamp>[-<label>]
```

It runs the folder's `rollback.sh <folder>` on the target (by hand there:
`sh <folder>/rollback.sh <folder> [--keep-db]`, as root): the ZIP and the
server data from before (renamed into place), `SRP_MIN_PATCH_REVISION`
back (or its line removed), the server source back, the image that ran
before tagged `srp-026g:latest`, and srp.db as it was before the release
(the folder's copy; or, when the run did not get to keep one, srp's own
backup in the volume made by the new start; none at all means the new srp
never opened srp.db, and it is kept as it is). Every change to srp.db since
the release is lost; `--keep-db` keeps today's srp.db, only when the
release changed no schema and no player got an item of its new patch. It
makes srp26 anew and waits for it.

When srp refuses the old files (the accounts name items the old data
lacks), the rollback stops srp26 instead of letting it loop, says **roll
forward with a fixed patch** (the next revision), and prints the command
that puts the release back from its folder. The updater deletes local
patches the server's list no longer has; published patches stay on the
patches server.

## The test environment

`--test` does everything the live release does, against the test
environment: `$SKHID_TEST` in `$SKHID_TEST_DIR` (compose project
`srptest`, `$SKHID_TEST_COMPOSE` to change it), its site and game server
(`build-zip --test` makes a ZIP for them, compared with the test site's),
its own patches server and list (`$SKHID_TEST_PATCHES_SSH/_DIR/_URL`,
`release/patches-test.tsv`; the scripts refuse settings that are the
production folder or URL), its own release folders and lock. The admin
page there allows the jump host. srp.db is not copied to the jump host for
it (it is there). The host also has `/opt/srp-test/redeploy` (a branch on
it, built and recreated) and `/opt/srp-test/teardown`; read
`/opt/srp-test/README-test.txt` there.

## The steps it runs

Each is a script of its own (`--help`; it asks y/N before every write and
shows the command, unless release.sh, already given the go-ahead, runs it):

| Script | Does | Writes |
|---|---|---|
| `release/build-patch <folder> <rev> [--out <file>]` | packs a folder into `resources.db.patch_<rev>` (read back and compared) | a local file |
| `release/upload-patch [--test] [--dry-run] <file>` | publishes one patch: `.tmp`, sha256 there, `chmod 644`, `mv -n`, GET must be 200 with the sha256, then its row in the list; never replaces one | the patches server, the list |
| `release/add-mirror [--test] <name> <url>` | another URL of a listed patch: downloaded as the updater does, not a web page, the row's size and sha256, then appended to the row | the list (and the URL cache) |
| `release/build-server-db [--test] [--out <file>]` | `server-game-data.db` from the stock db and the list's patches (each first URL GET 200 with the row's size and sha256; each mirror checked as above), `srp-game-data build --patch "<file>=<urls>"` | a local file |
| `release/build-zip [--test] [--ref <commit>] [--work <dir>]` | the players' ZIP from a fresh worktree of origin/main (never this checkout, never `git stash`): `--server`, `--site`, `--localization`, no resources.db regions; refuses an unbumped revision; entries against the site's ZIP | a worktree, the ZIP |
| `release/deploy --zip --server-data --revision [--label] [--dry-run]` | the swap on its own, in a window planned by hand: the release folder, the checks, the rollback on failure | the live host |
| `release/rollback [--test] [--keep-db] [<folder>]` | a release folder's rollback | the target |
| `release/setup-patches-server [--test] [--check]` | once: the patches folder and Caddy's site block for it (validate and reload; either failing puts the backup back) | the jump host's Caddy |

`release/test.sh` runs them all, release.sh and the rollback included,
against stand-ins on this machine (`nix shell nixpkgs#python3
nixpkgs#shellcheck -c release/test.sh`); it sends nothing anywhere.

## How the game gets its content

- The game mounts the stock `game/resources.db`, then every
  `game/resources.db.patch_<n>` that exists, n = 1…255 ascending (gaps are
  normal). A later patch overrides files of earlier ones and adds new files;
  a patch can't delete. `n` is the **revision** the patch was released in.
- The updater asks the game server for `GET /patches`
  (`{"patch_12": {"sha256": "…", "urls": ["…", "…"]}, …}`), downloads the
  missing ones from the first URL that gives each, checks every patch's
  sha256, deletes local patches not in the list, and keeps PLAY off until
  all match. It hashes a patch file only when it changed (its size, time
  or file ID), the list changed for it, or its last hash is a week old;
  `updater_skhid.exe check` hashes them all (the repair).
- Updater 0.3.0 reads `urls`; 0.2.0 reads only `url`. The release that
  ships 0.3.0 ships the ZIP and the server (which lists `urls`) together:
  a 0.2.0 asks for the overlay first, installs 0.3.0, and leaves the patch
  step to it (the launcher restarts it at once); it never reads the new
  list. A rollback past it puts 0.2.0 back the same way (0.2.0 cannot
  read the list 0.3.0 kept beside it, so a start without the server keeps
  PLAY off until 0.2.0 has run once with it).
- The **patches server** is plain file storage: `$SKHID_PATCHES_URL`
  (files in `$SKHID_PATCHES_DIR` on `$SKHID_PATCHES_SSH`). Patch files there
  are never deleted or changed.
- The **game server** runs on `server-game-data.db`: the files it reads,
  built by `srp-game-data build` from the stock `resources.db` plus the
  list's patches with their URLs, on the release PC (the live host never
  stores the stock db). It serves `/patches` from what that file records,
  so it can only advertise the patches it runs on; it refuses to start when
  `SRP_MIN_PATCH_REVISION` is below its highest patch, or when srp.db or
  its code name what the data lacks.
- Login, lobby, match and web are one process (container `srp26-srp26-1`):
  a restart ends every match and drops every lobby link; games reconnect by
  themselves. A planned maintenance lives in its memory: a restart ends it.

## Record

A PR with `CHANGELOG-SKHID-026G.md` (players' wording, the UTC time, "update
required"), the new rows of `release/patches.tsv`, and the hashes: ZIP,
server data, each new patch, its revision (release.sh keeps the ZIP and the
server data in its working folder and prints their hashes; the release
folder has them in `new.sha`).
