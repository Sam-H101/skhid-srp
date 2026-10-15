# Skhid updater (Rust, Win32)

`updater_skhid.exe`: the program the patched 0.26g launcher
(`srp-026g/client-patches/launcher`) starts in the place of the game's own
`survarium_updater.exe`. It keeps the few Skhid files of a player's game
(the patched exe, the certificate, the crash reporter, the start scripts,
the launcher and itself) the same as the ZIP the server's page offers, and
then builds the game's database, `resources_skhid.db`: the game's own
`resources.db`, never changed, with the release's database patches applied
("The game's database" below).

A 32-bit Windows program of about 180 KB, without the Rust standard library
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
- After the Skhid files, at every start, the database step: `GET
  /resources/<sha256>` for the patches the list `updater_skhid.patches`
  names, applied to a copy of `resources.db` ("The game's database"
  below). Required: PLAY stays off until `resources_skhid.db` is checked
  whole at the release's revision.
- Writes `updater_skhid.state` (what is installed)
  and `updater_skhid.log` (what it did; no account name, no password; it
  does hold the game folder's path) beside itself. `updater_skhid.lock`
  prevents simultaneous installs in the same folder. The database step
  writes its own files beside it too (`updater_skhid.patches.state`,
  `.status`, `updater_skhid.cache\`).
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
| `updater_skhid.exe ready` (`quiet`: no box) | `play_skhid.bat`, `play_skhid.sh` before they start the game | Reads only: no lock, no server, nothing written but the log. Exit code 1 (and a box that says to start the launcher) when the list beside it cannot be read, or `resources_skhid.db` is not found at the release's revision by the quick look; else 0 (also with no list) |

Exit codes: 0, 1 (failed), 2 (the launcher's pipe could not be opened), 3
(an internal error; logged).

It must be in the game's `game\binaries\x86` folder. From anywhere else it
says so and does nothing.

## Layout

| Folder | What |
|---|---|
| `logic/` | Everything that needs no Windows, as a library with no dependencies: SHA-256, DEFLATE and CRC-32, the ZIP reader and its name rules, the text formats (the server's answer, the installed state, the database patches and their state, the old resource step's record) and what the database step decides, the launcher's pipe messages, command-line splitting, the server address. `cargo test` there runs on any host |
| `exe/` | The program: WinHTTP, files, the pipe, the install with its undo, the texts a player sees. Built for `i686-pc-windows-msvc` with `-Z build-std=core,alloc` and `rust-lld`, as `../bugtrap` |
| `tests/` | `test_updater.py`: the real exe against a web server on 127.0.0.1, with the test in the launcher's place on a real named pipe (Windows only). `test_skhid_db.py`: the database step, `check quiet`, on Windows or Wine. `fake_exe/`: a stand-in for the game and the launcher that writes down how it was started |

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

## The game's database

The game's own `resources.db` (6.3 GB, build 2777's) is never changed. The
updater builds `game\resources_skhid.db` beside it: a copy with every
database patch of the release applied in the order of their revisions, and
checks the whole result by its SHA-256. The game opens that file.

The players' ZIP holds the list, `game/binaries/x86/updater_skhid.patches`
(`make_client_026g.py --db-patches=<folder>`). The formats are defined in
`logic/src/dbpatch.rs` (its Python twin, for the tools, is
`srp-026g/client-patches/db_patches.py`):

```text
skhid-db-patches 1
original <sha256> <size>              the stock resources.db
skhid-db-patch 1                      each patch, in the order of revisions
revision 14
from <sha256 of the whole input> <size>
to <sha256 of the whole output> <size>
blob <sha256 of the blob> <size>
write <offset> <length>               bytes put over the input, in place
append <length>                       bytes added at the end
end
end
```

The blob, `GET /resources/<blob sha256>`, is zstd-compressed and holds the
bytes of every `write` and `append` in the order of the lines; the updater
decompresses it as it applies it (`ruzstd`, pure Rust, no `std`). Writes are
inside the input, in rising order, not overlapping; `to` is `from` and the
appends. Revisions rise and each `from` is the `to` before it, from the
original; the last `to` is the release's database. With no patch it is a
copy of the original. A list of a higher format, cut short, or breaking a
rule changes nothing and keeps PLAY off. The patches are never taken off the
server: a player who missed releases applies all of them.

Revision 14, the first, is three writes and no append: the menus players had
written in place before (`main_menu` 1 194 596 bytes at 17 970 448, the FOV
slider; `lobby_menu` 38 985 769 at 19 165 044, the exchange and the matches
list; `login_menu` 3 508 744 at 58 150 813, the account hint): stock + the
live targets `edc60a3d…`, `3a2c470d…` and `20ab1d22…` gives `to`
`4ad16d16…` (the same size). Its blob is 43 MB (the movies are compressed
already).

The step runs after the overlay at every start the game may be played
after, under the installation lock (also when the server could not be
asked: then with what is kept here only). When the run replaced the updater
or the launcher, the new ones start at once and do it.

1. A quick look: the state beside the updater,
   `updater_skhid.patches.state` (the revision, the whole file's size and
   SHA-256 as last checked, and the SHA-256 of its first and last 16 MB),
   against the file's size and those 32 MB. The release's revision: done.
2. Checked at an older revision: the newer patches' blobs are fetched (into
   `updater_skhid.cache\`, checked by size and SHA-256), the free space
   checked (the growth, the blobs, 64 MB), the state removed (an
   interruption from here on leaves a file nothing vouches for), each patch
   applied in place, the whole file hashed and compared with the last `to`,
   the state written.
3. There but no state that fits (lost, a damaged file, an update cut
   short): hashed whole; a revision of the list is taken as it is (2.),
   anything else is built anew (4.).
4. Missing or built anew: the old file is removed, the free space checked
   (the release's size, the blobs, 64 MB), every blob fetched, then
   `resources.db` copied into `resources_skhid.db.skhid-build` and hashed as
   it is read (it must be the list's `original`), every patch applied, the
   whole file hashed, and only then renamed into its place and the state
   written. A `.skhid-build` left by a run that was cut short is removed at
   the next start.
5. A result that is not the release's `to` is built anew from the original
   once; then PLAY stays off. The intermediate revisions of a build are not
   hashed (each would be another 6.3 GB read); a wrong one shows in the end.

Applied blobs are removed from the cache. A file the game holds (it keeps
its database open while it runs) is not changed: PLAY stays off until it is
closed.

A `resources.db` that is not the original (the copy's SHA-256 says so) is
downloaded again, once a run (`exe/src/game.rs`, `reinstall_db`): from the
address in `updater_skhid.db-url` beside the updater (else
`logic::game::DB_URL`, none yet), a plain or zstd-compressed
`resources.db`, unpacked beside it and checked against the list's
`original` before it replaces the file; without an address, from the game's
own pinned archive, as for a first installation (2.9 GB, extracted with
`7zr.exe`). Like the game's mirror, the address changes only where it comes
from, never what is accepted.

What keeps PLAY off ("Error" in the launcher's line, the reason in its box),
each followed by "PLAY stays off until this is done.":

| Found | The player is told |
|---|---|
| The game holds a file | "The game is running, so Skhid could not finish preparing the game's database (game\resources_skhid.db). Close the game, then start the launcher again." |
| Not enough disk | "Not enough free disk space to prepare the game's database in <folder>: N MB needed, M MB free. Free some space, then start the launcher again." |
| A blob is not here and the server could not be asked | "The update of the game's database could not be downloaded: the server could not be reached. ..." |
| `resources.db` is not the original and could not be downloaded again | "game\resources.db is missing or not the original game file, and it could not be downloaded again: <why>" |
| Anything else (a blob the server lacks or spoils, a list that cannot be read, a result that is not the release's twice) | "Skhid could not prepare the game's database (game\resources_skhid.db); the reason is in game\binaries\x86\updater_skhid.log. ..." |

The log has `db: BLOCKED: <what was found>. The player is told: <the words>`,
and `updater_skhid.patches.status` says `ready` or `blocked <what>`, for
support. Every step is in the log after `db:`. The launcher shows the step
as an install: "downloading" with the bar for a blob, "Extracting file
resources_skhid.db" with the bar for the copy and the patches, "checking"
for a whole hash.

`play_skhid.bat` and `play_skhid.sh` run `updater_skhid.exe ready` before
they start the game without the launcher: it reads only, and says no (exit
1) unless the quick look finds the release's revision.

## Putting resources.db back to stock (once)

The updaters before the database patches wrote the menus into the players'
`resources.db` in place (the "resource step" of #79), with a record
`updater_skhid.resources.state`, a journal while a write ran, and copies of
the originals in `updater_skhid.backup\`. Before anything else the step puts
those bytes back (from the copy kept, else from the server's blob of the
original) and cuts off what they appended (`logic/src/legacy.rs` reads the
record). Then the whole file is checked against the list's original: as it
is copied for a build, or hashed when `resources_skhid.db` needs no build.
Only once it is the original are the record, the journal, the old status,
the old plan and the copies removed. If it is not (a copy and the server's
blob both missing, a byte changed elsewhere) it is downloaded again as
above. This goes once every player's db is stock again.

Not handled: a `resources_skhid.db` damaged outside its first and last
16 MB while its state says it is fine is not noticed until it is rebuilt
(the game would crash on what it reads there); a `resources.db` deleted
by hand to save room is downloaded again by the overlay's first step, even
when `resources_skhid.db` is ready (keeping the original compressed is a
later change).

## Build and test

Use the same nightly Rust as the game workspaces (`srp-026g/flake.nix`
supplies it with `rust-src`). No Windows SDK is needed. The two locked
dependencies of `exe/` are Microsoft's `windows-sys` and `windows-link`, and
`ruzstd` 0.8.3 (zstd decoding, without its default features: no `std`, no
dependency of its own).

```sh
./build.sh                                   # exe/target/i686-pc-windows-msvc/release/updater_skhid.exe
SKHID_UPDATE_URL=https://test.example ./build.sh   # another default server
(cd logic && cargo test)
python tests/test_updater.py                 # Windows
python tests/test_skhid_db.py                # Windows or Wine (the game's database; needs zstd)
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

Run on 2026-10-16 (Linux, Wine 11.8 staging, `check quiet`), the database
step (the in-place step and `test_resource_tail.py` are gone): the 40 unit
tests, and the 16 scenarios of `test_skhid_db.py` (a 4 MB stand-in for the
stock db: a fresh build of two patches, found done after with nothing
fetched or hashed whole; a new release applying only its own patch; no
patch; a database at an older revision without its state; a damaged one
built anew; a build and an update in place cut short; a patch whose result
is not its `to`, built twice, then PLAY off; a blob the site lacks or
spoils; not enough disk, said before anything is fetched; lists that cannot
be read, and no list; an original that is not the original, downloaded
again (zstd) or not, and with no address the game's archive; a db changed
in place put back from the copies and the server, or downloaded again when
it cannot be, and beside a ready database; `play_skhid.bat` run by Wine's
cmd). Real data: revision 14 (stock and the three live menus) from a
`resources.db` with the menus written in place, its record and the copies of
the originals, as the live players have it: put back, checked stock, built
and checked whole in 61 s on an SSD (the copy 29 s, the whole hash 24 s);
`resources_skhid.db` `4ad16d16…`, `resources.db` `c74078be…`; the next start
under a second, `ready` 0.2 s. Not run: `test_updater.py`'s three new
scenarios (Windows: the launcher's bar, a running game holding the database,
the updater that replaced itself), a hard disk, the real launcher.

## Not done

- A signature on the ZIP made with a key that is not on the server. Today
  the ZIP is trusted because the HTTPS connection to the server is.
- Resuming a broken download (the ZIP is a few megabytes; it is fetched
  again).
- Texts in other languages than English (the launcher shows them in an
  ANSI message box).
- Keeping the original `resources.db` zstd-compressed (level 3) to save its
  6.3 GB: the updater has the decoder now (`exe/src/unzstd.rs`); the copy
  would read through it. A later change, with the overlay's first step
  taught that a compressed original is the game being there.
- The revision kept inside `resources_skhid.db` (the free bytes after its
  file table) in the place of the state file, and the list from a server
  endpoint as well as from the ZIP: the next change of the stack.
