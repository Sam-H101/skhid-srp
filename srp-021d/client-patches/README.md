# Client patches for 0.21d (build 2010)

What a copy of the **original** Survarium 0.21d needs to play on this server:
a patched `survarium.exe` and the login certificate. The game is not here;
the tools below work on your own copy. The patched exe itself is never
published.

| | |
|---|---|
| Original exe | `game/binaries/x86/survarium.exe`, 10 463 352 bytes, sha256 `def82eeaed9d544b2e1c1e0ec971f7359c24a942dd52ace6b45e6d82e7c34a0a` |
| Result | the same size, sha256 `e20b043a1e0095ce1d048ffeb82904e1bda92bb15bc5f7a5b50e3f325cdb18db` |
| Checked | 2026-10-12: the tools here, run on the original, give exactly the exe and the certificate of the players' download of skhid.link (`Survarium-0.21d-srp-files.zip`), byte for byte |

## The files

| File | What |
|---|---|
| `patch.py` | Builds the patched exe from the original and the two assembled files. Python 3 |
| `snapshot_upload.asm` | The upload code and its two hooks (nasm source) |
| `battery_spawn.asm` | The mode 3 battery fix (nasm source) |
| `survarium_login_server.crt` | The login certificate (the same file as `../certs/survarium_login_server.crt`) |
| `lobby_menu.py` | Optional: fixes two bugs of the lobby's Flash UI inside `resources.db` |
| `spawn_model.py` | A model of the battery rule, original and patched (shows the bug; changes nothing) |

## How to apply

nasm is needed once, to assemble the two sources (nasm.us; 2.16.03 was used):

```
nasm -f bin snapshot_upload.asm -o snapshot_upload.bin
nasm -f bin battery_spawn.asm -o battery_spawn.bin
python patch.py <game>/binaries/x86/survarium.exe <out>/survarium.exe snapshot_upload.bin battery_spawn.bin
copy survarium_login_server.crt <game>/resources/ssl/survarium_login_server.crt
```

(`snapshot_upload.bin` is 373 bytes, sha256 `f48b8d42…3759`;
`battery_spawn.bin` is 288 bytes, sha256 `7ec2b47a…8bdd`.) Put the result in
place of `game/binaries/x86/survarium.exe` (keep the original) and start it
from that folder:

```
survarium.exe -localization=english -client=<host>:1234 -minimum_resources_memory_size=768
```

Without `-minimum_resources_memory_size=768` some maps run out of memory
while loading.

## What each patch does

Addresses are virtual addresses in the original exe (image base 0x10000;
file offset = address − 0x10C00). Every site is compared with the original
bytes first, so another build or an already patched file is refused.

### 1. The single-instance check

- **Where:** two bytes at 0x1267A, `jnz` (75 22) becomes `jmp` (EB 22).
- **Why:** so that several clients run on one PC (testing). Not needed to
  play.

### 2. The snapshot upload ("option B"; needed for any match)

- **Why:** 0.21d matches are lockstep: every client simulates, the server
  only relays inputs. A client can start or rejoin only from a snapshot of
  the world, and the original server made that snapshot itself. This server
  does not simulate the game, so a client must send its world when the
  server asks (message 0x59). A stock client has no code for that, and
  nothing else in it can produce a snapshot.
- **Where:** 361 bytes of new code in the zero padding after `.text`
  (0x6A1E0B), and two 6-byte hooks into it (`jmp` + `nop`): at 0x1A27A8 in
  `network_client::on_match_ready`, before the 0x42 is sent, and at 0x1A16B1
  in `on_match_packet_received`, the 0x59 case. The new code serializes the
  client's world with the game's own `game_state_history_item::serialize` and
  sends it. No header changes.
- Detail and the other ways that were weighed:
  `../docs/notes/option_b.md`, `option_c.md`.

### 3. Mode 3 batteries

- **The bug:** the rule that places batteries in mode 3
  (`match_time_vi_spawn_rule::tick`, 0x1FC388) keeps the time of the next
  battery in a field that it never serializes, and moves it on once a
  battery is placed. A lockstep client rolls back and simulates again on
  nearly every remote input, so the restored state has the battery unplaced
  while that time is already past it: no battery ever stays.
- **The fix:** a new `tick` in place of the old one (0xA7 bytes) that
  derives the time from serialized fields only, a hook in the rule's
  `deserialize` (0x1FC663) that takes back batteries a rolled-back
  simulation placed beyond the restored count, and 0x70 bytes for it at the
  end of the same padding (0x6A1F90).
- Every client of a match must have the same rule, or their worlds differ.
- Detail: `../docs/notes/battery_spawn.md`; `spawn_model.py` shows the
  original and the patched rule side by side.

### 4. The login certificate (a file, not an exe patch)

- **What:** `survarium_login_server.crt` goes over
  `game/resources/ssl/survarium_login_server.crt`.
- **Why:** the client verifies the login server's certificate against that
  file. The game's own ran out in 2014, so its first sign-in attempt fails
  with "certificate verify failed". This one is the same name and key issued
  again (valid to 2037); the key is in `../certs/`, which the login server
  reads.

### 5. Optional: the lobby UI (`lobby_menu.py`, changes `resources.db`)

Not part of the players' download; the server works without it.

```
python lobby_menu.py <game>/resources.db            apply
python lobby_menu.py <game>/resources.db --restore  put the original back
```

- **The shop's buy slider:** for stacks the cap was `step × step` times too
  low (`../docs/notes/shop_slider.md`).
- **The new-items badge:** it was drawn over the third button instead of the
  second (`../docs/notes/new_items_badge.md`).

The original Flash file is saved beside the db first, and anything but the
known original or this script's own result is refused.
