# Client patches for 0.26g (build 2777)

What a copy of the **original** Survarium 0.26g needs to play on this server:
a patched `survarium.exe` and the login certificate. The game is not here;
the tools below work on your own copy. The patched exe itself is never
published.

| | |
|---|---|
| Original exe | `game/binaries/x86/survarium.exe`, 12 894 840 bytes, sha256 `ac33d9937015c2299bf62cc6b127cb308b48a3475b30fff2ddd65479ae3bf153` (the tools check it and refuse anything else) |
| Result (full patch, no embedded server) | 12 895 352 bytes, sha256 `448f691608b56ed14e985c4e5c84d17c6862cab7fce415d940c2b88821701b72` (patch revision 2); embedding a server changes the hash and may increase the size |
| The players' download | `Survarium-0.26g-srp-files.zip` of skhid.link (and 026g.skhid.link), since 2026-10-12 21:20 UTC: 4 918 126 bytes, sha256 `fa73afa4c897dd099acd88021576b3fc173ae7c0b6b917e2795b318a6a05ff48`, seven files, made by `make_client_026g.py --pack-zip --server=skhid.link:1234 --localization=english` of `main` 955e37d (English is the language the launchers start the game in; a player changes it in `play_skhid.bat` or `play_skhid.sh`). From 21:10 to 21:20 UTC the site had the same zip with `ukrainian` in the two launchers (sha256 `4ac6ecf61c2869fb3c8587a89e7f020c55de79f8641cf5ef854b1a1b87b2e8fc`); every other file is the same in both. Its exe is the full section patch with that address inside it (patch revision 2, 12 895 352 bytes, sha256 `30e928dd3c19a25452293c11be5e0bc85fdcacbbe0b957941a1e34c7be944883`); `bugtrap.dll` is the Rust crash reporter built that day from the same commit (32 256 bytes, sha256 `3d8190b377a7534b76f7c21f99d95106abafdf91dc342a7ad034db0cd52f1d57`); beside them `play_skhid.bat`, `play_skhid.sh`, the login certificate, `README.txt` and `LINUX.txt` |
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
| `survarium_login_server.crt` | The login certificate (the same file as `../certs/survarium_login_server.crt`) |

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
the game. These are
the only seven files shipped; source, symbols, tests and development tools are
excluded. Back up the original `bugtrap.dll` and login certificate before
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
- **The number:** `REVISION` in `patch_026g.py`, 2 now. Revision 1 introduced this
  field; revision 2 adds the optional embedded login server. Raise it in the
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

## Not ported, not done

- The 0.21d mode 3 battery patch is not needed: 0.26g serializes the
  next-battery time.
- A defect of the client itself is known and not patched: an input with the
  "previous weapon" bit from a player who holds something other than its two
  weapons can crash every client of a match
  (`../docs/OPEN-ISSUES.md`, 26-24).
