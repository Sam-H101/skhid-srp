# Option C: can a stock 0.21d client be patched into the match server?

Scope: `survarium.exe` 0.21d (build 2010), with its PDB. All addresses are VAs, image base 0x10000. To get a file offset inside `.text`, subtract 0x10C00 (VA 0x11000 is file offset 0x400; for example 0x1267A → 0x1A7A and 0x151609 → 0x140A09).
Tags: **[V]** = read in the disassembly; **[I]** = inferred, not proven.

## Verdict

**Option C (the exe running headless as the authoritative server) is not feasible as a patch.**
- The binary holds the *simulation* half of the server: the same `game_world_core`, with a working `server_mode` flavour, and the snapshot **writer**.
- It holds **none** of the *server* half:
  - no listener and no peer table;
  - no server→client message construction;
  - no 0x51 builder;
  - no relay or finality logic;
  - no hash comparison;
  - no time-sync policy;
  - no result reporting.
- The only orderer instantiated is `network_packets_orderer<client::messages_enum, server::messages_enum>`. It sends client ids and receives server ids.
- Building C means writing a whole match server as injected code (a DLL of tens of KB) inside a process that still needs a D3D device. That is more work than srp's relay, and it gains nothing that a smaller patch cannot also get.

**What the binary does provide makes option B cheap.**
- `game_state_history_item::serialize(buffer_writer&, mask)` at 0x1FDE3E is the exact mirror of the 0x53 reader.
- It already runs on every client every 100 ms, because `hash()` uses it.
- A 0x53 payload is `u32 item.time` followed by these bytes.
- A client patch only has to call it into a packet writer and enqueue the packet. That is about 100–150 bytes of code cave plus one hook.

**Recommended:** srp stays the relay and authority (match.md §4). Snapshots come from a **patched client**, verified by CRC against the majority of 0x46 reports:
- either every ordinary client carries the upload hook (B);
- or one dedicated, hidden "observer" client does. It runs with the one-byte `server_mode` flip so that its world starts without a 0x53 (hybrid "C-lite", §5).

---

## 1. Every use of `server_mode`

Source: the byte `pvp_match_core_cook+0x21`, global 0x56E825D (the cook is at 0x56E823C).
- It is written only in `game::game`, at 0x151603: `C6 05 5D 82 6E 05 00`, `mov byte [56E825D],0`. **[V]**
- No other xref exists. **[V]**
- It is read once, in `pvp_match_core_cook::on_resources_loaded` at 0x1C77CA. It is passed as `pvp_match_core::pvp_match_core(bool server_mode, bool replay, …)` (0x1DC824). **[V]**

| # | Site | Server path (server_mode=1) | Client path | Nature |
|---|---|---|---|---|
| 1 | `pvp_match_core` ctor 0x1DC8D3 | allocates **1** state-history item (0xB9954 B each) | `(max_ping+250)/50+1` = 11 items; 0x80 items with `-debug_hash_mismatch` | parameter: no rollback on the server |
| 2 | `pvp_match_core` ctor 0x1DC92D | event horizon C7EC = **250 ms** | `max_ping_time_in_ms` (cvar 0x87770C, default 250) + 250 = 500 | parameter |
| 3 | `game_world_core` ctor 0x1CA5F8 | C80D = 1 | 0 | copy |
| 4 | `game_world_core` ctor 0x1CA5FE–0x1CA609 | C80E = 0: the world is **not frozen**, no 0x53 needed | C80E = 1 unless replay | gate |
| 5 | `new_game_state_history_item` 0x1CA8A3 | before reusing the single item, computes `hash()` of the current item and appends `{time, crc}` to a list at `core+0x6198` | plain `new_item` | **real server logic, write-only.** Nothing in the exe reads +0x6198 (only the ctor, dtor and this function touch it). The comparison against clients' 0x46 reports was in server-only code that is not here |
| 6 | `event_horizon_history_item` 0x1CAB13 | returns the newest item (`+0x2678`) | nearest item ≤ now − horizon | parameter |
| 7 | `is_there_event_horizon_item` 0x1DCBF9 | always true when an item exists | time check | parameter |
| 8 | `change_input` 0x1CB203 | horizon taken as 0: input at T is **rejected (code 2, "too old") when T ≤ world time** | rejected when T + 500 ≤ world time | real rule. The server-only caller turned code 2 into a 0x58 discard (inferred; that caller is absent) |
| 9 | `enter_match` 0x1CBA8A / 0x1CBAB0 | a late enter is **re-stamped to the next tick**, `((now − C7F0)/50 + 1)*50 + C7F0`, instead of rejected | rejected, code 4 | real rule |
| 10 | `enter_match` 0x1CBC73 | `insert_impl(..., rollback = !server_mode)` | rollback = 1 | parameter |
| 11 | `leave_match` 0x1CBD6D / 0x1CBD97 | same re-stamping as `enter_match` | rejected | real rule |

**Conclusion [V]:** the server branches are small *policy* differences in the shared core:
- a no-rollback history of 1 item;
- a 250 ms horizon;
- an unfrozen start;
- late join/leave stamped onto the next tick;
- late inputs rejected;
- per-state reference hashes recorded.

They are real authority semantics, but every **consumer** of them is missing:
- whoever sent 0x58 on a code-2 result;
- whoever compared the +0x6198 hash list;
- whoever ticked the server world at `now − 250`.

`move()` 0x1CCCA3 has no server branch. It moves to whatever time the caller passes. **[V]**

## 2. Capability table

| Capability | Status | Address / symbol | Evidence |
|---|---|---|---|
| Deterministic simulation (players, weapons, bullets, objects, rules, statistics) | **present** | `game_world_core::move` 0x1CCCA3, `atomic_tick`, `tick_events`, `pvp_match_core::tick` 0x1DCBAE | It is the client's own simulation; server mode uses the same code **[V]** |
| World → history item (save) | **present** | `game_world_core::serialize(item&)` (label at line 585414). Called every tick from `tick_events` 0x1CC53B, from `serialize()` 0x1CA765 (initial state, via `network_client::on_match_ready` 0x1A26CF and replay 0x16C3FE), and from `fill_debug_info` 0x1CD2B6 | **[V]** |
| **(a) History item → wire (0x53 writer)** | **present, reachable** | `game_state_history_item::serialize(buffer_writer&, u32 mask)` 0x1FDE3E | Writes mask, `player_history_item::serialize` ×N, then four `u16 len + bytes` blocks. This is exactly what `deserialize` 0x1FE021 reads after its leading `u32 time`. Callers: `hash()` 0x1FE1D4 (into a 0x8000-byte stack buffer, every 100 ms via `try_send_all` 0x1A3251) and `on_hash_mismatch` 0x1CD551. **[V]** |
| 0x53 framing | trivial | — | 0x53 = `u32 item[+0xB9938]` + `serialize(mask)`. The reader `game_world_core::deserialize(reader)` 0x1CAB3B reads nothing else **[V]**. Real snapshots are 11–14 KB, which fits the 32 KB hash buffer |
| (b) Build/send static info 0x51 | **absent** | only `match_options::deserialize` 0x1F59A4, `player_profile::deserialize_static`, `deserialize_slots`, `inventory_item_descr::deserialize` | No `serialize` counterparts in the labels **[V]** |
| (c) Send 0x50/52/54/55/56/57/58/59 | **absent** | the only orderer is `network_packets_orderer<client,server>`; `new_packet` takes `messages_enum@client@match` (0x524434) | Only 1 label mentions `messages_enum@server@match`, and it is the client's stats register **[V]** |
| (d) Server endpoint / listener / multiple peers | **absent** (transport partly reusable) | `udp_match_client` (one socket, one `udp_match_connection` embedded, state at +0x678DC). `udp_match_connection` is generic per-peer code: acks, multipart, resend | `listen`/`accept` are called only by the boost.asio `socket_select_interrupter` (0x243331/0x243506) and the Scaleform AMP profiler (0x672699). No `server`/`host`/`dedicated`/`listen` class in the game code **[V]** |
| Outgoing large messages (C→S) | **present** | `vostok::network::enqueue_impl` → `udp_match_client::enqueue(id, reader, size)` 0x23996E → `udp_match_connection::enqueue` 0x2871A6 → multipart | A client *can* send a >508 B message as 0xFF parts. `mutable_buffer::resize` 0x155E22 grows writers **[V]**. That the packet writer at pkt+0x294 grows this way is **[I]** |
| (e) Hash vote / desync detection | **partial (data only)** | server-mode reference-hash list at core+0x6198 (0x1CA8DC); `hash()` 0x1FE15A | It records hashes and nothing compares them. Clients never compare; they only report 0x46 **[V]** |
| (f) Match end / results | **partial** | in-sim: `timelimit_rule`, `winner_team` (line 588429), `shared_statistics::finish_match` (from `base_game_statistics_handler::finish_match` 0x19C1B4), `shared_statistics::serialize` (exists) | End and stats are computed by every client's simulation. `match_stats`, `player_result_full` and `player_result_brief` exist only as `deserialize` (lobby → client). Turning stats into results and posting them to the lobby is **absent** **[V]** |
| (g) Join/leave, team, spawn | **partial** | `enter_match`/`leave_match` with server stamping, `make_client_active`/`inactive`, `player_respawn_rule` (in-sim) | Spawning and respawning are deterministic in-sim. The roster and teams come only from 0x51 (`match_options` +C4E0 per player); there is no code that *chooses* them **[V]** |
| Input relay / finality / 0x58 policy | **absent** | — | Only the core's accept/reject codes exist (#8 above) |
| Time sync (0x52/0x55 policy) | **absent** | client side only: `process_set_time_request` 0x1A2CF0, `floating_timer::set_offset` | **[V]** |

## 3. Server classes; the replay path as a serverless feeder

- Label sweep for `server|host|listen|accept|peer|dedicat|headless` gives:
  - `server_game_project` (the level's object project; clients load it too);
  - Bullet `btGhostObject`;
  - OpenSSL `*_server_method`;
  - the asio and Scaleform uses above.
- There is **no** match-server class. **[V]**
- `journaling::match_client` (ctor 0x523903) is a fake transport. With `-replay_journal` (journal_usage==2, 0x16C9CB) it replays recorded server packets into the same `on_match_packet_received`. It is client-side only. **[V]**
- **`replay_network_client` (`-match_replay`) runs a whole match with no server at all [V]:**
  - It loads the core with `{replay=1}` (0x16C1FD), which leaves the world unfrozen.
  - It builds the initial state locally (`serialize()` 0x16C3FE).
  - `game::tick` (0x153925–0x153A6A) then reads a stream of typed records and calls the core directly:
    - type 0: time marker (u32);
    - type 1: `enter_match(u8, u32)` (0x153A51);
    - type 2: 20-byte `player_update` → `change_input(update, from_network=0)` (0x1539FE);
    - type 3: `leave_match(u8, u32)` (0x1539A6).
- So the core can be driven from any input source with no local controlling player. The normal `network_client` already does exactly this for 0x54/0x56/0x57.
- For srp this means a client can be fed live inputs over the normal protocol. Feeding them through the replay path instead would only make sense for an offline re-simulation tool, for example to compute results after the match from srp's input log. **[I]**

## 4. Headless blockers

- **No headless mode.** I listed all 92 `command_line::key` objects (`scope021d/cmdline_keys.txt`). None of them is `server`, `dedicated`, `headless`, `no_render`, a null renderer or no-sound. **[V]**
  - Relevant keys that do exist: `client` (login address), `spectator`, `replay_journal`, `match_replay`, `no_journal`, `debug_hash_mismatch`, `flow_emulator` and the loss/latency keys, `debug_single_thread`, `no_splash_screen`, `max_video_memory`, `no_effects_initialize`, `disable_game_statistics_gathering`, `account_name`, `account_password`, `skip_firewall_check`, and the network `max_unacknowledged_packets_count` and merge-packets keys.
  - A D3D device and window are mandatory. Hiding or minimizing the window is the only route. **[I]**
- **World ticking without rendering:**
  - `network_client::on_idle` 0x1A3360 itself advances the core to `now − horizon` (quantized to 50 ms) before `try_send_all`. **[V]**
  - `game_world::tick` 0x15AB69 is the render-driven second caller. **[V]**
  - With `server_mode`=1 the horizon is 250, so `on_idle` already gives server-style "tick to now − 250". **[V]**
  - The render tick may move the core to "now", which in server mode would make every input "too old" (#8). It would need to be disabled for an observer. **[I]**
- **Single instance:** `WinMain` 0x12660 calls `CreateMutexA("survarium_or_its_installer_is_al…")`. On `ERROR_ALREADY_EXISTS` it shows a MessageBox and calls `TerminateProcess`.
  - Bypass: change 0x1267A `75 22` (jne) to `EB 22` (file offset 0x1A7A). **[V]**
- **No local player:**
  - The replay path (above) and `-spectator` show that the world runs without an input-producing local player. `-spectator` skips the login and lobby menus at 0x153117, stores network_client+0x7A20, and registers the console command `attach_to_player_cc` 0x16CC14. **[V]**
  - Whether a live client accepts a 0x51 `local_player_index` outside the roster was not checked. **[I]**
- **0x52 in server mode:** `process_set_time_request` calls `game_world_core::clear()` at 0x1A2DC4, which wipes the single history item. A server-mode observer either must not receive 0x52 (and then needs another clock source) or that 5-byte call must be NOPed. **[V]** for the call; the consequence is **[I]**.

## 5. Option B, and the "C-lite" observer

**Patch for option B (the upload hook)** **[V]** for every building block:
1. Hook point: `try_send_all`, after `hash()` returns (0x1A3256). `[esp+10h]` already holds the event-horizon item, and `ebx` points to the players-mask history (core+0x5ED0).
2. `pkt = match_client->vtbl[0x18](id)`. The pattern is at 0x1A327D–0x1A3287; `match_client` is network_client+0x7A08.
3. Write `u32 item[+0xB9938]` with `buffer_writer::w` 0x155EBE into pkt+0x294.
4. Call `game_state_history_item::serialize(pkt+0x294 writer, mask)` 0x1FDE3E with `eax = item`. Take the mask the way `hash()` does (0x1FE197–0x1FE1CB): walk the mask history for time ≤ item time and use `[node+8]`.
5. Enqueue with `match_client->vtbl[0x1C](pkt)` (0x1A32BB). The transport splits the message into 0xFF parts.
6. Message id. `get_sending_message_info` 0x1A144C uses an **unchecked** 8-entry jump table for 0x40..0x47 (`sub eax,40h; jmp [eax*4+1A1494h]`). A new id 0x48 would jump into garbage. Either reuse a reliable id, for example **0x47** (srp can tell a snapshot by its length: 0x47 carries 4 bytes), or extend the table in a code cave.
7. Trigger: every N seconds, or on a flag set by a byte srp can reach. Uploading every 100 ms would be ~12 KB × 10/s per client, which is too much. Use a timer, for example every 5 s, or only when srp asks.

Rough size: one 5-byte jmp plus **~100–150 bytes** of cave code. srp needs:
- 0xFF multipart *reassembly* on receive;
- CRC-32 check against the majority of 0x46 reports for that time;
- forwarding as 0x53.

**First snapshot.** A client's world is frozen until it gets a 0x53, but its locally built initial item (`on_match_ready` 0x1A26CF) exists before 0x52 clears it. Two ways to get it:
- a variant of the hook uploads right after 0x1A26CF;
- or srp uses an observer that is never frozen (next point).

**C-lite observer** (one extra hidden client per match): B's hook, plus
- `server_mode` = 1: byte at **0x151609** from `00` to `01`. This gives no freeze, server late-join stamping and a 250 ms horizon, and the world runs from its own initial state;
- the mutex bypass;
- NOP the `clear` at 0x1A2DC4, or never send it 0x52;
- disable the render-tick `move`.

srp then:
- relays inputs to it like any client;
- takes its snapshots as the authority for first sync, late joiners and resyncs;
- reads its hashes (0x46, newest item in server mode) as the reference for majority checks;
- can decode its statistics blob for results later.

It still runs D3D. Its simulation is identical to the players' (same exe), so its hashes match theirs when no desync happens. **[I]**

## 6. Recommendation

| Path | Effort | Gives |
|---|---|---|
| **A**: skip-snapshot patch on all clients (C80E=0, keep the initial item) | smallest client patch | start only; no late join, no resync |
| **B**: upload hook on all clients | ~150 B client patch + srp reassembly/CRC/forward | first sync, late join, resync from the consensus state; no extra process |
| **C-lite**: B + one `server_mode` observer per match | B + 4 small byte patches + one hidden client process per match | the same, plus a reference simulation that never plays (cleaner authority and results) |
| **C**: exe as the full server | a full server written as injected code + D3D per match | nothing beyond C-lite |

**Go with B. Add the C-lite observer later if a trusted reference simulation is wanted.** Option C is not worth building.
