# Skhid updater (Rust, Win32)

`updater_skhid.exe`: the program the patched 0.26g launcher
(`srp-026g/client-patches/launcher`) starts in the place of the game's own
`survarium_updater.exe`. It keeps the few Skhid files of a player's game
(the patched exe, the certificate, the crash reporter, the start scripts,
the launcher and itself) the same as the ZIP the server's page offers, and
then rewrites in place the few entries of the game's own `resources.db` that
the ZIP's plan names, and, when the plan says so, grows the db by new
entries ("Resources" below). Last, it makes the game folder's
`resources.db.patch_<n>` files, which the game mounts over `resources.db`,
the ones the server lists ("Patches" below).

A 32-bit Windows program of about 130 KB, without the Rust standard library
and without a C runtime, like `../bugtrap`. It imports `kernel32.dll`,
`user32.dll` (its message box) and `winhttp.dll`.

## What it does, and does not

- Asks for Skhid updates at the host in `updater_skhid.url` beside it, else the one it
  was built with (`SKHID_UPDATE_URL`, default `https://skhid.link`). HTTPS
  through Windows' own WinHTTP (its TLS, its certificate store, the system's
  proxy settings); plain HTTP only for `127.0.0.1` and `localhost`. No
  redirect is followed, no cookie kept.
- `GET /api/client-files` → `skhid-client-files 1`, `sha256 <hex>`,
  `size <bytes>`; `GET /download` → the ZIP. Both are routes of
  `srp-026g/servers/web-server`.
- Installs only what passes every check, all files or none, and can replace
  the running launcher and itself. The steps, and what a player sees when
  one fails: `srp-026g/client-patches/launcher/README.md` ("How an update
  works", "When something goes wrong").
- If the original `survarium.exe` or `resources.db` is missing, downloads
  the original game from the Google Drive mirror, checks a compiled-in
  SHA-256 and installs the missing original files before the Skhid overlay.
  A new player can unpack the overlay into an empty folder and start
  `Skhid.bat`. No Google login, browser confirmation or administrator
  prompt is needed. Allow about 10 GB of free disk space. The game is
  unpacked with `7zr.exe`, a file of the Skhid archive beside the updater
  (`extractor/`): nothing that is run is downloaded on its own.
- After the Skhid files, at every start, the resource step: `GET
  /resources/<sha256>` for the blobs the plan `updater_skhid.resources`
  names, written over entries of the game's `resources.db` in place, and
  appended to it when the plan has a `tail` ("Resources" below). Best
  effort for a plan without a tail: it never fails the update and never
  keeps PLAY off. A plan with a tail is required: PLAY stays off until the
  db is checked whole.
- After the resource step, at every start, the patch step: `GET /patches`,
  the server's list of resources.db's patches, each with its SHA-256 and
  the address of its file (plain HTTP or HTTPS, any host); a listed patch
  that is missing or damaged is downloaded, a patch file the list has not
  is deleted, every patch file is hashed. Required: PLAY stays off until
  every listed patch is in place ("Patches" below).
- Writes `updater_skhid.state` (what is installed)
  and `updater_skhid.log` (what it did; no account name, no password; it
  does hold the game folder's path) beside itself. `updater_skhid.lock`
  prevents simultaneous installs in the same folder. The resource step
  writes its own files beside it too (listed under "Resources").
- No BitTorrent, no listening port, no UPnP, no registry, no service, no
  crash upload, no administrator rights (its manifest says `asInvoker`;
  without one Windows takes a program named "updater" for an installer and
  asks for elevation).

## How it is started

| Command line | Who | What |
|---|---|---|
| `"<exe>" "update" "\\.\pipe\vostok_updater_pipe-<product>" ...` (also `full_update`) | the launcher | The update, reported over the launcher's pipe in the messages of the original updater (`logic/src/pipe.rs`). It stays until the launcher says stop or goes away |
| `"<exe>" "copy" "<pipe>" ...` | the unpatched launcher | Answers "copy done" at once: nothing needs copying |
| `updater_skhid.exe check` | a person, a script | The update without a launcher; an error is a message box |
| `updater_skhid.exe check quiet` | a script | No box either. Exit code 0: done or nothing to do; 1: failed (the reason is in the log) |
| `updater_skhid.exe ready` (`quiet`: no box) | `play_skhid.bat`, `play_skhid.sh` before they start the game | Reads only: no lock, no server, nothing written but the log. Exit code 1 (and a box that says to start the launcher) when the plan beside it grows the db and the db is not checked whole, or the plan cannot be read and may be one (format 2 or newer); also when the last patch step did not end `ready`, or the patch files are not now what its list says (one missing, damaged, or one more); else 0 |

Exit codes: 0, 1 (failed), 2 (the launcher's pipe could not be opened), 3
(an internal error; logged).

It must be in the game's `game\binaries\x86` folder. From anywhere else it
says so and does nothing.

## Layout

| Folder | What |
|---|---|
| `logic/` | Everything that needs no Windows, as a library with no dependencies: SHA-256, DEFLATE and CRC-32, the ZIP reader and its name rules, the text formats (the server's answer, the installed state, the resource plan and its record, the list of resources.db's patches) and what the resource step and the patch step decide, the launcher's pipe messages, command-line splitting, the server address and a patch's. `cargo test` there runs on any host |
| `exe/` | The program: WinHTTP, files, the pipe, the install with its undo, the texts a player sees. Built for `i686-pc-windows-msvc` with `-Z build-std=core,alloc` and `rust-lld`, as `../bugtrap` |
| `tests/` | `test_updater.py`: the real exe against a web server on 127.0.0.1, with the test in the launcher's place on a real named pipe (Windows only). `fake_exe/`: a stand-in for the game and the launcher that writes down how it was started |

## Original game download and verification

`logic/src/game.rs` pins the original build-2777 archive at 2,906,544,103
bytes and SHA-256
`7435558eeb17098fc9337c4ba9470402cae4c9be974ef5d9bc544f19b122a028`.
The complete Drive download was checked against archive.org's independently
published SHA-1 `bb30b71fab155b3435917690fd2be87984546163` before pinning it.

Downloads use a 64 KiB buffer, including in the 32-bit updater. The complete
file is flushed, read back and hashed before extraction.

The extractor is the official standalone `7zr.exe` of 7-Zip 26.04, unchanged
(`extractor/`, with where it is from and its licence). It is not downloaded:
`make_client_026g.py` puts it into the Skhid archive as
`game/binaries/x86/7zr.exe`, beside the updater, and refuses another file
than the pinned one. The updater checks it against its compiled-in size and
SHA-256 before anything is downloaded, and runs no other file. The archive
and the extractor stay locked against writes and deletion until the
extractor exits. There is no verification bypass. (7-Zip's own address for
the file has no version in it: its bytes change with every 7-Zip release,
which is why it is not fetched from there.)

When the game is missing and `7zr.exe` is missing too, or is not that file
(deleted, damaged, an archive only half unpacked), the Skhid files are
installed first: the server's archive brings the extractor, and the game
follows in the same run (after the restart, when the launcher or the updater
was replaced). If the extractor is still not right after that, the updater
says so and downloads nothing.

Only five game files are extracted into a fresh staging folder: the original
exe, resources database, crash DLL, certificate and splash image. The
extracted exe must also match its known SHA-256. Only after successful
extraction and checking are missing files moved into place; existing files
are preserved, and a failed move rolls back files added by that attempt.
The normal verified Skhid overlay then replaces the original certificate
and crash DLL when needed. Existing complete games skip this bootstrap.

The direct Drive endpoint was tested without cookies or sign-in. HTML
confirmation pages, quota errors, redirects, interrupted transfers and hash
mismatches fail closed. A failed transfer is removed; a completed verified
archive is kept if extraction fails and is rehashed before retrying. The
cache is deleted after successful installation. Interrupted transfers are
not resumed.

What an attempt that was cut short leaves behind (the PC switched off, the
updater killed while the extractor ran) is removed at the next start, under
the installation lock: every `.skhid-extract-*` folder in the game folder
(up to 6.3 GB each; only the five game files and their folders are deleted,
so a folder with anything else in it is left and named in the log), and,
once the game is there, a `.skhid-game.7z` that is still beside it. The
extractor runs in a Windows job that ends with the updater, so an updater
that is killed takes it along and it does not write on by itself. The
room asked for is the room still needed: an archive that is already
downloaded is not counted again.

What the extractor prints (its messages and its errors, as UTF-8) goes to
`updater_skhid.7zr.txt` beside the updater while it runs. When it has ended,
whatever its exit number, the updater copies those lines into
`updater_skhid.log`, each after `7zr:`, and deletes the file: a failed
extraction then says in the log which file could not be written and why,
not "exit 2" alone. A file left by a killed updater is copied and deleted
at the next start. When the file cannot be made, the extractor runs all the
same.

To move hosting, put a direct HTTPS URL in `updater_skhid.game-url` beside
the updater. That file changes only the source, never the trusted hash. Plain HTTP is
accepted only on localhost for tests; FTP is not supported. No server
deployment or binary rebuild is needed to change the mirror this way.

`tests/test_game_download.py` runs the actual exe on Windows or Wine against
a local HTTP server, covering failed downloads, preservation of existing
files, an extractor that is missing or changed, and what a cut-short attempt
left. With `SKHID_TEST_GAME_ARCHIVE` pointing to the pinned archive, it also
downloads, verifies and extracts the full game with the extractor of
`extractor/`, then installs the Skhid overlay. It does not contact Drive or
archive.org.

## Resources

The players' ZIP may hold a plan, `game/binaries/x86/updater_skhid.resources`
(`make_client_026g.py --resource-region`, `--resource-grow`; the format and
its rules are in `logic/src/resources.rs`):

```text
skhid-resources 2
region <path> <file size> <offset> <size> <original sha256> <target sha256> <name>
tail <path> <file size> <new size> <tail sha256> <name> <region>
file <sha256> <size> <path>
end
```

A plan without a `tail` is written as format 1, as before, so that the
updaters already out there read it; a `tail` comes only in format 2. An
updater of format 1 refuses a plan of format 2 whole ("newer format",
nothing is changed): it never writes a table without its tail. The ZIP
brings the new updater, which starts at once and does the step.

A `region` line says that the bytes `[offset, offset + size)` of a game
file (today `game/resources.db`, 6.3 GB) are the blob `<original>` as the
game is shipped, and are to become the blob `<target>`: an entry of the db
rewritten in place with the same size and the same game CRC, so the db's
signed file table is not changed (the first two: the lobby's and the login
screen's Flash movies). Or the file table itself, rewritten with a `tail`.

A `tail` line says that the file, `<file size>` bytes as shipped, is to grow
to `<new size>`: the blob `<tail sha256>` appended at its end, the new
entries of the db. `<region>` names the region of the plan written with it:
the db's file table (the head, `[0, n)`), rewritten to name the new
entries. One tail per file. A region's `<file size>` stays the size as
shipped: its file is of its build when it has that size, or that size grown
by a tail of the plan or of the record. A `file` line is a whole file fetched by its
checksum, for later (new maps, if the game can load them); it may not name
a file of the overlay, an original game file (`game::FILES`) or one of the
updater's own. A line with an unknown word is skipped. A plan of a higher
format, one cut short (no `end`), one with a line that cannot be read, or
one where two lines claim the same bytes, changes nothing at all.

The step runs after the overlay at every start the game may be played
after: also when nothing new was installed, and when the server could not
be asked (then with what is kept here only: nothing is downloaded). When
the run replaced the updater or the launcher, the new ones start at once
and do it. It holds the installation lock, as the overlay does. For each
region:

1. The region is hashed (39 MB in 125 ms on the test PC with the file in
   Windows' cache; a cold read from a hard disk is slower, not measured).
   It holds the target: done, nothing downloaded or written.
2. A missing file, or a file of another size than the plan's (another
   build of the game): skipped, logged.
3. The file is opened by the updater alone (share mode 0) and closed again.
   The game keeps `resources.db` open from its start to its exit (share
   mode read and write), so this fails with error 32 while a game runs: the
   log says "the game is running" and no region is changed this run, and
   nothing is fetched for them either.
4. The target is fetched from `<site>/resources/<sha256>` into
   `updater_skhid.cache\<sha256>.part`, streamed to the disk 64 KiB at a
   time, flushed, its size and SHA-256 checked as read back from the disk,
   then renamed to `<sha256>`. A blob that is there and checks is used
   again. Two tries a run.
5. The file is opened by the updater alone again. A region that holds the
   original, with no checked copy of it yet, is copied into
   `updater_skhid.backup\<original sha256>` first (written, flushed, read
   back, checked); if that fails, the region is not written.
6. The region is recorded in `updater_skhid.resources.state` and noted in
   `updater_skhid.resources.journal` (both flushed) before its first byte is
   written: a region is never changed without a note of how to put it
   back. Then the blob is written at the region's offset (64-bit offsets),
   `FlushFileBuffers`, the region read back and checked, and the journal
   and the cached blob deleted.
7. If the write or the check fails, the original is written back (from the
   copy kept, else from the site's blob of the original) and checked. If
   that fails too, the region stays recorded for the next run.
8. A region that holds neither its original nor its target (a write cut
   short, a disk error, another program, the target of an earlier plan) is
   damaged: the target is written over it. The checksum decides; a journal
   left over is told in the log.

A tail, after the regions and the tails that leave, before the plan's
regions:

1. The file's size and the bytes past the size as shipped tell what it
   holds: the size as shipped (to append), the tail (done), or longer by
   something else. Longer by what this step appended before (an append cut
   short, an older tail; the record says so) is cut off first. Longer or
   shorter by anything else is another build or another program's: nothing
   is written, and the plan is not in place (below).
2. The disk must have room for the tail and, when it is not in the cache
   yet, its blob, plus 64 MB; else nothing is fetched.
3. The blob is fetched as a region's, the file opened by the updater alone.
4. The table (the tail's region), when it holds its original, is copied
   into `updater_skhid.backup\` first.
5. The tail is recorded and journalled (flushed) before the first byte is
   appended. The file is cut to its size as shipped, the blob appended,
   `FlushFileBuffers`, the size and the tail read back and checked.
6. If that fails, the file is cut to its size as shipped again.

Every region of a file with a tail (the table, and the lobby's and the
login's) waits until the tail is in place: a table is never written that
points past the file's end. If the table cannot be written, its original is
put back and the tail cut off.

A plan with a tail is *required* (the game with the new exe and the old db
crashes on the items the server sends). At the end of the step every file
with a tail is checked whole: its size, its tail, and every region of the
plan in it. If anything is not in place:

- the file goes back to its bytes as shipped: each region of the plan in it
  gets its original back, then the tail is cut off (the tail's blob stays
  in the cache, so the next start does not download it again). Not while
  the game holds it: then nothing was changed this run.
- the update fails with words that say what to do, and PLAY stays off
  ("Error" in the launcher's line, the reason in its box):

  | Found | The player is told |
  |---|---|
  | The game holds the db | "The game is running, so Skhid could not finish updating the game's files (game\resources.db). Close the game, then start the launcher again." |
  | Not enough disk | "Not enough free disk space to update the game's files in <folder>: N MB needed, M MB free. Free some space, then start the launcher again." |
  | The db is of another size, or grown by something else | "game\resources.db is not the game file this Skhid release is for (N bytes, expected M). Delete game\resources.db and start the launcher again: the original game is downloaded again (about 3 GB)." |
  | A blob is not here and the server could not be asked | "The update of the game's files could not be downloaded: the server could not be reached. Check the connection, then start the launcher again." |
  | Anything else (a blob the server lacks or spoils, a write that fails, a plan that cannot be read but may grow the db: format 2 or newer) | "Skhid could not finish updating the game's files (game\resources.db); the reason is in game\binaries\x86\updater_skhid.log. Start the launcher again; if this keeps happening, send that file to the server's operator." |

  each followed by "PLAY stays off until this is done."
- the log has a line `resources: BLOCKED: <what was found>. The player is
  told: <the words>`, and `updater_skhid.resources.status` beside the
  updater says `blocked <what was found>` (`ready` once it is in place),
  for support.

The next start repairs it: the checksums and the size decide, a journal
left over is told in the log. `check quiet` exits 1 in that case.

A recorded region the plan no longer has (or the plan itself is gone,
because the server's ZIP has none) gets its original back the same way,
before the plan's own regions are written. A planned region over the bytes
of a recorded one whose original could not be put back waits. A region of
the plan with a new target is written over the old target directly (the
copy of the original stays the way back). A recorded `file` the plan no
longer has is deleted if it is still the file placed. A recorded tail the
plan no longer has is cut off once its table is back (a tail whose table
the plan keeps, with a new tail, is cut off at once; the new tail and the
new table follow). Every handle on the db is closed before the launcher is
told that PLAY may be pressed.

What it never does: fail the update for a plan without a tail, show a
message box of its own, change the overlay's files or `updater_skhid.state`,
or touch a byte outside the regions and the tails. Every step and failure is in `updater_skhid.log`, each line after
`resources:`. The launcher shows it as an install: "downloading" with the
bar for a blob, "Extracting file <name>" while a region is written, then
"latest" as always. When there is nothing to do it says nothing.

Its files beside the updater: `updater_skhid.resources` (the plan, from the
ZIP), `updater_skhid.resources.state` (the record),
`updater_skhid.resources.journal` (only while a region is written or a
tail appended), `updater_skhid.resources.status` (a required plan's last
result),
`updater_skhid.cache\` (blobs not written yet; emptied and removed when
done) and `updater_skhid.backup\` (the originals of the regions planned or
recorded, 39 MB and 3.5 MB today; removed when no region needs them).

Not handled: a record that is lost or damaged (a region the plan still
names is recorded again when it holds its target; one that left the plan
meanwhile keeps its target); a game started with `play_skhid.bat` in the
second a region is being written (its own open of the db fails then); a
db of the plan's size but another build (the size is the only sign of the
build the plan gives, so its regions would count as damaged and be
overwritten).

`play_skhid.bat` and `play_skhid.sh` start the game without the launcher.
Before they do, they run `updater_skhid.exe ready`: while a required plan
is not in place, they say so ("Skhid has not finished updating the game's
files ... Start Skhid.bat (the launcher)") and do not start the game. With
no updater beside them (an archive without the launcher) they start it as
before.

## Patches

The game (client patch revision 14, patch 27 of
`srp-026g/client-patches/README.md`) mounts the retail `game/resources.db`
and then every `game/resources.db.patch_<n>` there, n = 1 to 255 in order,
a missing number skipped; a later patch's file wins. n is the patch
revision that brought the patch. The updater keeps those files the ones the
server lists (`logic/src/patches.rs`, `exe/src/patches.rs`).

**The list.** `GET <server>/patches`, from the server the updater asks for
everything else (`updater_skhid.url`, else the built-in
`SKHID_UPDATE_URL`; so HTTPS in production):

```json
{"patch_14": {"sha256": "<64 hex>", "url": "http://203.0.113.5/patch_14"},
 "patch_17": {"sha256": "<64 hex>", "url": "https://files.example/db/patch_17"}}
```

A JSON object and nothing else: every key `patch_<n>` with n from 1 to 255
and no leading zero (the game's own rule), no key twice; every value an
object of exactly `sha256` (64 hex characters, either case) and `url`
(`http://` or `https://`, a host name or a bare IPv4 address, an optional
port, and a path; no credentials, no fragment, no space, no other scheme;
at most 1024 bytes). No escape in any string. At most 512 KB. `{}` is a
list: no patch (every patch file is deleted). Anything else is refused
whole, with nothing changed. 404 is "no list", not "no patch".

**The files.** `GET <url>` of each entry, on whatever host the list names:
a plain file server is enough. Plain HTTP is taken there, to any host, as
it is not for the update server: the file is checked against the list's
SHA-256, and the list came from the update server over HTTPS. No redirect
is followed. The Content-Length, when the server sends one, is checked
against the disk first.

**A run**, after the resource step (also when nothing new was installed;
under the installation lock):

1. The list is fetched (three tries, 3 s apart) and read. It is kept beside
   the updater as `updater_skhid.patches` (one line, the same form), and
   `updater_skhid.patches.status` says `blocked checking` until the run is
   through.
2. Every `game\resources.db.patch_*` that is not a listed patch is deleted:
   one the list no longer has (a rollback), a name the game does not take
   (`_014`, `_0`, `_300`), a download left over (`.skhid-part`).
3. Every listed patch's file is hashed. A damaged one is deleted. A missing
   or damaged one is downloaded into `resources.db.patch_<n>.skhid-part`
   beside its place (the disk must have its Content-Length plus 64 MB free,
   else nothing is fetched), flushed, read back and hashed, and renamed to
   its name only when it has the list's SHA-256. Two tries a run; a file
   that is still wrong, or a network failure, ends the run. A `.skhid-part`
   that failed is deleted.
4. Every patch file is hashed once more against the list (step 2's rule
   too). Then `updater_skhid.patches.status` says `ready`.

When the update server could not be asked this run (the overlay's offline
case), nothing is fetched: the game may be played when the last run ended
`ready` and the folder is still what the kept list says; else PLAY stays
off. A first start with the server down therefore keeps PLAY off (before
this step it let the player try).

What keeps PLAY off ("Error" in the launcher's line, the reason in its box,
`check quiet` exits 1, `updater_skhid.patches.status` says `blocked <why>`,
the log has `patches: BLOCKED: <why>. The player is told: <words>`), each
told with "PLAY stays off until this is done." after it:

| Found | The player is told |
|---|---|
| The list cannot be fetched (network, a status but 404) | "The list of the game's patches could not be fetched. <the network error>" |
| `/patches` answers 404 | "<host> offers no list of the game's patches. Tell the server's operator." |
| The list is not one (above) | "The list of the game's patches from <host> cannot be used (<what>). Tell the server's operator." |
| A file cannot be downloaded | "game\resources.db.patch_<n> could not be downloaded. <the network error>" |
| A file has another SHA-256, twice | "<host> sent game\resources.db.patch_<n> with another checksum than its list says. Tell the server's operator." |
| Not enough disk | "Not enough free disk space to update the game's files in <folder>: N MB needed, M MB free. ..." |
| A patch file is held (the game runs) | "The game is running, so Skhid could not update the game's patches ... Close the game, then start the launcher again." |
| The server could not be asked and the patches were not checked in place | "The game's patches could not be checked: <why the server could not be asked>" |
| Anything else (a file that cannot be written or deleted, the ZIP bringing a `game/resources.db.patch_*`) | "Skhid could not finish updating the game's patches ...; the reason is in game\binaries\x86\updater_skhid.log. ..." |

The next start tries again. A file that is never half written in place:
what the game mounts is a checked file or nothing.

`ready` (play_skhid.bat, play_skhid.sh) checks the same as the offline case
(the status `ready`, the folder against the kept list, every file hashed)
and refuses otherwise: also before the launcher has ever run the step.

The resource step is not changed by this: a ZIP that drops the old menu
regions gives the players' resources.db its stock bytes back, and the menus
come with a patch instead.

## Build and test

Use the same nightly Rust as the game workspaces (`srp-026g/flake.nix`
supplies it with `rust-src`). No Windows SDK is needed. The two locked
dependencies of `exe/` are Microsoft's `windows-sys` and `windows-link`.

```sh
./build.sh                                   # exe/target/i686-pc-windows-msvc/release/updater_skhid.exe
SKHID_UPDATE_URL=https://test.example ./build.sh   # another default server
(cd logic && cargo test)
python tests/test_updater.py                 # Windows
python tests/test_resource_tail.py           # Windows or Wine (a plan that grows the db)
python tests/test_db_patches.py              # Windows or Wine (resources.db's patches)
python tests/test_game_download.py           # Windows or Wine
SKHID_TEST_BUNDLE=<a players' zip> python tests/test_updater.py
```

The build is reproducible: the same source, toolchain and `SKHID_UPDATE_URL`
give the same exe, byte for byte, in any folder:
- `exe/.cargo/config.toml` gives `rust-lld` `/Brepro` (the PE stamp is a hash
  of the content, not the time) and `/DEBUG:NONE` (no PDB, which would record
  the build folder), as `../bugtrap`;
- `exe` and `logic` are one Cargo workspace (`Cargo.toml` here, one
  `Cargo.lock`). Cargo names a path dependency outside the workspace by its
  absolute path, and that name reaches the exe's symbols and their order. The
  exe still builds to `exe/target/` (`target-dir` in `exe/.cargo/config.toml`);
  `cargo test` in `logic` builds to `target/` here.

`../check-reproducible.sh` builds the updater and BugTrap twice, in two
folders, and compares.

`make_client_026g.py` puts the built exe into the players' ZIP and writes
`updater_skhid.url` for the server the ZIP is made for.

Run on 2026-10-13 (Windows 11): the 28 unit tests and the 18 end-to-end
scenarios pass; the real launcher drove it through an install, a repair, a
replacement of the launcher itself and a start with the server down. Not
run under Wine yet.

Run on 2026-10-14 (Windows 11), with the resource step: the 39 unit tests
and the 30 end-to-end scenarios pass (one skipped without
`SKHID_TEST_BUNDLE`, as before), 12 of them the resource step's (a
4 MB stand-in for `resources.db`, a plan made by `make_client_026g.py`'s
own functions, and one region of the lobby movie's size). Not run yet: the
real launcher, a real `resources.db`, Wine.

Run on 2026-10-15 (Linux, Wine 11.8 staging, `check quiet`), with tails:
the 44 unit tests, and the 15 scenarios of `test_resource_tail.py` (a 4 MB
stand-in db grown by 1 MB: a fresh install, found done after, a db grown
already without a record, the plan dropped, a new release replacing the
tail and the table, a tail or a table or the lobby the site lacks or spoils,
a db of another size, an append cut short, a table without its tail, plans
that cannot be read, a plan without a tail staying best effort; `ready`, and
the players' `play_skhid.bat` run by Wine's cmd with a stand-in game,
refusing to start it until the db is in place).
`test_updater.py`'s two changed scenarios (a newer format; a running game
with a required plan) are not run yet: they need Windows.

Run on 2026-10-16 (Linux, Wine 11.8 staging, `check quiet`), with the
patch step: the 62 unit tests; the 16 scenarios of
`test_db_patches.py` (a fresh install of patches 1, 14, 17 and 255 and the
next start downloading nothing; an empty list; the files on a second
server; one missing; one damaged here; patch files the list has not,
names the game does not take and a download left over deleted, and a
rollback; a server file that is wrong, twice; a damaged file the server
spoils too; nine lists that cannot be read, changing nothing; no
`/patches`; a list that cannot be fetched; a file server that cannot be
reached; the server down before and after a checked run; not enough disk,
by a Content-Length past the free space; the ZIP bringing a patch file;
`ready` and play_skhid.bat run by Wine's cmd refusing until the patches are
checked, and again with a patch file added); `test_resource_tail.py` (15)
and `test_game_download.py` (16, four skipped as before) again, with their
servers now listing no patch. `test_updater.py`'s two changed scenarios (a
first start with the server down, a server without client files: PLAY
stays off now) and a game that holds a patch file are not run: they need
Windows. play_skhid.sh is not run with Wine (it calls the same `ready`).

## Not done

- A signature on the ZIP made with a key that is not on the server. Today
  the ZIP is trusted because the HTTPS connection to the server is.
- Resuming a broken download (the ZIP is a few megabytes; it is fetched
  again).
- Texts in other languages than English (the launcher shows them in an
  ANSI message box).
