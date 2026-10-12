# 0.21d match start: static info (0x51), handshake, game modes, end of match

Source: `v021d_disasm/survarium_disasm.txt` (VAs, image base 0x10000), `resources.db` dumps (server_project, victory_items/default.options, localization/english, hud.swf, lobby_menu.swf decompiled with ffdec).
Tags: **[V]** read in code/data at the cited address; **[I]** inferred; **[U]** unknown / guess.
Extends `match.md` §2–5 and `lobby_spec.md` §2.1/§3.0. Corrections to those notes are marked **(fix)**.

---

## 0. Summary of what the server must send (happy path, 2 players, fresh match)

| # | dir | msg | content | constraint |
|---|---|---|---|---|
| 1 | L→C | lobby 0x33 | `str host, u16 port, u32 match_id, u8 team` | sets lobby status 2, resets match state +7A1C=0, connects [V 0x1A1DEC..0x1A1F7E] |
| 2 | C→S | 0x40 | `u32 session, bool debug` | session = **lobby/login session id** (network_client+0x27C = lobby_client(+0x238)+0x44), not match_id **(fix)** [V 0x1A1F73, 0x1A29B2] |
| 3 | S→C | 0x50 | `u16 min, u16 max` (use min=max=port) | state 0→1 [V] |
| 4 | S→C | 0x51 | match_options + N×profile + `u8 local_idx` | state 1→2; per-client only the last byte differs [V] |
| 5 | C→S | 0x42 | — | sent at end of `network_client::on_match_ready` [V 0x1A27B2] |
| 6 | S→C | 0x52 | `u32 echo, u32 match_time_ms` | **only after 0x42** (null deref otherwise) [V 0x1A2DB8] |
| 7 | C→S | 0x43 | `u32 echo, u32 client_ms` | [V] |
| 8 | S→C | 0x53 | snapshot at T0 | **after 0x42 and after 0x52** (0x52 wipes history) [V 0x1A2EAF, 0x1CA788] |
| 9 | S→C | 0x56 | `u8 idx, u32 T` per player, T > T0 | needed to spawn and to start the countdown [V] |

Recommended: T0 = 50, server match clock starts at 50 when the match opens, match_time in 0x52 = server clock now.

---

## 1. `match_options::deserialize` 0x1F59A4 — wire order [V]

`match_options` (0xC4F4 bytes) = `player_profile[20]` (stride 0x9D0, at +0) followed by the fields below (ctor 0x1F5916: 0x13+1 = **20** profiles). In `network_client` it lives at `vfn40()+0x10`, so offsets seen from network_client are +0x10 (e.g. `[+C45C]` in `query_players` = C44C).

| # | wire type | field | ctor default | read by (VA) | meaning | safe value |
|---|---|---|---|---|---|---|
| 1 | u8 | C444 level_id | (0) | `process_static_match_info` 0x1A2E60 → `lobby_menu::switch_to_level_loading` 0x1723E0 | **loading-screen picture only** (mapped to `_load_images[]` frame, §3) [V] | per map, §3 |
| 2 | str (u8 len + bytes) | C460 project name, buf 0x20 → **≤31 chars** | "" | pvp_match_core_cook 0x1C7944 (class 0x41), gather_vi cook 0x1C8D04, respawn cook 0x1C9E7A, `query_players` 0x1A2925 (`game::load(name)`) | level project: `resources/projects/<name>/client_project` (fallback `<name>/default/client_project`) [V 0x1797D9/0x17982D] | `level_0X/day` |
| 3 | u8 → u32 | C448 game_mode | 0xFF (invalid_game_mode) | pvp cook 0x1C79D8 (==3 → rule class 0x5A else 0x59); `game_world_ui::initialize_match` 0x1815C6 → `root.set_game_type(mode)` (hud.swf `progress_indicator.gotoAndStop(mode+1)`) | §5 | 2 |
| 4 | u8 | C44C player_count N | 0xFF | everywhere (roster loops, kd_stats, timelimit +134, stats) | roster size; **≤ 20** | N |
| 5 | u8 | C45B battery_count | 0 | gather_vi cook 0x1C8CAD (requests N_b × `gameplay/victory_items/default.options`, class 0x57), `select_spawners`, HUD `root.set_artifacts_required` 0x181602, hud icons 0x16..0x16+n-1 0x181821 | number of batteries in the match | 5–8; must be ≤ spawner count (30..51) and ≤ 10 (HUD icon ids 0x16..0x1F, 0x20 used next) [I] |
| 6 | u8 | C44D respawn_delay_s | 0 | respawn cook 0x1C9E95 → `player_respawn_rule(C44D*1000 ms, …)` 0x1C9FE4 | death → respawn at next 50-ms grid tick ≥ death + delay [V 0x1FCA21] | 10 |
| 7 | u16 | C44E match_duration_s | 0 | timelimit ctor 0x1F54D4 → +124 = s×1000 | "in process" phase length; HUD time-left [V 0x198028] | 600–1200 |
| 8 | u32 | C440 seed | 0xFFFFFFFF | gather_vi cook 0x1C8DA5 (spawner selection LCG ×0x8088405), `generic_anomaly_cook::seed` 0x15A17E | RNG seed (batteries, anomalies) | any; same for all clients |
| 9 | f32 | C454 start_fraction | 0.0 | timelimit ctor → +128 | waiting→countdown when `popcount(entered mask) ≥ floor(N × C454)` **and mask ≠ 0** [V 0x1F581C] | **0.0** (first entered player starts it; no min-player blocking) |
| 10 | u8 | C458 wait_min_a | 0 | timelimit → +12C = (C458+C459)×60000 | waiting-for-players timeout (HUD `st_waiting_for_players` countdown; `rule_completed` state 1) | 5 |
| 11 | u8 | C459 wait_min_b | 0 | summed with C458 | as above (split meaning unknown [U]) | 0 |
| 12 | u8 | C45A countdown_s | 0 | timelimit → +130 = s×1000 | "final countdown" length (`st_final_countdown`) | 0–10 |
| 13 | u16 | C450 | 0 | **no reader found** [V grep] | unused by client [U: max players / rating?] | 0 |
| 14 | 24 × f32 | C480 score weights | 0 | `base_game_statistics_handler::on_event` 0x19C3D2 | per `match_stats_events_dict_enum` (0..23): `score += floor(value × w[e] × k + 0.5)`, k = 0.01 for events 6, 0x11, 0x12, else 1.0 [V; consts 0x876F40=1.0, 0x73EB78=0.5] | table below [U values] |
| 15 | N × u8 | C4E0 squad_id[i] | 0 | `initialize_match` 0x18174A | player list `squad_id`, `squad_state = (id≠0)` [V] | 0 |

Score-weight index (enum @41825f0 [V]): 0 kill, 1 assist, 2 death, 3 suicide, 4 team_kill, 5 artefact, 6 bring_victory_item(×0.01), 7 teammate_cure, 8 teammate_rescue, 9 revenge, 10 victory_item_carry_kill, 11 participating, 12 won, 13 melee_kill, 14 silent_kill, 15 headshot_kill, 16 winning_team_vi_bonus, 17 steal_vi(×0.01), 18 recover_vi(×0.01), 19 vi_found, 20 put_vi_in_container, 21 entrap_enemy, 22 bullet_fire, 23 bullet_hit.
Suggested [U]: kill 100, assist 50, artefact 20, bring_vi 100, cure 25, rescue 25, revenge 25, vi_carry_kill 50, won 0, melee/silent/headshot 25, steal/recover 100, vi_found 25, put_vi 50, entrap 25, others 0. Scores only drive the scoreboard (`emit_score_changed_event`); they do not decide the winner [V winner_team §5].

---

## 2. 0x51 layout and client handling — `process_static_match_info` 0x1A2DDC [V]

```
0x51 [u16 order_id: ordered ch0]
  match_options (§1)
  N × player_profile::deserialize_static   (identical function 0x1F5C52 as lobby 0x36 02 → lobby.rs write_profile_static)
  u8 local_player_index
```
Nothing else follows.

| item | client action | constraint |
|---|---|---|
| profiles | deserialized into match_options.profile[i], i=0..N-1 | N ≤ 20 (array of 20; more overwrites C440…) [V] |
| profile `u8 team` (+5AC) | `game_team_id`; spawn point selection (`spawn_player` 0x1FCC05), kd winner (team==0 vs ≠0, 0x1FD87D), HUD ally/enemy (team == local team → 1 else 2, 0x1816C7) | **0 or 1**; every map has respawn points only for team 0 and 1 (10–13 each) [V data] |
| profile `bool` (+5B0) | **overwritten**: `P[i].+5B0 = (i == local_idx)` [V 0x1A2E3B] | send 0 |
| local_idx | the profile with +5B0 becomes player+131; `on_match_ready` sets `+7A04` active player and core `C80C = idx` [V 0x1A2723] | **must be < N** — else +7A04 stays NULL and `on_match_ready` dereferences it (0x1A2776) unless stats gathering is disabled → crash [V] |
| C444 | `switch_to_level_loading` (skipped in journal replay) | |
| — | `query_players` → `game::load(C460, …)` with pvp_match_core request (class 0x5E) → on load `on_match_ready` → 0x42 | |

Resources requested by `pvp_match_core_cook::translate_query` 0x1C7889 [V]: class 0x41 project `C460`; 0x52 `game_material_manager`; **0x59 gather_victory_items rule, or 0x5A match_time_vi_spawn rule when C448==3** (network matches always add it: query flag byte = 1 set in `query_players` 0x1A2855); 0x5B player_respawn_rule; 0x5C timelimit_rule; 0x5D kd_stats_rule; N × 0x55 `gameplay/players/default.player`. The rule resources have empty names: built in code from match_options, not from data files.

### Concrete 0x51 example: level_01 day, mode 2, 2 players (client A gets idx 0)
Payload after `51 <u16 order>` (little-endian):
```
01                                  C444 level_id = 1  (Level1_day picture)
0C 6C 65 76 65 6C 5F 30 31 2F 64 61 79   C460 "level_01/day"
02                                  C448 mode = gather_victory_items
02                                  C44C N = 2
05                                  C45B batteries = 5
0A                                  C44D respawn 10 s
58 02                               C44E match 600 s
78 56 34 12                         C440 seed 0x12345678
00 00 00 00                         C454 start fraction 0.0
05                                  C458 waiting 5 min
00                                  C459
0A                                  C45A countdown 10 s
00 00                               C450
00 00 C8 42  00 00 48 42  00×4 …    C480 24 × f32 (kill 100.0, assist 50.0, …; 96 bytes)
00 00                               C4E0 squad ids
<profile 0: team 00, bool 00, name, u32 0, slots…, mods>   deserialize_static as lobby.rs
<profile 1: team 01, bool 00, name, u32 0, slots…, mods>
00                                  local_player_index (client B: 01)
```
match_options = 129 + N bytes here; two typical loadouts (~100–200 B each) usually fit one 508-B datagram, otherwise use 0xFF multipart (`match.md` §1).

---

## 3. Maps [V data + V code]

`C460` = `level_0X/<variant>`, variant ∈ `day`, `rain`, `evening` (all 6 maps have all three: `projects/level_0X/{day,rain,evening}/{client_project,server_project}` in resources.db). `level_0X` alone would fall back to `<name>/default/client_project`, which does not exist → do not use.

`C444` → `lobby_menu::switch_to_level_loading` switch 0x172404 → frame of `LobbyMenu._load_images` (lobby_menu.swf `LobbyMenu.as:161`); unmapped ids → frame 0.

| map | C460 | C444 day / rain / evening | spawners | containers | respawn pts t0/t1 | anomalies |
|---|---|---|---|---|---|---|
| level_01 | `level_01/day` … | 0x01 / 0x13 / 0x14 | 37 | 2 | 11/13 | 7 |
| level_02 | `level_02/…` | 0x03 / 0x15 / 0x16 | 51 | 2 | 10/10 | 2 |
| level_03 | `level_03/…` | 0x05 / 0x09 / 0x08 | 45 | 2 | 11/11 | 5 |
| level_04 | `level_04/…` | 0x0A / 0x17 / 0x18 | 33 | 2 | 11/11 | 1 |
| level_05 | `level_05/…` | 0x19 / 0x1B / 0x1A | 30 | 2 | 10/10 | 1 |
| level_06 | `level_06/…` | 0x1D / 0x24 / 0x23 | 32 | 2 | 10/10 | 2 |

(Counts from the `day` server_project; `victory_items_containers`: team 0 and team 1 [V level_01].) `team_base_points[0]` on every map → no data for base-capture modes. C444 is cosmetic: a mismatch with C460 only shows the wrong loading picture [V].

---

## 4. Handshake to the first controllable tick

### Time model [V]
- Core tick grid: `C7F0 = 50` (ctor 0x1CA5E7) is the grid origin; ticks are at 50 + k·50. `clear()` 0x1CA788 sets C7F4 (state time) = C7F8 (newest simulated time) = 50, C804 (snapshot time) = 0, C7FC = −1.
- Game time = the client's `floating_timer` ms. `0x52` → `game::set_time_in_ms(match_time)` 0x1548D2. Inputs are stamped with this clock (local `change_input` in `game_world::tick` 0x15AAC8; `try_send_all` quantizes to /50·50 0x1A3135).
- `on_idle(t)` 0x1A3360 moves the core to `floor50(t − C7EC − 50) + 50` when `C7F8 + C7EC < t`; `game_world::tick` also calls `pvp_match_core::tick` 0x15AB69. C7EC = 500 on clients.
- **match_time is absolute ms on the same timeline as snapshot/input/enter times.** The server keeps one match clock M (start 50 at match open) and puts M_now in 0x52 (optionally + RTT/2). `echo` is free for the server (e.g. its send time) and comes back in 0x43 for RTT.

### 0x52 set time — `process_set_time_request` 0x1A2CF0 [V]
`set_time_in_ms(match_time)`, sends 0x43 `{echo, floating_timer_ms}`, flushes, then (requires `+7A00` match core; in non-replay mode no wait → **null deref if before 0x42**) `game_world_core::clear()` and resets the hash/clock send timers (+7A10/+7A14 = 0). Any later 0x52 **wipes the world history** → must always be followed by a fresh 0x53.

### 0x53 snapshot — `process_dynamic_match_info` 0x1A2E88 → `game_world_core::deserialize` 0x1CAB3B [V]
- Requires `+7A00` (no state check besides 7A1C==2) → after 0x42.
- Calls `clear()`, reads one `game_state_history_item` (layout `match.md` §3), inserts it, records the active mask at T0, sets **C7F8 = C804 = T0** (C7F4 = T0 via private deserialize 0x1CC7B2). First time: C80E=0 ("THE VERY FIRST SYNCHRONIZATION"), and for every player in the mask an input-history item at T0 copied from player+2E8 (last input) is inserted. Later: "FULL RESYNC" + `on_resync` UI callback.
- Camera: game status (+7A18) == 3 → `switch_to_player_camera`, else `switch_to_warmup_camera` [V 0x1A2EC0].
- **T0 must be on the grid (T0 ≡ 0 mod 50, T0 ≥ 50).** With T0 = 0, `tick_events` computes the next tick as 50 + ((0−50)/50+1)·50 = 4 ms (u32 wrap) → off-grid world [V arithmetic 0x1CC49D]. **T0 = 0 is not fine; use 50** (the client's own initial world time).
- The client then fast-forwards T0 → current time ("large move" log), applying all inputs/enter/leave events with time > T0.

### 0x56 player entered — `enter_match` 0x1CB9BD [V]
| check | result |
|---|---|
| T ≤ C804 (snapshot T0) | rejected (2) |
| client: T + 500 ≤ C7F8 (newest simulated time) | rejected (2) |
| player bit already set in the mask at T | rejected (4) — harmless for players already in the snapshot mask |
| else | enter event inserted at T (rollback if needed) |
No upper bound on T. Effect at T (`on_player_entered_match` 0x1CC1B0): mask bit set, player added to the active (ticked) list `C104` (`make_client_active`), rules notified: **respawn rule spawns the player at exactly T** (0x1FCBB9 → tick 0x1FCA6C → `spawn_player`), timelimit rule sets its mask bit and re-evaluates.
**A player that is neither in the snapshot mask nor entered by 0x56 is never ticked or spawned** → the local player must be entered to play [V]. Send 0x56 `{idx, T}` for every connected player with T0 < T and T ≥ (client newest time − 500); T = server clock now is safe.

### Warm-up / game status — `timelimit_rule_core` 0x1F549E..0x1F590F [V]
State (+114) = `game_status` enum (@4921c7c): 0 inactive, 1 waiting_for_players, 2 final_countdown, 3 inprocess.
- `on_match_ready(C7F0=50)` → state 1, start=50 (from `pvp_match_core::on_match_ready` 0x1DCCD0).
- state 1 → 2 when mask ≠ 0 and popcount(mask) ≥ floor(N·C454); start = t. Log "Countdown: [%d] Match begins in %d".
- state 2 → 3 when t ≥ start + C45A·1000; start += countdown. Log "Match started".
- `rule_completed(t)`: state 1: t−start ≥ (C458+C459)·60000; state 3: t−start ≥ C44E·1000; state 2: never.
- `timelimit_rule::tick` 0x197FAA calls `network_client::set_game_status(state)` when the state changed and t > C7F8 (not during rollback re-sim). `set_game_status` 0x1A29D0 is purely local: state 3 → player camera, else warm-up camera. **No server message drives warm-up; it is all simulation + 0x56.**
- Fastest start: C454 = 0.0, C45A = 0 → the first 0x56 starts the countdown and the match is "in process" one logic tick later.
- The snapshot carries the rule state (`m_state_start_time_ms`, `m_current_time_in_ms`, `m_players_mask`, `m_current_state`), so a late joiner sees the current phase.

### Other messages [V]
- 0x55 `i16` → `floating_timer::set_offset` 0x1A1626 (clock nudge).
- 0x57 `{u8 idx, u32 T}` → `leave_match`; if idx is the local player: `close_current_match(3)` and lobby `just_left_match` = 1 (0x1A3098..0x1A30A8).
- 0x58 `{u32 T}` → `discard_input(local_idx, T)`.

---

## 5. Game modes

There is **no per-mode rule selection beyond one comparison**. Enum `game_mode` @42e3a44 [V]: 0 capture_enemy_base, 1 capture_neutral_base, 2 gather_victory_items, 3 match_time_vi_spawn, 0xFF invalid_game_mode (ctor default).

| C448 | name | rule objects created (network) | win / end condition | mode-specific data | maps | confidence |
|---|---|---|---|---|---|---|
| 0 | capture_enemy_base | timelimit, **gather_victory_items (0x59)**, player_respawn, kd_stats | same as mode 2 (no base-capture rule exists in the client) | none; `team_base_points[0]` everywhere | all 6 (behaves as mode 2) | rules [V 0x1C79D8]; "behaves as 2" [I] |
| 1 | capture_neutral_base | same as 0 | same as mode 2 | none | all 6 (as mode 2) | [V]/[I] |
| 2 | gather_victory_items ("batteries") | timelimit, gather_victory_items (0x59), player_respawn, kd_stats | C45B batteries placed at match start on spawners chosen by priority + seed C440 (`select_spawners` 0x1FB90D). Event 1 = battery put into / taken out of a team container (±1 per team, 0x1FB8A9). **Completed when one team's container holds all C45B batteries** (`+15C` = t) or by time (timelimit state 3, C44E s). Winner: team with more batteries (0 if t0>t1, 1 if less, 2 = draw) [V 0x1FBF3E] | C45B, C440 | all 6 (2 containers, 30–51 spawners each) | [V] |
| 3 | match_time_vi_spawn | timelimit, **match_time_vi_spawn_rule (0x5A, subclass of gather)**, player_respawn, kd_stats | batteries appear over time: one every `spawn_time` 30 s ± `spawn_variation` 10 s (from `gameplay/victory_items/default.options` `parameters.spawn_parameters`, read at +1CC/+1D0 ×1000, 0x1FC728), LCG on seed, until C45B are out (0x1FC388). Same completion/winner as mode 2 | C45B, C440 | all 6 | [V] |

Common to all modes [V]:
- **Time limit** = C44E seconds of "in process" (+ waiting C458+C459 min, countdown C45A s). No score limit field exists; scores (C480) are display only.
- **Rule priority for the winner** = `game_world_core::winner_team` 0x1CEE03 picks the rule with the largest vfn+0x4C value and returns its `winner_team`; kd_stats winner = team with more kills (team 0 vs ≠0) 0x1FD84C; respawn rule always returns 3 (undefined) [V]. Which rule wins the priority is not resolved [U].
- **Team sizes**: N ≤ 20 total (profile array 20, respawn arrays 0x14, mask u32); teams 0 and 1; ≥10 respawn points per team per map → ≤ 10 per team recommended [I].
- **UI per mode**: only `root.set_game_type(mode)` in hud.swf → `progress_indicator.gotoAndStop(mode+1)` (HUD test code uses type 3 with 6 artifacts) [V]; `set_artifacts_required(C45B)`. There are **no mode-name localization keys** (`localization/english`: none; exe: none) and the lobby has no mode/queue UI per mode (lobby status 1 shows only `st_mm_*` place/wait texts, lobby_spec §3.0) [V]. In-match texts: `st_waiting_for_players` ("Awaiting other players:"), `st_final_countdown` ("Awaiting match start:"), `st_victory`, `st_label_teamA/B`, `st_stats_items_gathered`, `st_stats_stolen_victory_items` [V]. Faction quests mention "at least 6/7/8 batteries in your base" → real matches likely used C45B ≥ 8 [I].

---

## 6. Match end and return to lobby

- **No client code calls `rule_completed`** (only defined; no caller found for the vtable slot, and no client path ends the match on time/batteries) [V negative search, I]. The end decision was server-side (the server variant of the core).
- `close_current_match(type)` 0x1A23B8 (`disconnect_event_types_enum` @4753d80: 0 timeout, 1 connection_lost, 2 initiator, 3 on_match_finished, 4 on_reconnect); does nothing if +7A18 (game status) == 0 [V]:
  | type | action |
  |---|---|
  | 0 timeout | `game_world::on_finish_match` |
  | 1 connection_lost | log, `just_left_match`=1, on_finish_match |
  | 2 initiator | **send 0x47 `{u32 floating_ms}`**, flush, log, just_left=1, on_finish_match |
  | 3 on_match_finished | on_finish_match + `lobby_menu::show_match_statistic` (victory screen) — no 0x47, no just_left here |
- Triggers: 0x57 for the local index → `close_current_match(3)` then `just_left_match`=1 [V 0x1A3098]; transport disconnect → `on_match_disconnected(type)` 0x16DD64 → `close_current_match(type)` [V]; in-game "finish_match_clicked" (Flash, not in hud.swf) → `on_finish_match` 0x15BD44 → match client disconnect → (via callback) `close_current_match` [I].
- `on_finish_match`: match client disconnect if connected, `game_world::unload`, sets lobby+0xC0=1 if the lobby is not connected, `game::switch_to_lobby` [V].
- `just_left_match` (lobby_client+0x5A5C) is sent once as 0xFF in the next lobby sign-in 0x26 and then reset (lobby_spec §1.2). Meaning: "returning from a match" → the lobby should send status 0 and fresh profiles; stats come only via messaging `#mstats N` → `query_match_stats` 0x176619 [V].
- **Server end procedure (recommended):** at end time (timelimit state 3 elapsed, or battery win if the server tracks it), send **0x57 `{local_idx, T_end}` to each client** → each shows the victory screen and leaves cleanly; then close the UDP connection; set lobby status 0. Clients that leave early send 0x47 (relay 0x57 {idx, T} to the others).

---

## 7. Late join

Allowed by the client [V]: the roster is fixed; a later player is just entered later.
Server must send the late joiner, after its 0x42: 0x52 (M_now) → 0x53 snapshot at a grid time T_s (any T_s ≤ M_now; the newer, the shorter the fast-forward) → **every relayed input with time > T_s** (0x54; older ones are rejected as ≤ C804) → 0x56/0x57 for enter/leave events with time > T_s (players entered before T_s must be in the snapshot mask) → 0x56 `{joiner_idx, T ≥ M_now − 500}` to everyone including the joiner. Inputs after the snapshot must satisfy T + 500 > C7F8 (C7F8 = T_s right after the snapshot, so all are accepted).
The snapshot must be byte-exact (match.md §3) → late join needs a snapshot source (hosted core or patched-client upload). Without one, only the skip-snapshot patch start works and late join is impossible.

---

## 8. Recommended defaults

| field | value | why |
|---|---|---|
| C444 | per map/variant table §3 | picture |
| C460 | `level_0X/day` | must exist |
| C448 | 2 | real battery mode; 3 for timed spawns |
| C45B | 5 (≤10) | HUD icon range, ≤ spawners |
| C44D | 10 | respawn s |
| C44E | 600 | match s |
| C440 | random u32 | same for all clients |
| C454 | 0.0 | start on first entered player |
| C458/C459 | 5/0 | waiting timeout (server-side meaning only) |
| C45A | 10 (0 = instant) | countdown s |
| C450 | 0 | unused |
| C480 | §1 table | display only |
| C4E0 | 0 | no squads |
| team | 0/1 alternating | spawn points exist for 0/1 |
| T0 | 50 | grid origin, client initial world time |

## Open questions
- Who decides the end in the original server (rule priority between timelimit / gather / kd) [U]; whether a server-side transport disconnect arrives as type 2 or 3 [U].
- C458 vs C459 split, C450 meaning [U]. Real score weights C480 [U].
- HUD `progress_indicator` frames for modes 0/1 (cosmetic) [U].
