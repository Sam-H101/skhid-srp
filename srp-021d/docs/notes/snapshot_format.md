# 0.21d world snapshot: byte layout for srp's decoder

Scope: the body that `game_state_history_item::serialize(writer, mask)` 0x1FDE3E writes. That is the same body as a 0x53 snapshot minus its leading `u32 time`. srp needs two things from it: **(A) end-of-match results** from the *statistics* section, and **(B) rule state** from the *game_rules* section (batteries, time limit, kills).

Tags: **[V]** = read in the disassembly (address given); **[I]** = inferred; **[U]** = unknown.

Conventions [V]:
- All integers are **little-endian** raw `memcpy`s (`buffer_writer::w` 0x155EBE).
- `f32` = IEEE single.
- `bool` = 1 byte, written as **0x00 or 0xFF** (`w<bool>` 0x155EF9: `sete; dec`). Any non-zero byte reads as true (`r<bool>` 0x155E6E).
- `float3` = 12 bytes (`w<float3>` 0x1E8024, size 0x0C).
- The `serialization_operation_descriptor` / `aggregate` calls are debug-only (`g_debug_hash_mismatches`) and write **nothing** to the buffer.

Inputs the decoder needs that are **not** in the snapshot:
- `N` = roster size (match_options `C44C`).
- `NB` = battery count (`C45B`).
- the game mode (`C448`, where 3 = match_time_vi_spawn).
- `T` = the snapshot's own state time. This is the `u32` in front of the body in 0x53 (`deserialize` reads it into +B9938 at 0x1FE041). The patched client must send it along with the body.

---

## 0. Time fields ("time_offset" encoding) [V]

`game_world_core::serialize` 0x1CC570 passes every sub-serializer the argument `time_offset = 1_000_000_000 − T` (0x3B9ACA00 − C7F4, 0x1CC59C/0x1CC5C5). Every absolute time field `t` is written as:

| stored `t` | wire value |
|---|---|
| `u32::MAX` (unset) | `0xFFFFFFFF` (timelimit, respawn) |
| `0` (unset) | `0` (stats `last_killed_enemy_ms`, curing `last_canceled_affect_time`) |
| otherwise | `t + 1e9 − T` (mod 2³²) |

Decode: `t = wire − 1_000_000_000 + T`, or equivalently `t − T = (wire as i64) − 1e9` (negative means in the past). The strings in the exe confirm the rule, e.g. `"last_killed_enemy_ms ? last_killed_enemy_ms + time_offset : 0"` and `"(m_state_start_time_ms==u32(-1)) ? m_state_start_time_ms : m_state_start_time_ms+time_offset"`.

**Exceptions (raw absolute ms, no offset):** the three times of `match_time_vi_spawn_rule` (§3.2).

---

## 1. Outer layout: `game_state_history_item::serialize` 0x1FDE3E [V]

| # | type | content | ref |
|---|---|---|---|
| (0x53 only) | u32 | `T` (state time). **Not written by `serialize`**; read first by `deserialize` 0x1FE033 | 0x1FE041 |
| 1 | u32 | `active_clients_mask` (the `mask` argument) | 0x1FDECA |
| 2 | for each set bit `i` of the mask, **lowest bit first** (`x & −x` → `bit_index`): `player_history_item` of roster index `i` | | 0x1FDED6..0x1FDEFB |
| 3 | u16 `L_b` + `L_b` bytes | bullet_manager | 0x1FDF23 |
| 4 | u16 `L_s` + `L_s` bytes | **statistics** (§2) | 0x1FDF64 |
| 5 | u16 `L_o` + `L_o` bytes | game_objects | 0x1FDFA5 |
| 6 | u16 `L_r` + `L_r` bytes | **game_rules** (§3) | 0x1FDFE6 |
| — | end of body | nothing follows; the last `aggregate` is debug-only | 0x1FE013 |

Each section is a byte copy of a buffer that `game_world_core::serialize` 0x1CC570 built earlier. Their capacities (history item ctor, around 0x1FDB73..0x1FDC1F) are: statistics 0xC00, game_rules **0x200**, game_objects 0x2000 [V].

### 1.1 `player_history_item::serialize` 0x20C18F [V]

| # | type | content |
|---|---|---|
| a | u16 `L_p` + `L_p` bytes | player_serialized_state (+804 data, +80C size) |
| b | bool (0x00/0xFF) | `animation_tree != 0` (+1050) |
| c | **only if b is true:** compressed animation state, `animation_player::serialize_state_and_compress` 0x24D2BB | |

The animation state has **no byte length**. It is a self-delimiting, MSB-first bitstream (the `n_ary_tree_serializer` ctor 0x24E1EA). To skip it:

The bit writer is `append(value, nbits)` 0x24D52A. It writes the low `nbits` of `value` **most-significant bit first**, filling each byte from bit 7 down to bit 0.

| part | bits | meaning |
|---|---|---|
| header | 16 | `H` = number of tree bits that follow the header (patched in at 0x24E4F5..0x24E556: `(cur−start)·8 − bitpos − 9` = bits used − 16) |
| tree | `H` | the n_ary_tree; skip it |
| times (`save_times_in_ms` 0x24DD35) | see below | |
| floats (`save_floats` 0x24DF70) | see below | |
| pad | to a byte boundary | the buffer is resized to `ceil(bits/8)` (0x24E57F) |

`W(x) = max(1, ceil_log2(x))` for x ≥ 1 (`round_up_to_the_next_power_of_two` 0x24D46A, then `bit_index` 0x24D48F). For example W(1)=1, W(2)=1, W(3)=2, W(5)=3.

**Times section [V 0x24DD35]:**
```
n     : 10 bits                 // number of times
flag  : 1 bit
first : 32 bits                 // min(times)
if n != 1:
   w  : 5 bits
   if flag == 0:  n × w bits    // t − first
   else:          u : W(n+1) bits            // unique count
                  (u−1) × w bits             // sorted deltas − 1
                  n × W(u+1) bits            // index into unique list
```

**Floats section [V 0x24DF70]:**
```
flag  : 1 bit
m     : 10 bits                 // number of floats
if flag == 0:  m × 32 bits
else:          k : W(m+1) bits               // unique count
               k × 32 bits
               m × W(k+1) bits
```

The total length is `ceil((16 + H + times_bits + floats_bits) / 8)` bytes [V layout; I that no other field follows, since the ctor's last write is `save_floats` at 0x24E562 and then the resize].

**Practical shortcut [I]:** srp does not need the player sections. It can still only reach the statistics by walking through them, so the anim-skip above is required, or else the patched client must also send section offsets.

### 1.2 Corrections to match.md §3
- **Confirmed:** the u16 length prefixes on all four tail sections; the player triple (u16 len + state, bool, anim).
- **Corrected:**
  - `serialize` does **not** write `time_ms`; only the 0x53 message (and `deserialize`) has it.
  - The bool is `0xFF` for true, and the anim block is present **only** when the bool is true.
  - The anim block has no byte-length prefix: it is a bitstream whose first 16 bits (big-endian bit order) give the tree's bit length, followed by a self-describing times section and floats section (§1.1).

---

## 2. Statistics section [V]

Contents: `base_game_statistics_handler::serialize` 0x19C365 (vtable +0x28; it tail-jumps) → `shared_statistics::serialize` 0x1DE11C on handler+8.

**`L_s == 0`** means the core had no statistics handler (`C7E0 == 0`, skipped at 0x1CC617). That happens when stats are disabled, or after `base_game_statistics_handler::finish_match` 0x19C19C cleared `C7E0` (0x19C1CD). srp should keep the last non-empty statistics block.

### 2.1 `shared_statistics::serialize` 0x1DE11C

| # | count | element | ref |
|---|---|---|---|
| 1 | `N` (stats+1A38 = C44C, set by `start_match` 0x1DDAEE) | `player_shared_statistics` (§2.2), roster index 0..N−1. **Every roster slot is written, active or not** | 0x1DE12A..0x1DE150 |
| 2 | `N` (match_options C44C via stats+1600) | `curing_event_status` (§2.4), index 0..N−1 | 0x1DE152..0x1DE17D |
| 3 | 0 or 1 | `victory_item_event_manager` (§2.5). Present iff its +3F8 (the gather rule) is set, i.e. the core has a rule of type 1 (`game_rule` 0x1CAAED, `start_match` 0x1DDB51). **Always present in network matches**, where the gather or mts rule always exists [I from §3.1] | 0x1DE185 |
| 4 | 1 | u8 `m_team_points[0]` (stats+1A30) | 0x1DE1B7 |
| 5 | 1 | u8 `m_team_points[1]` (stats+1A31) | 0x1DE1D2 |

`deserialize` 0x1DE1E0 mirrors this [V].

The statistics team points change with victory item events (0x1DDED5):
- `brought` (type 0): +1 for the carrier's team.
- `stolen` (type 1): −1 for the other team.

These are the stats copy. The gather rule's own copy (§3.1) is the one `winner_team` uses.

### 2.2 `player_shared_statistics::serialize` 0x20569F (struct stride 0xF0)

| off in struct | wire | field |
|---|---|---|
| +0x00 | 24 × u16 | `event_count[e]`, e = `match_stats_events_dict_enum` 0..23 (table §2.3) |
| +0x30 | var | `intermediate_shared_statistics` (§2.2.1) |
| +0xE8 | 7 × u8 | `faction_representative_count[0..6]` |

#### 2.2.1 `intermediate_shared_statistics::serialize` 0x205334

| # | type | field | note |
|---|---|---|---|
| 1 | u8 `a` | inflictors_count = number of non-zero entries in `damage_from[20]` (f32 at +0x00..+0x50) | NaN counts as non-zero |
| 2 | `a` × {u8 idx, f32 damage} | damage received from roster idx | ascending idx |
| 3 | u8 `b` | hit_receivers_count = number of non-zero `hit_time[20]` (u32 at +0x50..+0xA0) | |
| 4 | `b` × {u8 idx, u32 time} | last hit on roster idx; **time_offset-encoded** | ascending idx |
| 5 | u32 | `last_killed_enemy_ms` (+0xA0): 0 if unset, else time_offset-encoded | |
| 6 | u8 | `last_hit_enemy_id` (+0xA4) | |
| 7 | u16 | `killstreak` (+0xA6), the **current** streak (+1 per kill, 0x1DDD7C) | |
| 8 | f32 | `player_action_coef` (+0xA8) | |

So each player is `48 + 1 + 5a + 1 + 5b + 4 + 1 + 2 + 4 + 7` = **68 + 5(a+b)** bytes.

### 2.3 The 24 counters (enum @41825f0 [V]) and how they move

`counter[e] += value` (`shared_statistics::on_event` 0x1DE2E8, u16 wrap). Score weight factor: `k = 0.01` for events 6, 17 and 18, else 1.0.

| e | name | increment | source |
|---|---|---|---|
| 0 | kill | +1 to the killer when the victim is on another team | on_player_killed 0x1DDCED |
| 1 | assist | from `give_assists` | 0x1DDD1A |
| 2 | death | +1 to the victim | 0x1DDC99 |
| 3 | suicide | killer == victim | 0x1DDCAA |
| 4 | team_kill | +1 to the killer when on the same team | 0x1DDD90 |
| 5 | artefact | artefacts collected | [I] on_artefact_took |
| 6 | bring_victory_item | battery put in a container when nobody owned it (owner_team == 3): `floor(share·100+0.5)` per contributing carrier | 0x204F5F, 0x2051D5 |
| 7 | teammate_cure | | [I] |
| 8 | teammate_rescue | killer avenged a teammate within 2 s (0x7D0) | 0x1DDD4D |
| 9 | revenge | killed own killer within 2 s | 0x1DDD6D |
| 10 | victory_item_carry_kill | | 0x1DDD0A |
| 11 | participating | +1 at `finish_match` per active-mask player | 0x1DDBBD |
| 12 | won | +1 at `finish_match` if the player's team == winner | 0x1DDBE1 |
| 13 | melee_kill | | [I] |
| 14 | silent_kill | | [I] |
| 15 | headshot_kill | | 0x1DDCF8 |
| 16 | winning_team_vi_bonus | +1 × stats `team_points[winner]` at `finish_match` for winners | 0x1DDBF3 |
| 17 | steal_victory_item | battery put in a container, owner_team ≠ the player's team: share·100 | 0x204F64 |
| 18 | recover_victory_item | battery put in a container, owner_team == the player's team: share·100 | 0x204F64 |
| 19 | victory_item_found | first `spotted` event of that battery | 0x204E72 |
| 20 | put_victory_item_in_container | +1 per battery the player put into a container (type 0 brought) | 0x204E8F |
| 21 | entrap_enemy | | 0x1DDEC2 |
| 22 | bullet_fire | | [I] |
| 23 | bullet_hit | | 0x1DDE9C |

Events 11, 12 and 16 are added **only by `finish_match`**, which no snapshot taken during the match contains. srp must apply them itself (§4.4).

**Score** is not serialized. After `deserialize` the client recomputes it (`recalculate_scores` 0x19C410):
`score_i = u16( floor( Σ_e counter_i[e] · C480[e] · k_e + 0.5 ) )`. C480 is the 24 × f32 weights from match_options.

### 2.4 `curing_event_status::serialize` 0x20C34D (stride 0x28, array at stats+12E0)

| type | field |
|---|---|
| u32 | `last_canceled_affect_time`: 0 if unset, else time_offset-encoded |
| u16 | `m_medkits_active` |
| bool | `m_event_conditions_are_met` |

That is 7 bytes × N.

### 2.5 `victory_item_event_manager::serialize` 0x204FAC (at stats+1608; entries at +0x20, stride 0x60, at most 10)

The count is **NB** (`C45B`). For each battery j in 0..NB−1:

| type | field |
|---|---|
| u32 | `players_mask`: bit p set iff `carried_distances[p] > 0` (p < N) |
| popcount(mask) × f32 | `carried_distances[p]` for the set bits, ascending p |
| f32 | `nearest_positions[team_1]` (+0x50) |
| f32 | `nearest_positions[team_2]` (+0x54) |
| u8 | `owner_team` (+0x58; 3 = undefined, set to the carrier's team when brought, 0x204FA1) |
| bool | `already_found_once` (+0x5C) |

---

## 3. game_rules section [V]

`game_world_core::serialize` calls `rule->vfn+0x3C(writer, time_offset)` for every rule, **in vector order** (C160..C164, 0x1CC656). There are no per-rule lengths or tags.

**Vector order [V].** The rule pointers come straight from the cook's query results:
- `pvp_match_core_cook::translate_query` 0x1C7889 requests resources in this order: 0x41 project, 0x52 materials, then **[0x59 gather_victory_items, or 0x5A match_time_vi_spawn when C448==3]** (only when the network flag is set), 0x5B player_respawn, 0x5C timelimit, 0x5D kd_stats, then N players.
- `on_resources_loaded` 0x1C7426 takes results 2 .. 2+count (count = 3 + flag, 0x1C7572).
- It passes them unchanged through the `pvp_match_core` ctor 0x1DC957 into the `game_world_core` ctor fixed_vector 0x1CA541.

That gives:

| idx | rule (network match) | vtable | serialize | size |
|---|---|---|---|---|
| 0 | gather_victory_items_rule (mode ≠ 3) / match_time_vi_spawn_rule (mode 3) | 0x820BFC / 0x820D2C | 0x1FBA38 / 0x1FC42F | variable |
| 1 | player_respawn_rule | 0x820FFC | 0x1FCDDB | 84 + 2R |
| 2 | timelimit_rule (timelimit_rule_core::serialize) | 0x8144FC | 0x1F5569 | **13** |
| 3 | kd_stats_rule | 0x82113C | 0x1FD6CE | **5·N** |

[I] This assumes queries_result keeps request order, which is the normal vostok behaviour. The pass-through itself is [V].

**Robust decode:** parse kd_stats from the **end** (the last 5N bytes) and timelimit from the 13 bytes before it. Parse gather/mts from the **start**. The respawn rule fills what is left (a consistency check: `L_r = len(gather) + 84 + 2R + 13 + 5N`).

### 3.1 `gather_victory_items_rule::serialize` 0x1FBA38 (mode 0/1/2)

| # | type | field | rule offset |
|---|---|---|---|
| 1 | u32 | `m_spawner_random.seed()` | +140 |
| 2 | u8 | **`m_team_points[0]`** = batteries in **team 0's** container | +144 |
| 3 | u8 | **`m_team_points[1]`** = batteries in **team 1's** container | +145 |
| 4 | u16 | `items_mask`: bit i set iff battery i has **no carrier** (`item+8 == 0`); i over all items (+110..+114) | 0x1FBAA0 |
| 5 | for each item with a mask bit, ascending i | `victory_item_core::serialize` (§3.1.1) | 0x1FBB05 (vfn+0x50 = 0x1E7A74 for both victory_item and victory_item_core [V vtables 0x81269C, 0x81D7EC]) |
| 6 | for each container in rule order (+130..+134; 2 on every map) | `victory_items_container_core::serialize` (§3.1.2) | 0x1FBB2F |

Team points change on `gather_victory_item_event` (id 1, `on_event` 0x1FB8A9):
- `team_points[container.team] += 1` on put, `−= 1` on take.
- The event data is `{container+160 team, flag}`; take is sent from `victory_item_core::initialize` 0x1E75EF with flag 1.

So **`m_team_points[t]` is the battery count in team t's container** [V].

#### 3.1.1 `victory_item_core::serialize` 0x1E7A74 (here always with no carrier)

| type | field |
|---|---|
| u32 | `m_spotted_mask` (+1B4) |
| u8 | carrier roster id. **Always 0xFF here**, because carried items are not in the mask |
| u8 | container index in the rule's container vector (`get_container_id`), or **0xFF** = lying in the world |
| *if container == 0xFF:* float3 | `m_position` (+1A0) |
| *if container == 0xFF:* f32 | `m_orientation` (+1AC) |
| *if container == 0xFF:* usable_object | §3.1.3 |

That is 6 bytes for a battery in a container and 23 + users bytes for one lying in the world. (The carried-item path, `fsm::serialize` plus the state's vfn+0x14, never occurs in this section.)

#### 3.1.2 `victory_items_container_core::serialize` 0x1F7BE6

| type | field |
|---|---|
| usable_object | §3.1.3 |
| u32 `c` | `m_victory_items.size()` = batteries in this container |
| `c` × u8 | battery index (`get_victory_item_id` 0x1FBDBA = position in +110) |

The container's team is **not** on the wire. It comes from the level data (`load` 0x1F7A14: `team` → +160). Data order of `victory_items_containers` [V data]:
- level_01, 02 and 04: [team0, team1].
- **level_03, 05 and 06: [team1, team0]**.

Use `m_team_points[]`, not the container order. Container order is data order [I].

#### 3.1.3 `usable_object::serialize_usable_object` 0x1E8B65
`u8 n` (+28), then `n × u8` user roster id, walking the user list.

### 3.2 `match_time_vi_spawn_rule::serialize` 0x1FC42F (mode 3)

| # | type | field |
|---|---|---|
| 1 | u32 | spawner seed (+140) |
| 2 | u8 | **team_points[0]** (+144) |
| 3 | u8 | **team_points[1]** (+145) |
| 4 | u32 | `m_available_victory_items_count` = A (+168) |
| 5 | u32 | `m_match_start_time_ms` (+160), **raw absolute** |
| 6 | u32 | `m_victory_items_spawn_time_ms` (+16C), raw |
| 7 | u32 | `m_victory_items_spawn_variation_ms` (+170), raw |
| 8 | u16 | items_mask over the **first A items only** |
| 9 | items | as in §3.1 #5 (first A items) |
| 10 | containers | as in §3.1 #6 |

Completion and winner are inherited from gather (`rule_completed` 0x1FBF1F, `winner_team` 0x1FBF3E, vtable 0x820D2C).

### 3.3 `player_respawn_rule::serialize` 0x1FCDDB

| type | field |
|---|---|
| u32 | `m_respawn_random.seed()` (+178) |
| 20 × u32 | `m_player_respawn_times[i]` (+128): 0xFFFFFFFF or time_offset-encoded |
| R × (u8, u8) | per respawn point, ascending `point_id` (std::map, +110): `players_checker::serialize` 0x20BF22 → u8 `m_inside_objects_count`, for two checkers (+24, +28; ally and enemy zone [I]) |

R = the number of distinct `point_id` values in `server_objects.respawn_points` (`resolve_links` 0x1FCF65 → map insert_unique) [V]. The day projects give R = 24 / 20 / 22 / 22 / 20 / 20 for level_01..06 [V data; uniqueness I]. So the size is 84 + 2R.

### 3.4 `timelimit_rule_core::serialize` 0x1F5569 (13 bytes)

| # | type | field | meaning |
|---|---|---|---|
| 1 | u32 | **`m_state_start_time_ms`** (+118), 0xFFFFFFFF or time_offset-encoded | start of the current phase |
| 2 | u32 | `m_current_time_in_ms` (+11C), same encoding | time of the last tick. Normally the snapshot time, so the wire value ≈ 1_000_000_000. **A good sanity check** [I] |
| 3 | u32 | `m_players_mask` (+120) | players that entered (on_player_entered/left 0x1F56B6/0x1F56DA) |
| 4 | u8 | **`m_current_state`** (+114) | enum @4921c7c: 0 inactive, 1 waiting_for_players, 2 final_countdown, 3 inprocess |

The phase logic (`logic_tick` 0x1F5700, details in match_start_spec §4) [V]:
- 1 → 2 when `popcount(mask) ≥ floor(N·C454)` and mask ≠ 0; start = t.
- 2 → 3 when `t ≥ start + C45A·1000`; then start += countdown.
- There is **no state 4**: "finished" exists only as `rule_completed`.

### 3.5 `kd_stats_rule::serialize` 0x1FD6CE (5 bytes × N, +1B0 = C44C)

For p in 0..N−1:

| type | field |
|---|---|
| u16 | `m_kd_stats[p].kills` (+110+8p) |
| u16 | `m_kd_stats[p].deaths` (+112+8p) |
| bool | `m_kd_stats[p].online` (+114+8p; set or cleared on enter/leave 0x1FD820/0x1FD836) |

`on_event` 0x1FD656 (player_killed):
- deaths[victim] += 1 always.
- kills[killer] += 1 only if killer ≠ 0xFF, killer ≠ victim, and the teams differ.

**Kills per team** = Σ kills over players with profile team == 0, and Σ over team ≠ 0 (`winner_team` 0x1FD84C). The team comes from the roster (profile +5AC).

---

## 4. Match over and winner

### 4.1 `rule_completed(t)` (vtable +0x48)

| rule | completed when | ref |
|---|---|---|
| gather / mts | `m_completed_time (+15C) != u32::MAX && +15C <= t`. +15C is set to the event time when, after a put/take, `team_points[0] == items_count` **or** `team_points[1] == items_count` (items_count = size of +110..+114 = NB [I]). Otherwise it is reset to −1 | 0x1FBF1F, 0x1FB8DF..0x1FB904 |
| timelimit | state 1: `t − start ≥ +12C` (= (C458+C459)·60000, the waiting timeout); state 3: `t − start ≥ +124` (= C44E·1000); states 0 and 2: never | 0x1F58E3 |
| player_respawn, kd_stats | never (slot = `xor al,al` 0x1FF958) | vtables |

**+15C is not serialized** [V: absent from 0x1FBA38; `deserialize` 0x1FBB49 does not restore it]. srp derives it as `completed_gather = (tp0 == NB || tp1 == NB)`, taking the completion time as the first snapshot time where this held [I].

`game_world_core::serialize` stores `item+B9942 = any(rule.rule_completed(C7F4))` (0x1CC698..0x1CC6CD). This is the **"match over" flag of each state**. It is not on the wire and nothing in the client reads it; its only other reference is the ctor at 0x1FDC46 [V]. It is evidently what the server variant used.

**srp's rule:**

    over(T) = completed_gather || (tl.state==3 && T − tl.start ≥ C44E·1000) || (tl.state==1 && T − tl.start ≥ (C458+C459)·60000)

### 4.2 `game_world_core::winner_team` 0x1CEE03 [V]

The function walks the rules and keeps `best` = the first rule. For each later rule `r`, if `prio(best) < prio(r)` (signed, `cmovl`), then `best = r`. It returns `best->winner_team()` (vfn+0x50). Ties keep the earlier rule.

| rule | prio (vfn+0x4C) | winner_team (vfn+0x50) |
|---|---|---|
| gather / mts | **2** (0x1FBF3A `push 2`) | 0x1FBF3E: tp0 > tp1 → **0**; tp0 < tp1 → **1**; equal → **2** |
| kd_stats | 1 (0x1BDBD6) | 0x1FD84C: k(team0) > k(team≠0) → 0; less → 1; equal → 2 |
| timelimit | 0 (0x18EC72) | 3 (0x5F719) |
| player_respawn | 0 | 3 |

`game_team_id` (@4183d8c) values: team_1=0, team_2=1, team_neutral=2 (draw), team_undefined=3.

**In network matches the gather/mts rule always wins the priority.** The winner is therefore always decided by battery counts (also at timeout); kills never decide [V]. kd_stats would decide only in a match without a gather rule, which the network path never creates.

### 4.3 Early end on batteries
When either `team_points[t] == NB`, the gather rule is completed and `winner_team` returns t. This holds because the other team then has fewer, unless NB == 0.

### 4.4 Finishing the statistics (what `shared_statistics::finish_match(winner, active_mask)` 0x1DDB6F does) [V]

For every roster p < N:
- if p is in `active_mask`: counter[11] += 1;
- if `team(p) == winner`: counter[12] += 1, then counter[16] += `stats.team_points[winner]` (+1A30[winner]; for winner == 2 this reads padding, but no player is on team 2).

The caller (`base_game_statistics_handler::finish_match` 0x19C19C) passes `winner_team()` and `core+C808`. That is the players mask [I].

---

## 5. Mapping onto the lobby results (lobby_spec §3.17) [V UI; I sources]

The UI meanings come from `lobby_menu::fill_match_statistic` 0x177271 and `fill_player_statistic` 0x17785E. The Flash labels come from the label table at 0x173354 (`results_work/lobby_labels.txt`).

**match_stats**

| field | UI | suggested source from the snapshot |
|---|---|---|
| u32 +740 | match_id | srp |
| u8 +752 | winner team. `match_result = 5` if it equals the viewer's brief team, else 0 (0x1774AC) | §4.2 winner (2 = draw: everyone sees 0) |
| u8 +753 / +754 | team 0 / team 1 score (shown as own/enemy by the viewer's team, 0x177449) | gather `team_points[0]` / `[1]` |
| str +0, u16 +744 | best player, score | max recomputed score (§2.3) |
| str +40, u8 +750, u16 +746 | best sniper: accuracy %, shots | counters 23/22 |
| str +80, u16 +74A, u16 +748 | best support: saved, healed | counters 8, 7 |
| str +C0, u16 +74C, u16 +74E | best explorer: artefacts, items | counter 5; counter 19 or 20 [I] |
| u8 n ≤ 20 × brief | name, u32 id (+40), u16 score (+44), u16 kills (+48), u16 deaths (+4A), u8 team (+4C, compared with the viewer's), u16 rank (+46) | counters 0 and 2 (or kd_stats), recomputed score |

**player_result_full** (viewer only)

| field | UI label | source |
|---|---|---|
| u32 +0 | profile id | srp |
| u16 +4A | statistics_2 assists | counter 1 |
| u16 +4C | statistics_3 healed | counter 7 |
| u16 +4E | statistics_4 saved | counter 8 |
| u16 +50 | statistics_5 headshots | counter 15 |
| u16 +52, +54 | not shown [U] | (silent kills 14 / melee 13?) [U] |
| u16 +56 | statistics_8 "Batteries found" | counter 19 (or 20) [I] |
| u16 +58 | statistics_9 "Batteries defended" | counter 18 / 100 [I] |
| u16 +5A | statistics_10 artefacts | counter 5 |
| u8 +65 | level | profile |
| u16 +5C | statistics_7 "Batteries stolen" (UI shows **value/100**, 0x177948) | counter 17 (already share·100) |
| u16 +40 / +42 | exp / money | srp economy |
| u16 +44 / +46 | previous rating / rating (delta = +46 − +44, 0x177555) | srp |
| u8 +66, u16 +48 | reputation faction, reputation | srp |
| u16 +5E | statistics_11 max killstreak | max over snapshots of intermediate `killstreak` [I] |
| u8 +64 | statistics_12 accuracy "%d %%" | 100·c23/c22 [I] |
| u16 +60 / +62 | weapon1/2 dict id | loadout |
| drops, u32 +3C | items_drop, match_progress | srp |

statistics_0 and statistics_1 come from the brief (+48 kills, +4A deaths). statistics_6 (silent kills) is hard-coded 0 (0x177931).

---

## 6. Decoder outline (Rust-ish)

```
read u32 mask
for i in bits(mask) ascending:
    skip u16-prefixed; b = u8; if b != 0 { skip_anim_bitstream() }   // §1.1
skip u16-prefixed (bullets)
stats  = u16-prefixed slice → parse §2 with N, NB (empty slice ⇒ none)
skip u16-prefixed (objects)
rules  = u16-prefixed slice
kd     = rules[len-5N ..]                  // §3.5
tl     = rules[len-5N-13 .. len-5N]        // §3.4
g      = rules[0..]: seed u32, tp0 u8, tp1 u8, (mode 3: avail u32, 3×u32), mask u16, items…, containers…
```

---

## Open questions
1. **No sample snapshots yet:** `logs/snapshots/` does not exist, so nothing is validated against real bytes. First checks to run when samples arrive:
   - the timelimit `current_time` wire value should be ≈ 1e9;
   - `L_r` should equal len(gather) + 84 + 2R + 13 + 5N;
   - the statistics length should equal Σ(68+5(a+b)) + 7N + battery-manager bytes + 2.
2. Queries_result order = request order (rule vector order) is [I]. The pass-through code is [V]. The end-anchored kd/timelimit parse does not depend on it.
3. The anim bitstream skip is derived from the serializer, not tested. The tree part is skipped through its header, so only the times and floats sections must be parsed exactly.
4. items_count (+110 size) == NB for both gather and mts is [I] (cook requests NB items).
5. Who calls `base_game_statistics_handler::finish_match` on the client was not found; it is also what zeroes C7E0. srp should apply §4.4 itself.
6. The lobby fields +52/+54, "Batteries defended" (+58) and "Batteries found" (+56) sources are unconfirmed. Exp, money and rating are srp's choice.
7. `single_player_respawn_rule` (no network flag) has a different serialize (0x1FD0xx); not covered, since network matches never use it [I].
