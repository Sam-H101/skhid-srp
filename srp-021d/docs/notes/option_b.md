# Option B: the client patch that uploads snapshots

Scope: `survarium.exe` 0.21d (build 2010). Addresses are VAs, image base 0x10000. The image has no ASLR (DLL characteristics 0x8000 only), so absolute addresses are safe to patch in.
To turn a `.text` VA into a file offset, subtract 0x10C00.
Tags: **[V]** = read in the disassembly; **[I]** = inferred; **[L]** = seen live.
Background: match.md (protocol) and option_c.md (why B).

## Summary

- The patch stays inside the client's existing send pattern: `new_packet(id)` → `buffer_writer::w` → `enqueue`, the same calls that send 0x46 every 100 ms. It adds no new thread, allocator or transport code.
- The snapshot travels as ordinary 0x47 messages of at most 480 bytes each. srp reassembles them.
  - 0x47 is reliable and unordered (`{500,200,0x7F}`).
  - A real "leave" 0x47 carries exactly 4 bytes, so srp tells the two apart by length.
- There are two triggers:
  1. **First sync:** `on_match_ready`, right after the client builds its initial world and before it sends 0x42.
  2. **On request:** srp sends 0x59. A client without `-debug_hash_mismatches` ignores 0x59, so the hook takes over that ignored branch.
- The code goes into the 501 zero bytes after `.text` (VA 0x6A1E0B–0x6A1FFF, file 0x69120B–0x6913FF). These bytes are mapped executable. The patch needs about 200 bytes. **[V]**
- Single-instance bypass: 0x1267A `75 22` → `EB 22` (file offset 0x1A7A). Applied in `binaries/x86/survarium_srp.exe`; the original `survarium.exe` is untouched. Two clients ran side by side and signed in with separate accounts. **[L]**

## Build

The patch is in `client-patches/`:
- `snapshot_upload.asm`: the cave and the two hooks, as NASM sections with `vstart` at their VAs and `align=1`. Without `align=1`, NASM pads each section to 4 bytes and the hooks shift.
- `patch.py`: checks every site's original bytes, then writes the mutex bypass, the cave and the hooks into a copy.

```
nasm -f bin snapshot_upload.asm -o snapshot_upload.bin
nasm -f bin battery_spawn.asm -o battery_spawn.bin
python patch.py <game>/binaries/x86/survarium.exe <game>/binaries/x86/survarium_srp.exe snapshot_upload.bin battery_spawn.bin
```

`battery_spawn.asm` is the mode 3 battery fix (`battery_spawn.md`). It uses the last 0x70 bytes of the same padding, so the upload cave may take 0x185.

NASM 2.16.03 is the portable zip from nasm.us, unpacked to `E:\sur-res-pro\tools\nasm-2.16.03`. The build is 361 bytes of cave plus two 6-byte hooks; 319 bytes of the exe change in all.
- Checked with `ndisasm` on the patched file: both hooks jump into the cave, and every call target and return address is right. **[V]**
- The patched copy starts, signs in and runs the lobby next to a stock client. **[L]**
- The hooks only run inside a match, so the upload itself is untested until srp has a 0.21d match server.

## Why not the client's own multipart sender

- A packet's write buffer is the `mutable_buffer` at pkt+0x284. The `udp_match_packet` constructor (0x1A0A26) sets it to `{allocator 0, data pkt+0x7B, capacity 0x208, size 0x0C}`.
- `buffer_writer::w` (0x155EBE) calls `mutable_buffer::resize` (0x155E22). Past the capacity, resize reallocates through the allocator, and here that allocator is null. **Writing more than about 500 bytes into one packet crashes.** **[V]**
- Large messages go out through `udp_match_client::enqueue(id, buffer_reader&, size)` (called at 0x23996E), which makes the 0xFF parts.
  - It runs on the network thread, from `vostok::network::enqueue_impl` 0x23988E.
  - `match_client::enqueue(pkt)` (0x23997B) only posts an `enqueue_order` to that thread.
  - enqueue_impl joins a packet's part list (pkt+0x28) into one message, but only the receive-side splitter (0x239C5F…) builds such lists. Reaching it from game code means hand-building rbtree part lists of packets.
- Splitting at application level avoids all of that and keeps the patch on the game thread. **[V]**

## The pieces the patch calls

| What | Address | Convention **[V]** |
|---|---|---|
| network_client → core | `[[nc+0x7A00]+0x138]` | |
| network_client → match_client | `[nc+0x7A08]`; `vtbl+0x18` = new_packet(id), `vtbl+0x1C` = enqueue(pkt) | thiscall: ecx = match_client, one stack argument |
| network state | `[nc+0x7A1C]`, 2 = static info received | |
| newest history item | `[core+0x2678]` (in server mode it is what `event_horizon_history_item` returns) | |
| event-horizon item | `is_there_event_horizon_item` 0x1DCBF9 (eax = pvp_match_core `[nc+0x7A00]`); `event_horizon_history_item` 0x1CAB13 (eax = core) | |
| item time | `[item+0xB9938]` | |
| players-mask history | core+0x5ED0. The head is at +8. Each node is `{+4 next, +8 mask, +0xC time}`, newest first. Take the first node with time ≤ item time. `hash()` dereferences it with no null check; the patch uses mask 0 when the list is empty | |
| serializer | `game_state_history_item::serialize(writer&, u32 mask)` 0x1FDE3E | eax = item, push mask, push &writer; callee pops 8 |
| buffer write | `buffer_writer::w(writer*, src, n)` 0x155EBE | 3 stack arguments; callee pops 12 |
| stack probe | `__alloca_probe_16` 0x2B3F00 | eax = size, as in `hash()` |
| packet writer | pkt+0x294 | |

The scratch writer is laid out exactly as in `hash()` 0x1FE180–0x1FE1AF:
- `mutable_buffer {0, data, 0x8000, 0}`;
- `buffer_writer {0, ?, 0, 0, u8 0xFF, +0x14 = &mutable_buffer}`.

Real snapshots are 11–14 KB, and `hash()` already serializes them into a 32 KB stack buffer every 100 ms.

The CRC srp checks is `hash()` ([V]): boost CRC-32 (standard: reflected 0x04C11DB7, init and final xor 0xFFFFFFFF) over exactly the bytes `serialize(mask)` writes. That is the 0x53 body without its leading `u32 time`. Every 0x46 report is `{event-horizon item time, that CRC}`.

## Hooks

1. **First sync, at 0x1A27A8** (`lea esi,[ebx+7A08h]`, 6 bytes, in `network_client::on_match_ready`; ebx = network_client).
   - Replace it with `jmp cave_ready` + `nop`.
   - `cave_ready` uploads `[core+0x2678]`, the item that `game_world_core::serialize()` just built at 0x1CA765.
   - It then replays the displaced `lea esi,[ebx+7A08h]` and jumps back to 0x1A27AE, so 0x42 is sent as usual.
   - The upload goes out before 0x42, and srp gets it before it sends 0x52, the message that makes every client `clear()` its history (0x1A2DC4).
2. **On request, at 0x1A16B1** (the 0x59 case once `g_debug_hash_mismatches == 0`; edi = network_client, ebx = reader).
   - Replace it with `jmp cave_request`.
   - `cave_request` does nothing unless state == 2, the core exists and `is_there_event_horizon_item`. Otherwise it uploads `event_horizon_history_item`.
   - Either way it jumps to 0x1A1721, which skips the rest of the payload and returns.
   - Before state 2 the original code has already skipped the payload (0x1A169E). The 0x59 payload is ignored, so srp can send it empty.
   - This branch runs on the game thread: the same dispatcher calls `change_input` on the core for 0x54. **[V]**

Upload routine, called with esi = item and edi or ebx = network_client; it preserves all registers:

```
pushad; mov ebp,esp
mov eax,8000h; call __alloca_probe_16          ; buf = esp
build mutable_buffer + writer on the stack (as hash())
mask = first players-mask node with time <= [esi+B9938h], else 0
eax = esi; push mask; push &writer; call 1FDE3Eh
size = mutable_buffer.size; time = [esi+B9938h]
for off = 0; off < size; off += 480:
    pkt = match_client->new_packet(47h)
    w(pkt+294h, &{u32 time, u16 off, u16 size}, 8)
    w(pkt+294h, buf+off, min(480, size-off))
    match_client->enqueue(pkt)
mov esp,ebp; popad; ret
```

## Wire format added to srp

- **C→S 0x47 with length > 4:** a snapshot chunk, `u32 item_time, u16 offset, u16 total, bytes`.
  - Chunks can arrive out of order, because 0x47 is unordered.
  - Once `total` bytes are present, the body is `serialize(mask)`, and the 0x53 is `u32 item_time` + body.
- **C→S 0x47 with length 4:** leave, as before.
- **S→C 0x59 with no payload:** "upload your event-horizon state". It must be **sent only to patched clients**: a stock client ignores it and only logs a hint, so it is harmless.

srp's rules:
- **First sync:** take the first complete initial upload. With several patched clients, compare their CRCs, which should be equal (live test 1). Send 0x52, then 0x53 with that item, to everyone. The 0x53 time is the item's own time.
- **Late join or resync:** ask one or more clients with 0x59. Accept the upload only when CRC(body) equals the CRC that the majority reported in 0x46 for `item_time`. Send it as 0x53, then every relayed input with time > item_time (match.md §2 step 7, §4).
- **No snapshot ever goes out unchecked** after the first sync; the first sync has nothing to check against.

## Open, to check live (two clients now fit on one PC)

1. Do two clients build byte-identical initial items? Compare the CRCs of the two first-sync uploads. If they differ, the local player index leaks into the state; diff the uploads to find where.
2. What is the initial item's time and mask (probably 0 and 0, before any 0x56)? The answer decides T0 and the 0x52 match time.
3. Does a client accept a 0x53 built from another client's initial item, and do its players spawn after 0x56?
4. Does 0x59 reach the dispatcher in state 2 with an empty payload? The stock path reads nothing before the debug check, so it should. **[I]**
5. Do about 30 reliable 0x47 packets in one burst stay under the send limits? The client's unacknowledged limit is 4096. The meaning of the `{500,200}` per-message timings is still unverified (match.md §1).

## Still needed on the srp side (option B, all new for 0.21d)

- The 0.21d transport: 12-byte header, 64-bit acks, 508-byte datagrams, per-id channel table, u16 lengths, sending 0xFF parts for 0x51 and 0x53. Use `min == max` ports.
- Handshake: 0x40/0x41/0x50, then 0x51 static info (`match_options` + N `player_profile` statics), then 0x42, 0x52/0x43 and 0x53.
- Relay: the match clock, 250 ms finality, 0x54 relay and input log, 0x58 discards, 0x55 time nudges, 0x56/0x57 roster activation.
- Snapshots: chunk reassembly, a CRC table keyed by (time, client) from 0x46, majority checks, 0x59 requests.
- Lobby: `0x33` host/port/match_id/team, and a roster fixed before anyone connects.
