# Client patches for 0.26g (build 2777)

What a copy of the **original** Survarium 0.26g needs to play on this server:
a patched `survarium.exe` and the login certificate. The game is not here;
the tools below work on your own copy. The patched exe itself is never
published.

| | |
|---|---|
| Original exe | `game/binaries/x86/survarium.exe`, 12 894 840 bytes, sha256 `ac33d9937015c2299bf62cc6b127cb308b48a3475b30fff2ddd65479ae3bf153` (the tools check it and refuse anything else) |
| Result (full patch, no embedded server) | 12 895 352 bytes, sha256 `3bd35a2078ab9288c1e30773cb4ade74f719fee15b2b5bc6ee39ca2f237d39fd` (patch revision 4); embedding a server changes the hash and may increase the size. With `--server=skhid.link:1234`, as the players' download is made: sha256 `ae3ddf43f38eecb9e5080b10aa6b8397ce5dbed0f133b381461c82cb17329f25` |
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
| `default_server_026g.asm`, `.section.bin`, `.hash-mismatch.bin` | Optional default-login-address stub, appended to the snapshot cave; source and assembled code for both placements |
| `make_client_026g.py` | Makes the players' overlay folder or ZIP: patched exe, Rust BugTrap DLL, certificate, start file and README |
| `play_skhid.sh`, `LINUX.txt` | Linux Wine/DXVK launcher and instructions, included in the generated folder and ZIP |
| `selftest_026g.py` | Builds every variant and checks it (sections, disassembly of every hook, checksum, signature); needs the original exe and nasm |
| `run_battery_rule.ps1` | Runs the battery rule's handler (patch 9) out of any 0.26g exe on a made-up rule object, in the 32-bit Windows PowerShell: what an exe does with a put that is played again after a rollback. Starts nothing of the game |
| `survarium_login_server.crt` | The login certificate (the same file as `../certs/survarium_login_server.crt`) |
| `launcher/` | The game's launcher made to work again, and to keep these files current: its patch tool, the one piece of new code, the self-test and its own README. Its updater is `../../client/updater` |

## How to apply

### Experimental new-player late join

Experimental; excluded from release overlays. Client hooks and server admission
and lobby routing are implemented. Native 2-to-3 and 2-to-4 starter-loadout joins
stay in sync; a different-loadout 2-to-3 join also stays in sync. The experimental
client signs in with patch revision `0x80`. The lobby keeps these sessions in a
separate queue because their snapshot format differs from released clients.
Play fills eligible running matches before forming another match. Squads reserve
one team together and enter on the same tick; clan matches retain fixed teams.

The patch reserves twenty histories/statistics slots and three-byte input masks.
Each slot gets fixed callback and weapon ID ranges. Animation IDs append in
roster/kit order, preserving existing IDs when equipment is added. Running E4
loads an immutable profile asynchronously, installs inactive player resources,
and acknowledges completion.
Profiles remain allocated until their resources are destroyed, including after
leaving a match.

The server waits for resource acknowledgements, then sends a fresh hash-verified
snapshot and subsequent inputs. A matching newcomer hash permits future E9
entry. Existing players retain their clocks and histories. Failed preparation
releases the newcomer’s seat but reserves its descriptor and closes further
admission for that match.

Build and check (NASM, x86 C compiler, GNU ld; optional `CC`/`LD` overrides):

```sh
python late_join_026g.py /path/to/survarium.exe /tmp/survarium-late-join.exe --nasm /path/to/nasm
python check_late_join_026g.py /path/to/survarium.exe --nasm /path/to/nasm
cc -std=c11 -g -O1 -Wall -Wextra -Werror -pthread -fsanitize=address,undefined \
  late_join_runtime_026g.c test_late_join_runtime_026g.c -o /tmp/srp-late-runtime-test
/tmp/srp-late-runtime-test
```

Machine-code checks require Unicorn 2.1.4. They cover native profile decoding,
query/callback ABI, slot/weapon binding, incremental animation indexing,
preparation/acknowledgement/cleanup, fixed capacities, and PE integrity.
Resource scheduling, physics/animation/UI setup,
transport and player destruction are intercepted. Host tests cover cancellation,
retries, failed loads and concurrent destruction. Under ptrace, run with
`ASAN_OPTIONS=detect_leaks=0`; tests also check allocation/reference balance.
These fixtures do not establish live client agreement.

Server regressions cover preparation retries, packet loss, stale acknowledgements,
failed catch-up, reconnects, departures, living/dead/respawned snapshot states,
match ending, TDM tickets, duplicate seats, atomic squad admission, compatible
queues, session persistence, and growth to twenty players. A full-stack test
covers login, lobby routing, catch-up, entry, input relay, and results:

```sh
cargo test --locked -p session -p login-server -p lobby-server -p udp-transport -p survarium -p match-server --lib
cargo test --locked -p srp --test v026g_end_to_end
```

The death/respawn cases use scripted snapshots, not native combat. The native
mixed-equipment run entered at 53.4 s and matched snapshots through 133.3 s,
including uploads from the newcomer, with no resync. Its driver stopped early
when the local `xdotool` binary disappeared; native deaths and kit switching
remain manual coverage.

`--test-session 0x1000` builds a test-only executable that bypasses login and
connects directly to its `-server=host:port` argument. Use separate sessions
for each client with the `late_join_native` match-server example. For six initial
players: `cargo run -p match-server --example late_join_native -- 2249 61 starter 6`.
Sessions start at `0x1000`; enter `join` to reserve each additional player.

### Released patch

```
python patch_026g.py <game>/binaries/x86/survarium.exe <game>/binaries/x86/survarium_skhid.exe
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

## Not ported, not done

- The 0.21d mode 3 battery patch is not needed: 0.26g serializes the
  next-battery time. (Patch 9 above is another defect, 0.26g's own.)
- A defect of the client itself is known and not patched: an input with the
  "previous weapon" bit from a player who holds something other than its two
  weapons can crash every client of a match
  (`../docs/OPEN-ISSUES.md`, 26-24).
