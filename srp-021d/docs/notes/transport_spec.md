# 0.21d match UDP transport: byte-exact spec for the server

Scope: Survarium 0.21d client, `vostok::network_core::udp_match_connection` / `udp_match_client`, read from `v021d_disasm/survarium_disasm.txt`.
- Addresses are VAs, image base 0x10000.
- `conn` means `udp_match_connection` (it is the base of `udp_match_client`). `pkt` means `udp_match_packet`.
- Tags: **[V]** = read in the disassembly, at the address given. **[I]** = inferred.
- This file supersedes match.md §1 where they differ. The corrections are listed in §12.

All integers are little-endian. Sequence numbers and order ids are `u16` and wrap. Comparisons use half-range ordering: `a` is newer than `b` iff `0 < (a-b) mod 65536 < 0x8000`. The functions are `??M` 0x218AD3 (newer), `??N` 0x167D11 (newer or equal) and `operator-` 0x167CCF (signed `a-b`). **[V]**

---

## 1. Datagram header (12 bytes)

| off | type | field | writer: `fill_packet_header` 0x28B887 | reader: `process_incoming_packet` 0x28797D |
|---|---|---|---|---|
| 0 | u16 | `local_seq` | the sender's sequence id of this datagram: the newest entry of the pkt+0x54 ring (0x28B8B3) | `s` (0x2879BB) |
| 2 | u16 | `remote_seq` | the newest peer sequence received, `conn+0x678F2` (0x28B8BB) | `r` (0x2879C7) |
| 4 | u64 | `ack_multi` | `(recv_bits << 1) \| multi`, where `recv_bits = conn+0x678E0` (0x28B98E–0x28B9B2) | `acks = (ack_multi >> 1) \| 1<<63` (0x2879F2–0x287A03). `multi = ack_multi & 1` (0x287B94) |

**Multi flag = bit 0** of the u64, which is bit 0 of byte 4. **[V]**
- 0: the rest of the datagram is exactly one message (the "single" format).
- 1: the rest is a list of length-prefixed messages (§2).
- On send, the flag is taken from a marker byte, `data[0] == 1`, set by `send_packets_list` 0x28BC48 and `new_low_level_packet` 0x28BFC9. It is written into the header just before sending.

**Ack bit mapping** **[V]**
- The receiver's state `recv_bits` (`conn+0x678E0`) uses this layout: bit 63 = `R` (the newest received), bit 62 = `R-1`, …, bit 0 = `R-63`. It is updated at 0x28DA02–0x28DA3B: on a newer `s`, `bits >>= (s-R)` (cleared if the gap is ≥ 64), then `bits |= 1<<63` and `R = s`. On an older `s`, `bits |= 1<<(63-(R-s))` (0x28D93C–0x28D9FD).
- On the wire, bit 63 (`R` itself) is shifted out and is implied by `remote_seq`. **Wire bit j (1..63) means "I received `remote_seq − (64−j)`"**: wire bit 63 = `remote_seq−1` and wire bit 1 = `remote_seq−63`.
- Encoder for the server: `ack_multi = multi | Σ_{d=1..63} [received(R−d)] << (64−d)`.
- The receiver puts `remote_seq` back as bit 63, so every datagram acks its own `remote_seq`.

### 1.1 What the client does with an incoming header (`update_acknowledgements` 0x28D884, called at 0x287AC0)

The client stores the following state:
- `R` = `conn+0x678F2`, the newest server seq received. Initial value 0xFFFF.
- `A` = `conn+0x678F4`, the newest of its own seqs that the server has acked. Initial value 0xFFFF.
- `acked_bits` = `conn+0x678E8`, in the same layout as `recv_bits`.
- `L` = `conn+0x678F0`, its last sent seq. Initial value 0xFFFF; its first datagram is seq 0.

It applies these steps in order:

1. `conn+0x678C0 = now` (time of last receive) is set **for every datagram that passes the source filter**, duplicates included (0x287992–0x2879A0). **[V]**
2. **Duplicate check on `s`:**
   - `s == R` → the datagram is dropped. **[V]** 0x28D89E
   - `s` older than `R` by 1..63 with its bit already set → dropped. **[V]** 0x28D96F–0x28D97F
   - `s` older than `R` by ≥ 64 → **accepted** (no bit to check). **[V]** 0x28D945–0x28D95D
   - **Consequence: the server's first datagram must not use seq 0xFFFF.** It would equal the initial `R` and be dropped. **[V]**
3. **Ack processing.** A failure here does not drop the datagram; its messages are still processed.
   - If `r` is newer than `L` (an ack for a seq never sent): the acks are ignored ("skipping very old sequence", 0x28DA3E–0x28DA4E). **[V]**
   - If `r == A`: **nothing is processed**, even if the bits carry new acks (0x28DB0C). **[V]**
   - If `r` is older than `A`: the newly acked bits, `acks XOR (acked_bits << (A−r))`, are processed. Only `r`'s own bit is added to `acked_bits` (0x28DB01–0x28DBF1). **[V]**
   - If `r` is newer than `A`: let `shifted = acked_bits >> (r−A)` (0 if the gap is ≥ 64).
     - If `(shifted & acks) != shifted`, it logs **"inconsistency in acknowledgements"** and ignores the whole ack (0x28DC3F–0x28DCD3). **[V]**
     - Otherwise it sets `acked_bits = acks` and `A = r`, then processes `acks XOR shifted` (0x28DCFE–0x28DD28). **[V]**
   - **Server rule:** never stop reporting a received seq that is still inside the 64-window. The ack bitmap must stay monotonic.
     - If you reset it, every later ack is rejected until `remote_seq` jumps by ≥ 64.
     - Once the acks stop, unacked messages pile up and the client hits the 4096 limit (§4.4).
4. `acknowledge_packets(r, bits)` 0x28D637 walks the bits from bit 63 (= `r`) downward. For each set bit it deletes every unacked reliable packet whose **last 8 sequence ids** (ring at pkt+0x54, 9 slots, 8 usable; `push_back` 0x28E778) contain that seq (`sequence_id_predicate` 0x28D34E). **[V]**

---

## 2. Payload after the header

Dispatch is in `process_incoming_packet` 0x287B94–0x287D4D. **[V]**

| multi | layout | handling |
|---|---|---|
| 0 | `message` = all remaining bytes | `call_predicate(reader = rest)` 0x287C24. **No length check: a header-only datagram with multi=0 makes the client read an id byte past the end** (0x28816C). Never send one. |
| 1 | repeated `{ u8 len; if len==0: u16 len; byte[len] message }` until the end of the datagram | Loop at 0x287C45–0x287D4D. `len` counts the whole message: id, any order id, and payload. The sender writes a u8 when the length is < 256, otherwise `00` + u16 (0x28BD23–0x28BD7D). An empty multi datagram processes nothing (0x287C2E). Lengths are not bounds-checked. |

**Low-level packets.** A multi datagram whose **first** message (index 0) ends exactly at the end of the datagram is treated as low-level (0x287C7E–0x287CA4). Only the first byte of that message is read, as the type (0x28DD4E–0x28DD5F). `is_low_level_packet` 0x28B48F applies the same rule.
- So a low-level datagram is: header with multi=1, then `01 tt` (14 bytes). **[V]** `new_low_level_packet` 0x28BFA6 writes `01`, then `type`.
- **Corollary:** a multi datagram must carry ≥ 2 messages. Send a lone game message in the single format. **[V]** The client itself only builds multi datagrams when the head packet has a successor (0x28B9CB).

| type | name | the client on receive (`process_low_level_message` 0x28DD36) |
|---|---|---|
| 0 | initiate_disconnection | In state 0 (connected): goes to state 2, sets `conn+0x678D4 = now`, queues a confirm (0x28E25A–0x28E2F3). Otherwise it is logged and ignored. **[V]** |
| 1 | confirm_disconnection | In state 1 (the client initiated): `instant_disconnect(2)` (0x28DF95–0x28E009). Other states: logged and ignored. **[V]** |
| 2 | continuous_flow | Calls the optional functor at conn+0x67878 (null in the client). No reply. **[V]** 0x28DE01–0x28DE8B |
| other | — | Logged and ignored. **[V]** |

Low-level packets carry no other payload. They are unreliable: the default pkt flags are 0x3F (pkt ctor 0x1A0A87). They still consume a sequence id and carry acks. **[V]**

Every message is dispatched by `call_predicate` 0x288154: it reads `u8 id`, and `id == 0xFF` means a multipart part (§5). **[V]**

---

## 3. Message encoding, ordering and dedup

**Encoding.** `u8 id`, then a `u16 order_id` **only if the receiver's info flags for that id have bit 7 set**, then the payload. **[V]**
- Sender: `construct_packet` 0x238AAF writes the id, then the placeholder 0xFFFF if `flags & 0x80`. `enqueue_impl` 0x28CF07 patches `data+0x0D` with the channel counter.
- Receiver: 0x28819D–0x28824E.
- The receiver uses **its own table**, keyed by id (`get_received_message_info`, vtbl+4). So both sides must agree on which ids are ordered.

### 3.1 Info tables (`udp_match_message_type_info {u16 a; u8 b; u8 flags}`)

`flags`: bit 7 = ordered, bit 6 = reliable, bits 0..5 = channel. Read from the jump tables with jt.py. **[V]**

**C→S** (`get_sending_message_info` 0x1A144C, table 0x1A1494). No C→S message has an order id.

| id | a | b | flags | meaning |
|---|---|---|---|---|
| 0x40 connect, 0x41 port probe, 0x42 ready, 0x43 set-time reply, 0x47 leave | 500 | 200 | 0x7F | reliable, unordered. Resent every 300–700 ms |
| 0x45 input | 200 | 0 | 0x7F | reliable, unordered. Resent every 200 ms |
| 0x44 clock report, 0x46 hash | 0xFFFF | 0xFF | 0x3F | unreliable, sent once |

**S→C** (`get_received_message_info` 0x1A14B4, table 0x1A14F8). The server must use these same values.

| id | a | b | flags | meaning |
|---|---|---|---|---|
| 0x50 ports | 500 | 200 | 0x7F | reliable, unordered, **no order id** |
| 0x51 static info, 0x52 set time, 0x53 snapshot, 0x55 clock offset, 0x59 hash mismatch | 500 | 200 | 0xC0 | reliable, **ordered, channel 0**, u16 order id |
| 0x54 input relay, 0x56 player entered, 0x57 player left, 0x58 input discard | 200 | 0 | 0x7F | reliable, unordered, **no order id** |

- **Neither function bounds-checks the id** (`sub eax,40h/50h; jmp [table+eax*4]`). **[V]** A server message id outside 0x50..0x59, other than the multipart marker 0xFF, jumps through garbage on the client. Never send one.
- **`a` and `b` set the resend timer:** `resend_at = send_time + uniform_int[a−b, a+b]` ms (boost `uniform_smallint` with the MT19937 at conn+0, inclusive bounds). **[V]**
  - It is set at `handle_send` 0x28B5D7–0x28B616 for single sends, and at `send_packets_list` 0x28BE6A–0x28BEA7 for messages inside a joined datagram.
  - `pkt+0x40` = time sent (0x28C738), `pkt+0x44` = resend deadline.
  - `move_to_list_predicate` 0x28BEDB re-queues the packet when `now >= pkt+0x44`.
  - `{0xFFFF, 0xFF}` is never used, because unreliable packets are deleted after sending (0x28B628).

### 3.2 Ordered messages (receiver side, the client)

- **There is only one `channel` object**, `conn+0x67828` (0x2C bytes). The constructor (0x28B2ED) and destructor (`??_I(…,0x2C,1,…)` 0x28B441) handle exactly one. The index is `flags & 0x3F`, so every ordered id must use channel 0. All S→C ordered ids do. **[V]**
- Channel layout: `+0x00` = rbtree of early messages, keyed by order id. `+0x14` = rbtree of multipart messages being assembled. `+0x28 u16 received` (initial 0xFFFF). `+0x2A u16 next_send` (initial 0). (Constructor 0x28B268.) **[V]**
- **Order ids start at 0 on both sides.** The sender uses `next_send` and then increments it (0x28CF24–0x28CF4D). The receiver initialises `received` to 0xFFFF and expects `received+1 = 0`. **[V]**
- `call_predicate` 0x28824C–0x28868D, for an incoming order id `o`:
  - If `received` is newer than or equal to `o` → "skipping duplicated already processed", dropped (0x2883CD–0x2883E0).
  - If `o` is already in the early set → "skipping duplicated not yet processed", dropped (0x28851A–0x28852A).
  - Otherwise: copy it into a pool packet and call `add_ordered_packet` 0x288D1E. That inserts it into the set, then delivers while `set.first.order == received+1`, incrementing `received` each time (0x288F3D–0x289172). **[V]**
- **Gaps:** there is no timeout and no skip. A missing order id blocks every later ordered message until it arrives. Every early message holds one client pool packet (§4.5). **[V]**
- One counter serves 0x51/52/53/55/59 together, including the order id consumed by a multipart message.

### 3.3 Unordered messages (reliable or not)

- **Delivered immediately. The transport does no dedup** (0x2881A1–0x288242). **[V]** A resent unordered message is processed again.
- **Client side, for S→C 0x50/54/56/57/58:** the game layer must tolerate duplicates. **[I]**
  - `change_input`, `enter_match` and `discard_input` return code 1 for a duplicate (match.md §2).
  - 0x50 is only honoured in network state 0 (match.md).
- **Server side:** the client resends 0x40–0x43, 0x45 and 0x47 until they are acked, each time with a new seq. Datagram-level dedup does not catch these copies. **The server must dedup at application level:**
  - 0x40/0x41/0x42: idempotent, per connection.
  - 0x43: by its echo value.
  - 0x45: by `(player, tick_time_ms)`. Inputs can also arrive out of order.
  - 0x47: once.

---

## 4. Reliability, acks, timers (client behaviour)

### 4.1 Sending loop (`send_queued_packets` 0x28C0BF; `now` = the caller's ms timer)

1. `conn+0x678CC = now`. A switch on `state = conn+0x678DC`; the states are 3 = idle/not connected (ctor 0x28B3A4), 0 = connected, 1 = client-initiated disconnect, 2 = peer-initiated disconnect.
   - **State 0:** if `last_recv (678C0) != 0` and `now >= last_recv + timeout (678C4)` → "client disconnected by timeout", `instant_disconnect(0)` (0x28C31D–0x28C4DB). **[V]**
     - **The timeout is 5000 ms, or 120000 ms when a debugger is attached** (udp_match_client ctor 0x2863A8–0x2863C7). **[V]**
     - `connect()` sets `last_recv = now` (0x28CEED–0x28CEF0). **So the server must send some datagram within 5 s of the client's connect, and at least every < 5 s afterwards.** **[V]**
   - **State 1:** if `now >= last_send + 100`, queue `initiate_disconnection` (0x28C238–0x28C25A). There is no transport-level timeout in this state. **[V]**
   - **State 2:** while `now < 678D4 + 500`, queue `confirm` every 100 ms. After 500 ms, `instant_disconnect(2)` (0x28C190–0x28C236). **[V]**
2. If the unacked list `conn+0x67818` holds ≥ `max_unacknowledged_packets_count` packets (global 0x8DD970 = **0x1000 = 4096**, command-line key `max_unacknowledged_packets_count`, initialised at 0x692C24) → "has been disconnected by too many unacknowledged packets", `instant_disconnect(1)` (0x28C25F–0x28C318). **[V]**
3. Unacked packets whose deadline has passed are moved back to the send queue `677F8` (0x28C50D). **[V]**
4. If the send queue is empty and `now >= last_send (678C8) + 678D0 (=100)`, it queues a **continuous_flow** (0x28C51D–0x28C5BE); otherwise it returns. **[V]**
5. The queue is sorted by message size (`packets_predicate` 0x28EE9A) and packets are popped largest first. **The client's message order within and across datagrams is not chronological.** **[V]**
6. Per datagram: `last_send = now`, `++L`, push `L` into the head packet's ring, `pkt+0x40 = now`, `++pkt+0x78` (sends) (0x28C6FC–0x28C74E).
   - **Joining.** Smaller single packets with the same `pkt+0x74` (destination port) are joined into one multi datagram up to 0x1FC bytes. Each joined packet records the same seq in its own ring (0x28CC19–0x28CCB2).
   - Joining is on by default; it is disabled only by the command-line key `disable_joining_net_packets` (key 0x56E40F0, initialised at 0x692BFE).
   - The joined datagram is a temporary unreliable packet. Each reliable original goes to the unacked list with its own deadline (0x28BE62–0x28BEB1). **[V]**
7. **Window stop.** If the server has acked nothing for about 32 K client datagrams (`A` newer than or equal to `L+1` in half-range terms), sending stops ("can't send"; 0x28C71D–0x28C724, 0x28CD35). **[V]**

### 4.2 How often the loop runs **[V]**

- The game thread flushes in `network_client::tick` 0x1A2C0E–0x1A2C32 when `now >= last_flush + 100`. `try_send_all` 0x1A3342–0x1A3355 also flushes when the "dirty" flag is set, and `match_client::enqueue` 0x1A13C8 sets that flag. `match_client::send_queued_packets` 0x1A13DB clears it.
- The network thread's `send_heart_beat` 0x239B48 runs the loop if it has not run for 300 ms.
- **Consequence:** the client sends at least one datagram (continuous_flow when idle) every ~100–200 ms. Every outgoing datagram carries its current `remote_seq` and bits. **[I]** (The timing comes from [V] code; the frame rate is not modelled.)

### 4.3 How the client acks

- **Every non-duplicate received datagram is recorded in `recv_bits`**, before its contents are looked at (0x287AC0). This includes datagrams that carry only unreliable or low-level messages. **[V]**
- There is no separate ack packet. Acks travel in every outgoing header, so the continuous_flow is the pure ack. **[V]**

### 4.4 What the server must do so the client keeps running

1. **Receive and ack.** Keep a 64-bit received bitmap of client seqs, kept monotonic (§1.1 step 3).
   - Put `remote_seq` = the newest client seq and the bitmap in every datagram.
   - Send **at least one datagram every ≤ ~100–200 ms**, a continuous_flow if there is nothing else. Then the client's reliable messages are acked within a few hundred ms, and `last_recv` never ages to 5 s.
   - The client sends ~10–30 datagrams/s, so a 100 ms server cadence stays far inside the 64-datagram window. **[I]**
2. **Count to stay under.** The client deletes an acked message only when its unacked list `67818` drops it.
   - That list holds every reliable client message not yet acked, about 20/s from 0x45 alone.
   - Without acks the client hits 4096 after a few minutes (`instant_disconnect(1)`); the 4096-packet pool (§4.5) may run out first.
3. **Ack quickly.** A message is acked only through one of the **last 8 seqs** it was sent under. With the 200 ms resend of 0x45, that covers about 1.6 s of resends. An ack that names an older seq is ignored. **[V]**
4. **Advance `remote_seq` to add acks.** A late extra ack bit is ignored when `remote_seq` has not advanced (§1.1 step 3, `r == A`). This only costs resends, not correctness. **[V]**
5. **Resend reliable S→C messages yourself.** The client never asks for anything again: there are no NACKs. **[V]**
   - Use the §3.1 timings: 0x50 and the ordered ids every 300–700 ms; 0x54/56/57/58 every 200 ms. **[I]** Any timing works.
   - Use a new datagram seq for every resend, because a reused seq equal to `R` is dropped.

### 4.5 Client buffers

- **One shared packet pool of 4096 packets.** It is `single_size_buffer_allocator<0x2AC>` over 0x2AC000 bytes, built in the `match_client_impl` ctor at 0x241D7E. **[V]**
- These all draw from it:
  - its queued, unacked and in-flight sends;
  - every **received ordered message waiting behind a gap** (0x28864A);
  - every **multipart part** until its message is delivered (0x288ACC). **[V]**
- Delivered messages are passed to the game thread through a separate 4096-packet response allocator (`udp_match_fixed_packets_allocator<0x1000>`, 0x2391A3). **[V]**
- If the pool runs out, the client crashes (0.100b experience, see srp `transport.rs`). **[I]**
- **Server rule:** do not run far ahead of an unacked gap in the ordered channel.

---

## 5. Multipart (0xFF) messages

### Sender (the client's `enqueue(id, reader, size)` 0x28D0B9 and `construct_multipacket_part` 0x28CFC1; mirror this)

- **When to split:** split when `(ordered ? 2 : 0) + payload + 13 > 0x1FC` (0x28D107–0x28D128). So a single message holds at most 493 payload bytes if ordered, 495 if unordered. **[V]**
- **Order id:** one order id for the whole message, taken from channel `flags&0x3F`, the same counter as single ordered messages (0x28D188–0x28D1C5). **[V]**
- **Part 0:** `FF, u8 id, u16 order_id, u8 part_index=0, u8 part_count`, then data. The count is written after the other parts are built (0x28D2D4–0x28D2F0). Data length is `0x1FB − 17 = 490`, which fills the datagram to 508. **[V]**
- **Part k ≥ 1:** `FF, u8 id, u16 order_id, u8 part_index=k`, then data. Data length is `min(0x1FC − 17, rest) = min(491, rest)` (0x28D246–0x28D2B4). **[V]**
- **Data:** the data is **only the payload**. The id and order id are not repeated inside it; the receiver takes them from the part header. **[V]**
- **Reliability:** each part is its own reliable packet, with the id's flags, timer and resend. **[V]** For the server, every part must be acked or resent independently.

### Receiver (`process_multipacket_message` 0x288717)

1. It reads `id` and calls `get_received_message_info(id)` (so the id must be in 0x50..0x59). It then reads `u16 order`, and the channel is `flags & 0x3F`. **The ordered bit is not checked:** every multipart message goes through the ordered channel. **Only ids whose flags have channel 0 can be multiparted: 0x51/52/53/55/59.** An unordered id (channel 63) would index past the single channel. **[V]**
2. It drops the part as a duplicate if `received` is newer than or equal to `order` (0x288804–0x28880D), or if `order` is in the early set (0x2888BE–0x2888C8). **[V]**
3. It reads the `u8 part_index`. It looks up the assembly for `order` in channel+0x14. If this part index is already there, the part is a duplicate and is dropped (0x2889D5–0x288A37). **[V]**
4. It makes a packet for the part: +0x10 = index, +0x72 = id, +0x70 = order.
   - If no assembly exists yet, this part becomes the assembly head. The head's `+0x73 part_count` starts at **0xFF** (pkt ctor 0x1A0A83).
   - If `part_index == 0`, it reads `u8 part_count` into the head's +0x73 (0x288BA6–0x288BC6). **[V]**
5. It copies the rest of the part's bytes, inserts the part into the head's part set (sorted by index), and checks completion with `parts_in_set >= part_count` (0x288C7A–0x288C8F).
   - On completion it removes the assembly and calls `add_ordered_packet`.
   - On delivery, `call_predicate_and_destroy_packet` 0x289BD8 concatenates every part's data **in part-index order** into one buffer and calls the handler with `(id, reader)`. **[V]**
6. **Parts may arrive in any order, interleaved with other messages, alone or joined into multi datagrams.** **[V]**
   - The whole message is **ordered and reliable**: it waits for every earlier order id and holds up every later one.
   - Limits: 1–255 parts (u8 count; index 0xFF is the "not a part" marker, so use indices 0..254). Each part's data must fit the receive datagram, ≤ 491 B. The total is at most about 125 KB. **[V]**
7. The server may choose smaller parts. The client concatenates whatever sizes it receives. **[I]** The code takes each part's length from the datagram or the multi length, so this follows. Matching the client's 490/491 layout is safest.

---

## 6. Connection setup, ports, teardown

### Client setup (`udp_match_client::connect_impl` 0x286B1F)

- It opens a UDP socket bound to `0.0.0.0:0` (an ephemeral source port) and sets SO_RCVBUF/SO_SNDBUF to 0x20000. **[V]**
- The server address comes from `address::from_string(host)` (0x286E77). **The lobby must give a numeric IPv4 address**; anything else throws. **[V]**
- Source filter: `[683F8,683FA] = [port,port]`. Random-port range: `67A1C = {port,port}`. **[V]** 0x286DD4–0x286E09
- `udp_match_connection::connect` 0x28CE00 sets state 0 and `last_recv = now`, then queues the 0x40 packet. Receiving starts and the packet is sent at once (0x286E26–0x286E42). **[V]**
- **No low-level handshake exists.** The client is "connected" at once and needs no reply before game messages. The game layer waits for 0x50 (match.md §2). **[V]**

### First client datagram

- It is `local_seq 0`, `remote_seq 0xFFFF`, `ack_multi 0` (single), followed by `40 | u32 session | u8 g_debug_hash_mismatches` (`match_client::connect` 0x1A1354–0x1A1377). **[V]**
- If it is lost, the next datagram may be a continuous_flow (seq 1), sent after 100 ms idle, and then the 0x40 resend (new seq, after 300–700 ms). **The server must accept the connection on whatever datagram arrives first.** It must not insist that seq 0 carries 0x40. **[I]** (This follows from §4.1.)

### Receive filter (`handle_receive` 0x2868A5–0x286916)

- A datagram is processed only if its **source IP equals the server IP** and its **source port equals the connect port or lies in `[min,max]`**. Anything else is logged as "unexpected sender" and dropped. **[V]**
- **The server must reply from the IP and port the client connected to.** Watch out on multi-homed hosts bound to 0.0.0.0.

### 0x50 `{u16 min, u16 max}` (`udp_match_client::set_connection_ports` 0x287002)

- It sets the source filter range to `[min,max]` (the connect port stays valid) and the random range `67A1C = {min,max}`. **[V]**
- If `min <= max`, it builds one **0x41 `{u32 session}` per port p in [min,max]**, each with `pkt+0x74 = p` (a fixed destination), and queues and sends them all. Each one is reliable and is resent to the same port. **[V]**
- Every later packet with `pkt+0x74 == 0xFFFF` (all game messages and all low-level packets; `conn+0x678F8` is never changed from 0xFFFF) goes to a **uniformly random port in [min,max]** (`send_packet` 0x2874B4–0x2874DE). **[V]**
- **`min == max == connect port`:** exactly one 0x41 goes to the connect port, and all traffic stays on one port. This is the recommended setting. **[V]**
- `min > max` sends no 0x41 at all (0x287040). **[V]**

### Teardown

- **Client leaves** (`disconnect` 0x28E563): state 1. **All its queued and unacked packets are dropped** (0x28E686–0x28E702). It then sends `initiate (01 00)` immediately and every 100 ms until it receives `confirm`, which triggers `instant_disconnect(2)`. **[V]** Server: answer every initiate with a confirm `01 01`.
  - Game messages sent just before this, such as 0x47, may be lost if they were not yet acked. **[I]**
- **Server kicks:** send `initiate (01 00)`. The client goes to state 2, sends `confirm` at once and every 100 ms, discards new outgoing messages (`enqueue` needs state 0, 0x28CF82), and disconnects itself after **500 ms** (event 2). **[V]**
- `instant_disconnect` 0x28E2FF resets every sequence to 0xFFFF and clears both channel trees and the counters. **[V]** A reconnect therefore starts again from seq 0 and order id 0.

---

## 7. Size limits

- **Client receive buffer: 0x1FC = 508 bytes** (`start_receiving` 0x286B05). **Every server datagram must be ≤ 508 bytes, header included.** **[V]**
  - A larger datagram fails the read with an error (WSAEMSGSIZE). `on_error` 0x2864D9 only re-arms the receive (it stops only on 0x3E3 or 10061), so **the datagram is silently lost**. The resend is lost the same way, and the ordered channel stalls for good. **[V]** (WSAEMSGSIZE as the cause is **[I]**.)
- A zero-length datagram goes to the "unable to read" path and is ignored (0x28681B). **[V]** Never send datagrams shorter than 12 bytes.
- **Packet buffer:** `pkt` data starts at +0x7B, capacity 0x208, size starts at 12 (pkt ctor 0x1A0A8B). The writer has a null allocator, so writing past the capacity crashes (option_b.md).
  - On receive, an ordered message or a part copies "rest of reader" into such a buffer starting at offset 12, so up to 508 bytes. Any datagram ≤ 508 is safe. **[V]**
- **Single message size:** `id + [order] + payload ≤ 496` (508 − 12).
- **Multi:** `Σ(1 or 3 + len) ≤ 496`.

---

## 8. Per-packet fields (for further RE)

| off | meaning |
|---|---|
| +0x10 | u8 part index (0xFF = not a part) |
| +0x24 / +0x28.. | part set (count, rbtree header) |
| +0x3C | intrusive list next |
| +0x40 | u32 time sent |
| +0x44 | u32 resend deadline |
| +0x54 | ring of the last 8 seq ids (9 slots; head at +0x68, tail at +0x6C) |
| +0x70 | u16 order id |
| +0x72 | u8 message id (low-level types use type−6: FA, FB, FC) |
| +0x73 | u8 part count (initial 0xFF) |
| +0x74 | u16 destination port (0xFFFF = random in range) |
| +0x76 | u16 port actually used |
| +0x78 | u8 send count |
| +0x79 | u8 info flags (initial 0x3F) |
| +0x7A | debug duplicate flag |
| +0x7B | data, capacity 0x208 |
| +0x288 | data pointer |
| +0x290 | size |
| +0x294 | writer |

Connection fields:

| field | meaning |
|---|---|
| 677F8 | send queue |
| 67808 | in-flight list (awaiting socket completion) |
| 67818 | unacked list |
| 67828 | channel[1] |
| 678C0 | last receive |
| 678C4 | timeout |
| 678C8 | last send |
| 678CC | now |
| 678D0 | 100, the idle period |
| 678D4 | disconnect start |
| 678DC | state |
| 678E0 | recv_bits |
| 678E8 | acked_bits |
| 678F0 | L |
| 678F2 | R |
| 678F4 | A |

---

## 9. Server checklist (derived)

**Per client, keyed by source `ip:port`:**
- `next_seq`: start at 0, never use 0xFFFF as the first value.
- `R_client` plus a 64-bit received bitmap.
- `order_next = 0` for channel 0.
- An unacked map of sent reliable messages, each with the last ≤ 8 seqs it went out under (or simply every seq) and a deadline.
- App-level dedup of client reliable messages (§3.3).

**Receive:**
1. Parse the header.
2. Duplicate seq → drop the datagram (it still counts as liveness).
3. Process the client's acks of server seqs.
4. multi=1: if the first message fills the datagram, it is low-level; otherwise iterate the u8/u16-length list.
5. Messages: `u8 id` + payload. C→S never carries an order id. Accept only ids 0x40..0x47. 0xFF would be a client multipart, which never occurs.

**Send:**
- Datagrams ≤ 508 bytes.
- Single format for one message; multi for ≥ 2.
- Ordered S→C ids get `u16 order_id` from `order_next++`. A multipart message takes one order id.
- Resend reliable messages until they are acked, each time under a new seq.
- Send something at least every ~100 ms. A continuous_flow is `header(multi=1) 01 02`.
- Never send a header-only datagram with multi=0.
- Never send ids outside 0x50..0x59 or 0xFF.

**Teardown:**
- Answer `01 00` with `01 01`.
- To kick: send `01 00`. The client confirms and is gone after 500 ms.

---

## 10. Worked bytes

### (a) Client's first datagram (0x40 connect)

Session 0x0001E240, debug flag 0.

```
00 00                    local_seq   = 0
FF FF                    remote_seq  = 0xFFFF (nothing received yet)
00 00 00 00 00 00 00 00  ack_multi   = 0 (no acks, single)
40                       id 0x40 (C->S, unordered: no order id)
40 E2 01 00              u32 session
00                       bool g_debug_hash_mismatches
```
18 bytes.

### (b) Server datagram carrying 0x51 part 0 (ordered, order id 0)

- 0x51 payload: 12000 bytes, split as 490 + 23×491 + 217, so 25 parts.
- The server's earlier datagrams were seq 0 and 1. This one is seq 2.
- The client's datagrams seq 0, 1 and 2 were all received, so `R=2` and seqs 1 and 0 are set (wire bits 63, 62).

```
02 00                    local_seq   = 2
02 00                    remote_seq  = 2
00 00 00 00 00 00 00 C0  ack_multi   = 0xC000000000000000 (acks 1,0; multi=0)
FF                       multipart marker
51                       message id
00 00                    u16 order_id = 0 (first ordered S->C message)
00                       part_index = 0
19                       part_count = 25 (part 0 only)
<490 bytes>              payload[0..490)
```
508 bytes in all.
- Part 1 (seq 3): `03 00 02 00 00 00 00 00 00 00 00 C0 FF 51 00 00 01` + payload[490..981), 508 bytes.
- Part 24: `… FF 51 00 00 18` + the last 217 bytes, 234 bytes.
- The next ordered message (0x52, 0x53, 0x55 …) uses order id 1.

### (c) Server datagram carrying two small messages (multi)

The two messages are 0x54, unordered, and 0x55, ordered with order id 3.
- Server seq 30. The newest client seq is 20, and 19..13 were also received (wire bits 63..57).
- The 0x54 payload follows match.md: `u8 player=1, f32 dx=0, f32 dy=0, u32 actions=4, u32 time=12345`.

```
1E 00                    local_seq  = 30
14 00                    remote_seq = 20
01 00 00 00 00 00 00 FE  ack_multi  = 0xFE00000000000001 (acks 19..13; multi=1)
12                       len = 18
54 01 00 00 00 00 00 00 00 00 04 00 00 00 39 30 00 00
05                       len = 5
55 03 00 FB FF           id 0x55, u16 order_id = 3, i16 offset = -5
```
37 bytes.
- The first message does not end at the datagram's end, so the client does not treat the datagram as low-level.
- A message of 256 bytes or more would take the length prefix `00 LL HH`.

### (d) Server keepalive / pure ack (continuous_flow)

```
1F 00 14 00 01 00 00 00 00 00 00 FE  01 02
```
14 bytes: multi=1, then one 1-byte message that fills the datagram, type 2.

---

## 11. Open questions

1. **Game-thread tolerance of duplicate unordered S→C messages.** This covers 0x50 resent after a lost ack, and 0x56/0x57 resent. Not checked beyond the return codes in match.md. **[I]**
2. **The exact client flush cadence in a match.** `try_send_all` with its 100 ms / dirty flag is per frame. Only the minimum spacing matters for the server.
3. **Pool exhaustion behaviour in 0.21d.** We assume a crash, as in 0.100b; it was not traced.
4. **Smaller-than-490 multipart parts from the server.** The code accepts any part length; not tested live.

## 12. Corrections and additions to match.md §1

- `{a,b}` are **verified**: the resend delay is `uniform[a−b, a+b]` ms (§3.1).
- There is only **one channel object**. Channel bits other than 0 are unusable for ordered or multipart messages.
- **Unordered messages are not deduplicated** by the transport, in either direction.
- The packet pool is **4096** (0x241D7E). It is shared by sends, unacked messages, ordered messages buffered behind a gap, and multipart parts.
- The receive timeout is **5000 ms** (120000 ms under a debugger). It counts from `connect()` and is refreshed by every datagram that passes the source filter.
- **Source filter:** the source IP must match, and the source port must be the connect port or in `[min,max]`.
- **Ack-acceptance quirks:**
  - `r == A` is ignored.
  - "Inconsistency" rejects the whole ack.
  - A message is acked only through its last 8 seqs.
- **Unchecked ids:** message ids are not range-checked (jump tables), and multi lengths are not bounds-checked.
- **No empty datagrams:** a header-only datagram with multi=0 makes the client read past the end.
- **Merged order:** the client joins packets by default (`disable_joining_net_packets` turns it off), largest first, so C→S message order is not chronological.
