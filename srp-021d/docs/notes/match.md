# What a 0.21d match server must do

Scope: Survarium 0.21d (build 2010, April 2014) match protocol, worked out from `v021d_disasm/survarium_disasm.txt`.
- All 0.21d addresses are VAs with image base 0x10000. 0.100b addresses are from `survarium_disasm.txt`.
- "Unverified" marks inference that the code does not show directly.
- Helpers: `scope021d/jt.py` reads jump tables. `scope021d/labels.txt` lists every function label with its line number in the disassembly.

**Headline**
- 0.21d is client-side deterministic lockstep with rollback. The server's jobs are:
  - relaying inputs;
  - deciding when inputs are final;
  - keeping the clocks in sync;
  - supplying full world snapshots.
- The snapshot is mandatory:
  - A client's world stays frozen until it has deserialized one `0x53` snapshot.
  - The snapshot is the byte-exact serialized game state. This includes compressed animation-tree state.
  - No client ever uploads its state.
  - The original server therefore ran the same game core in "server mode". The client binary still contains that mode: the `game_world_core` constructor flag at +0xC80D.
- A server that does not simulate can relay inputs and compare hashes. It cannot make the first snapshot unless one of these holds:
  - a patched client supplies the snapshot;
  - a re-implementation reproduces the client's serializers bit for bit;
  - the original core code is hosted.

---

## 1. Transport layer: same family as 0.100b, different wire format

The class names match 0.100b (`udp_match_connection`, `udp_match_client`, `match_client_impl`, `network_packets_orderer`), but the wire format changed. srp's `transport.rs` cannot talk to 0.21d without a rework.

| | 0.100b (srp `transport.rs`) | 0.21d |
|---|---|---|
| datagram header | 6 B: `u16 local_seq, u16 remote_seq, u16 (ack16<<1 \| multi)` (`fill_packet_header` 0x555BC0) | **12 B**: `u16 local_seq, u16 remote_seq, u64 (ack64<<1 \| multi)` (`fill_packet_header` 0x28B887, parse in `process_incoming_packet` 0x28797D). The receiver ORs bit 63 into the ack bits |
| ack window | 16 packets | **64 packets** (`update_acknowledgements(seq, seq, u64)` 0x28D884) |
| max datagram | 256 B | **508 B (0x1FC)**: `start_receiving` 0x286AA9 reads into a 0x1FC buffer, and `enqueue` 0x28D0B9 fragments at 0x1FC |
| message info | 1 byte, always 0xC0, so every message is ordered on channel 0 (`get_sending_message_info` 0x0A7DB0) | 4 bytes `{u16 a; u8 b; u8 flags}`. Flags: bit7 = ordered, bit6 = reliable (tracked until acknowledged), bits0..5 = channel (0..63). `a`/`b` look like lifetime/resend times in ms (unverified) |
| message encoding | `u8 id [u16 order]` + payload | same: `u8 id`, then `u16 order_id` only if ordered (`construct_packet` 0x238AAF; the order id is filled in by `enqueue_impl` 0x28CF07 from the per-channel counter at conn+0x67852+ch*0x2C) |
| multi-message packet | `u8 len` per message | `u8 len`, and **`len==0` → `u16 len` follows** (0x287C5C) |
| low-level | multi flag + `01 type` | same: multi flag + `01 type` (`new_low_level_packet` 0x28BFA6). Types: 0 initiate disconnect, 1 confirm, 2 continuous_flow. A packet whose first and only message exactly fills it is taken as low-level, so a lone game message must use the single format |
| fragmentation | none | **`0xFF` multipart** (`construct_multipacket_part` 0x28CFC1, `process_multipacket_message` 0x288717). Each part is `FF, u8 msg_id, u16 order_id, u8 part_index`. Part 0 also carries `u8 part_count`, then data. The order id comes from the channel counter. At most 255 parts × ~490 B ≈ 120 KB. Needed for 0x51 and 0x53 |
| heartbeat | idle 33 ms | continuous_flow after **100 ms** idle (conn+0x678D0 = 0x64, ctor 0x28B38E) |
| disconnect | initiate → confirm | same. 500 ms grace in state 2 (`send_queued_packets` 0x28C190) |
| client unacked limit | — | command-line `max_unacknowledged_packets_count`, default **4096** (0x8DD970). Over it, the client calls `instant_disconnect` |
| resends | — | each resend gets a new sequence id. The packet keeps its last 8 ids in a circular buffer at pkt+0x54, so an ack of any of them counts. Send count is at pkt+0x78 |
| packet pool | 8192 (srp note) | `single_size_buffer_allocator<0x2AC>`. Its count was not checked |
| **ports** | one | **port range.** Server message 0x50 `{u16 min, u16 max}` makes the client send 0x41 to every port in [min,max] (`udp_match_client::set_connection_ports` 0x287002). After that, every packet whose port is 0xFFFF goes to a **uniformly random port in the range** (`send_packet` 0x2874A2, Mersenne-twister). The server must listen on all of them and join them into one connection per client address. `min == max` (the connect port) disables this |

`network_flow_emulator` and packet duplication (pkt+0x7A) are debug features.

### Per-message info tables
Read from the jump tables of `get_sending_message_info` 0x1A144C and `get_received_message_info` 0x1A14B4.
- C→S 0x40, 41, 42, 43, 47: `{500, 200, 0x7F}`, reliable and unordered (channel 63).
- C→S 0x45 input: `{200, 0, 0x7F}`, reliable and unordered.
- C→S 0x44, 46: `{0xFFFF, 0xFF, 0x3F}`, unreliable.
- S→C 0x50: `{500, 200, 0x7F}`, reliable and unordered.
- S→C 0x51, 52, 53, 55, 59: `{500, 200, 0xC0}`, **ordered on channel 0**.
- S→C 0x54, 56, 57, 58: `{200, 0, 0x7F}`, reliable and unordered. Their order comes from the timestamps.

The server must use the same table when sending, because the receiver decides from the message id whether a `u16 order_id` follows.

## 2. Message table

### Client → server (`network_packets_orderer<client,server>`)
| id | when | payload |
|---|---|---|
| 0x40 connect | `match_client::connect` 0x1A130F | `u32 session` (`network_client+0x27C`; it comes from the lobby's connect-to-match message, parsed at 0x1A1DEC: host string, u16 port, u32, u8) + `bool g_debug_hash_mismatches` |
| 0x41 port probe | after 0x50, one per port in the range | `u32 session` (same value) |
| 0x42 ready | `network_client::on_match_ready` 0x1A25CC → 0x1A27B2, once the level and all player resources have loaded | empty |
| 0x43 set-time reply | `process_set_time_request` 0x1A2CF0 | `u32 echo` (the first u32 of 0x52) + `u32 client floating-timer ms` |
| 0x44 clock report | `try_send_all` 0x1A32FC, every 1000 ms once the world runs | `u32 client floating-timer ms` |
| 0x45 input | `packet_sender::on_player_input_changed` 0x1A2A34, from `game_world_core::synchronize<packet_sender>` 0x1A33CE | `f32 rot_dx, f32 rot_dy, u32 actions, u32 tick_time_ms` (16 B). One per 50 ms tick whose input changed; deltas and masks are merged within the tick (`change_input` local path 0x1CB3AD) |
| 0x46 hash | `try_send_all` 0x1A3285, every 100 ms | `u32 event_horizon_item_time, u32 crc32` (0xFFFFFFFF when the debug hotkey flag 0x1D087BA is set) |
| 0x47 leave | `close_current_match` 0x1A2448 | `u32 elapsed ms` |

There is no kill, hit, score or result message. Clients send only input, hashes and clocks.

### Server → client (`on_match_packet_received` 0x1A1520, jump table 0x1A1908)
The client keeps a state at `network_client+0x7A1C`: 0 = connecting, 1 = ports known, 2 = static info received. A message that arrives in the wrong state is **skipped**.

| id | state | payload → handler |
|---|---|---|
| 0x50 ports | 0→1 | `u16 min_port, u16 max_port` → `set_connection_ports(0x41, session, min, max)` |
| 0x51 static info | 1→2 | `match_options::deserialize` 0x1F59A4 + N × `player_profile::deserialize_static` 0x1F5C52 + `u8 local_player_index` → `switch_to_level_loading`, `query_players` (resource load) |
| 0x52 set time | 2 | `u32 echo, u32 match_time_ms` → `game::set_time_in_ms`, replies 0x43, then **`game_world_core::clear()`** 0x1CA788. It dereferences the match core, so it must come **after 0x42** (in journal-replay mode the client waits instead) |
| 0x53 dynamic info (snapshot) | 2 | `game_world_core::deserialize` 0x1CAB3B, see §3. Then the warm-up or player camera, depending on game status |
| 0x54 input relay | 2 | `u8 player, f32 dx, f32 dy, u32 actions, u32 time` → `change_input(update, from_network=true)` 0x1CB12C |
| 0x55 clock offset | 2 | `i16 offset_ms` → `floating_timer::set_offset` |
| 0x56 player entered | 2 | `u8 player, u32 time` → `enter_match` 0x1CB9BD |
| 0x57 player left | 2 | `u8 player, u32 time` → `leave_match`. If the player is local, the client disconnects |
| 0x58 local input discard | 2 | `u32 time` → `discard_input(local_index, time)` 0x1CB54F |
| 0x59 hash mismatch | 2 | `u32 time, u16 strings_bytes, u16 string_count, strings…, u16 desc_count, desc×{4×u16 string idx, u16, u16, u8}, then the server's serialized state` → `on_hash_mismatch` 0x1CD2E5. It is **only parsed when the client runs with `-debug_hash_mismatches`**; otherwise the client skips it and logs a hint |

`change_input`, `enter_match` and `discard_input` return a code (1 duplicate, 2 too old, 3 too new, 4 other/older than the snapshot). The client only counts it in its statistics (`register_message_in_stats` 0x1A2EEE). A rejected relay produces no error and no request, so the client silently diverges.

### Static info contents
`match_options` fields are named by offset only; nothing gives their names:
- `u8 @C444`: level id. It is passed to `lobby_menu::switch_to_level_loading`.
- `string @C460`: the level / server-project resource name. `pvp_match_core_cook::translate_query` 0x1C7944 requests it as resource class 0x41.
- `u8→u32 @C448`: **game mode**. Value 3 selects a different rules resource (class 0x5A instead of 0x59) at 0x1C79D8.
- `u8 @C44C`: **player count N** (the roster size).
- `u8 @C45B`: battery count (used by `gather_victory_items_rule_cook` and the UI).
- `u8 @C44D`: respawn parameter (`player_respawn_rule_cook`).
- Unidentified fields: `u16 @C44E`, `u32 @C440`, `f32 @C454`, `u8 @C458/9/A`, `u16 @C450`, `24 × f32 @C480`.
- Finally `N × u8 @C4E0`, one per player (probably the team; unverified).

Each `player_profile` static block contains, in this order:
- `u8` (@5AC) and `bool` (@5B0);
- `string name`;
- `u32` (@9C8);
- slots: `u32 mask` over 23 slots. Each set slot holds an `inventory_item_descr` (0x1F5B53): `u16, u32, [u16], [u32], u8 n, n×u32`. The optional fields depend on the per-slot mode table at 0x877168. **This is the loadout.**
- `u32 mask` over 20 floats, then each set float, added to a base. These look like skills (unverified).

**The roster is fixed for the whole match.** The world creates N players from static info (`on_match_ready` loops over `C44C`). Later arrivals are only activated or deactivated with 0x56/0x57, and the index byte in 0x54/56/57 indexes this roster. So the server must send the full planned roster, including players who have not connected yet, to every client. srp's 0.100b design grows a live roster per connection; that does not carry over.

### Handshake up to the first controllable tick
1. C→S 0x40 `{session, debug_flag}`. This is the first packet: sequence 0, and an order id is not used because the message is unordered.
2. S→C 0x50 `{min_port, max_port}`. C→S 0x41 goes out on each port.
3. S→C 0x51 static info. The client loads the level and the players' resources.
4. C→S 0x42 ready.
5. S→C 0x52 `{t_send, match_time}`. C→S 0x43 `{t_send, client_ms}` (round-trip sample). The client clears its history.
6. S→C 0x53 snapshot `{T0, mask, players…, 4 blobs}`. The log line is "THE VERY FIRST SYNCHRONIZATION" (0x1CAC5E). This clears flag C80E and **unfreezes `move()`** (0x1CCCB4: `move` returns at once while C80E != 0).
7. S→C 0x56 `{idx, T}` for each player who becomes active after T0, or the snapshot mask already includes them. The client fast-forwards from T0 to now ("large move"), applying every input after T0. **The server must replay to a newcomer every relayed input with time > T0.**
8. From then on: C→S 0x45 / 0x46 / 0x44, and S→C 0x54 / 0x55 / 0x58 / 0x56 / 0x57.

## 3. The world snapshot (0x53): who makes it and what it contains

Wire layout (`game_state_history_item::deserialize` 0x1FE021):
- `u32 time_ms`;
- `u32 active_clients_mask`;
- for each set bit, a `player_history_item`. That is `u16 len + player_serialized_state`, then `bool`, then the animation player's compressed state (`serialize_state_and_compress`, called from `player_history_item::serialize` 0x20C1A3…);
- `u16 len + bullet_manager bytes`;
- `u16 len + statistics bytes`;
- `u16 len + game_objects bytes`;
- `u16 len + game_rules bytes`.

The hash `game_state_history_item::hash` 0x1FE15A is the boost CRC-32 (reflected 0x04C11DB7, init and final xor 0xFFFFFFFF) over exactly this layout **without** `time_ms`, with the mask taken from the players-mask history at that time.

**Who must produce it:**
- **Nothing in the client produces it for another peer.**
  - The client does serialize its own locally built initial world at `on_match_ready` (`game_world_core::serialize()` 0x1CA765), but only into its local history.
  - 0x52 then wipes that history (`clear`).
  - There is no client → server path for state; 0x46 carries only the CRC.
- **The original server simulated.** The client binary carries the server variant of the same core:
  - The `game_world_core` constructor 0x1CA3E5… sets C80D = is_server and C7EC = event horizon. The horizon is **250 ms on the server** and `max_ping_time_in_ms`(250) + 250 = **500 ms on the client** (0x1DC92D, the console var at 0x87770C has range 250..1000).
  - The server history is 1 item, the client history 11 (0x1DC8C1).
  - In server mode, `new_game_state_history_item` 0x1CA899 records `(time, hash)` for every new state. These are the reference hashes that clients' 0x46 reports were compared against.
  - In server mode `change_input` and `enter_match` stamp late events onto the next tick instead of rejecting them (0x1CBAB0…).
- **Can a server avoid simulating?** Not with stock clients:
  - The first sync, every late join and every desync repair need a byte-exact state.
  - That state includes player state, the inventory, weapon state with RNG, breath and recoil, compressed animation-mixer state, bullets in flight, statistics, every map object (anomalies, artefacts, containers, traps) and game-rule state.
  - Every one of those must match the client's own serializers. A wrong byte gives a world that differs from the other peers and an immediate hash mismatch, or a crash on deserialize.

**What a server needs to fill a snapshot itself:**
- the level's object list in registration order, with the initial state of each object (comes from the data: the level server project in `resources.db`);
- the rules' state;
- each player's full serialized state, including the logic animation tree;
- the item dictionaries for the loadouts;
- the bullet manager state.

In practice that means running the game core.

**Options that do not simulate (none tried):**
1. *Client-sourced snapshot (needs a client patch).* A patched client uploads its event-horizon item's bytes on request. The server accepts them only if the CRC equals the hash that most clients reported in 0x46 for that time, then forwards them as 0x53. For the very first sync, every client's world is the same locally built initial state, so any client can provide it (the patch would have to upload before 0x52's `clear`, or re-serialize after it).
2. *Skip the snapshot (also a patch).* Patch the client so C80E starts at 0 and so 0x52 does not clear the initial item (around 0x1CA605/0x1CA609 and 0x1A2DC4). That gives a pure lockstep start from identical local worlds. Late join and resync stay impossible.
3. *Host the original core* (a headless 0.21d process or DLL driven by srp) in server mode. This gives bit-exact snapshots, hashes and results.

## 4. Input relay rules
Ticks are 50 ms (C7F0 = 0x32). Inputs carry the tick time; `try_send_all` quantizes game time with `/50*50`.

**Client acceptance** (`change_input` 0x1CB12C) for a relayed input at time T:
- `T <= C804` (the time of the last snapshot): ignored (4).
- `T + 500 <= client_world_time`: **too old (2), dropped.** That client then permanently lacks the input and desyncs.
- `T > now + 5*500`: too new (3), dropped.
- The same input at the same T: duplicate (1). Echoing a player's own input back to it is therefore harmless (whether the original server echoed is unverified).
- Otherwise the input is inserted and the client rolls back to replay from T.

**The server's job (the server-mode rules, inferred from the C80D branches):**
- The server clock is authoritative.
- Tick T is **final** once `server_time >= T + 250` (the server horizon). The server only needs a clock for this, not a simulation.
- A client input arriving for a T that is already final must **not** be relayed. The server sends that client **0x58 `{T}`** (`local_player_input_discard`), and the client deletes its own predicted input at T so that it agrees with everyone else.
- Otherwise the server relays it as 0x54 to all other clients, and optionally echoes it to the sender. It also logs it for late joiners.
- A client's own input for a tick that is already final gives the client "cannot discard local player input" (2). That is harmless.
- The 250 ms gap between the server horizon and the client horizon is the latency budget. A relay that takes longer than about 250 ms one way to some client desyncs that client.

**Time sync:**
- 0x52 sets an absolute match time.
- The 0x43 reply gives the round trip.
- 0x44 sends the client clock every second.
- 0x55 nudges it by a signed 16-bit offset.

The exact correction policy is unknown; it is a server-side choice.

**Hashes:** each client sends `(event-horizon time, crc)` every 100 ms. The event-horizon item is the newest history state at or before `now - 500` (`event_horizon_history_item` 0x1CAB13).

## 5. Match lifecycle
- **Start:** the handshake in §2. The server chooses T0, the roster, the level and the mode.
- **Game modes and rules** are data-driven rule objects inside the simulation:
  - `timelimit_rule(_core)` (it sets the game status: warm-up, playing, finished, via `network_client::set_game_status` 0x1A29E9 from `timelimit_rule::tick` 0x197FAA);
  - `gather_victory_items_rule` + `match_time_vi_spawn_rule` (batteries);
  - `kd_stats_rule` (kills and deaths);
  - `player_respawn_rule`, `single_player_respawn_rule`.

  Which ones a match uses depends on the mode and the level project. That part depends on the data.
- **Who decides the end:** every client, deterministically (time limit and score inside the simulation). The server has no "match finished" message in 0.21d. The client shows the end screen, and `game_world::on_finish_match` 0x15BD55 (from the UI callback / `close_current_match`) sends 0x47 and returns to the lobby.
- **Results:**
  - Nothing is reported by the client.
  - Scores and statistics live in the simulation (`base_game_statistics_handler`, `shared_statistics::finish_match`, the winner from `game_world_core::winner_team` 0x1CEE03).
  - After the match the client asks the **lobby** (TCP) with `lobby_client::query_match_stats` 0x19D6F0 and reads `match_stats` + `player_result_full` (`read_match_stats` 0x19E6CE).
  - So the server side must *compute* results. That means a simulation, or decoding the statistics blob of a verified snapshot (option 1 above; the `shared_statistics::serialize` format would need reversing).

## 6. Hash mismatch on the client
- The client never compares hashes. It only reports them (0x46).
- Detection is the server's job. With a simulation it compares against its own record; without one it compares clients with each other (a majority vote finds the odd one out, but cannot prove anyone right).
- **Recovery** is a new 0x53 snapshot. Its log line is "FULL RESYNC" (0x1CACED), and it calls the `on_resync` callback (`game_world::on_resync` 0x15BE5E, a UI notice). The snapshot replaces the whole state, and inputs after it apply on top.
- **What a resync does not replace** (disassembly, 2026-10-11) [V]:
  - The per-player input lists (`game_world_core`+0x2684, 0x2D0 bytes a player; node: next +0, dx +8, dy +0xC, buttons +0x10, time +0x14) are made by the constructor (0x1CA41F) and freed by the destructor (0x1CA713) only. `clear()` (0x1CA788, called by 0x52 and again by the 0x53 load 0x1CAB3B) wipes the world history (+0x2670), sets C7F0/C7F4/C7F8 to 50 and C804 to 0, and leaves the input lists alone.
  - So a resynced client keeps every input it made before, its own included. An own input the server never relayed stays in its world and nobody else's until a 0x58 removes it. srp now sends 0x58 for the inputs that came while the client loaded the snapshot (they used to be dropped silently, and that broke about 3 resyncs in 4 where it happened).
  - `discard_input` (0x1CB54F) refuses (code 2) `T <= C804`; the load sets C804 = C7F8 = the snapshot's time (0x1CAD6D/0x1CAD79). It then looks for the player's first node with time >= T and removes it only if its time is exactly T (else it logs and refuses), and only if the world history (+0x2670) has an item older than T to replay from (0x1CEE73 walks back from the newest; else it logs and refuses). A 0x58 for a tick after the snapshot that has its node works, also for a tick ahead of the client's clock.
  - A local input (`change_input` with the remote flag 0) looks for the player's first node later than the start of the current tick and merges into it (0x1CB494: dx/dy added, buttons or'ed, its time set to the input's). A resync sets the clock back about 260 ms (srp's `SYNC_LEAD_MS`), so ticks the client played before are played again and a new input there lands in an old node, which moves to the new input's tick. Seen live (match 0x123, 16:06): a resend for a tick the others had, with other contents, dropped as a copy, and another discarded by srp's 0x58 with the original in it; the client was out of step again 600-700 ms after the snapshot each time. The 0x45 sender (0x1A2A34) sends the node's own time, which is always on the 50 ms grid. srp now starts a resynced client past its last input the others have and clears its other own inputs with 0x58s before the 0x52 (WORK.md, 2026-10-11).
  - Which local inputs are sent: `try_send_all` (0x1A310B) → 0x1A33CE walks the world-history items and sends the players' inputs whose "changed" bits (+0x150) differ from their "sent" bits (+0x144); the bits live in the history items, so `clear()` resets them.
- **Input lists, history and discards in detail** (disassembly, 2026-10-11 evening) [V]:
  - Local `change_input` (0x1CB3AD): `g` = the last 50 ms grid point (from C7F0) strictly before the input's time. It takes the player's first node with time > `g`. With none, it appends a new node with the input. Otherwise, unless a node after that one has exactly the input's time (then it returns 1 if the contents are equal, else 4), the found node gets the input's time and the input is added in (dx/dy summed, buttons or'ed, 0x1CB494). So a node later than the tick being played is **pulled back** to that tick with its old contents in it.
  - Input nodes are freed only by `discard_input` and the destructor. The binary has 11 references to the lists' offset +0x2684: constructor 0x1CA41F, 0x1CA62F, destructor 0x1CA713, snapshot load 0x1CADBF, `change_input` 0x1CB3D5/0x1CB4CC, `discard_input` 0x1CB61A, 0x1CC408, 0x1CCC52, sender 0x1A34ED, and one at 0x2EF146 not looked at. The only targeted unlink of an input node is in `discard_input` (0x1CB966). **Correction (2026-10-12):** each list is a ring of 27 nodes: its allocator is `single_size_fixed_allocator<0x18, 0x1B>`, and `new_item` (0x1CF257) calls `history<player_input_history_item>::pop_front` when it is full (0x1CF260-0x1CF267). So an own input that no 0x58 removed stays until 27 newer nodes of that player push it out. The event history is a 27-item ring too (capacity written at 0x1CEF94, oldest popped at 0x1CA84F).
  - The world history (+0x2670; +0x2674 the oldest item, +0x2678 the newest; items 0xB9940 bytes, linked at +0xB9930/+0xB9934, time at +0xB9938) does not grow with time. Advancing a tick (0x1CA765) takes the oldest item (0x1CA899 → 0x1CF0AE), fills it (0x1CC570) and appends it as the newest (0x1CF103). `move()` logs "cannot find older item for time %d.%03d : the oldest is %d.%03d" (0x81A420).
  - 0x1CEE73 returns the newest item with time < T; 0x1CEE91 the newest with time ≤ T. The event-horizon item (0x1CAB13) is the newest item with time ≤ C7F8 − C7EC (the newest item if C80D is set).
  - `discard_input` has no horizon test of its own: it refuses T ≤ C804 (code 2), a first node at or after T whose time is not T (code 4), and a T with no history item before it (code 4, log line 0x2B3). So a 0x58 for a tick at or before the oldest history item is refused, and that input stays in the list for good. A later snapshot load from before that tick plays it again.
  - The sender (0x1A34ED) sends a player's node only when a history item has exactly the node's time and that player's changed bit. It sends the node's contents with that time.
- **`max_ping_time_in_ms`** [V for the bytes, meaning unverified]: a static initialiser at 0x68E125 registers an option object at 0x56D8C20 named "max_ping_time_in_ms" (string at VA 0x81C668), storing 250 at +0x44 and 1000 at +0x48, probably the default and the maximum. The client horizon C7EC is set from a constructor argument (0x1CA5D1); srp takes it as max_ping_time_in_ms + 250 = 500 ms, but the link from the option to that argument was not followed. It has not been tested whether a player can set this option on the command line, as `max_unacknowledged_packets_count` can be.
- **A player who left is not in a snapshot.** The snapshot's mask is the active players only (dumps: player V's slot 4 is missing from the one at 421600 ms in match 0x126, while he was away). The others keep his old player object; a client that rejoins and loads such a snapshot makes a new one, and its world differs from theirs from the tick it enters (10 of 13 rejoins). After a load, 0x1CC6DA compares the snapshot's mask with the active mask C808: players gone are made inactive (0x1CC158), players new to it are made active with `make_client_active` 0x1CC0E7, the same call `on_player_entered_match` (0x1CC1B0, from a 0x56) makes at 0x1CC284. So srp enters a returning player for the others first and starts it from a snapshot after that entry, with its player in the mask as the others have it. The player camera does not need its own 0x56: `timelimit_rule::tick` (0x197FAA) calls `set_game_status` (0x1A29D0) whenever the rule's state differs from the network client's last reported one (+0x7A18, nothing on a new connection), and state 3 gives the player camera (0x1A2A29).
- 0x59 is a debugging aid only:
  - With `-debug_hash_mismatches` the client diffs the server's state against its own, descriptor by descriptor, and writes the differences to the log and `printf` (`on_hash_mismatch` 0x1CD2E5).
  - Otherwise 0x59 is ignored.
  - A client that sends `debug=1` in 0x40 builds serialization descriptors, which carry field names. **Running a 0.21d client with `-debug_hash_mismatches` would document the whole snapshot format field by field.**

## 7. Reuse estimate for srp's match server

| srp module | verdict |
|---|---|
| `transport.rs` (1361 lines) | **Adaptable, substantial rework.** The ack, resend and in-order logic carries over. To change: the 12-byte header and 64-bit acks, the 508 B limit, the per-message reliability/channel table, the u16 extended lengths, 0xFF fragmentation both ways (big 0x51/0x53 out; nothing big comes in), and port-range aggregation (or `min == max`). Heartbeat 100 ms. `sequence_number.rs` is reusable |
| `message/` | **Rewrite.** New ids and layouts (above). The client side is small (8 messages); the server side has 10 messages plus the static-info serializer (match_options + profiles with the 0.21d item descriptor) |
| `match_server.rs` (5282 lines) | **Mostly rewrite.** Peer and socket management and the console can be adapted. The core loop becomes: clock, the input finality and relay rule, the input log for late joiners, the 0x58 rule, time sync, hash collection and voting, snapshot distribution. No per-player state machine |
| `lifecycle.rs` | **Largely not needed / rewrite.** In 0.21d the phases (warm-up, countdown, end) run inside the client simulation from the rule objects. The server only needs a "match open → started (T0) → all left" lifecycle and a planned roster |
| `game.rs` | **Adaptable.** Turning a lobby profile into a static `player_profile` is the same idea, with a 0.21d item format (data-dependent) |
| `combat/` (≈10k lines: weapon FSM, ballistics, damage, pose, collision, traps) | **Not needed** for relay. Useless for snapshots, because it is not bit-exact with the 0.21d core (and it is modelled on 0.100b) |
| `batteries.rs`, `supplies.rs` | **Not needed.** Simulated by the clients (gather_victory_items rule, inventory) |
| `map.rs` | **Not needed** for relay. Only the level name and mode go in static info. Needed only for a home-grown simulation |
| `console.rs`, `utils.rs` | reusable |

**New pieces:**
- an authoritative match clock, the finality/relay rule, and an input log;
- time sync (0x52/0x43/0x44/0x55);
- a hash table keyed by (time, client) with mismatch policy;
- **a snapshot source**: the patched-client upload (option 1), the skip-snapshot patch (option 2), or the hosted original core (option 3);
- a lobby-side results path (`match_stats` / `player_result_full`);
- 0.21d lobby, login and resource formats are out of scope here.

**Simpler or harder than 0.100b?**
- The network logic itself is **much simpler**. There is no movement, ballistics, damage, weapon FSM, traps, batteries or respawns: the relay is about 1–2k lines.
- A **complete** server is **harder**, because the one thing it must supply, the first and resync snapshots, cannot be produced without the original simulation code or a client patch.
- A rough path to "two players in a match":
  - with the skip-snapshot client patch, small (transport + relay + static info);
  - with the patched-client upload, medium;
  - with a home-grown deterministic re-implementation, not realistic.

**Biggest risks:**
1. **Snapshot source / determinism.** The state includes compressed animation-mixer state, a seeded LCG and RNG, and Bullet physics (`btAxisSweep3`, `btGjkEpaSolver2` are in the exe). Float results depend on the MSVC CRT and SSE versus x87 code paths; the core uses SSE `movss`/`addss`, with some x87 left. A Rust re-simulation would desync immediately.
2. **Desync with no repair.** Without snapshots, any dropped relay (late arrival past the 500 ms client horizon), packet loss beyond the 200 ms input lifetime, or a client running slow leads to permanent divergence. The hashes only report it.
3. **Cheating trust.**
   - Lockstep stops teleport and hit spoofing, because only inputs exist. But every client sees the full world (wallhacks) and can send any action mask; there is no server rule check unless the server simulates.
   - Without a simulation, results come from client consensus.
   - A snapshot uploader could forge state. The CRC check against the majority's 0x46 reports limits this to collusion.
4. **Port-range UDP** and the 500-ish byte datagrams versus NAT and firewalls. Use `min == max`.
5. **Fixed roster.** Matchmaking must fix N and the profiles before anyone connects.

## Data needed from `resources.db` (to name to the coordinator)
- The level/server-project resource named in `match_options.C460` for each map (0.21d equivalent of `projects/<level>/server_project.lua`).
- The game-rule resources (classes 0x59/0x5A in `pvp_match_core_cook::translate_query`), giving the mode → rules mapping, time limits and battery counts.
- The item dictionary and slot layout behind `inventory_item_descr` (23 slots, mode table at 0x877168), for building static profiles.
- Mode ids (`C448`) and the meaning of the unidentified match_options fields. A 0.21d lobby capture or the lobby's own match-options writer would settle these; the lobby server code is not in the client.
