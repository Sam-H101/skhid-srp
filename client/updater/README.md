# Skhid updater (Rust, Win32)

`updater_skhid.exe`: the program the patched 0.26g launcher
(`srp-026g/client-patches/launcher`) starts in the place of the game's own
`survarium_updater.exe`. It keeps the few Skhid files of a player's game
(the patched exe, the certificate, the crash reporter, the start scripts,
the launcher and itself) the same as the ZIP the server's page offers.

A 32-bit Windows program of about 80 KB, without the Rust standard library
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
  prompt is needed. Allow about 10 GB of free disk space.
- Writes `updater_skhid.state` (what is installed)
  and `updater_skhid.log` (what it did; no account name, no password; it
  does hold the game folder's path) beside itself. `updater_skhid.lock`
  prevents simultaneous installs in the same folder.
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

Exit codes: 0, 1 (failed), 2 (the launcher's pipe could not be opened), 3
(an internal error; logged).

It must be in the game's `game\binaries\x86` folder. From anywhere else it
says so and does nothing.

## Layout

| Folder | What |
|---|---|
| `logic/` | Everything that needs no Windows, as a library with no dependencies: SHA-256, DEFLATE and CRC-32, the ZIP reader and its name rules, the two text formats, the launcher's pipe messages, command-line splitting, the server address. `cargo test` there runs on any host |
| `exe/` | The program: WinHTTP, files, the pipe, the install with its undo, the texts a player sees. Built for `i686-pc-windows-msvc` with `-Z build-std=core,alloc` and `rust-lld`, as `../bugtrap` |
| `tests/` | `test_updater.py`: the real exe against a web server on 127.0.0.1, with the test in the launcher's place on a real named pipe (Windows only). `fake_exe/`: a stand-in for the game and the launcher that writes down how it was started |

## Original game download and verification

`logic/src/game.rs` pins the original build-2777 archive at 2,906,544,103
bytes and SHA-256
`7435558eeb17098fc9337c4ba9470402cae4c9be974ef5d9bc544f19b122a028`.
The complete Drive download was checked against archive.org's independently
published SHA-1 `bb30b71fab155b3435917690fd2be87984546163` before pinning it.

Downloads use a 64 KiB buffer, including in the 32-bit updater. The complete
file is flushed, read back and hashed before extraction. The official
standalone [7-Zip extractor](https://www.7-zip.org/download.html) is also
downloaded and checked against its own compiled-in size and SHA-256 before
it is run. Both files stay locked against writes and deletion until the
extractor exits. Its download URL may change bytes when 7-Zip releases a
new version; a mismatch stops installation and requires a reviewed pin
update or a mirror of the pinned version. There is no verification bypass.

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

To move hosting, put a direct HTTPS URL in `updater_skhid.game-url` beside
the updater (and `updater_skhid.extractor-url` for the pinned extractor).
These files change only the source, never the trusted hash. Plain HTTP is
accepted only on localhost for tests; FTP is not supported. No server
deployment or binary rebuild is needed to change the mirror this way.

`tests/test_game_download.py` runs the actual exe on Windows or Wine against
a local HTTP server, covering failed downloads and preservation of existing
files. With `SKHID_TEST_GAME_ARCHIVE` and `SKHID_TEST_EXTRACTOR` pointing to
the pinned files, it also downloads, verifies and extracts the full game,
then installs the Skhid overlay. It does not contact Drive or archive.org.

## Build and test

Use the same nightly Rust as the game workspaces (`srp-026g/flake.nix`
supplies it with `rust-src`). No Windows SDK is needed. The two locked
dependencies of `exe/` are Microsoft's `windows-sys` and `windows-link`.

```sh
./build.sh                                   # exe/target/i686-pc-windows-msvc/release/updater_skhid.exe
SKHID_UPDATE_URL=https://test.example ./build.sh   # another default server
(cd logic && cargo test)
python tests/test_updater.py                 # Windows
python tests/test_game_download.py           # Windows or Wine
SKHID_TEST_BUNDLE=<a players' zip> python tests/test_updater.py
```

`make_client_026g.py` puts the built exe into the players' ZIP and writes
`updater_skhid.url` for the server the ZIP is made for.

Run on 2026-10-13 (Windows 11): the 28 unit tests and the 18 end-to-end
scenarios pass; the real launcher drove it through an install, a repair, a
replacement of the launcher itself and a start with the server down. Not
run under Wine yet.

## Not done

- A signature on the ZIP made with a key that is not on the server. Today
  the ZIP is trusted because the HTTPS connection to the server is.
- Resuming a broken download (the ZIP is a few megabytes; it is fetched
  again).
- Texts in other languages than English (the launcher shows them in an
  ANSI message box).
