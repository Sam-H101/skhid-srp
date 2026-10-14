# Client patches for 0.26g (build 2777)

What a copy of the **original** Survarium 0.26g needs to play on this server:
a patched `survarium.exe` and the login certificate. The game is not here;
the tools below work on your own copy. The patched exe itself is never
published.

| | |
|---|---|
| Original exe | `game/binaries/x86/survarium.exe`, 12 894 840 bytes, sha256 `ac33d9937015c2299bf62cc6b127cb308b48a3475b30fff2ddd65479ae3bf153` (the tools check it and refuse anything else) |
| Result (full patch, no embedded server) | 12 907 128 bytes, sha256 `03202d5ee70b9523560f8e9b362b80939f9f7cf05a6f699bc08e9945700820b7` (patch revision 9); embedding a server changes the hash. With `--server=skhid.link:1234`: sha256 `f30c3c666bf87ed6eecaea613523f7259306b7881d3157dde1ed4d80149211a1`. Revision 8 was `9673dc5a…4756` and, with that server, `5701a96d…6aea`. With `--no-late-join` the tool gives the revision 4 exe byte for byte: 12 895 352 bytes, `3bd35a2078ab9288c1e30773cb4ade74f719fee15b2b5bc6ee39ca2f237d39fd`, and with that server `ae3ddf43f38eecb9e5080b10aa6b8397ce5dbed0f133b381461c82cb17329f25`, the exe of the players' download since 2026-10-13 15:36 UTC |
| Revision 5 to 9 | Revision 5 is revision 4 with patch 11, the late join (a player enters a match that is being played), and patch 12, a game that goes on without the focus. Revision 6 adds patch 13, three faults the first late joins of real players showed (the scoreboard without the newcomer, and two crashes); 5 and 6 play together, and a server asks for 6 because a game of 5 can be killed by a newcomer. Revision 7 adds patch 14: a newcomer with a grenade or a trap in a kit was never let in; it does not play with 5 and 6 (the order of a world's game objects is another), so a server asks for 7. Revision 8 adds patch 15: the game's clock is brought to the match clock six times faster after a start, a late join and a resync; it plays with 7 (nothing a match computes changes), and a server need not ask for it. Revision 9 adds patch 16, a newcomer's score on the scoreboard of the others was counted again at every rollback and showed an earlier match's score until its first, and patch 17, the team points drawn while a match counts down (the original kept the last match's until play began); it plays with 7 and 8 (neither is in the world), and a server need not ask for it. **5 was the players' download from 2026-10-13 23:27 UTC, 6 from 2026-10-14 00:44 UTC, 7 from 2026-10-14 02:26 UTC, 8 from 2026-10-14 03:48 UTC; 9 is not in a download yet.** Revision 5 is the first revision that does not play with the ones before it: such a game keeps room for twenty players in every match and sends other packets, so the server puts games from revision 5 on in a queue of their own (`docs/notes/matchmaking.md` section 9). A server that offers it asks for it (`SRP_MIN_PATCH_REVISION=5`), or its players are two queues that never meet. Against the revision 4 exe made the same way it differs in the revision byte, the 15 sites of patch 11, the two bytes of patch 12, four bytes of the ready hook and the headers of the two sections, and in nothing else of the original's bytes (the self-test's check 15). What was run and what was not: patch 11 below |
| Revision 4 | Revision 3 with patch 10 (a start with a Documents folder the game may not write to). **In the players' download since 2026-10-13 15:36 UTC.** Made as the download is, it differs from the revision 3 exe (`ac947abd…6d7b`; without a server `eedc5b5c…cf0d`) in the four bytes at file 0x35F0B, two bytes of an address at file 0x8410C8, the revision byte and the PE checksum, and in nothing else (compared byte by byte). The built exe was started in both situations of patch 10, not past the sign-in screen, before the release; the first sign-in with revision 4 on the players' server was logged three minutes after it. It plays together with revision 3 |
| Revision 3 | Revision 3 is revision 2 with patch 9 (the battery rule). **It was the players' download from 2026-10-13 12:22 to 15:36 UTC** (the rows below are the downloads so far): it and the revision 2 exe differ in the 13 bytes of patch 9, the revision byte and the PE checksum, and in nothing else (compared byte by byte). Its rule was run outside the game (`run_battery_rule.ps1`). Eight sign-ins with revision 3 were logged in those hours; whether one of them played a match to a team's last battery was not looked at. A game of revision 3 or later and an older one part at a team's last battery (patch 9) |
| The players' download | `Survarium-0.26g-srp-files.zip` of skhid.link (and 026g.skhid.link), since 2026-10-13 15:36 UTC: 6 793 103 bytes, sha256 `29b2ac002a1787c1352d021065f18a87ea5caa66fb16298c8e603a5347416bb4`, 23 files, made by `make_client_026g.py --pack-zip --server=skhid.link:1234 --localization=english` of `main` dbd70f0 with `--bugtrap=` naming the build of `client/bugtrap` that had been run in the game (that DLL does not build to the same bytes twice). It differs from the download before in two files: the game exe (sha256 `ae3ddf43f38eecb9e5080b10aa6b8397ce5dbed0f133b381461c82cb17329f25`, patch revision 4: patch 10) and `bugtrap.dll` (39 936 bytes, sha256 `9e3295c1312854c455c474e1dbd457b0a792b5ae406ff0b55d3b19b4f2fc4353`: the start-up notice of `client/bugtrap/README.md`); the other 21 files are byte for byte the same. Built twice: the same ZIP. Checked after the replacement: `/api/client-files` and `/download` of both names give this sha256. No restart: nothing on the server changed |
| The download of 12:22 UTC | 2026-10-13, 12:22 to 15:36 UTC: 6 788 856 bytes, sha256 `a63842de812d0d8057f1d357795f02e4176a1e9ff022742bde24f020988da9a9`, 23 files, of `main` 78070c5. It differs from the one before in five files: the game exe (sha256 `ac947abd212d05f3172035967076517f057019b9e1499be06c28b622baf36d7b`, patch revision 3: the battery rule), `updater_skhid.exe` (93 184 bytes, sha256 `6502180e038fe384451adb5216100072a9e985e2eae1014c541a113baef60d8e`: it installs the original game for a new player), `7zr.exe` (new; 602 624 bytes, sha256 `256feca8e274e5da655e2a284fabafd9f554365eb164862089dacd4e8276d282`), `README.txt` and `LINUX.txt`. An updater with the ZIP before installed took the five files at its next check |
| The download of 10:30 UTC | 2026-10-13, 10:30 to 12:22 UTC: 6 440 522 bytes, sha256 `efccf0ce1f36a8bf0ec21b6751c4094d5cdf777cc1b03968d69f975697d9cd13`, 22 files, of `main` 3d6ab12. It differs from the one before in the launcher's ten local pages only (the original artwork, a Discord link). The first release that reached an installed launcher by itself: an updater with the ZIP of 09:50 installed took the ten files at its next check |
| The download of 09:50 UTC, the first with the launcher | 2026-10-13, 09:50 to 10:30 UTC: 6 293 491 bytes, sha256 `adb90bccf0254bc2c9fbc8710ffaa9152f95948fd46dcb96a390e04124cc0c4f`, 22 files, made by `make_client_026g.py --pack-zip --server=skhid.link:1234 --localization=english` of `main` 4a68dc7 with `--bugtrap=` naming the `bugtrap.dll` of the download before. It is that download with the launcher (`launcher/README.md`) added: `launcher_skhid.exe` (4 735 488 bytes, sha256 `9110bf7526794d73c2d3713f61b509c526dd8d442e442ca5e08db77427b97d38`), `updater_skhid.exe` (79 360 bytes, sha256 `0ae3c3463b95073fe29797d87dd2ffda3be911832fb489ffb1b9daec299949fd`), `updater_skhid.url` (`https://skhid.link`), the ten pages of `launcher_skhid/`, and `Skhid.bat` and `Skhid.sh` beside `game/`. The game exe (sha256 `30e928dd…4883`, patch revision 2), `bugtrap.dll` (`3d8190b3…1d57`) and the login certificate are byte for byte those of the download before; `play_skhid.bat` is too, and `play_skhid.sh`, `README.txt` and `LINUX.txt` have the launcher in their texts. From this download on the launcher keeps a player's Skhid files the same as the ZIP the site offers, so a release is: build the ZIP, replace the file |
| Checked, the launcher | 2026-10-13. The same build made for the test server (it differs in the two addresses only), over HTTPS with a real game folder on Windows 11: unpacked over a fresh copy of the game it added 20 files and replaced two (`bugtrap.dll`, the certificate); the first start took the ZIP once and changed nothing; a second ZIP put on the server, differing in the launcher, its pages, `updater_skhid.url` and `README.txt`, was installed at the next start (13 files, the launcher replaced itself and came up again, 2 seconds in all; of 58 files in the folder 43 were untouched and the other two were the updater's own state and log); PLAY started the game, which signed in and reached the lobby. This download itself: taken from the site, unpacked, and `updater_skhid.exe check` run in it (the ZIP fetched, 22 files right, nothing written but its state). Not run: Wine; a Windows other than 11 |
| The third download | 2026-10-12 21:20 UTC to 2026-10-13 09:50 UTC: 4 918 126 bytes, sha256 `fa73afa4c897dd099acd88021576b3fc173ae7c0b6b917e2795b318a6a05ff48`, seven files, made by `make_client_026g.py --pack-zip --server=skhid.link:1234 --localization=english` of `main` 955e37d (English is the language the launchers start the game in; a player changes it in `play_skhid.bat` or `play_skhid.sh`). From 21:10 to 21:20 UTC the site had the same zip with `ukrainian` in the two launchers (sha256 `4ac6ecf61c2869fb3c8587a89e7f020c55de79f8641cf5ef854b1a1b87b2e8fc`); every other file is the same in both. Its exe is the full section patch with that address inside it (patch revision 2, 12 895 352 bytes, sha256 `30e928dd3c19a25452293c11be5e0bc85fdcacbbe0b957941a1e34c7be944883`); `bugtrap.dll` is the Rust crash reporter built that day from the same commit (32 256 bytes, sha256 `3d8190b377a7534b76f7c21f99d95106abafdf91dc342a7ad034db0cd52f1d57`); beside them `play_skhid.bat`, `play_skhid.sh`, the login certificate, `README.txt` and `LINUX.txt` |
| Checked | 2026-10-12: the exe differs from the one of the download before it (revision 1) in the ten bytes of the missing-server hook, the revision byte, one relocation entry, the 67 bytes after the upload code (the stub and the address) and two header fields (the checksum and the section's size), and in nothing else (compared byte by byte). An exe from the same tools with another address inside (it differs from this one in the address and those two header fields only) was started with no `-server` option, signed in to the test server (logged as patch revision 2) and began a match there; the exe of the download itself was started with no option and came up. The crash reporter: one crash made on purpose in the game on Windows 11 gave its dialog and a dump that reads as a proper minidump (191 MB: it holds the game's data segments). The Linux launcher was not run by us. **`bugtrap.dll` cannot be made again byte for byte**: a second build of the same source gave another sha256, so the file in the download is kept, not rebuilt |
| The second download | 2026-10-12, 18:43 to 21:10 UTC: 4 871 691 bytes, sha256 `16eb1f23c99d5d73c5228c168021c58adff5e0ebf3d543984f916b416cef9528`, four files; its exe is the full patch of revision 1 (sha256 `3c7d7bf0195db000f0c6dc81e57fa15a50decec4fa0c0ea5494f361f91049c9d`), which the revision-1 tools gave byte for byte. A game with it signed in to a local server (logged as patch revision 1) and played a match. Revisions 0, 1 and 2 play together: none of the differences is in what a match computes (by the comparison of the bytes; a match of a revision 1 and a revision 2 game was not watched) |
| The first download | 2026-10-12, until 18:43 UTC: exe sha256 `166d25bcc49f6e1b83fc0d85c25e822c85c7284d4d846143358ca2f28bd31ddf`, the full patch without patches 6 and 7 (it says patch revision 0). The revision-1 exe differs from it in the one byte of patch 6, the six bytes and one relocation entry of patch 7 and the PE checksum, and in nothing else (compared byte by byte). The two play together |

## The files

| File | What |
|---|---|
| `patch_026g.py` | Builds the patched exe from the original. Python 3, nothing else |
| `snapshot_upload_026g.asm` | The source of the upload code (nasm) |
| `snapshot_upload_026g.section.bin`, `.hash-mismatch.bin` | That code assembled, for the two places it can go; `patch_026g.py` reads them. Assembling the source again gives the same bytes |
| `late_join_026g.asm`, `late_join_validate_026g.c`, `late_join_runtime_026g.c`, `late_join_install_026g.c`, `late_join_runtime_026g.h` | The source of the late-join code (patch 11): the hooks and the game's calling conventions in assembly, the checks of what the server sends and the bookkeeping in C |
| `late_join_026g.section.bin`, `late_join_026g.section.json` | That code built, which `patch_026g.py` reads, and beside it where the C functions are that the assembly calls and what built it (gcc 13.2.0, GNU ld 2.41.90, NASM 2.16.01). The same tools give the same bytes; another C compiler gives other bytes that do the same, so the file is kept, not rebuilt for a release |
| `late_join_026g.py` | Builds those two files (`--write-bin`), says whether they are what the sources give on this machine (`--check-bin`), or builds an exe straight from the sources for work on the code. Needs nasm, a C compiler for x86 and GNU ld |
| `check_late_join_026g.py`, `test_late_join_runtime_026g.c` | Checks of the late-join code: the first runs the patched machine code on made-up objects (needs Unicorn 2.1.4), the second the C bookkeeping under the sanitizers |
| `default_server_026g.asm`, `.section.bin`, `.hash-mismatch.bin` | Optional default-login-address stub, appended to the snapshot cave; source and assembled code for both placements |
| `make_client_026g.py` | Makes the players' overlay folder or ZIP: patched exe, Rust BugTrap DLL, certificate, start file and README |
| `play_skhid.sh`, `LINUX.txt` | Linux Wine/DXVK launcher and instructions, included in the generated folder and ZIP |
| `selftest_026g.py` | Builds every variant and checks it (sections, disassembly of every hook, checksum, signature, and what the late join adds to the build without it); needs the original exe and nasm |
| `run_battery_rule.ps1` | Runs the battery rule's handler (patch 9) out of any 0.26g exe on a made-up rule object, in the 32-bit Windows PowerShell: what an exe does with a put that is played again after a rollback. Starts nothing of the game |
| `survarium_login_server.crt` | The login certificate (the same file as `../certs/survarium_login_server.crt`) |
| `launcher/` | The game's launcher made to work again, and to keep these files current: its patch tool, the one piece of new code, the self-test and its own README. Its updater is `../../client/updater` |

## How to apply

### The late join (patch 11, patch revision 5)

A player who presses Play goes into a match that is being played, if one
takes players. It is part of the default build since revision 5;
`patch_026g.py --no-late-join` and every `--cave=hash-mismatch` build are
without it and say revision 4.

What the game does differently, from its first connection to a match:

- it keeps twenty histories and twenty statistics records whatever the
  roster, and its input masks (0xED, 0xEE) have three bytes;
- it says so in its match connection: 0xD8's debug byte is `80` (`81` with
  `-debug_hash_mismatch`), where a game without the patch sends `00` or `FF`;
- each roster place has its own ranges of callback and weapon ids, so a
  place added later does not move the ids of the others; animation ids are
  appended in roster and kit order;
- on the server's 0xE4 during play it checks the newcomer's profile, loads
  its kits in the background, installs the player without making it enter,
  and answers (0xE0 with a marker no snapshot upload can have) "installed",
  "failed" or "busy". The profile stays allocated until its players are
  destroyed.

The server waits for every game's "installed", then gives the newcomer a
fresh world and the inputs since, checks that it computes the same, and
lets it enter at a tick all are told. The players already in the match keep
their clocks and histories. A join that fails frees the newcomer's place in
the lobby, keeps its roster place taken, and closes that match to later
joins.

**A game with this patch and one without never play one match**: the worlds
they keep differ. The server tells them apart by the patch revision (5 or
later) and has a queue for each (`docs/notes/matchmaking.md` section 9).

Build the code and check it (nasm, a C compiler for x86 and GNU ld; `CC` and
`LD` name others than `cc` and `ld`):

```sh
python late_join_026g.py --write-bin          # late_join_026g.section.bin and .json
python late_join_026g.py --check-bin          # are they what the sources give here?
python check_late_join_026g.py /path/to/survarium.exe --nasm /path/to/nasm
cc -std=c11 -g -O1 -Wall -Wextra -Werror -pthread -fsanitize=address,undefined \
  late_join_runtime_026g.c test_late_join_runtime_026g.c -o /tmp/srp-late-runtime-test
/tmp/srp-late-runtime-test
```

`patch_026g.py` and `selftest_026g.py` need none of that: they read the
checked-in `.bin`. The self-test (check 15) assembles the 4096 bytes of
assembly again and compares, and checks the build with the late join
against the build without it.

The machine-code checks need Unicorn 2.1.4. They cover native profile
decoding, the query and callback conventions, slot and weapon binding,
animation indexing, preparation, acknowledgement and cleanup, the fixed
capacities, where a slot's grenades and traps are among the saved objects
(patch 14) and the PE file. Resource scheduling, physics, animation and UI
set-up, transport and player destruction are intercepted there. The C tests
cover cancellation, retries, failed loads and concurrent destruction, and
count allocations and references (where AddressSanitizer loops at start,
run the test with `setarch -R`). Neither shows that games agree in a match.

Server tests cover preparation retries, packet loss, stale
acknowledgements, failed catch-up, reconnects, departures, living, dead and
respawned players in the snapshot, the match's end, Team Deathmatch
tickets, duplicate seats, squads entering together, the two queues, session
persistence and growth to twenty players, and a test through the whole
stack covers login, the lobby, catch-up, entry, input relay and results:

```sh
cargo test --locked -p session -p login-server -p lobby-server -p udp-transport -p survarium -p match-server --lib
cargo test --locked -p srp --test v026g_end_to_end
```

**What was run in the game, and what was not** (2026-10-13). The author of
the code ran the exe built by hand before it had a revision (it signed in
as revision `0x80`, with the code 0xE5C bytes lower in the section): joins
of two games into three and four on map 61, a battery mode, with the
starter kit and with other equipment, in step from 53.4 s to 133.3 s with
no resync. The revision 5 exe has the same code at another address; its
build passes the self-test, the machine-code checks and the C tests
(2026-10-14). **Run with the revision 5 exe (2026-10-13, the test server): a match of two games, in step. Not run with it: a join that completes** (the one tried was given up by the server because the games stood on one PC and only the one in front played: patch 12). Never run with either: a join into a Team
Deathmatch, a fight, a death and a respawn after a join, a change of kit,
a roster over four, a match that nobody joins played to its end.

`late_join_026g.py ORIGINAL OUTPUT --test-session 0x1000` builds an exe for
tests only, which passes the login by and connects straight to its
`-server=host:port` as that session. Use a session for each game with the
`late_join_native` example of the match server; for six players at the
start: `cargo run -p match-server --example late_join_native -- 2249 61 starter 6`.
Sessions start at `0x1000`; `join` on its input reserves each further
player.

### Released patch

```
python patch_026g.py <game>/binaries/x86/survarium.exe <game>/binaries/x86/survarium_skhid.exe
    (add --no-late-join for the revision 4 exe, for a server that does not ask for 5)
copy survarium_login_server.crt <game>/resources/ssl/survarium_login_server.crt
```

The source exe is never changed; the output must be another file. Then start
the game **from `game\binaries\x86`** (it finds the certificate by a path
relative to where it is started):

```
survarium_skhid.exe -localization=english -server=<host>:<login port> -mt_allocator=768 -minimum_resources_memory_size=768
```

Or let `make_client_026g.py` do all of it, without touching the game folder.
First build the crash reporter with `../../client/bugtrap/build.sh`, then:

```
python make_client_026g.py <game folder> <new folder> --server=<host>:<port> [--localization=ukrainian] [--check]
python make_client_026g.py <game folder> <new file.zip> --pack-zip --server=<host>:<port> [--localization=ukrainian] [--check]
```

It writes `game/binaries/x86/survarium_skhid.exe`,
`game/binaries/x86/bugtrap.dll` (the Rust crash reporter),
`game/binaries/x86/play_skhid.bat` and `play_skhid.sh` (Windows and Linux launchers),
`game/resources/ssl/survarium_login_server.crt` and
`README.txt` and `LINUX.txt` into the new folder, or a ZIP with `--pack-zip`, to be copied over
the game. With `--no-launcher` these are
the only seven files shipped; source, symbols, tests and development tools are
excluded.

Without that option it adds the launcher (`launcher/README.md`), made from the
game folder's own `survarium_launcher.exe`: `game/binaries/x86/launcher_skhid.exe`,
`updater_skhid.exe` (build it first with `../../client/updater/build.sh`, or pass
`--updater=<path>`), `7zr.exe` (7-Zip's standalone extractor, the copy of
`../../client/updater/extractor/`: the updater unpacks the original game with it when
a player has none; `--extractor=<path>` names another copy, which must be the same file),
`updater_skhid.url`, the ten pages of
`game/binaries/x86/launcher_skhid/`, and `Skhid.bat` and `Skhid.sh` at the top, beside `game/`
(`Skhid.sh` is the file of that name here: on Linux, `bash Skhid.sh` in the game's folder runs
`play_skhid.sh` from there).
`--site=https://<host>` names the server's web
page when it is not `https://<host of --server>`. A player then unpacks the ZIP into
the game's folder (the one that holds `game`) once and double-clicks `Skhid.bat`,
which starts `launcher_skhid.exe`: it looks for a newer ZIP on that site, installs it, and its
PLAY button starts the game in the language chosen in it. The same input gives the
same ZIP byte for byte, which the updater relies on. Back up the original `bugtrap.dll` and login certificate before
installing. `--check` does every check and writes nothing. Existing ZIPs are
never overwritten. `--bugtrap=<path>` selects another built DLL; by default
the builder reads `client/bugtrap/target/i686-pc-windows-msvc/release/BugTrap.dll`.

`--resource-region=<name>=<entry file>` (once per entry; `lobby_menu` and
`login_menu` are known, `RESOURCE_REGIONS` in the tool) with
`--resources-out=<folder>` adds `game/binaries/x86/updater_skhid.resources`,
the plan of the updater's resource step: the updater rewrites that entry of
the player's `resources.db` in place with the entry file
(`../../client/updater/README.md`, "Resources"). The tool refuses an entry
file of another size or another game CRC than the original entry, and a
source `resources.db` whose entry is not the original (it reads that part of
the file only). The blobs the server must hand out at `/resources/<sha256>`,
the entry file and the original entry, go into `<folder>`, named by their
sha256: they are uploaded before the ZIP is offered
(`launcher/README.md`, "Putting it on a server"). Without the option the ZIP
is byte for byte what it was before the option existed (checked 2026-10-14
against a build of `main` a63f37d with the same inputs).

Full overlays embed the supplied `--server` address in the executable, so
players do not need `-server=` at launch. The generated batch file retains
the language and allocator options. To use another server, add
`-server=host:port` to its start command. Login supplies the lobby address;
the lobby supplies the match address. Neither needs embedding.

For the standalone patcher, add `--server=host:port` at patch time to embed
the default. Without that option it retains the original address behavior.
An embedded hostname is limited to 63 ASCII bytes and the port to 1–65535.
`--cert-only` does not support embedding; cert-only overlays still pass the
address through their batch file.

On Linux, install Wine with support for 32-bit Windows applications and
Winetricks, then run `bash play_skhid.sh` from `game/binaries/x86`. The launcher
creates its own Wine prefix, installs DXVK on first use, and preserves saved
game settings. Full builds use the embedded server; edit `server=` at the top
of the script for an override. Certificate-only builds fill in that value.
Both launchers use the builder's `--localization` setting. See `LINUX.txt` for
details, including the English-keyboard-layout startup workaround.

`bash play_skhid.sh launcher` starts `launcher_skhid.exe` in the same Wine
prefix instead of the game (not run under Wine by us yet).

The launcher is executable in folder output and has Unix executable permissions
in ZIP metadata. Launcher and packaging tests use synthetic fixtures and mocked
Wine commands, without game files or downloads:

```
python -m unittest discover -s . -p 'test_*.py'
bash -n play_skhid.sh
```

Other builds of the patch: `--cert-only` (only patch 1), `--cave=hash-mismatch`
(patch 4 in another place, see below), `--check`.

## Login menu account hint

`login_hint_026g.py` replaces the old registration button with plain text:

- English: "Account created on first sign-in"
- Ukrainian: "Акаунт створюється під час першого входу"
- Russian: "Аккаунт создаётся при первом входе"
- German: "Konto wird beim ersten Login erstellt"
- Polish: "Konto powstaje przy pierwszym logowaniu"

The hint wraps inside the login panel. The old button is hidden and disabled,
removed from keyboard focus, and its browser callback is erased. All five game
languages have a translation. The language comes from the native client's
localization data, so it follows `-localization`.

Build a new resource archive with Python 3, a JDK and JPEXS FFDec 26.0.0:

```sh
python login_hint_026g.py /original/game/resources.db /output/game/resources.db \
  --ffdec=/path/to/ffdec.jar
```

`--java` and `--javac` can specify the JDK executables. The original archive is
only read; an existing output is refused. Install the output as the client's
`game/resources.db` while the game is closed, keeping the original for rollback.
This archive is a separate output from the small `make_client_026g.py` overlay;
include it when preparing a complete client distribution.

The patch verifies the original build 2777 login movie's SHA-256 and edits one
ActionScript block. All other movie tags and existing unrelated methods remain
byte-identical. Recompressing the movie leaves enough room in its original
archive entry; padding preserves the entry size and offsets, with four ignored
trailer bytes retaining the original content CRC. Both the signed file table
and the resource checksum validation remain intact. No archive signature bypass
is added. The sources are in `login_hint/`;
game assets are generated locally and are not checked in.

## What each patch does

Addresses are virtual addresses in the original exe. The evidence for each
is in `../docs/notes/client_patch.md`. Every site is compared with the
original bytes first, so another build or an already patched file is refused.

### 1. The code-signing certificate check (needed to start at all)

- **Where:** the function at 0x12A10 becomes `xor eax,eax; ret`.
- **Why:** at start the client takes the survarium.com certificate from its
  own Authenticode signature, asks Windows whether its chain is valid now,
  and only then verifies the signature of the `resources.db` file table with
  that key. The certificate ran out on 2015-06-06, so every unpatched 0.26g
  client stops with "Resources archive is corrupted".
- **What stays:** only the chain check is skipped. The file table's
  signature is still verified.

### 2. The single-instance check

- **Where:** `jz` at 0x124F9 becomes two `nop`s.
- **Why:** so that several clients run on one PC (testing). Not needed to
  play.

### 3. The world-hash interval, 100 ms to 50 ms

- **Where:** one byte at 0x27CC53, the default of
  `-hash_check_interval_in_ms` (`mov dword [esp+18h],64h` at 0x27CC4F).
- **Why:** a stock client reports the hash of its world for every second
  50 ms tick, on a phase of its own. Two clients on opposite phases never
  name the same tick, and the server cannot compare their worlds. With 50
  every client reports every tick (`../docs/MATCH-SYNC-026G.md`). The key on
  the command line still overrides it.

### 4. The snapshot upload (needed for any match)

- **Why:** 0.26g matches are lockstep: every client simulates, the server
  only relays inputs. A client can start or rejoin only from a snapshot of
  the world, and the original server made that snapshot itself. This server
  does not simulate the game, so a client must send its world when the
  server asks. A stock client has no code for that; without the patch the
  world stays frozen and no match starts.
- **Where:** two 5-byte jumps, at 0x27AEA1 (in `on_match_ready`) and at
  0x27A505 (the 0xF6 case of `on_match_packet_received`), into 420 bytes of
  new code that serializes the client's world with the game's own functions
  and sends it in chunks (0xE0).
- **Where the new code goes (default, `--cave=section`):** a new PE section
  `.srp` after `.reloc` (VA 0xA425000, 0x200 bytes of file). The section
  table gets an 8th entry, `SizeOfImage` grows by a page, and the
  Authenticode blob moves 0x200 bytes towards the end of the file, where the
  security directory finds it again. Nothing of the original code is lost.
- **The fallback (`--cave=hash-mismatch`):** the code overwrites
  `game_world_core::on_hash_mismatch` (0x325A60), which only the branch the
  second hook replaces could reach. No header changes. For a client that
  misbehaves with one more section; not needed so far.
- Base relocations that overlap rewritten bytes are made no-ops, and the PE
  checksum is recomputed.

### 5. The login certificate (a file, not an exe patch)

- **What:** `survarium_login_server.crt` goes over
  `game/resources/ssl/survarium_login_server.crt`.
- **Why:** the client accepts a login server only if its certificate
  verifies against that file. The file the 0.26g game ships is of another
  key and ran out in 2015. This one is the game's own name issued again
  (valid to 2037), and its key is in `../certs/`, which the login server
  reads.

### 6. The minimum video-memory check

- **Where:** `jae` at VA 0x106411 (file offset 0xF5811) becomes `jmp`:
  `73 0F` → `EB 0F`, the same one-byte patch used by `play_skhid.sh`.
- **Why:** the client otherwise calls its fatal warning when detected
  video memory is below 320 MiB, which can prevent startup under Wine.
- **What stays:** the function returns the detected video-memory size;
  its existing 320 MiB fallback for an unknown size is unchanged. The
  allocator launch options above still apply.
- Included in both full builds (`section` and `hash-mismatch`).
  `--cert-only` keeps the original check. The self-test verifies the jump
  target, unchanged return paths, and absence of overlapping relocations.

### 7. The patch revision in the sign-in

- **Where:** `mov cl,[9DAE9Eh]` at VA 0x380862 (file offset 0x36FC62) in
  `sign_in_on_connected` becomes `mov byte [esp+27h],REVISION` / `nop`:
  `8A 0D 9E AE 9D 00` → `C6 44 24 27 01 90`. The base relocation under
  it (0x380864) is disabled.
- **Why:** nothing a client sends tells a server which patches its exe
  has. The first login message has a 12-byte version field, `0.26g0` and
  six zero bytes; the patch puts a revision number into byte 7 of it.
  The text stays `0.26g0`, so a server that does not read the byte takes
  the exe as before, and an exe without this patch says 0.
- **What reads it:** the login server writes it in its log
  (`Client version: 0.26g0, patch revision 1`). It refuses nobody for it.
- **The number:** `REVISION` in `patch_026g.py`, 4 now. Revision 1 introduced this
  field; revision 2 adds the optional embedded login server; revision 3 the
  battery rule (patch 9); revision 4 the two start-up sites (patch 10). Raise it in the
  change that makes a full build something else for the game, so that
  the two kinds of client can be told apart.
- It is what the exe says of itself, not proof: a changed exe can say
  anything.
- Full builds only; `--cert-only` keeps the code. The self-test works the
  field out from the instructions that fill it. Tried on 2026-10-12: a
  game with the revision-1 exe signed in to a local server, which logged patch
  revision 1 (a client sending the stock field: 0). Evidence: `../docs/notes/client_patch.md` section 6.

### 8. An embedded default login server (optional)

- **Where:** the missing-server error at VA 0x1EB25B becomes a jump to a
  51-byte stub after the snapshot upload, followed by the address string.
- **When:** only when neither `-server` nor `-replay` was supplied. Explicit
  options keep their original behavior, including invalid-address handling.
- **How:** the stub initializes the parsed `server` key's 512-byte buffer,
  end pointer and string type, then rebuilds the temporary string and repeats
  the original option read from 0x1EAE2D. It uses relative addressing, so rebasing works.
- Both full variants support it. The PE section grows when needed, and the
  existing relocation and checksum handling covers the new hook. Build
  hashes depend on the embedded address.

### 9. The battery rule of modes 2 and 3 (the last battery)

- **Where:** two instructions of the rule's container event handler
  (0x313A50) become `nop`s: `jnz 313B3Bh` at 0x313A62 (6 bytes) and
  `mov byte [ecx+160h],0` at 0x313B34 (7 bytes).
- **Why:** when a team's count reaches the number of batteries, the rule
  stores its completion time and clears a byte that every container asks
  before it lets anybody use it; from then on it counts no event. Neither
  of the two is part of the world a client saves and loads. A client plays
  a tick, and plays it again when an older input of another player arrives
  (a rollback): the battery is in the player's hands again and the count is
  one less again, but the byte stays cleared. So the last battery cannot be
  put in on the replay, and nobody can put a battery in or take one out
  for the rest of the match. Seen on the server: the kept logs to
  2026-10-13 11:00 UTC have 19 matches in which a team had all batteries
  but one: 14 ended on time with that count, 5 by the rule with all of
  them. Two of the five were finished by a player alone (the other had
  left); in three, two or three players were still in, so the last
  battery does go in when no rollback crosses that tick.
- **What stays:** the rule counts every battery put in and taken out, sets
  the completion time when a team has all of them and takes it back when
  not, which is what the 0.21d client's rule does. The containers are never
  closed. The match is ended by the server, 3 s after it reads the full
  count, as before.
- **Run outside the game:** `run_battery_rule.ps1 <exe> [<exe> ...]` (the
  32-bit Windows PowerShell) copies the handler's 238 bytes out of an exe
  and calls them on a made-up rule object: nine puts, the tenth, a
  rollback of the count, the same put again, a take, a put. With a
  revision 2 exe (the players' download until 12:22 UTC of that day) the
  replayed put is not counted
  and the byte stays cleared; with revision 3 it is counted, the byte
  stays set, and a take sets the completion time back to none (run on
  2026-10-13, Windows 11).
- **State:** written on 2026-10-12; patch revision 3. Built and
  self-tested, the handler run outside the game; in the players' download
  since 2026-10-13 12:22 UTC. **A match played to a team's last battery
  with it was not watched yet.** A client without it and a
  client with it stay in step until a team's last battery goes in, and part
  there (the server then starts the fewer of them from the others' world).
  The login server writes each sign-in's revision into its log, so the
  two can be told apart there; it does nothing else with it.
- **Who gets it:** a game started through the launcher takes the new ZIP
  by itself at its next start. A game with a download from before the
  launcher (2026-10-13 09:50 UTC) keeps the old rule until its player
  takes the ZIP once more; on 2026-10-13 most accounts still signed in
  with revision 0, the first download.
- Evidence: `../docs/notes/match.md` section 26,
  `../docs/notes/client_patch.md` section 3.9.

### 10. A start with a Documents folder the game may not write to

- **Where:** two sites, eight bytes, no added code.
  `xor ecx,ecx` / `jmp short 46B12h` at VA 0x46B0B (file offset 0x35F0B)
  in `vfs::mounter::finish` becomes `jmp short 46B36h` / two `nop`s:
  `33 C9 EB 03` → `EB 29 90 90`. The address in `mov eax,9DB51Ch` at VA
  0x851CC7 (file offset 0x8410C7), the initializer of the option
  `-record_journal`, becomes 0x9DBF0C: `B8 1C B5 9D 00` → `B8 0C BF 9D 00`.
  Its base relocation stays: the new value is an address too.
- **Why:** the game keeps its settings, its log and a journal in the folder
  `Survarium` of the player's Documents folder, and no option moves it.
  A game that could not write there stopped two seconds after its start,
  before the sign-in, in one of two ways (two crash reports of one player,
  2026-10-13):
  - The folder is not there and cannot be made. Its mount (`user_data`)
    fails and has no root, the engine still announces it as mounted, and
    the receiver reads a null pointer: access violation at 0xF9A8F.
  - The folder is there, but a file cannot be made in it. The journal
    `Survarium_<Windows user>.journal` is made anew at every start, and
    the failure is a fatal assertion: "failed opening file … with open
    flags:create_always+read_write", a breakpoint at 0x40141.
- **What the patch does:** a mount without a root is not announced, and
  the journal is not recorded unless the game is started with
  `-record_journal=1` (the exe's own option; "1" was its default). Such a
  game starts and can be played. Its settings are not saved. The crash
  reporter of the download (`client/bugtrap`) tells the player so at the
  game's start, with what to do (its README, "Start-up notice").
- **What is lost:** the journal, a recording of a session's input and
  packets that only the same build's `-replay_journal` reads. It was
  overwritten at every start and nothing here uses it.
- **Does not change** what a match computes: revision 4 and revision 3
  play together.
- **Tried** (2026-10-13, Windows 11, the built exe, 28 s each, to the
  sign-in screen): a Documents folder that refuses new folders; a folder
  `Survarium` that refuses new files; an ordinary folder (no journal file
  is made); the same with `-record_journal=1` (the journal is made). No
  crash in any. Before the patch the first two give the two crashes
  above, with the addresses and registers of the reports.
- **Not tried:** a sign-in, a match, and saving the options into a folder
  that refuses the file.
- **What makes a Documents folder like that** is not known from the
  reports. Windows' "Controlled folder access" does it to a program it
  does not know, and so do file rights on a Documents folder moved to
  another drive.
- **State:** patch revision 4; in the players' download since 2026-10-13
  15:36 UTC, together with the crash reporter that has the notice. The
  first sign-in with revision 4 on the players' server was logged three
  minutes later.
- Full builds only; `--cert-only` keeps both. Evidence:
  `../docs/notes/client_patch.md` section 3.10; self-test check 14.

### 12. A game that goes on without the focus (patch revision 5)

A 0.26g game whose window is not the active one stands still. Its logic
thread calls `game::tick` only while `is_application_active` says yes
(`engine_world::logic_tick` 0x1A030; the call at 0x1A0E1, `test al,al`,
`jz 1A113h` at 0x1A0EE). Otherwise it only sends what is queued and keeps
the lobby link up (`docs/notes/transport.md` section 6.1): the link looks
healthy and nothing of the match moves. When the player comes back the
game makes up the time at once (its log says `eating time: N`).

In a match that is a player the others see frozen, who is resynced or
taken for gone on the way back. With the late join it is worse: before a
newcomer enters, every game in the match has to prepare for it and say so,
and a game without the focus says nothing, so one player who looks at
another window holds every join up until the server gives it up after 60 s.

The patch: the `jz` at 0x1A0EE becomes two nops (`74 23` -> `90 90`). The
call and the test stay. It is in the builds of patch revision 5 only
(`RUN_UNFOCUSED` in `patch_026g.py`); `--no-late-join` gives the revision 4
exe as before.

Measured on one PC, 2026-10-14, one game in the lobby of the test server,
windowed 1280x720, 15 s each (`eating time` is the game's own word for a
freeze it makes up):

| | without the patch | with it |
|---|---|---|
| Window not in front | froze: `eating time: 18.4`; 101 % of one core | no freeze; 46 % of one core |
| Window minimised | froze: `eating time: 15.6`; 100 % of one core | no freeze; 146 % of one core |
| Back in front | went on | went on |
| The mouse, window not in front | Windows' cursor shown, free on the whole desktop, also over the game | the same |

**Not run:** a match (so: mouse look and the cursor in a match, a fight
seen by the others while one game is not in front, a join with a game not
in front); exclusive fullscreen (`r_fullscreen on`) with alt-tab; another
graphics card. Known cost: a minimised game uses about half a core more
than before. Sound goes on while the game is not in front.

### 13. Three faults of a match that takes players in (patch revision 6)

Found on 2026-10-13: the first two in the first hour of late joins between real
players, the third on the test server the hour before.

**The scoreboard without the newcomer.** The hud's lists of players are
made once, by 0x234880 when the match is ready: the scoreboard
(`root.list_set_players`), a second list (`root.set_players_list`) and the
enemies' markers (`root.set_enemy_icon_ids`). Their loops end at the count
of players there is then. The late join raises the count and nothing made
the lists again: the players of the match did not see the newcomer in
their scoreboard (the newcomer, whose game made its lists after loading,
saw everybody). Now `install_context` calls `rebuild_lists` when the
newcomer has its place: it builds 0x234880's frame and goes into that
function where the lists are made (0x234E71), and a jump at 0x23572B
(`lists_hook`), right after the third list, takes that run out through the
function's own release of its two arrays and its own return. Nothing else
of 0x234880 runs again: not its subscription, not `root.reset`, not the
hud's items. The rows' numbers come back with the game's own refresh
(0x236D30, every tick the list is shown).

**A game that was given a newcomer died** (access violation at 0x399A8,
reading 0x20). A model that was in the scene was moved
(`scene::update_model` 0x14FED0, the tree's `move` 0x3A1C0) to a place of
numbers that are no numbers (the stack of the dump is full of them). The
tree then finds the box outside itself and grows to hold it
(`update_bounds` 0x396F0): it takes a node for a new root and doubles its
size until that reaches the size it worked out, which for such a box is
infinity, so it never does (`xmm3`, the size, is infinity in the dump).
The scene's trees have 1024 nodes each and nothing behind them (0.26g's
are fixed pools; the engine's older source, `loose_oct_tree.cpp`, asks the
heap), so the list of free nodes ends and the next one is read through
null. A bigger pool would end the same way. Which model, and why its place
was not set, is not known: the other game of that match took the same
newcomer and lived, and so did both games of the join before. The
newcomer's own game loaded the world twice and neither time got as far as
agreeing on a tick with the others (the joins that completed took about a
second for that); the first time it stopped answering 5 s after its world,
50 s before the server gave the join up. Its player reports the same tree
out of nodes while loading (their dump was not seen here). A jump at
0x39E52 in `loose_oct_tree::insert` (`box_hook`), the
one way a box gets into a tree (`move` goes through it too), takes a box
with a not-a-number or an infinity in it as a box of two metres at the
origin; the next move of the object puts it where it is.

**A game whose join the server gave up died** (access violation at
0x15EE8D, writing 0xD1; seen on the test server, in a game that had loaded
a match and was sent back before it was let in). `scene::remove_model`
0x14FF60 looks the model up in the scene's list; when it is not there it
writes "remove_model: model is not inserted now" and goes on all the same,
and erasing the list's end moves what lies after it and adds to a
reference count through it. The tree's erase before that did not fault, so
the model's object was in a tree: LIKELY the model is another scene's,
which holds it. The first instruction of that branch (0x14FF9B) is now a
jump to the function's own end (0x1501F2), where it lets go of the model
it was given (the pointer comes by value, the function counts it down) and
returns: this scene is not touched, the other lets go of the model in its
own time.

Checked: `selftest_026g.py` (682 checks) and `check_revision6_026g.py`,
which runs the three with Unicorn on made-up objects: the three lists made
again for 2 to 20 players, every value released once, registers and stack
as before, no slot of the frame read that nothing wrote, the hud and the
options not written, a hud of another match left alone, the game's own
start of a match unchanged; twenty boxes; a model in the scene, and one not
in it held by two and by one (one count less, the model's end called for
the last and only then). `check_late_join_026g.py` passes as before. Read
in the game's own ActionScript (`hud.swf`, `player_icons.swf`,
`replay_player.swf`, decompiled): each of the three lists replaces what
the movie held (`PlayerList.addPlayersList` sets `_players` and makes both
teams' data anew, keeping which row is the player's own;
`set_players_list` makes a new data provider; `set_enemy_icon_ids` sets
the array), and a row's kills and deaths come with the next
`list_update_player`, which the game sends for every place each tick the
list is shown (0x23B421, `[hud+0BAh]` = `root.show_player_list`). A
newcomer's marker is made by the install itself (`player_ready` 0x1F29D0
calls 0x23BD60, `root.create_player_icon`). **Not run: a game.**

A newcomer has its row from the moment its place is prepared, before it is
let in; one whose join fails keeps the row, as a player of the first
roster who never loaded does.

### 14. A newcomer's grenades and traps (patch revision 7)

Found on 2026-10-14, in the first hour of revision 6: a player with
grenades in its kits was taken into matches being played four times and
never let in, while six players without them were. In revision 5 the games
already in such a match died as well.

**The cause.** The world saves and loads its game objects in the order of
one list (core+0x18914 the first, +0x18918 the last; `0x2D2699` saves into
the state's "game objects" bytes, `0x2D2B74` loads, each object reading as
many bytes as it wrote). The game appends to it: a slot's grenades and
traps when the slot is bound (`grenade_core` 0x303820 and the trap item
0x3067A0, one node for each of a stack, called from the inventory's bind
0x2FC7D0), and then the map's objects (0x285750). So a match's start gives
the slots' objects first and the map's after them. The late join binds a
newcomer's slot when the map's objects are in the list already: the games
in the match had the newcomer's grenades after them, and the newcomer's own
game, which starts with its slot, before them. One game's world was read by
the other's objects in another order, the hashes of the two never agreed,
and the server never let the player in. Seen in the worlds the players'
server kept: six zero bytes at the very end of the game objects after such
a player's place was prepared (six grenades, one byte each untouched),
where a match's start has its grenades in front. An object that reads a
byte not its own can take it for a thrown grenade and read a place of
numbers that are no numbers after it: LIKELY the crashes of patch 13's
second fault. Medkits, oxygen, artefacts and weapons are saved inside
their player's own block and were never part of this.

**The order.** `bind_slot_reserved` (the code every slot is bound through,
at a match's start and later) puts the list aside, lets the game bind the
slot into an empty one, and puts what was aside behind the slot's nodes. A
slot's grenades and traps are in front of everything the list held, so the
list is the same in every game whenever a slot was bound: the newest
slot's first, the map's objects last.

**The states to roll back to.** A game keeps the states of the last half
second and loads one whenever an input for a past tick arrives. Those
saved before a slot was added have no bytes for its objects, and a load
would read every object after them out of step. `lj_native_install` counts
the nodes the slot added and puts as many zero bytes in front of the game
objects of every kept state (core+0x2FBC, item+0xBDF30): a grenade or a
trap nobody has touched saves itself as one zero byte
(`grenade_core::serialize` 0x303DA0, `booby_trap_core::serialize`
0x3052E0). A state with no room for them is left (the buffer is 8192
bytes; the largest seen is 525).

**Revisions.** The order is another one than revisions 5 and 6 keep when
two slots have grenades, so a game of 7 and an older one part as soon as
one of those is thrown: a server asks for 7. While the games of a match
install a new place their worlds differ for a moment (a game with it, a
game not yet); the match server does not judge hash reports then
(`Game::installing`), where it resynced a game that was a second or more
behind the others with a world of the other kind.

Checked: `check_late_join_026g.py`, with Unicorn on made-up objects. The
game's own binders (slot, kit, inventory, grenade item and grenade, trap
item) run on kits with grenades and traps: slots bound at a start, the
map's objects added, slots bound after them, and each time the list is the
newest slot's objects, the older slots', the map's, every link there and
back right and every id of its place's range. The whole install runs for
newcomers with 2, 1, 9 and 2 such objects into a world with two saved
objects and four kept states: each state has the zero bytes in front and
its size, one that fits to the byte too, one with no room is untouched,
and a newcomer with none changes nothing. Seven copies of the code with
one thing broken each all fail that check. The one zero byte is the
game's own word: a grenade and a trap as their constructors leave them,
saved and loaded by their own code there, write that byte and read it
back with no change (a thrown grenade and an armed trap fail it).
`selftest_026g.py`: 682 checks.
The match server's tests: two new ones (games that install a place five
seconds apart are not resynced for it; a game really out of step meanwhile
is resynced afterwards), both failing without the change. **Not run: a
game.** Not known: whether anything else of a grenade or a trap differs
between a slot bound at the start and one bound later (none was found in
the code read); what a join does while a grenade is in the air.

### 15. The game's clock is brought up faster (patch revision 8)

A game's clock is a timer the server sets when it gives the game a world,
and corrects afterwards (0xE7: so many ms behind, or ahead). The game makes
a correction up by running its clock a little faster or slower than real
time, and by how much is a setting of the game itself:
`time_floating_factor`, a console variable (the object at 0xA9D0D0), 0.05
in the original, from 0.001 to 0.5. The `game` constructor copies it into
the timer once (0x1E9848, timer+28h), and `set_offset` (0x1EE9E0) runs the
timer at 1 + it, or 1 - it, for `offset / factor` ms. Nothing else reads
it, and it is not among the settings the game keeps in `user.cfg`.

**Why it matters.** A game that is given a world starts behind the match
clock: the world is its uploader's of half a second ago, the clock is put
350 ms past it, and the wait for the game's answer to the new time and the
way there come on top. The further away the player, the more: about 260 ms
at a round trip of 22 ms, 465 ms at 196 ms, 540 to 690 ms at 300 ms
(players' server, 2026-10-14). The match server drops every input that
comes more than 250 ms after its tick. So while the clock is on its way
the player's keys do nothing: for about a second in the first case, 6 to
9 s in the second, 13 to 15 s in the third, after every start, every late
join and every resync. A player at 196 ms whose link stalled every 10 to
20 s was resynced twelve times in four minutes and was out of step or
without keys for most of them.

**The patch.** The default becomes 0.3: four bytes of data at 0xA799C8
(`cd cc 4c 3d` to `9a 99 99 3e`), no code, no address. 465 ms are made up
in 1.6 s instead of 9.3 s; the clock runs at 130 % of real time for that
long, or at 70 % for a correction backwards. Only in the builds with the
late join (revision 8).

**Revisions.** Nothing that a match computes reads the factor: a game of 8
and a game of 7 play one match, and a server need not ask for 8. The match
server times its corrections by the revision a game signed in with
(`match_msgs::clock`, `docs/notes/match.md` section 29): when it measures
the clock again, and for how long it puts a resync off because a clock is
on its way. A game whose revision it has wrong is brought up all the same,
each correction being the lag that is left (two tests).

Checked: `selftest_026g.py`, 700 checks (check 16: the four bytes; the
console variable's name, its value's address, its least and its most; the
constructor as the value's only reader and the timer field it fills;
`set_offset` and the 0xE7 handler; no relocation under the bytes). Against
the revision 7 exe of the same server the build differs in the revision
byte, those four bytes and two bytes of the PE checksum (compared byte by
byte). The match server's tests: a game of each factor put 400 ms behind is right
after 8 s and after 1.3 s, in one correction, never a ms too far. **Not
run: a game.** Not known: how a game looks and sounds while its clock
runs at 130 % or at 70 % for a second or two; whether anything limits how
many ticks one frame may play (the load after a sync plays seven at once).

### 16. A newcomer's score on the scoreboard (patch revision 9)

The score in a player's row of the scoreboard (Tab) is the game's own
count, not the server's: the statistics handler adds each event's points
to the place's score as it happens (`on_event`, 0x26DB80), and works every
score out again from the statistics whenever the game loads a kept state
(`recalculate_scores` 0x26DC30, from `deserialize` 0x26DB60), which it does
whenever an input for a past tick comes. But it works out only as many
places as the roster had when the match began: `start_match` (0x26D8D0)
copies that count, and the late join raises the roster, not this copy.

**Why it matters.** Seen on the players' server on 2026-10-14: on the
scoreboard of a player who was there from the start, every newcomer had 2.7
to 4 times its score (1088 for 327, 467 for 116), because every rollback
over one of its kills added the kill once more; a newcomer with no kill had
953, a score of an earlier match left in its row. The results screen, the
rewards and the quests are the server's and were right; the starting
players' scores, and every kill and death, were right too.

**The patch.** The count `start_match` keeps becomes 20: `mov al,[edx+577B5h]`
at 0x26D8F3 becomes `mov al,14h` and four nops. Every place's score is then
worked out again at every state loaded, and at a match's start every row
is told a 0. Places with no player have zero statistics, so a 0 that is
never told again. The shared statistics' own count, which rewards and
events read, stays the roster's. Only in the builds with the late join
(revision 9).

**Revisions.** The score is neither in the world nor in its hash: games of
9, 8 and 7 play one match, and a server need not ask for 9.

Checked: `selftest_026g.py`, 733 checks with patch 17's (check 17: the six bytes; how
`start_match` uses the count, and the shared count's copy beside it; the
handler's `deserialize` and `recalculate_scores`; its vtable; every reader
of the count in the code; no branch or address into the six bytes).
`check_late_join_026g.py` runs the game's own `start_match`, `on_event` and
`deserialize` on a made-up handler: a newcomer that scores 13 in a tick has
13, 26 and 52 after 0, 1 and 3 rollbacks in the original and 13 each time
in the build, and every row starts at 0 in the build only. Against the
revision 8 exe of the same server the build differs in the revision byte,
the six bytes, one byte of patch 17 and the PE checksum (compared byte by
byte). **Not run: a game.**

### 17. The team points during the countdown (patch revision 9)

While a match waits or counts down ("Awaiting match start"), the hud shows
the countdown and nothing else of the match: `game_world_ui::update_ui`
(0x23B200) jumps from the countdown to the players' rows (0x23B328), past
the drawing of the team points (0x2364C0), Team Deathmatch's last players
(0x236610) and the respawn time (0x236C30), which play goes through. The
hud's state has the match's own numbers then (the rules fill it every
tick), but the bars kept the last match's until play began: seen on the
players' server, 63 and 53 under the countdown, the end of the match
before. The original game's code (0.21d's is the same), not srp's.

**The patch.** The jump goes to those three drawings instead
(`jmp 0x23b421` at 0x23B328 becomes `jmp 0x23b383`, one byte of its
displacement), which then go on to the rows as play does; the match time
and the start's tip stay out. They draw then what the first refresh of
play would: the start's team points (150 and 150 in Team Deathmatch, 0
and 0 with batteries); the last players, which the hud movie hides while
a team has tickets; and a respawn time of 0 (no player waits for one
during a countdown, as the saved worlds of a real match show), an empty
text. Only in the builds with the late join (revision 9).

**Revisions.** Nothing a match computes: it plays with 7, 8 and 9 alike.

Checked: `selftest_026g.py` check 18 (the five bytes; both paths of the
original around them; the calls from the new target to the rows and no
branch leaving elsewhere; each call's `ret`, so both paths reach the
target as deep; the three hud movie functions; no branch or address
inside the jump). `check_late_join_026g.py` runs the game's own
instructions from the countdown's test to the rows on a made-up hud, the
functions taken at their entry, for modes 4, 3 and 2: in the original the
countdown calls the pregame's two alone, in the build then the three play
calls, with the same arguments; play is the same in both; the stack, the
registers it keeps and the player's count end as they began; given the
original for the build it fails. **Not run: a game.**

## Not ported, not done

- The 0.21d mode 3 battery patch is not needed: 0.26g serializes the
  next-battery time. (Patch 9 above is another defect, 0.26g's own.)
- A defect of the client itself is known and not patched: an input with the
  "previous weapon" bit from a player who holds something other than its two
  weapons can crash every client of a match
  (`../docs/OPEN-ISSUES.md`, 26-24).
