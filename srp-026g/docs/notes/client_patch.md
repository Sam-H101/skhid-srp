# Porting the srp client patch from 0.21d to 0.26g

Version scope: client 0.26g (build 2777, "0.26g0", linked 2015-01-15, no PDB) vs client 0.21d (build 2010, April 2014, PDB). Every address is a VA, image base 0x10000 in both; `.text` VA 0x11000 = file 0x400 in both.

Tags: **[V]** read in the exe/disassembly; **[I]** inferred. Patch-site status: **PROVEN** (0.26g VA + original bytes + the 0.21d site it corresponds to), **LIKELY**, **GUESS**.

Summary: the mutex patch ports 1:1 (PROVEN). The snapshot upload is still needed (the world is still frozen until the 0xE8 snapshot, and the client never sends its state); it ports with new addresses. Only 0x5D bytes of .text padding exist, so the upload code goes into a new PE section `.srp` (default) or over the dead `on_hash_mismatch` (fallback). The patcher, the assembly and a self-test are in `srp-026g/client-patches/` (reviewed against the disassembly 2026-10-12, §3.1–§3.5; not run live yet). The mode 3 battery patch is not needed: 0.26g serializes the next-battery time.

## Function map (`tools/re-kit/v026g/functions_matched.txt`)

- Made by `tools/re-kit/common/scripts/match_functions.py` (stdlib only, ~1 min): `python match_functions.py [--ref 021d --target 026g] [--stats] [--holdout 0.3 --holdout-from exact|tokens]`.
- Format: `<0.26g VA> <length> <0.21d name(s)>  # <confidence>; <method>: <details>; 021d <0.21d VA> len <0.21d length>`. `#` lines at the top are the header (format, counts, method legend). Hex without `0x`, like `v021d/functions.txt`.
- Hit rate: **22 478 of 35 070** distinct 0.21d functions (64 %); 16 956 **high**, 5 522 **medium**; weaker candidates are not written. 41 263 function starts found in 0.26g.
  - By method: exact bytes 9 807, exact shape 678, tokens 7 315, propagated 1 870, vtable 1 436, address order 1 372.
  - By area: Scaleform ≈ 93 %, STLport/boost/Bullet/OpenSSL high; **game code much lower** (survarium:: ≈ 30 %, network ≈ 18 %). 0.26g game code was recompiled with different settings: frame-pointer omission (`mov ebx,[esp+8]` instead of `push ebp; mov ebp,esp`), 16-byte function alignment with `int3` padding (0.21d is packed), different inlining. Library code (prebuilt) is byte-identical apart from relocations.
- Method (details in the script docstring): x86 length decoder → per-function normalized bytes, shape (prefix/opcode/modrm sequence), tokens (strings incl. `__FUNCSIG__` log strings, imports, RTTI vtables/type descriptors, later matched callees and globals). Passes: unique exact bytes, unique exact shape, reciprocal-best weighted tokens, propagation through matched pairs (instruction-aligned calls/pointers; call-sequence gaps need 2 agreeing callers), RTTI vtable slots, address-order gaps. Every non-exact candidate needs a structural check: shape similarity, or opcode-histogram cosine + instruction-count ratio (tolerant of the recompilation).
- Confidence:
  - **high**: exact bytes (≥ 6 insns) / exact shape (≥ 30 insns), or a strong signal plus structure ≥ 0.8 shape or (histogram ≥ 0.9 and size within ×0.75–1.33). Hold-out test (2 775 exact matches, 30 %, hidden from the hash passes): 2 680 recovered by the other methods, **1 wrong** (a high token match).
  - **medium**: one signal + a looser structural check. Hold-out on 222 hidden "≥ 2 unique strings" pairs (blocked from the token pass): 107 recovered by propagation/vtable/order, **0 wrong**. One wrong medium match was found by hand and fixed by tightening propagation (`game_world_core::clear` was given to 0.26g `new_game_state_history_item` 0x2D09C0); expect a few % of medium rows to be wrong, mostly where inlining differs.
- Function starts in 0.26g are heuristic (call targets, data pointers, int3-padding ends, tail jumps). A "length" runs to the next found start and can include an undiscovered neighbour.

## Exes used

- 0.21d original: `E:/sur-res-pro/downloads/v021d/extracted/game/binaries/x86/survarium.exe` (sha256 def82eea...). **Note:** `E:/ssrp/Survarium-0.21d-srp/binaries/x86/survarium.exe`, which `common/scripts/exes.py` lists as "original", is the srp-patched copy (same sha as `survarium_srp.exe`: mutex `EB 22`, cave filled, new tick). [V]
- 0.26g: `E:/sur-res-pro/downloads/v026g/extracted/Survarium/game/binaries/x86/survarium.exe` (12 894 840 bytes). Not modified.

## 1. Single-instance mutex — PROVEN

| | 0.21d | 0.26g |
|---|---|---|
| code | inline in `WinMain` 0x12644 | own function 0x124D0 (returns bool), same strings |
| mutex name | `survarium_or_its_installer_is_already_running` 0x7EB7B4 | same string at 0x9DAECC, pushed at 0x124D6 [V] |
| check | `cmp eax,0B7h` (ERROR_ALREADY_EXISTS) then `jne 0x1269E` (= OK) at **0x1267A** `75 22` | `cmp eax,0B7h` then `jz 0x12504` (= fail path) at **0x124F9** `74 09` [V] |
| patch | `75 22` -> `EB 22` | `74 09` -> `90 90` (fall through to `mov al,1; ret`) |

The fail path is the same message box (`%s or its installer is already running...`). Still needed: yes, if several clients must run on one PC.

## 2. Is the snapshot upload (option B) still needed? — yes

- **The world still freezes until the first snapshot.** [V] `game_world_core` ctor (0.26g, around 0x2D00B0–0x2D0136) sets, exactly like 0.21d 0x1CA5F8–0x1CA609:
  - +0x18FDD = server-mode argument (0.21d +0xC80D);
  - +0x18FDF = frozen = !(server_mode || replay) (0.21d +0xC80E), written at 0x2D0130;
  - `game_world_core::move` 0x2D2E40 returns at once while +0x18FDF is set (0x2D2E4A; 0.21d 0x1CCCB4);
  - only `game_world_core::deserialize` clears it (0x2D0FE7 `mov byte [edi+18FDFh],0`; 0.21d 0x1CAD89). It is reached from the 0xE8 handler (0x27BFF0), the 0.26g dynamic-info message (0.21d 0x53).
- **The client still never sends its state.** [V] `game_state_history_item::serialize` 0x324BE0 has three callers: `hash()` 0x325150 (CRC only), `game_world_core::on_hash_mismatch` 0x325A60 (debug compare of a server state, only with `-debug_hash_mismatch`) and the new `game_world_core::dump_game_state` 0x3285D0 (writes a file; string `survarium::game_world_core::dump_game_state`). None of them builds a packet.
- The hash report is still there: `try_send_all` 0x27CBF0 sends **0xDF** = `{u8 (event-horizon item time / 50), u32 crc}` (0.21d 0x46 had a u32 time), using `is_there_event_horizon_item` 0x2D1070, `event_horizon_history_item` 0x2D0B30 and `hash()` 0x325150 with the players-mask history at core+0x12280. [V] The CRC table generator 0x1C210 uses polynomial 0x04C11DB7, reflected, init/final 0xFFFFFFFF (boost crc_32, as 0.21d). [V]
- So a non-simulating srp still needs a patched client to obtain the first and the resync snapshots. The 0.26g snapshot message also changed (below), so srp's 0.21d forwarding needs work too.

### What changed around it [V unless tagged]

- Message ids moved. Server→client dispatcher `on_match_packet_received` **0x27A300** (FUNCSIG string 0xA08908; 0.21d 0x1A1520), jump table 0x27A738, ids 0xE3–0xF6 (20; 0.21d 0x50–0x59). Client→server ids 0xD8–0xE1 (10; 0.21d 0x40–0x47). Message-info tables: C→S 0x279A60 (table 0x279AE0), S→C 0x279B10 (table 0x279BF0); entries are `{u32, u16, u8 flags}` (0.21d `{u16, u8, u8}`); meaning of the fields not re-derived [I].

| 0.21d | 0.26g | evidence |
|---|---|---|
| C→S 0x40 connect | 0xD8 | `match_client::connect` 0x27A1B0 pushes 0xD8 |
| C→S 0x41 port probe | 0xD9 | 0xE3 handler 0x27B830 passes 0xD9 to `set_connection_ports` (vtbl+0xC) |
| C→S 0x42 ready | 0xDA | end of `network_client::on_match_ready`, 0x27AEAE |
| C→S 0x43 set-time reply | 0xDB | 0xE5 handler 0x27B570 (0x27B5E7); also sent by `try_send_all` 0x27CE7A after 0xE6, with three u16 |
| C→S 0x44 clock | 0xE1? | `try_send_all` every 1000 ms (0x27CE3E), empty [I] |
| C→S 0x45 input, new ones | 0xDC / 0xDD / 0xDE | not traced; 0xDE is built in 0x27EB90 [I] |
| C→S 0x46 hash | 0xDF | `try_send_all` 0x27CDC9, after `hash()` |
| C→S 0x47 leave (u32) | 0xE0 (**empty**) | `close_current_match` 0x27A790, send at 0x27A833 |
| S→C 0x50 ports | 0xE3 | state-0 case, 0x27B830 |
| S→C 0x51 static info | 0xE4 | state 1→2 case, 0x27B880: u8 flag, then a payload that goes through a decompressor (`[nc+0x4C]` vtbl+4) into a 0x578A0 buffer |
| S→C 0x52 set time | 0xE5 | 0x27B570: u8 sequence (window ±128 against `[nc+0x1F1C68]`), u16, u32 time → `game::set_time_in_ms`, reply 0xDB |
| — | 0xE6 | 0x27B770: two u16 timing values, sets `[nc+0x1F1C69]` (then 0xDB from `try_send_all`) |
| S→C 0x55 clock offset | 0xE7 | 0x27B7F0 → `floating_timer::set_offset` |
| S→C 0x53 dynamic info (snapshot) | **0xE8** | 0x27BFF0: u8 sequence (same window), **u8 compressed**; if non-zero the rest is decompressed into 0x8000 bytes first, if zero it is read as is; then `game_world_core::deserialize` 0x2D0B70 (string "--- THE VERY FIRST SYNCHRONIZATION ---"), whose item reader takes the u32 item time first |
| S→C 0x56 entered | 0xE9 | 0x27C5F0 → `enter_match` 0x2D1470 |
| S→C 0x57 left | 0xEA | 0x27C690 → `leave_match` 0x2D18A0 |
| S→C 0x54, 0x58, new ones | 0xEB–0xF5 | not traced (EB/EC, ED/EE, EF/F0, F1/F2 share handlers) |
| S→C 0x59 hash mismatch | **0xF6** | case 0x27A4F4; parsed only when `g_debug_hash_mismatches` (byte 0x1F1D32C) is set, else a log hint and skip |

- `network_client` grew: core pointer, match client and state moved from +0x7A00 / +0x7A08 / +0x7A1C to +0x1F1C40 / +0x1F1C48 / +0x1F1C64. `game_world_core` is now **embedded** in `pvp_match_core` at +0x1F8 (0.21d: a pointer at pvp+0x138).
- Packets: the `udp_match_packet` ctor 0x278D00 sets the write buffer at pkt+0x298 to `{allocator 0, data pkt+0x99, capacity 0x1FC, size 3}` and the writer at **pkt+0x2A8** (0.21d: pkt+0x284 `{0, pkt+0x7B, 0x208, 0x0C}`, writer +0x294). The datagram header looks 3 bytes long now [I: from the initial size]. Writing past the capacity still reallocates through a null allocator (`mutable_buffer::resize` 0x7AB30 → 0x7AB70).

## 3. Snapshot-upload port table

Status: **PROVEN** = 0.26g VA + original bytes [V] + the 0.21d counterpart; **LIKELY** = right place, one convention or meaning inferred.

### Patch sites

| what | 0.21d | 0.26g | original bytes (0.26g) → patch | status |
|---|---|---|---|---|
| mutex check | 0x1267A `75 22` | **0x124F9** `jz 0x12504` | `74 09` → `90 90` | PROVEN |
| default hash interval (full builds only, §3.6) | none (not in the 0.21d patch) | **0x27CC4F** `mov dword [esp+18h],64h` in `network_client::try_send_all` 0x27CBF0 | `C7 44 24 18 64 00 00 00` → `C7 44 24 18 32 00 00 00` (one byte, 0x27CC53) | PROVEN (the bytes and what reads them); the effect is not tried live |
| first-sync hook (before the ready send; ebx = network_client) | 0x1A27A8 `lea esi,[ebx+7A08h]` in `on_match_ready` 0x1A25CC | **0x27AEA1** `mov ebx,[ebx+1F1C48h]` in `on_match_ready` **0x27AA20** (then `mov esi,[ebx]` … `push 0DAh; call [edx+18h]`); returns to 0x27AEA7 | `8B 9B 48 1C 1F 00` → `E9 rel32 90` | PROVEN |
| request hook (0x59 / 0xF6 without the debug flag) | 0x1A16B1 `mov edi,[g_log_filter_tree]`, edi = nc | **0x27A505** `cmp byte [1F1D32Ch],0` (the `g_debug_hash_mismatches` test, one instruction earlier than in 0.21d so the debug branch becomes unreachable); esi = nc, ebx = reader, edi = 0 | `80 3D 2C D3 F1 01 00` → `E9 rel32 90 90` | PROVEN |
| request done (skip payload, return) | 0x1A1721 | **0x27A5AB** `mov edx,[ebx+8]; add edx,[ebx]; mov [ebx+4],edx; jmp 27A6FCh` | `8B 53 08 03 13 89 53 04 E9 44 01 00 00` (unchanged) | PROVEN |
| code cave | 501 zero bytes after .text at 0x6A1E0B (0x185 upload + 0x70 batteries) | **only 0x5D zero bytes** after .text (0x86F5A3–0x86F5FF; .rdata's raw data starts right after): too small. Cave = **`game_world_core::on_hash_mismatch` 0x325A60**, 0x47B bytes of code + 5 int3 (FUNCSIG "void __thiscall survarium::game_world_core::on_hash_mismatch(class vostok::network_core::buffer_reader)"). Its only reference is the `call` at 0x27A632 inside the debug branch (no pointer to it, no other branch into it: full-.text sweep) | sha256 of 0x325A60..0x325EDF = `f757a414…040c`; 47 base relocations inside, disabled by the patcher | PROVEN (dead once the request hook is in) |

The default cave is now a new section, which keeps `on_hash_mismatch` intact; the overwrite in the table above is the `--cave=hash-mismatch` fallback (§3.3).

### Functions and data the injected code uses

| role | 0.21d | 0.26g | convention (0.26g) | evidence | status |
|---|---|---|---|---|---|
| item serializer | `game_state_history_item::serialize` 0x1FDE3E (eax = item, push mask, push &writer, ret 8) | **0x324BE0** | **stdcall(item, writer*, mask), ret 0Ch** | called from `hash()` at 0x3251D7 with the same scratch writer; ends `ret 0Ch`; writes the u32 mask, then `player_history_item::serialize` … | PROVEN |
| `hash()` | 0x1FE15A | 0x325150 (eax = item, push &masks, ret 4) | | same 0x8000 alloca, writer layout, mask walk, CRC; cache at item+0xC0BB4 / +0xC0BB8 (0.21d +0xB993C / +0xB9940) | PROVEN |
| buffer write | `buffer_writer::w(writer*, src, n)` 0x155EBE, ret 0Ch | **0x7AC10** | **push src; push writer*; edi = n; ret 8** | `resize` 0x7AB30 (eax = mutable_buffer), then `memcpy` 0x424580; the only small function with that call pair | PROVEN |
| stack probe | `__alloca_probe_16` 0x2B3F00 | **0x424CA0** | eax = size | CRT code; used by `hash()` at 0x325182 | PROVEN |
| has event-horizon item | `pvp_match_core::is_there_event_horizon_item` 0x1DCBE6 (eax = pvp) | **0x2D1070**, now on the core (eax = **core**) | → al | same tests (head, server-mode flag, time window); `try_send_all` calls it at 0x27CD7A before 0xDF | PROVEN |
| event-horizon item | `game_world_core::event_horizon_history_item` 0x1CAB13 (eax = core) | **0x2D0B30** (eax = core) | → eax | server mode → newest; else walks back from the newest while time > horizon | PROVEN |
| initial item builder | `game_world_core::serialize` 0x1CA765 (called at 0x1A26CF) | 0x2D04F0 (esi = core), called at 0x27AD15 in `on_match_ready` | | new item 0x2D09C0, world serialize 0x2D2570, append at the tail | PROVEN |
| match client | `[nc+0x7A08]`, vtbl +0x18 `new_packet(id)`, +0x1C `enqueue(pkt)` | **`[nc+0x1F1C48]`**, vtbl +0x18 = 0x279FE0 (→ 0x37A0E0), +0x1C = 0x27A2A0 | thiscall, one stack arg | `match_client@survarium` vtable 0xA08864; same calls in `on_match_ready` / `try_send_all` | PROVEN |
| core | `[[nc+0x7A00]+0x138]` | **`[nc+0x1F1C40]` + 0x1F8** | | `try_send_all` 0x27CC76 / 0x27CD66, `on_match_ready` 0x27AD0D, 0xE8 handler 0x27C0C9 | PROVEN |
| network state | `[nc+0x7A1C]` == 2 | **`[nc+0x1F1C64]`** == 2 | | every state test in the dispatcher | PROVEN |
| newest history item | `[core+0x2678]` | **`[core+0x2FC0]`** (oldest +0x2FBC, count +0x2FB8) | | 0x2D04F0 appends there; 0x2D0B30 returns it in server mode | PROVEN |
| players-mask history | core+0x5ED0, head +8, node {+4 next, +8 mask, +0xC time} | **core+0x12280**, same node layout | | `hash()` mask walk; `try_send_all` 0x27CD8A | PROVEN |
| item time | item+0xB9938 | **item+0xC0BB0** | | `hash()`; item deserialize 0x324DA0 | PROVEN |
| packet writer | pkt+0x294 | **pkt+0x2A8** | | ctor 0x278D00; every 0.26g send (`lea esi,[ebx+2A8h]`) | PROVEN |
| carrier message | C→S 0x47 (leave, u32), reliable, unordered | **C→S 0xE0** (leave, **empty**), info `{0xFFFFFFFF, 500, 0x9F}` like 0xD8 / 0xDA: reliable (resent every 500 ms until acked), plain, fixed length 0 | | 0x27A833; table 0x279AE0; send path §3.2 | PROVEN (a non-empty 0xE0 leaves the client intact, alone in its datagram) |
| chunk size | 480 (12 + 1 + 8 + 480 ≤ 508) | 480 (3 + 1 + 8 + 480 = 492 ≤ 0x1FC; 16 bytes left for tail ack bytes) | | packet capacity 0x1FC, initial size 3, id written by `construct_packet` 0x3780B1, no x byte for flags 0x9F (0x3780E5) | PROVEN (§3.2) |
| packet pool | — | `[[nc+0x1F1C48]+0x578B0]`: +0x30 packets in use, +0x34 capacity = **256** | | `new_packet` 0x37A0E0, allocator 0x279D80, pool built at 0x378A77 with 0x2C000 bytes / 0x2C0 | PROVEN |
| packet flags | — | pkt+0x98 = 0x60 from the ctor: bit 5 single message, bit 6 may be joined | | 0x278D98, 0x3FCA86, 0x3FCAD5 | PROVEN |

### 3.1 Review of the injected code (2026-10-12) — PROVEN unless tagged

`snapshot_upload_026g.asm` was read instruction by instruction against the original 0.26g exe (`dis.py 026g`). The numbered list at the end of the .asm file repeats this per line.

| what the code relies on | read at | result |
|---|---|---|
| `serialize` is stdcall(item, writer*, mask) | 0x324BE0 `push ebx; mov ebx,[esp+0Ch]` (writer), `push ebp; mov ebp,[esp+0Ch]` (item), mask read at 0x324CB1, `ret 0Ch`; `hash()` pushes mask, &writer, item (0x3251D1) | right; saves ebx, ebp, esi, edi |
| scratch writer layout | `hash()` 0x32518A..0x3251B0 | `mutable_buffer {0, esp, 0x8000, 0}`, writer `{+0 0, +8 0, +0xC 0, +0x10 0xFF, +0x14 &buffer}`: same |
| players mask for the item | `hash()` 0x325197..0x3251CE: head `[masks+8]`, node +0xC time, +4 next, +8 mask | same walk; `hash()` reads address 8 when the list is empty, the patch passes mask 0 |
| `buffer_writer::w` | 0x7AC10: writer = `[esp+8]` after one push (last pushed), src = `[esp+10h]`, n = edi, `ret 8` | right; edi is only read; saves ebx, esi |
| `_alloca_probe_16` | 0x424CA0 (tail-jumps to `_chkstk` 0x429F20) | only eax and esp change; `hash()` takes the buffer from esp the same way |
| `is_there_event_horizon_item` / `event_horizon_history_item` | 0x2D1070 / 0x2D0B30, called with eax = `[nc+1F1C40h]+1F8h` at 0x27CD66..0x27CD85 | eax in, al / eax out; esi and edi survive |
| `new_packet` / `enqueue` | vtbl+0x18 = 0x279FE0 → 0x37A0E0 (`ret 4`, packet in eax); vtbl+0x1C = 0x27A2A0 (`ret 4`) | thiscall with one stack argument; esi, edi survive |
| packet writer | ctor 0x278D9F..0x278DD7: buffer at pkt+0x298 `{0, pkt+0x99, 0x1FC, 3}`, writer pkt+0x2A8 with +0x14 → the buffer | right |
| ready hook 0x27AEA1 | 0x27AE9A `mov byte [ebx+1F1C68h],0FFh` just before: ebx = network_client. After it the function reloads esi, edx, eax and pops edi; no flags are used | `pushad`/`popad` around the upload, then the replaced `mov ebx,[ebx+1F1C48h]`, then `jmp 27AEA7h`. The 6 bytes hold no relocation |
| request hook 0x27A505 | 0x27A300: ebx = reader (`[ebp+0Ch]`), esi = network_client (`mov esi,ecx`). Jump table 0x27A738 entry 19 = 0x27A4F4, which skips the payload when state ≠ 2 and **falls into 0x27A505 in every state** | the cave tests state == 2 itself; it ends at 0x27A5AB (reader := end, then the common exit 0x27A6FC, which uses ebx and ebp only). The 7 bytes hold one base relocation (0x27A507): the patcher disables it |
| thread | both sites are followed by the client's own `new_packet`/`enqueue` on the same object (0x27AEAE; the 0xE5 handler sends 0xDB at 0x27B5E7 from the same dispatcher) | the upload runs where the client itself sends |
| nothing else enters the replaced bytes | linear disassembly of both functions, the jump table, and a brute-force scan of `.text` for rel8/rel32 branches and pointers into 0x27AEA2..0x27AEA6 and 0x27A506..0x27A50B (`selftest_026g.py`) | none |

Fixed or added in the review:
1. **Never joined.** Each chunk packet gets `and byte [pkt+98h],0BFh` (§3.2).
2. **Pool guard.** The upload is skipped unless `chunks + packets in use + 16 ≤ pool capacity` (§3.2).
3. **Rebase safe.** The code has no absolute operands; the patcher disables the base relocation under the request hook (the draft only did so inside the overwritten function). The image has no ASLR flag (DLL characteristics 0x8000, file characteristics 0x122), so a rebase happens only if 0x10000 is taken.
4. The 0xF6 request must carry **one payload byte** (§3.4); the draft said "any payload".

Limits that stay: a world over 0x8000 bytes asserts in `mutable_buffer::resize` (0x7AB70, null allocator), exactly as the client's own `hash()` does every 100 ms, so the patch adds no new failure there.

### 3.2 The carrier: C→S 0xE0 with a payload — PROVEN from the send path

Every C→S id is fixed length (transport.md §4), so no id carries a length; 0xE0 (leave, length 0) is kept because a real leave is told apart by being empty, and it is reliable (lifetime ∞, resend 500 ms). What the client does with a 0xE0 packet that has a payload:

- **Build.** `construct_packet` 0x378080 appends the id byte and, only for flags 0x40/0x20, a placeholder x byte (0x3780E5). 0xE0 has flags 0x9F: the packet is `3 header bytes, E0`, then whatever the caller writes. Nothing checks the size against the id's fixed length on the way out.
- **Enqueue.** `enqueue_impl` 0x3FCE30 touches the x byte only for ordered/unique ids, then links the same packet object into the connection's queue. No copy, no split (the splitter 0x3F9D00 is only in `enqueue(id, reader, size)` 0x3FD060, which the patch does not use).
- **Size.** `send_queued_packets` asserts only when the packet is over 0x1FC bytes (0x3FC310). A chunk packet is 3 + 1 + 8 + 480 = **492**.
- **Tail ack bytes.** `free = 0x1FC − size` (0x3FCA27); the client appends `min(wanted, free)` ack bytes (0x3FCA35..0x3FCA4B; wanted = (depth − 7 + 6)/7 ≤ 17, 0x3FC27E..0x3FC2AA). With 16 bytes free a chunk datagram can carry all but the 17th; when some did not fit, one extra keep-alive datagram with the full set follows the flush (0x3FCBCD..0x3FCBF4). The send completion handler removes the ack bytes from the packet again (0x3FAFE9: resize by −n), so a resend starts from 492 bytes.
- **Single datagram.** With no joined follower (`[pkt+4] == 0`, 0x3FB970) the packet's own buffer is sent: header filled in place by 0x3FB390, F = 0 because bit 5 of pkt+0x98 is set (0x3FB41A). **So the server receives `seq rseq ack0 E0 <payload> [tail ack bytes]`, single format.**
- **Joining.** Only with `-join_net_messages` (0x3FCA57..0x3FCA7D), and only when the first packet (0x3FCA86) **and** the candidate (0x3FCAD5) have bit 6 of pkt+0x98. The ctor sets 0x60 (0x278D98). The patch clears bit 6 on its packets, so a chunk is never the head of a joined datagram and never joined into one, whatever the option says. Bit 6 is tested nowhere else in `.text` (only these two `test byte [reg+98h],40h` exist); the low nibble of the byte is an in-flight counter (0x3FB2A4, 0x3FAFBE) and bit 4 "queued" (0x3FCEC4), both untouched.
- **Burst size.** A snapshot is at most 0x8000 bytes = **69 chunks** (0.21d worlds were 11–14 KB = 23–30). Three limits apply, all satisfied by one upload:
  - send window: the client sends while its next seq is 1..127 ahead of the newest acked (0x3FC2C3..0x3FC2FB); the rest of the queue is put back (0x3FCB81) and goes out on a later pass, nothing is lost;
  - unacked limit: ≥ 256 packets on the unacked list disconnects (0x3FC146). Only reliable packets count (0x3FB620);
  - **packet pool (new finding):** game-side packets come from a 256-packet pool (0x378A77: 0x2C000 bytes / 0x2C0) and stay out of it until acked or expired. `new_packet` 0x37A0E0 does not check for an empty pool: 0x279E8D asserts and returns 0, and `construct_packet` then writes to a null packet. So a burst that empties the pool **crashes** the client. The patch reads the pool's use count (`[pool+30h]`, compared with `[pool+34h]` by the client itself at 0x279D87) and skips the upload unless `chunks + in use + 16 ≤ capacity`. Since every unacked packet is a pool packet, this also keeps the unacked list under 256.
  - Pacing is therefore not needed for one upload. The server must not ask again before the previous upload is complete or about a second old (§3.4), or the guard will skip the new one.
- **Alternative carriers considered:** none is better. All ten C→S ids are fixed length and plain; 0xD8/0xD9/0xDA/0xDE are also reliable but have live meanings with non-zero lengths (a chunk could equal a real message's length), and the send-once ids would lose chunks. 0xE0's real form is the only empty reliable one.

### 3.3 Cave placement — section by default

**`--cave=section` (default).** A new section holds the upload code; `on_hash_mismatch` stays intact.

| header item | original | patched |
|---|---|---|
| NumberOfSections | 7 | 8 |
| section table | ends at 0x328; 216 zero bytes to SizeOfHeaders 0x400 | entry 8 at 0x328: `.srp`, VirtualSize = code size (0x1A4), RVA **0xA415000** (VA **0xA425000**), raw 0xC4AA00 size 0x200, characteristics 0x60000020 (code, execute, read) |
| SizeOfImage | 0xA415000 | 0xA416000 |
| SizeOfCode | 0x85E600 | 0x85E800 |
| security directory (file offset, size) | 0xC4AA00, 0x1878 | **0xC4AC00**, 0x1878 |
| CheckSum | 0xC5A7B2 | recomputed (imagehlp agrees) |
| file size | 12 894 840 | + 0x200 |

- **The Authenticode blob.** The attribute certificate table is found by the **file offset** in the security data directory and is the last thing in the file (0xC4AA00 + 0x1878 = file size), right after `.reloc`'s raw data. The new section's raw data must follow `.reloc` (sections are contiguous in the file), so the blob moves 0x200 bytes towards the end and the directory's offset follows it; it stays 8-byte aligned and last, as the PE format asks. Its bytes are unchanged.
- **Why it must stay readable.** At start `vostok::crypt::verify_sign` 0x13310 calls `CryptQueryObject(CERT_QUERY_OBJECT_FILE, own path, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED 0x400, CERT_QUERY_FORMAT_FLAG_BINARY 2, …, &store, &msg, …)` (0x13336..0x1335B), then looks up "survarium.com" in that store and uses the certificate's key on the resources.db file table (§0). Dropping the blob would stop every start.
- **A changed file does not break it.** `CryptQueryObject` only extracts and decodes the PKCS#7 blob; it does not compare the Authenticode hash with the file (that is `WinVerifyTrust`; the exe has no `WinVerifyTrust`/wintrust or imagehlp checksum import, its only use of the signature is this `CryptQueryObject`). PROVEN two ways: the cert-only build already runs (§0), and `selftest_026g.py` makes the client's exact call on each built file: the store opens and the survarium.com certificate (1545 bytes, sha1 4b330026…eea6) is byte-identical to the original's, for the section, hash-mismatch and cert-only builds.
- **Relocations.** `.reloc` is untouched except the one entry under the request hook (0x27A507 → type 0). The new code needs none.
- **Risk, LIKELY harmless:** whether anything in the client minds an 8th section (a packer or anti-tamper check of its own image). None was seen: no code reads its own section table apart from the CRT/TLS startup (not searched exhaustively). If the section build misbehaves live, use the fallback.

**`--cave=hash-mismatch` (fallback).** As the draft: the code overwrites `game_world_core::on_hash_mismatch` 0x325A60 (0x47B bytes; unreachable once the request hook replaces the debug test), the rest becomes int3, the 47 relocations inside it and the one under the hook are disabled. No header change except the checksum. Loses the `-debug_hash_mismatch` compare.

### 3.4 Wire format for the server

**C→S 0xE0.** Flags 0x9F: plain, reliable, no x byte, fixed length 0 in the client's table.

| datagram | bytes after the id | meaning |
|---|---|---|
| single (F = 0) | none | a real leave (`close_current_match` 0x27A828) |
| single (F = 0) | `u32 item_time, u16 offset, u16 total, data[n]` with `n = min(480, total − offset)`, so 9..488 bytes | a snapshot chunk from a patched client |
| multi (F = 1) | none (take length 0, as the table says) | a real leave; a chunk never appears here (its packet cannot be joined, §3.2) |

- In a single datagram the message is everything between the id and the tail ack bytes (transport.md §3.1), so the server knows the length: 0 = leave, otherwise a chunk. Accept a chunk only if `offset % 480 == 0`, `offset < total ≤ 0x8000` and `length == 8 + min(480, total − offset)`; ack the datagram in any case.
- A chunk is resent every 500 ms until acked, and 0xE0 is plain (no transport dedup): drop duplicates by `(item_time, offset)`. Chunks can arrive in any order (the client sorts its send queue, transport.md §2.5, and UDP reorders): place each by its `offset`.
- When `total` bytes are present, the body is exactly what `game_state_history_item::serialize(mask)` wrote: `u32 active_clients_mask`, the player blocks, then the six u16-prefixed blobs (match.md §3). Its boost CRC-32 is what 0xDF reports for that item. The 0xE8 to send is `u8 generation, u8 0, u32 item_time, body`.
- Two uploads of one client never interleave within one `item_time`; a new request can produce a new `item_time` while old chunks are still being resent, so key the reassembly by `item_time`.

**S→C 0xF6 = "upload your event-horizon state".** 0xF6 is unique and variable length: `F6 <unique id> <payload>`, and the length getter rejects an empty payload (0x2799C1: `ok = remaining > 0`; a failed parse drops the whole datagram, transport.md §2.3). **Send one payload byte, `00`.** The hook ignores the payload. Send it only to patched clients (a stock client skips it and logs a hint).

The request is silently not answered when:
- the client is not in state 2 or has no match core yet;
- `is_there_event_horizon_item` is false: no history, or the event-horizon item came from a snapshot (item+0xC0BB9), or it is younger than the horizon (0x2D1070). So right after a 0xE8 the client has nothing to upload until its simulation passes the snapshot's time by the horizon (`max_ping_time_in_ms` + 250 ms);
- the pool guard skips it (§3.2).

Server rules: one request per client at a time; wait for the upload to complete or about 1 s before asking again; keep acking during the burst (up to 69 datagrams, then resends after 500 ms).

Code (added 2026-10-12): `crates/survarium/src/v026g/match_msgs/upload.rs`
(`Chunk::parse` with exactly these checks, `Reassembler` keyed by
`item_time`, `split` = what the patch sends); the request is
`ServerMessage::UploadRequest`. The last chunk of a full 0x8000-byte world
starts at 0x7F80 and has 128 data bytes.

**First sync.** In `on_match_ready`, after `game_world_core::serialize` 0x2D04F0 built the initial item and before 0xDA, the client uploads `[core+2FC0h]` unasked. The chunks are enqueued before 0xDA; do not rely on their order on the wire.

### 3.5 Build and self-test (`srp-026g/client-patches/`)

- `snapshot_upload_026g.asm` → `nasm -f bin -DCAVE=0xA425000 … -o snapshot_upload_026g.section.bin` and `-DCAVE=0x325A60 … -o snapshot_upload_026g.hash-mismatch.bin` (433 bytes each = 420 cave + 6 + 7 hook bytes; both checked in).
- `patch_026g.py <survarium.exe> <out.exe> [--cave=section|hash-mismatch] [--cert-only] [--bin=<file>]`, or `<survarium.exe> --check …` (everything in memory, nothing written). It checks the exe sha256 (`ac33d993…f153`), each site's original bytes, the free section table entry and the file layout (section mode) or the function's sha256 (hash-mismatch mode), and refuses a .bin assembled for the other cave. `--cert-only` output is byte-identical to the `survarium_srp.exe` installed in step 0.
- `selftest_026g.py` builds the three variants into `srp-026g/target-a4/scratch/` and checks each: PE re-parse (sections aligned and contiguous, SizeOfImage, data directories, certificate table last and identical); every byte outside the intended ranges equals the original (section build: 40 changed bytes in 12 ranges below the old end of file, plus the 0x200 new bytes; hash-mismatch: 1199 bytes in 55 ranges; the counts include the checksum's bytes that happen to differ); no active relocation under rewritten bytes and the rest of `.reloc` untouched; ndisasm of the cave and hooks (every direct branch inside the cave or to one of the 7 external addresses, the two indirect calls are `[eax+18h]` / `[eax+1Ch]`, both entries start with `pushad`); no branch or pointer into the hooks anywhere in `.text`; imagehlp's checksum equals the header's; the client's own `CryptQueryObject` call still yields the survarium.com certificate; the hash interval site (§3.6) is `mov dword [esp+18h],32h` in the full builds and the original in `--cert-only`. **All checks pass (2026-10-12, again with §3.6).** Nothing was run or installed; the builds in the scratch folder are for inspection only.
- Not tested: the code itself running (needs a 0.26g match server to reach `on_match_ready`).

### 3.6 Default hash interval 100 → 50 ms (added 2026-10-12) — PROVEN bytes, not run live

- **Why.** A stock client sends its world hash (0xDF) when `last_sent + interval <= now` on the 50 ms grid, with interval 100: every second tick, on a phase of its own that is drawn again at each 0xE5 (match.md §14). Two clients on opposite phases never send a hash for the same tick, and the match server trusts a world only when two clients named its tick: worlds are compared only at the tick of an upload that happens to fall on the other client's ticks (about one upload in two; the others are not trusted), a client out of step is found late, so the end by the mode's rule and the results come late or not at all. With 50 every client reports every tick, so there is no phase.
- **Where.** `network_client::try_send_all` 0x27CBF0, the one-time block guarded by bit 0 of 0xA2B3940: `lea ecx,[esp+14h]` / `push ecx` / `mov esi,0AA19F8h` (the option object of `-hash_check_interval_in_ms`) / **0x27CC4F `mov dword [esp+18h],64h`** / `call 0x141B0` / `mov edx,[esp+14h]` / `mov [0A2B393Ch],edx`. After the push `[esp+18h]` is the variable whose address was pushed: the instruction stores the default, and 0x141B0 writes over it only when the key was given (it returns at 0x141CA without touching `[ecx]` when 0xDDF20 finds no value). [V]
- **Patch.** `C7 44 24 18 64 00 00 00` → `C7 44 24 18 32 00 00 00`: the imm32's low byte at VA 0x27CC53 (file offset 0x26C053), `64` → `32`. Full builds only (`--cave=section` and `--cave=hash-mismatch`); `--cert-only` is unchanged and still byte-identical to the installed `survarium_srp.exe`.
- **Checked** (the self-test repeats each): no base relocation covers the immediate (the nearest are at 0x27CC40 and 0x27CC4B, the two absolute operands before it); no branch of the function lands inside the instruction; the static 0xA2B393C has one writer (0x27CC60) and two readers, both the same `last_sent + interval` gate (0x27CD2E and 0x27CD58); the option object 0xAA19F8 is named only at 0x27CC4A and at its registration 0x853F50, so nothing else holds or reads this default. [V]
- **Without the patch:** start every client with `-hash_check_interval_in_ms=50` (LIKELY syntax, as `-server=`; not tried live). The key still wins over the patched default.
- The match server logs "… opposite hash phases …" when it sees the starved state (match.md §13).

### 3.7 Minimum video-memory startup check

The full patch includes the one-byte bypass already used by `play_skhid.sh`.
Verified against the original build 2777 executable (sha256 checked by the
patcher) and its disassembly:

- VA **0x106411**, file offset **0xF5811**: `73 0F` (`jae 0x106422`) becomes
  `EB 0F` (`jmp 0x106422`). The preceding `cmp ebx,0x14000000` compares the
  detected video-memory size against **320 MiB**. The skipped path pushes
  320 and a warning string, then calls 0x113580. [V]
- The target at 0x106422 returns the original detected size from EDI:EBX
  in EDX:EAX. The zero/unknown-size path at 0x1063F1 still returns
  0x14000000. Neither the measurement nor the fallback is changed. [V]
- This bypasses the startup rejection for a low reported VRAM size; it does
  not increase memory or replace the allocator launch options.
- **Who is affected** (read in the exe, 2026-10-12, not run): for a size of 320 MiB or more, and for a size that cannot be detected (0, answered as 320 MiB at 0x1063F7), the original `jae` already went to 0x106422, so the patched exe does there exactly what the original does. Only a detected size under 320 MiB takes another way: the original stops in 0x113580 (it formats the text and never returns), the patched one returns the size. The function's one caller (0xE9C3F) stores the size (0x1B3A1F0, 0x1B3A1F8) and then has a check of its own, 0xE9D26..0xE9D47: under the minimum it holds at `[esp+28h]`, which is 320 MiB again (0xE990A), it stops with "Not enough video memory to run Vostok Engine v1.0". So by the code a card under 320 MiB is still stopped, with that other text, unless that second check does not see it; this was not tried. The size goes to the memory statistics, a log line and one initializer (0x15583 → 0x13C40) and to nothing of a match's world.
- Applied to `section` and `hash-mismatch` builds, before the existing PE
  checksum update. `--cert-only` stays unchanged. The self-test checks the
  branch target, unchanged return paths, exact changed byte, relocation
  coverage, and rejection of unexpected instruction bytes.

## 4. Mode 3 batteries: not needed in 0.26g

- 0.26g splits the rule: `match_time_vi_spawn_rule` (vtable 0xA06C04) wraps `match_time_vi_spawn_rule_core` (vtable 0xA15CAC). The battery tick is no longer a vtable slot: the core's ctor 0x3147A0 stores a delegate `{this, 0x3148A0}` at +0x1C4 / +0x1C8 (0x3147F1). [V]
- **Tick 0x3148A0** (0.21d `match_time_vi_spawn_rule::tick` 0x1FC388; thiscall, 3 args, `ret 0Ch`): the same LCG (`imul 8088405h; inc`, 0x314929), the same re-roll test `(variation − start + t) mod spawn_time == 0`, the same `t ≥ next` placement. Fields: items +0x110 / +0x114, spawners +0x120 (position at +0x18), LCG +0x140, start +0x168, **next time +0x16C**, A +0x170, spawn time +0x174, variation +0x178, started +0x17C (the tick itself now starts the rule once core+0x18FE4 == 3). A battery that is still inserted (+0x2C8) is removed (vtbl+0xA0) before it is inserted again (vtbl+0x9C). [V]
- **The 0.21d bug is fixed:** `serialize` 0x3149B0 writes, and `deserialize` 0x314B90 reads, +0x168, **+0x16C**, +0x174, +0x178 and +0x17C (0x314B22 / 0x314C82). A rollback restores the next-battery time, so the re-simulation places the battery again. [V] Batteries placed beyond the restored A are still not taken back by deserialize (it loops over the first A only, 0x314BE8–0x314C43), but the re-simulation up to the present re-places them (remove + insert) within the same `move()` [I]. No patch is drafted; a live mode 3 match should confirm.

## 5. Manually identified 0.26g functions (not all in the automatic map)

0x124D0 single-instance check · 0x27A300 `network_client::on_match_packet_received` · 0x27AA20 `network_client::on_match_ready` · 0x27CBF0 `network_client::try_send_all` · 0x27B830 / 0x27B880 / 0x27B570 / 0x27B770 / 0x27B7F0 / 0x27BFF0 / 0x27C5F0 / 0x27C690 = handlers of 0xE3 / E4 / E5 / E6 / E7 / E8 / E9 / EA · 0x27A1B0 `match_client::connect` · 0x279A60 / 0x279B10 sending / received message info · 0x278D00 `udp_match_packet` ctor · 0x37A0E0 `network::match_client::new_packet` · 0x324BE0 `game_state_history_item::serialize` · 0x324D80 `game_state_history_item::deserialize` · 0x325150 `game_state_history_item::hash` · 0x325A60 `game_world_core::on_hash_mismatch` · 0x3285D0 `game_world_core::dump_game_state` · 0x2D04F0 `game_world_core::serialize` · 0x2D09C0 `game_world_core::new_game_state_history_item` · 0x2D0B30 `event_horizon_history_item` · 0x2D1070 `is_there_event_horizon_item` · 0x2D2E40 `game_world_core::move` · 0x7AB30 `mutable_buffer::resize` · 0x7AC10 `buffer_writer::w(src, n)` · 0x424CA0 `_alloca_probe_16` · 0x1C210 CRC-32 table init · 0x3147A0 / 0x3148A0 / 0x3149B0 / 0x314B90 `match_time_vi_spawn_rule_core` ctor / tick / serialize / deserialize.

## 6. A patch revision in the sign-in (added 2026-10-12) — PROVEN bytes, run with a client

A server cannot tell one patched exe from another: nothing a client sends depends on a patch. What a client says of its build is the 12-byte version field of the first login message and nothing else (login_chat.md §4.1 and §7.8; no message has a hash of the exe).

- **PROVEN, the field** (`sign_in_on_connected`, 0x380847..0x38088C): eight zero bytes on the stack (`pxor` / `movq [esp+20h]`), `"0.26"` at +0, `"g0"` at +4, the string's own zero at +6 (`mov cl,[9DAE9Eh]` 0x380862, `mov [esp+26h],cl` 0x38086C) and a zero again at +6 (`mov [esp+26h],al` 0x38087B, `eax` being 0); then the eight bytes and four more zero bytes (`mov [edi+8],eax`) go into the message. Bytes 7..11 of the field are always 0.
- **PROVEN, the server:** `parse_sign_in` takes the field up to its first zero, so what stands after it is not read by any server built from this repository so far.
- **The patch** (`REVISION_SITE`, full builds only; `--cert-only` keeps the code): the six bytes at 0x380862, `8A 0D 9E AE 9D 00` (`mov cl,[9DAE9Eh]`), become `C6 44 24 27 NN 90` (`mov byte [esp+27h],NN` / `nop`): byte 7 of the field is the revision NN. Byte 6 still gets its zero at 0x38087B (0x38086C then writes the `'0'` left in `cl` there first, which is overwritten). The base relocation at 0x380864 (HIGHLOW; the three of this code are at 0x380849, 0x380850, 0x380864) is made a no-op as the hooks' are. The string, the title and replays are untouched. NN is `REVISION` of `patch_026g.py`. It is raised in the change that makes a full build something else for the game: **1** is the patch set of this change (the VRAM check of §3.7 and this byte beside what the download of 2026-10-12 has). 0 is every exe without this patch: the download of 2026-10-12 and anything built before.
- **Checked** (`selftest_026g.py` check 11): the field is worked out from the instructions that fill it (the bytes of the string are read from the image): `0.26g0` and six zero bytes in the original and in cert-only; `0.26g0`, a zero, the revision in byte 7 and four zero bytes in the full builds, the EULA flag following as before. Of the three relocations only the one at 0x380864 is disabled; no branch of the function lands inside the instruction.
- **Why nothing else changes** (read in the exe, 2026-10-12): the replaced instruction only loaded `cl`, which `mov ecx,[ebp+0Ch]` (0x380870) overwrites before any other use; neither it nor its replacement touches a flag. The eight stack bytes are read once, by the `movq` at 0x38087F, and nothing of the function reads them after that (to its `ret` at 0x380930 it builds the send callback above them and sends). The message's length does not change. The four references to the version string are this code, the title, the `%s (%d)` text and the replay check (login_chat.md §7.8); the patch touches none of the string's bytes. Nothing of it is in the world a match computes, so a patched and an unpatched client play one match as two unpatched ones do.
- **Builds:** section sha256 `3c7d7bf0195db000f0c6dc81e57fa15a50decec4fa0c0ea5494f361f91049c9d`, hash-mismatch `9f7f8b65203df7abf9720a174ad6782db42a5e42517430ec700a0c97f4707e91`; 48 bytes in 15 ranges and 1207 in 58 differ from the original. The section build differs from the exe of the first download of 2026-10-12 (`166d25bc…1ddf`) in the VRAM byte (file 0xF5811), these six bytes (file 0x36FC62), one relocation entry (file 0xB86623, `38` → `08`) and the PE checksum, and in nothing else (compared byte by byte).
- **The server** (2026-10-12): `parse_sign_in` reads byte 7 of a whole 12-byte field whose text ends before it (`patch_revision`; 0 otherwise), and the login server writes it in the sign-in's log line: `Client version: 0.26g0, patch revision 1 (…)`. Nothing else is done with it yet (the owner: "only log at first"). Tests: `a_patched_0_26g_client_says_its_patch_revision`, `a_client_that_says_a_patch_revision_signs_in_as_any_other`.
- **Run with a client** (2026-10-12, a local server and one game with the section build): the sign-in was logged as `Client version: 0.26g0, patch revision 1`, a client that sends the stock field (the test bot) as `patch revision 0`; the game went through the lobby and a match. The players' download of 026g.skhid.link has this exe since 2026-10-12 18:43 UTC (`Survarium-0.26g-srp-files.zip`, sha256 `16eb1f23c99d5d73c5228c168021c58adff5e0ebf3d543984f916b416cef9528`); the first download's exe says nothing, so 0.
- **What it gives:** the version text stays `0.26g0`, so a server that knows nothing of the byte takes the new exe as it takes the old, and a server that reads it sees 0 from every exe given out so far. 
- **What a server can do with it beyond the log:** show it on the admin page; refuse a sign-in under a set minimum with the answer 0x22, for which the client shows its own `st_login_menu_status_invalid_version`, "You're using an outdated version of the game." (PROVEN: 0x22 → 0x21ACE5 → `set_status(9)` 0x21AD8D → the text key 0xA0005C at 0x21F0F3; the server has sent 0x22 to two sign-ins of another build, 0.26e0, in the kept logs); or, in a match of both kinds whose worlds part, take the newer one's world.
- **What it is not:** proof. The byte is what the exe says of itself; a changed exe can say anything.
- **Not taken:** changing the string itself (`0.26g1`): one byte, no code, but the title and the replay check change with it, and every server not yet updated refuses the new exe. A hash of the exe sent by the client: new code in the exe for no more than the byte gives.

## Open questions

1. Did 0.26g's original server still supply 0xE8 itself (lockstep with a server-mode core), as 0.21d's did? The client code says so (frozen world, no upload), but the new 0xE5 / 0xE6 sequence bytes and the 0xDB timing exchange are not understood yet.
2. Compression of 0xE8 / 0xE4: which codec sits at `[nc+0x4C]` (vtbl+4 = decompress)? srp can avoid it for 0xE8 with flag byte 0; check whether 0xE4 (static info) has the same flag.
3. C→S 0xDC / 0xDD / 0xDE and S→C 0xEB–0xF5 are not mapped (input, input relay, discard, …). 0xDF carries the item time as one byte (time / 50): how should srp key its CRC table by it?
4. Answered (transport.md §4, §3.2 here): 0xE0 is reliable and plain; 0xF6 is unique (not ordered), variable length, and **needs at least one payload byte**.
5. Live checks, as for 0.21d: identical initial uploads from two clients; a 30–69 packet burst of 0xE0 arriving complete (statically safe, §3.2); 0xF6 `00` reaching the hook in state 2; the client starting at all with an 8th section (§3.3; fall back to `--cave=hash-mismatch` if not).
6. Mode 3 in 0.26g: confirm live that batteries stay without a patch.
7. Function map: game-code coverage is only about 30 %. More could come from matching the recompiled structure with constants and struct offsets masked, or from 0.26g's own `__FUNCSIG__` / log strings (244 FUNCSIG strings name 0.26g functions directly, including ones 0.21d lacks).

## 0. Code-signing certificate check: required for every 0.26g client (found 2026-10-12, PROVEN live)

A stock 0.26g client stops at start with "Resources archive is corrupted.
Please reinstall the application and try again" (string 0x9DEF00, pushed at
0x432E1 in `vostok::vfs::archive_mounter::mount_archive_to_parent`).
- The mounter reads the resources.db file table plus a 0x100-byte signature
  (size from the function 0x12A00) and calls the verifier installed at start
  (boost::function at 0x21E5158 = 0x13310, `vostok::crypt::verify_sign`).
- 0x13310 takes the exe's own Authenticode store (`GetModuleFileNameW`,
  `CryptQueryObject`), finds the certificate with subject "survarium.com"
  (`CertFindCertificateInStore`, CERT_FIND_SUBJECT_STR), then calls 0x12A10.
- 0x12A10 builds the certificate's chain for **now** (`CertGetCertificateChain`,
  usage OID 1.3.6.1.5.5.7.3.3 code signing) and fails on
  CERT_TRUST_IS_NOT_TIME_VALID (returns 0xA), revoked (0xB) or a base-policy
  error (3). The certificate (C=UA, O=Vostok Games. Survarium Inc.,
  CN=survarium.com, issuer Certum Code Signing CA) was valid 2014-06-06 to
  **2015-06-06**, so the chain check always fails today.
- Patch: 0x12A10 `83 EC 2C` -> `31 C0 C3` (`xor eax,eax; ret`; cdecl, no
  relocations in those bytes). The file table signature is still verified
  with the certificate's key afterwards. Live: the patched client gets past
  the check and opens its window.
- Ruled out: a damaged unpack (resources.db CRC 0x4A01F332 equals the 7z
  record) and a half-applied update (the leftover
  `temp/patch_temp/resources.db.patch` names target size 6,285,533,906 =
  our resources.db).
- 0.21d has `verify_sign` too, but its clients start; not checked why.
