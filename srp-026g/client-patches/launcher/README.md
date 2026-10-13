# The Skhid launcher: the 0.26g launcher made to work again (build 2777)

The game's own launcher (`survarium_launcher.exe`), patched so that it

- starts from an unpacked game folder, with no installer, no registry
  values, no service and no administrator rights;
- asks **one server only** (the Skhid one, or the one it is built for): no
  request to any of the game's old sites can leave it;
- shows that server's own page in its window (the best players, what the
  server counts, the legal notice);
- starts a **new updater** (`updater_skhid.exe`, `../../../client/updater`)
  that keeps the Skhid files of the game current from the same ZIP the
  server's page offers, the launcher and the updater themselves included;
- starts the game in the language chosen in its menu.

The original launcher is not in this repository. The tools below work on
your own copy, and the patched exe is made from it.

| | |
|---|---|
| Original | `game/binaries/x86/survarium_launcher.exe`, 4 740 728 bytes, sha256 `710956d2347864effb353064dd7beaee1dd2773c74756d3f6677df71431c5266` (the tools check it and refuse anything else) |
| Result for `https://skhid.link` | `launcher_skhid.exe`, 4 735 488 bytes; the hash depends on the site, the game exe's name and its arguments |
| Status, 2026-10-13 | Built and run on Windows 11 against a local srp server, in a test game folder (see "What was run"). **Not yet** run under Wine, on another Windows version, or against the public server. Not in the players' download |

## The files

| File | What |
|---|---|
| `patch_launcher_026g.py` | Builds the patched launcher from the original. Python 3, nothing else |
| `launcher_values_026g.asm`, `.bin` | The one piece of new code (224 bytes and three text slots): the values the launcher used to read from the installer's registry key, worked out from where the launcher is. Assembling the source again gives the same bytes |
| `launcher_pages.py` | The ten small pages the launcher shows from its own folder while the server's page loads and when it cannot be loaded |
| `selftest_launcher_026g.py` | Builds it in memory and checks it: every changed byte is one named below, the PE is well formed, the hook and its way back, the addresses, the relocations, no old address left, the `.asm` gives the `.bin` |
| `../make_client_026g.py` | Puts all of it into the players' ZIP (the default; `--no-launcher` leaves it out) |
| `../../../client/updater/` | The updater: Rust source, its tests |
| `../../servers/web-server/src/launcher.rs` | The server's side: the page and "which archive do you offer" |

## How to build

```
python patch_launcher_026g.py <game>/binaries/x86/survarium_launcher.exe <out>/launcher_skhid.exe [--site=https://skhid.link]
python selftest_launcher_026g.py <game>/binaries/x86/survarium_launcher.exe --nasm=<nasm>
```

Normally `make_client_026g.py` does it, with the updater and the pages:

```
../../../client/bugtrap/build.sh
../../../client/updater/build.sh
python ../make_client_026g.py <game folder> <new.zip> --pack-zip --server=<host>:<port> [--site=https://<host>]
```

In the ZIP they are, all in `game/binaries/x86/`: `launcher_skhid.exe`,
`updater_skhid.exe`, `updater_skhid.url` (the site, one line) and the folder
`launcher_skhid/` with `index-<code>.html` and `index-blank-<code>.html` for
`en`, `ru`, `ua`, `de`, `pl`. At the top of the ZIP, beside `game/`, is
`Skhid.bat`: the one file a player starts. It starts `launcher_skhid.exe`
in its folder and does nothing else; if the launcher is not where it looks
(the ZIP was unpacked somewhere else), it says where the files belong.
Beside it is `Skhid.sh`, the same for Linux: `bash Skhid.sh` runs
`play_skhid.sh` (the game), `bash Skhid.sh launcher` the launcher under
Wine.

The same input gives the same ZIP byte for byte. That matters: the updater
installs whenever the ZIP's checksum changes.

## What a player sees

1. Double-click `Skhid.bat` in the game's folder (or `launcher_skhid.exe`
   itself in `game\binaries\x86`). The launcher's window opens as it
   always looked, with the server's page in it. The line at the bottom says
   "Checking game version...".
2. `updater_skhid.exe` asks the server which ZIP it offers now. If it is the
   one installed, and no file of it is missing or damaged: "The latest
   version of the game is installed", PLAY is enabled. That takes a second.
3. If not: the bar fills while the ZIP is downloaded ("... left", speed),
   then "Extracting file ..." for each file, then PLAY. If the launcher
   itself was among the files, its window closes and opens again by itself.
4. PLAY starts `survarium_skhid.exe -localization=<language>
   -mt_allocator=768 -minimum_resources_memory_size=768` from
   `game\binaries\x86`, and the launcher closes.
5. The language menu (EN, RU, UA, DE, PL) changes the launcher's texts and
   the page at once, is remembered, and is the language the game starts in.

Gone from the window: the four social buttons, CREATE ACCOUNT (an account is
made by the first sign-in), and the settings gear (its two options, "Allow
torrent" and the torrent port, are of the old updater). PROJECT WEBSITE
opens `<site>/?lang=<code>` and SHOP opens `<site>/exchange?lang=<code>` in
the player's browser.

## Matching the original design

The merged PR #23 kept the native window resources but approximated the
embedded page. It replaced `back.jpg` with a flat background, added borders,
used 24-pixel opaque captions instead of the original 32-pixel shadow strips,
and placed the second and third tiles one pixel too low.

The online, loading, and offline pages now share
`../../servers/web-server/launcher-skin/style.css` and the original, unmodified
`back.jpg` and `shadow.png`. Their provenance and SHA-256 hashes are beside
them in `launcher-skin/README.txt`. The shared assets live under `servers/`
so they are included by the existing Docker build too. Both renderers embed
the images; the ZIP still contains ten self-contained HTML pages and needs
no new image files or HTTP routes.

The reference is the game's `game/launcher/index-en.html` and
`public/style.css`, not a recreated screenshot. Measured at 100% scale:

| Element | x, y | width × height |
|---|---|---|
| Browser page | 0, 0 | 899 × 335 |
| Left panel | 15, 10 | 456 × 314 |
| First tile | 479, 11 | 404 × 98 |
| Second tile | 479, 118 | 404 × 98 |
| Third tile | 479, 226 | 404 × 98 |
| Tile captions | 479, 77 / 184 / 292 | 404 × 32 |

Skhid's rankings, counts, maintenance, and legal text replace the old news.
A Discord link at the right of the legal caption opens the project's existing
invite (`https://discord.gg/Pxa4prXfwu`, also used by BugTrap) in the player's
browser. It is available on the online, loading, and offline pages. The link
markup is shared in `launcher-skin/discord-link.html`; no Discord content is
loaded into the launcher.
The removed obsolete controls described above remain removed. The native
window, progress bar, and button artwork remain the game's own; the binary
self-test checks the entire 4,468,736-byte resource section unchanged.

The fixed-layout rankings table truncates long player/clan names with an
ellipsis, keeping level/rating columns within the panel. Caption strips use
the original white, bold, uppercase Arial/Tahoma text, padding, and image.
Amber remains reserved for the Skhid headings/numbers and maintenance text.

Verification, 2026-10-13: all 18 rendered page states matched the original
panel/caption rectangles and skin pixels in Firefox at 899 × 335. For the
pixel comparison only, news and Skhid content are removed and all three
captions receive the same text; the original artwork, CSS geometry, and
caption typography remain. The actual pages are also captured and checked
for content overlapping captions or escaping the viewport. This covers all
ten local pages plus English/Ukrainian online pages with ten long names,
an empty board, upcoming maintenance, and downtime with the test badge.
It is a browser comparison, not a new Windows/MSHTML runtime test.

The page uses CSS supported by the original embedded browser. Both data URLs
are below the [IE8 limit of 32,768 characters](https://learn.microsoft.com/en-us/previous-versions/cc848897(v=vs.85));
the Rust regression test decodes them and checks their bytes and length.

To repeat the comparison (Python 3, Node 22+, Firefox, Rust):

```sh
# From srp-026g:
LAUNCHER_VISUAL_DIR=/tmp/launcher-visual cargo test -p web-server export_visual_fixtures -- --ignored
python3 client-patches/launcher/launcher_pages.py /tmp/launcher-visual
# Run Firefox separately with a fresh profile:
mkdir -p /tmp/launcher-firefox
firefox --headless --no-remote --profile /tmp/launcher-firefox --remote-debugging-port 9448
# In another terminal, from srp-026g:
node client-patches/launcher/check_visual.mjs /path/to/game/launcher /tmp/launcher-visual
```

The comparison writes `comparison.json`, the original and normalized skin
screenshots, and screenshots of all eighteen real pages into the fixture
folder. The optional final argument selects a different Firefox debug port.

Captured comparison (899 × 335 Firefox page renders, not native window captures):
[original game](../../docs/images/launcher-look/original.png),
[merged PR #23](../../docs/images/launcher-look/merged.png),
[patched page with Discord](../../docs/images/launcher-look/patched.png),
[Ukrainian maintenance](../../docs/images/launcher-look/maintenance-uk.png),
[Ukrainian offline](../../docs/images/launcher-look/offline-uk.png), and
[measurements](../../docs/images/launcher-look/comparison.json).
The merged/patched screenshots use the same deliberately long test names;
they do not show live player data.

## How an update works

The server (`servers/web-server`, the 0.26g page):

- `GET /api/client-files` answers with the SHA-256 and size of the file it
  hands out at `/download` (`SRP_DOWNLOAD_FILE`):

  ```
  skhid-client-files 1
  sha256 <64 hex digits>
  size <bytes>
  ```

  It reads the file again when its size or time changes, so putting a new
  ZIP in its place is all a release needs: no restart, no setting.
- `GET /launcher/v2/<product>/?lang=<en|ru|ua|de|pl>` is the page.

The updater (`client/updater`):

1. Reads `updater_skhid.url` beside it (else the address it was built with)
   and asks `/api/client-files`. HTTPS only, through Windows' own WinHTTP;
   plain HTTP is taken for `127.0.0.1` and `localhost` alone.
2. Compares the answer with `updater_skhid.state` beside it: the checksum of
   the ZIP it installed last, and the size and checksum of every file of it.
   An install is due when the ZIP is another one, when a file is missing, or
   when a program file (exe, dll, certificate) has other bytes. A text file
   a player may have edited (`.bat`, `.sh`, `.txt`, `.url`) is no reason.
3. Downloads `/download` into memory, and goes on only if its size and
   SHA-256 are what the server announced.
4. Unpacks **every** file in memory and checks its CRC before anything is
   written. A name that is not a plain relative path of ordinary characters
   refuses the whole archive (no `..`, no drive, no backslash).
5. Writes each file that differs beside its place as `<name>.skhid-new`,
   reads it back, and only when all are written renames them into place;
   the file that was there becomes `<name>.skhid-old`. Windows allows
   renaming a running program's file, so the launcher and the updater are
   replaced the same way while they run. If one rename fails, those done
   are undone: the game folder is as it was.
6. Files the ZIP before had and this one has not are deleted, if they still
   are what was installed.
7. Writes `updater_skhid.state`. Leftover `.skhid-old` files are deleted at
   the next start.

Where files go: an entry `game/...` goes into the game folder (two levels
above `binaries\x86`, whatever its name), any other entry beside the game
folder when that is named `game` (as the game is unpacked), else into it.

Updating itself: the launcher was replaced → the updater tells the launcher
it is done (state 15) and closes the pipe, on which the launcher closes; the
updater waits until the launcher's "only one of me" mutex is gone and starts
the new `launcher_skhid.exe`. Only the updater was replaced → nothing more:
the old one finishes this run, the new one runs next time.

`updater_skhid.exe check` does the same by hand, without a launcher
(`check quiet`: no message box; the exit code is 0 or 1).

## When something goes wrong

Every failure is said in words, in a box titled "Skhid updater", and written
to `game\binaries\x86\updater_skhid.log`. Nothing is half installed.

| What happened | What the player gets |
|---|---|
| The server cannot be reached (no internet, server down, firewall), three tries | The reason ("Cannot connect to skhid.link. The server may be down, or a firewall blocks updater_skhid.exe"), then PLAY is enabled if the installed files are complete, or were never checked (first start) |
| ...and a file the updater installed is missing or damaged | "x86/survarium_skhid.exe is missing or damaged and cannot be repaired now: <reason>", status "Error", PLAY stays off |
| The answer is not the server's (a Wi-Fi sign-in page, a proxy) | "The answer from <host> is not the update server's ..." |
| The server has no ZIP to offer | "<host> offers no client files now" |
| The server's download slots are taken (it answers 503) | Waits 15 s and tries again, four times |
| The download breaks off, or its checksum is wrong | Tried again, four times; then "The files downloaded from <host> are damaged (the checksum is wrong). Nothing was installed." |
| The ZIP is not usable (not a ZIP, a bad name, a damaged entry) | "The downloaded archive cannot be used (<why>). Nothing was installed. Tell the server's operator." |
| Not enough disk space | "Not enough free disk space for the update in <folder>: N MB needed, M MB free." |
| The game folder cannot be written (Program Files, read-only) | "Cannot write <file>: Access is denied (error 5). Move the game out of Program Files or start the launcher as administrator; an antivirus can block it too." |
| A file is held by another program (the game is running) | "Cannot replace <file>: ... (error 32). Nothing was changed. Another program holds the file: close the game and try again." |
| `updater_skhid.exe` is missing | The launcher's own box: "Cannot start updater_skhid.exe. Copy the files again." |
| The launcher is closed during an update | The updater stops; nothing was changed, or everything was |
| The updater is started from another folder than `game\binaries\x86` | "The updater must be in the game's folder game\binaries\x86 ..." |

The texts are English only (the launcher shows them in an ANSI message box,
so only ASCII is safe). The launcher's own lines are in its five languages.

## What each patch does

Addresses are virtual addresses in the original exe (image base 0x10000;
`.text` file offset = address - 0x10C00, `.rdata` and `.data` = address -
0x11000). Every site is compared with the original bytes first. How each was
found: `../../docs/notes/launcher.md`.

### 1. Run where it is (no copy in `temp`)

- **Where:** `jne` at 0x1474A in `launcher::run_as_copy` becomes `jmp`
  (`0F 85 58 06 00 00` → `E9 59 06 00 00 90`); `je` at 0x14442 in the
  constructor becomes six `nop`s.
- **Why:** the original launcher shows no window at all when started from
  `game\binaries\x86`. It asks its updater to copy both of them to
  `<install>\temp`, starts the copy and ends; only a launcher whose path
  ends with `\temp\survarium_launcher.exe` makes a window. That was how the
  old updater could overwrite the launcher. The new updater renames instead
  (above), so no copy is needed.

### 2. No hardware warnings

- **Where:** `launcher::check_hardware` 0x145C5 returns 0 at once
  (`81 EC 04` → `33 C0 C3`).
- **Why:** before its window the launcher measured RAM, the CPU (registry)
  and video memory (WMI and DirectX) and warned below 1920 MB / 448 MB. The
  game does its own checks; under Wine these probes are slow and wrong.

### 3. Never through the Windows service

- **Where:** `je` at 0x15521 becomes `jmp` (`74` → `EB`).
- **Why:** the launcher first asks "Survarium Update Service" to start the
  updater. A PC that once had the real game installed may still have that
  service, which would start the old updater as SYSTEM. Now the updater is
  always started directly, as the player.

### 4. The installer's registry values are not needed

- **Where:** `registry_key::get_string` 0x14027 starts with a jump into the
  new section (`55 8B EC 51 8D 04 36` → `E9 <rel32> 90 90`); four
  `push HKEY_LOCAL_MACHINE` become `push HKEY_CURRENT_USER` (0x15231,
  0x159A4, 0x16301, 0x170AD: `68 02 00 00 80` → `68 01 00 00 80`).
- **Why:** the launcher read five strings that only the game's installer
  wrote, from `HKLM\Software\Vostok Games\Survarium`: what PLAY starts
  (`file to execute`, `arguments`, `working folder`), where the updater asks
  (`update server`) and its own path (`launcher`). Without them it shows
  "Error" and PLAY never works. The new code answers for those five names:

  | Name | Answer |
  |---|---|
  | `file to execute` | `<the launcher's folder>\survarium_skhid.exe` |
  | `working folder` | `<the launcher's folder>` |
  | `launcher` | the launcher's own path |
  | `arguments` | `-mt_allocator=768 -minimum_resources_memory_size=768` |
  | `update server` | the site's host (the new updater does not use it) |

  Every other name goes on to the registry as before. The key the four
  sites open is now the user's own, which the launcher makes itself for its
  language setting, so it is always there; a key an old installation left
  in HKLM is never read for these.
- **The new code:** `launcher_values_026g.asm`, in a new section `.skhid`
  after `.reloc` (VA 0x49A000). It finds where it was loaded and adds that
  to the two image addresses it uses, so it also works if the launcher is
  loaded at another base. The game's version shown under the logo is read
  from the file `file to execute` names, as before.

### 5. The updater starts even when the game exe is missing

- **Where:** 0x1577A in `launcher::start_updater`: the three pushes of the
  message "Can't retrieve first install version or local version" become
  `and word [ebp-0ECh],0` / `jmp 0x157A4` (the version is an empty string,
  go on). Two relocations under them are disabled.
- **Why:** with no `survarium_skhid.exe` to read a version from, the
  original stops there, and the one program that could bring the file back
  is never started.

### 6. The updater is `updater_skhid.exe`

- **Where:** the string `survarium_updater.exe` at 0x3DF6C.
- **Why:** the game's own updater is not touched and not used. It asked for
  administrator rights at every start, ran BitTorrent with DHT and UPnP
  port mapping, and uploaded crash reports (`../../docs/notes/launcher.md`,
  "What the original updater did").

### 7. One server: the page, the two buttons

- **Where:** three `push <address>` operands point at new strings in
  `.skhid`: 0x1AFA2 (the page: `<site>/launcher/v2/`, to which the launcher
  adds `survarium/?lang=<code>`), 0x1BCBA (PROJECT WEBSITE: `<site>/?lang=`)
  and 0x1BBFC (SHOP: `<site>/exchange?lang=`). The old texts are emptied.
- **The local pages:** the string `\..\game\launcher\index-` at 0x3F514
  becomes `\launcher_skhid\index-`: while the page loads, and when it does
  not, the launcher shows the pages of the bundle, not the game's own
  `game\launcher\index-*.html` with their links to the old sites.

### 8. The buttons that are gone

- **Where:** the click handlers of CREATE ACCOUNT, YouTube, Facebook,
  Twitter and VK return at once (first byte → `C3` at 0x1BC45, 0x1BD03,
  0x1BD16, 0x1BD29, 0x1BD3C); their rectangles, and the settings gear's,
  get a left edge of 30000 (0x45854, 0x458A4, 0x458B4, 0x458C4, 0x458D4,
  0x45814), which is outside the window; PROJECT WEBSITE moves from 478 to
  614, beside SHOP (0x45864). Their addresses are emptied.

### 9. Nothing of the old sites is left

- **Where:** the crash-report address, the support address and e-mail
  (0x3DBB4, 0x3DB94, 0x3DB6C, handed to `bugtrap.dll` only) are emptied.
  The self-test searches the result for `survarium.com`, `vostokgames`,
  `facebook.com`, `twitter.com`, `vk.com` and `youtube.com`, in ASCII and
  UTF-16, and finds none.

### 10. Two texts

- "Can't run survarium_updater from service nor directly" (0x3EA90) becomes
  "Cannot start updater_skhid.exe. Copy the files again."; the title of the
  updater's error box "Updater error:" (0x3EF80) becomes "Skhid updater".

### 11. The file itself

- The Authenticode signature is taken off: it cannot be right for a changed
  file, and the launcher never reads it (unlike the game exe). The `.skhid`
  section takes its place at the end of the file, the section table gets a
  sixth entry, `SizeOfImage` grows by a page and the PE checksum is
  recomputed.

## The pipe, in short

The launcher makes `\\.\pipe\vostok_updater_pipe-Survarium`, then starts the
updater with that name as its second argument. A message is a 16-bit length
and that many bytes, the first being its type.

| Updater says | Launcher shows |
|---|---|
| `02 04` | "Checking game version..." |
| `02 05` | "An error has occurred during checking current version, retrying..." |
| `02 09`, then `01 <done> <total> <speed> <time left>` (four 64-bit numbers) | the bar, "... left", "Speed ..." |
| `02 0A` | "An error has occurred during downloading, retrying..." |
| `02 0C` | "Download complete" |
| `02 10`, then `07 <name>` and `01 ...` | "Extracting file <name>", the bar |
| `02 0E` | "The latest version of the game is installed", PLAY enabled |
| `02 01` | "Error" |
| `05 <text>` | a message box with the text (at most 255 ASCII characters) |
| `02 0F` and the pipe closed | the launcher closes |

The launcher says `1B` (stop) when its window is closed. The updater stays
connected after `02 0E` until then: a closed pipe would turn the line into
"Error". Types 0, 24 and above 25 would crash the launcher (its table has no
bounds check), so the updater's tests check that none is ever sent. The
whole protocol: `../../docs/notes/launcher.md`.

## What was run (2026-10-13, Windows 11)

- `client/updater/logic`: 28 unit tests (SHA-256 vectors, DEFLATE against
  zlib's output and every cut and bit flip of a stream, ZIP reading and
  hostile archives, the two text formats, the pipe messages).
- `client/updater/tests/test_updater.py`: 18 scenarios with the real
  `updater_skhid.exe`, a web server on 127.0.0.1 and the test in the
  launcher's place on a real named pipe: first install, up to date, a new
  archive, repair, the launcher and updater replaced while running and the
  restart, no server (three cases), a wrong checksum, a broken download, a
  hostile archive, a locked file with the undo, closing during a download,
  `check` by hand. One of them installs a real players' ZIP and compares
  every file with Python's unpacking of it.
- The real patched launcher with the real updater and a local srp server
  (`cargo run -p srp`), in a test game folder under another product name
  (`-product_name=SkhidTest`, so that its registry key, mutex and pipe are
  its own): the window and the server's page; the language switched to
  Ukrainian (texts, page, the registry value); PLAY with a stand-in for the
  game exe (it was started as `"<folder>\survarium_skhid.exe"
  -localization=ukrainian -mt_allocator=768
  -minimum_resources_memory_size=768` in `binaries\x86`, and the launcher
  closed); a changed game exe repaired at the next start; a new ZIP with
  another launcher in it (the launcher replaced itself and came up again);
  the server stopped (the local page, the message, PLAY enabled).
- `selftest_launcher_026g.py`: all checks. `test_release_overlay.py`: 10
  tests; one runs the real `Skhid.bat` in a folder with a space and a
  bracket in its name, with a stand-in for the launcher (it was started by
  its full path, in its own folder), and without the launcher (the text
  that says where the files belong). `test_linux_launcher.py`: 25 tests
  (in WSL, with a stand-in for Wine); one runs `Skhid.sh` from another
  folder, with and without `launcher`, and without the script it looks
  for.

- Later that day, with the servers on the internet (HTTPS, Windows 11). The
  test server, a fresh copy of the game with its ZIP unpacked over it: the
  first check (the ZIP taken once, no file changed, the state written);
  `Skhid.bat`, the window with the server's page and its top 10 with a
  player in it, PLAY enabled; a second ZIP offered, with another launcher
  in it (13 files installed in two seconds, the launcher replaced itself
  and came up again; every other file of the folder was left as it was,
  compared by checksum before and after); the language changed in the
  window (saved at once; the updater is not told and stays); PLAY started
  the real game, which signed in and reached the lobby. The players'
  server: its ZIP taken from the site, unpacked, `updater_skhid.exe check`
  run in it.

**Not run:** Wine (`bash play_skhid.sh launcher`); another Windows than 11;
a download slow enough to watch the bar move in the launcher's window (6 MB
came in under a second each time); a PC that still has the real game's
installation and service.

## Limits, and what is not done

- **The ZIP is trusted because HTTPS is.** The updater checks that what it
  downloaded is what the server announced; that the server is the server
  rests on its certificate. A signature of the ZIP with a key kept off the
  server would also cover a taken-over server. Not done.
- The launcher has no line for "could not check, playing with what is
  there": after the message box it says "The latest version of the game is
  installed".
- The updater looks for a newer ZIP when the launcher starts, not while it
  stays open.
- One launcher at a time (its mutex), as before.
- The language is the one value still kept in the registry
  (`HKCU\Software\Vostok Games\Survarium`, `language`), by the launcher's
  own code.
- `play_skhid.bat` still starts the game alone and looks for no update.

## Putting it on a server

1. The server must run a build with `/api/client-files` and `/launcher/`
   **before** a ZIP with the launcher is offered: a launcher asking a server
   without them gets "offers no client files now" at every start.
2. Build the ZIP with `make_client_026g.py --pack-zip` for that server and
   put it where `SRP_DOWNLOAD_FILE` points. Players who have the launcher
   get it at their next start; the others take it from the page once.
   The site's "How to play" follows the file: while the ZIP it offers holds
   `launcher_skhid.exe`, the steps say to unpack it into the game's folder
   once, double-click `Skhid.bat` and press PLAY, with the start script and
   the command line folded under "Without the launcher"; for a ZIP without
   it they are the steps as before. Nothing is
   set by hand and no restart is needed.
3. From then on a release is: build the ZIP, replace the file.
