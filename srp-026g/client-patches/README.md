# Client patches for 0.26g (build 2777)

What a copy of the **original** Survarium 0.26g needs to play on this server:
a patched `survarium.exe` and the login certificate. The game is not here;
the tools below work on your own copy. The patched exe itself is never
published.

| | |
|---|---|
| Original exe | `game/binaries/x86/survarium.exe`, 12 894 840 bytes, sha256 `ac33d9937015c2299bf62cc6b127cb308b48a3475b30fff2ddd65479ae3bf153` (the tools check it and refuse anything else) |
| Result (full patch) | 12 895 352 bytes, sha256 `053c359586aaef3d04fb6de541366b719fe413d53f8634575c118ca06bb1fd86` |
| The players' download | `Survarium-0.26g-srp-files.zip` of 026g.skhid.link, made on 2026-10-12, holds the full patch **without patches 4 and 7** (added later that day): sha256 `166d25bcc49f6e1b83fc0d85c25e822c85c7284d4d846143358ca2f28bd31ddf`. The two exes differ in the 13 bytes of patch 4, the one byte of patch 7 and the PE checksum, and in nothing else |
| Checked | 2026-10-12: with patches 4 and 7 left out, the tools here, run on the original, give exactly the exe and the certificate of that download, byte for byte. With them: `selftest_026g.py` passes its checks of every site; the patched rule has **not been run in a match yet** |

## The files

| File | What |
|---|---|
| `patch_026g.py` | Builds the patched exe from the original. Python 3, nothing else |
| `snapshot_upload_026g.asm` | The source of the upload code (nasm) |
| `snapshot_upload_026g.section.bin`, `.hash-mismatch.bin` | That code assembled, for the two places it can go; `patch_026g.py` reads them. Assembling the source again gives the same bytes |
| `make_client_026g.py` | Makes the whole players' overlay from a game folder: the patched exe, the certificate, a start file, a README |
| `selftest_026g.py` | Builds every variant and checks it (sections, disassembly of every hook, checksum, signature); needs the original exe and nasm |
| `survarium_login_server.crt` | The login certificate (the same file as `../certs/survarium_login_server.crt`) |

## How to apply

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

Or let `make_client_026g.py` do all of it, without touching the game folder:

```
python make_client_026g.py <game folder> <new folder> --server=<host>:<port> [--localization=ukrainian] [--check]
```

It writes `game/binaries/x86/survarium_skhid.exe`,
`game/binaries/x86/play_skhid.bat` (the launch line; `SERVER` and `LANGUAGE`
at its top), `game/resources/ssl/survarium_login_server.crt` and
`README.txt` into the new folder, to be copied over the game. `--check` does
every check and writes nothing.

Other builds of the patch: `--cert-only` (only patch 1), `--cave=hash-mismatch`
(patch 5 in another place, see below), `--check`.

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

### 4. The battery rule of modes 2 and 3 (the last battery)

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
  for the rest of the match. Seen on the server: every match in which a
  team had all batteries but one ended on time with that count (9 of 9);
  the one match that ended by the rule was played by a player alone.
- **What stays:** the rule counts every battery put in and taken out, sets
  the completion time when a team has all of them and takes it back when
  not, which is what the 0.21d client's rule does. The containers are never
  closed. The match is ended by the server, 3 s after it reads the full
  count, as before.
- **State:** added on 2026-10-12, built and self-tested; not run in a match
  yet, and not in the players' download yet. A client without it and a
  client with it stay in step until a team's last battery goes in, and part
  there (the server then starts the fewer of them from the others' world).
- Evidence: `../docs/notes/match.md` section 26,
  `../docs/notes/client_patch.md` section 3.8.

### 5. The snapshot upload (needed for any match)

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

### 6. The login certificate (a file, not an exe patch)

- **What:** `survarium_login_server.crt` goes over
  `game/resources/ssl/survarium_login_server.crt`.
- **Why:** the client accepts a login server only if its certificate
  verifies against that file. The file the 0.26g game ships is of another
  key and ran out in 2015. This one is the game's own name issued again
  (valid to 2037), and its key is in `../certs/`, which the login server
  reads.

### 7. The minimum video-memory check

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

## Not ported, not done

- The 0.21d mode 3 battery patch is not needed: 0.26g serializes the
  next-battery time. (Patch 4 above is another defect, 0.26g's own.)
- A defect of the client itself is known and not patched: an input with the
  "previous weapon" bit from a player who holds something other than its two
  weapons can crash every client of a match
  (`../docs/OPEN-ISSUES.md`, 26-24).
