# 0.21d after-match results: trigger, wire layouts, rewards

This covers the "victory screen" after a match: how it opens, how the client asks for stats, what every byte of `match_stats` / `player_result_brief` / `player_result_full` means, and which 0.21d data srp can use to compute rewards. Wire layouts were first given in `lobby_spec.md` §3.17. This file adds what each field means.

Tags:
- **[V]** verified in the 0.21d disassembly (address given) or in data/SWF (file given);
- **[I]** inferred;
- **[U]** unknown (it was server-side logic).

Sources:
- `survarium.exe` 0.21d disassembly with symbols (VA, image base 0x10000);
- `lobby_menu.swf` AS3 (`VictoryScreen.as`, `LobbyMenu.as`, `PlayerList*.as`);
- the `lobby_labels` table in the exe (Flash key → localization key, dumped from 0x173354);
- `gameplay/drops/drops.options`, `gameplay/quests/quests.options`, `gameplay/db_static_dictionaries`, `db_skills_tree`, `localization/english`.

---

## 1. Trigger and screen lifecycle

### 1.1 Where the screen appears and how it waits [V]

`network_client::close_current_match(3 = on_match_finished)` 0x1A2420 calls:
1. `game_world::on_finish_match`;
2. `lobby_menu::show_match_statistic` 0x17724B.

`close_current_match(3)` runs when the client receives match 0x57 for its own index (see `match_start_spec.md` §6).

`show_match_statistic` does two things:
- calls Flash `root.show_victory_screen`;
- sets lobby_menu+0x6AD = 1. That flag is never read; `close_victory_screen` clears it.

`show_victory_screen` calls `VictoryScreen.prepare()`, which:
- hides every panel;
- shows `loading_text` = `st_statistic_sync_text` ("Receiving match stats") with 1–3 animated dots (a 1 s `setInterval`).

**There is no timeout** [V: AS3 `prepare`/`updateText`; C++ has no timer]. The screen waits until one of these happens:
- `0x36 13` arrives. `read_match_stats` → `fill_victory_screen` → `setData` stops the dots and shows the data.
- The user clicks Close (`btn_close` → `ExternalInterface "close_victory_screen"` → C++ 0x171679 clears +0x6AD; nothing is sent to the server).
- The user switches a top-menu tab (`closeVS`).

`read_match_stats` 0x19E7CF calls `show_match_statistic` itself before filling [V]. So **an unsolicited `0x36 13` opens the victory screen at any time**; no `#mstats` is needed for display. If the screen is already open, `prepare()` runs again (harmless: one orphaned dot timer on a hidden text field [I]).

### 1.2 `#mstats` line: messaging 0xC9 → `0x21 13` [V]

`messaging_client::process_incoming_text_message` 0x19B858 reads this server → client frame:

| type | field | value for stats |
|---|---|---|
| u8 | 0xC9 | |
| u8 | sender_type | **8** (`stats_processor_server_client_type`) |
| u32 | sender_id | any (unused for type 8; the ignore list is checked only for type 5) |
| string(≤63) | sender_name | any, e.g. `""`. Read, then ignored |
| u8 | channel | any 0..8. Read, then ignored (`on_stats_message_arrived` never looks at it) |
| string(≤255) | body | must contain **`#mstats`** (`strstr`, 0x19B907; string at 0x7DB8C4) |

For type 8 without `#mstats`, the line is dropped silently. For type 8, nothing is shown in chat.

`lobby_menu::on_stats_message_arrived` 0x1765A5 parses the number:
1. `p = strstr(body, "[")`, then `q = strstr(p, "]")`;
2. `N = atoi(strncpy_s(buf16, p+1, q−p−1))`.
   - If either bracket is missing, **N = 0** (the request still goes out).
   - Keep N to at most 15 digits (16-byte buffer).
   - Canonical body: **`#mstats [<match_id>]`**.
3. If lobby_client+0x1B4 is set (lobby TCP socket connected; set by `on_connected` 0x19D05E and cleared by `on_disconnected`), it calls `lobby_client::query_match_stats(N)` 0x19D6F0. That sends lobby **`21 13 <u32 N>`** and logs "query match stats for %d".
4. Otherwise it stores N in lobby_client+0x5A58 and logs "query match stats on connected f…".

Requirements on the client's connections at that moment:
- **Messaging** must be signed in (0xC9 is handled only by the post-sign-in dispatcher).
- **Lobby:** only TCP-connected is checked, not signed in (+0x1B4 is set when the TCP connect completes, before `0x26` is answered). If the line arrives in the gap between connect and `0x30`, `21 13` is sent before sign-in completes, so srp's lobby must accept a `0x21` right after `0x26` [I].
- During the match the lobby is normally still connected [I]. `network_client::on_connected_to_match` 0x16D2AA disconnects the lobby only when network_client+0x7A20 (a constructor flag) is 0.

### 1.3 `pending_match_stats_id` on reconnect [V]

When N was stored (§1.2 step 4), the next lobby connect (`on_connected` 0x19D04F) sends it in **0x26 at offset 6**, then resets +0x5A58 to −1. `just_left_match` (+0x5A5C) works the same way: it is sent once and then cleared.

The client does **not** re-query after sign-in. If `pending_match_stats_id ≠ 0xFFFFFFFF`, srp should push `36 13` for that match after `0x30` and the normal sign-in data. The screen opens by itself (§1.1).

### 1.4 Per-player detail: `0x21 0E` → `0x36 0E` [V]

Clicking a name in the victory-screen player list → Flash `victory_screen_request_player_data(uint(item.id), match_id)` → 0x172383 → `query_player_match_stats(player_id, match_id)` 0x19D672. That sends lobby **`21 0E <u32 player_id> <u32 match_id>`**. `player_id` is the `id` that srp put in `player_result_brief`.

The reply is **`36 0E` + `player_result_full`** of that player. `read_player_match_stats` 0x19E81F does the following:
1. finds the brief whose id == full+0 (index 0 if none matches);
2. calls `fill_player_statistic(brief, full)`.

This updates only the "personal achievements" panel (level, weapons, statistics_0..12). The reply carries no match id, so the client does not check it.

---

## 2. Field meanings

The `0x36 13` body is `match_stats` followed by **the recipient's** `player_result_full`.

`read_match_stats` 0x19E6B1 then:
- stores match_stats at lobby_client+0x1D4. Brief k is at +0x2D4 + k·0x50, and the count is at +0x925;
- finds the **own index** = the first brief with `brief.id == full.id` (default **0** if none matches; then player 0's kills/deaths/team are treated as the recipient's) [V 0x19E79C];
- copies full+0x48 (reputation gained) to P+0x9E0 of the profile whose P+0 == full.id. P+0 is never set from the wire, so in practice this never matches. P+0x9E0 only reaches Flash as `fill_char_info.profile`, which AS3 never reads [V]. Harmless.
- calls `show_match_statistic`, then `fill_match_statistic(stats, full, own_idx)` 0x177271 → `root.fill_victory_screen`, then `fill_player_statistic(brief[own], full)` 0x17785E → `root.fill_victory_screen_player`.

Flash keys map to localization keys through the exe table `lobby_labels` (0xE3 pairs, sent as `root.set_localization_data`) [V, dumped].

### 2.1 `match_stats` (wire order) [V layout 0x19C790; V meaning 0x177271 + AS3]

| # | wire | mem | meaning | Flash key → UI label (loc key) |
|---|---|---|---|---|
| 1 | u32 | +0x740 | **match_id** (also logged "read match stats for %d") | `match_id`; sent back in `21 0E` |
| 2 | u8 | +0x752 | **winner team** (`game_team_id`: 0 / 1; 2 or 3 = no winner). Constructor default 3 | `match_result` = **5 if own brief.team == winner, else 0**. AS3: `_victory = match_result > 2`; `ds.gotoAndStop(match_result+1)`; text `st_victory` "Victory" / `st_defeat`. A draw therefore shows as a defeat |
| 3 | u8 | +0x753 | **team 0 score** (batteries in base) | `team1_score` (own side) if own team == 0, otherwise `team2_score` |
| 4 | u8 | +0x754 | **team 1 score** | the mirror of row 3. AS3 frame = score + (victory ? 23 : 1) / (victory ? 34 : 12), so **0..10** fit the 11-frame ranges [I] |
| 5 | string(≤63) | +0x000 | **best player** name ("" = hide the block) | `title_0_player`, header `st_best_player` "PLAYER" |
| 6 | u16 | +0x744 | best player's **score** | `title_0_value_0`, `st_best_player_score` "Score:" |
| 7 | string(≤63) | +0x040 | **best shooter** name | `title_1_player`, `st_best_sniper` "SHOOTER" |
| 8 | u8 | +0x750 | shooter's **accuracy %** | `title_1_value_0`, `st_best_sniper_accuracy` "Accuracy:" (raw number, no % sign) |
| 9 | u16 | +0x746 | shooter's **shots** | `title_1_value_1`, `st_best_sniper_shots` "Shots:" |
| 10 | string(≤63) | +0x080 | **best support** name | `title_2_player`, `st_best_support` "SUPPORT" |
| 11 | u16 | +0x74A | support's **saved** (teammate rescues) | `title_2_value_0`, `st_best_support_saved` "Saved:" |
| 12 | u16 | +0x748 | support's **healed** (teammate cures) | `title_2_value_1`, `st_best_support_healed` "Healed:" |
| 13 | string(≤63) | +0x0C0 | **best explorer** name | `title_3_player`, `st_best_explorer` "EXPLORER" |
| 14 | u16 | +0x74C | explorer's **artifacts** | `title_3_value_0`, `st_best_explorer_artefacts` "Artifacts:" |
| 15 | u16 | +0x74E | explorer's **items** (batteries found [I]) | `title_3_value_1`, `st_best_explorer_items` "Items:" |
| 16 | u8 n | +0x751 | number of players. **≤ 20** (the array holds 20 entries at +0x100; more overflows) | |
| 17 | n × `player_result_brief` | +0x100 + k·0x50 | scoreboard rows | `players[]` |

Notes on the string fields:
- **All four strings are player nicknames shown as-is.** They are not localization keys.
- `match_stats` has **no map, mode or duration field**.
- An empty string hides that "Best …" block (`if (value.title_N_player)`).
- `statistics_score` (own brief score) is also put into the Flash object, but no AS3 code reads it.

### 2.2 `player_result_brief` (wire order) [V layout 0x19C69C; V meaning 0x1775A7 + PlayerList AS3]

| # | wire | mem | meaning | Flash member |
|---|---|---|---|---|
| 1 | string(≤63) | +0x00 | nickname | `name`. PlayerList marks the row whose name == the local nickname as "local" |
| 2 | u32 | +0x40 | **player id**. Must equal full+0 for the recipient's own row; returned in `21 0E` | `id` |
| 3 | u16 | +0x44 | **score** (match points) | `score` |
| 4 | u16 | +0x48 | **kills** | `kills` |
| 5 | u16 | +0x4A | **deaths** | `deaths` |
| 6 | u8 | +0x4C (stored as u32) | **team** (`game_team_id` 0 / 1) | `team` = **1 if same team as own row, else 2** (1 = `st_label_teamA` "Allies", 2 = `st_label_teamB` "Opponents") |
| 7 | u16 | +0x46 | **level** (player rank) | `rank`, shown as text in the row (`ui_rank`) |

There is no per-player battery count, assist count or ping in the brief. Assists and batteries are only in `player_result_full`.

### 2.3 `player_result_full` (wire order) [V layout 0x19C939; V meaning 0x177271 / 0x17785E + AS3]

In memory: +0x00 id; +0x04..0x13 list A (4×u32), +0x14 count A; +0x18..0x37 drops (4×{u32,u32}), +0x38 count; +0x3C progress; +0x40..+0x62 u16 fields; +0x64..+0x66 u8 fields.

| # | wire | mem | meaning | Flash key → label |
|---|---|---|---|---|
| 1 | u32 | +0x00 | **player id**; identifies the recipient's brief row (§2) | — |
| 2 | u16 | +0x4A | **assists** | `statistics_2` → `st_stats_assists` "Assists:" |
| 3 | u16 | +0x4C | **healed** (teammate cures, event 7 [I]) | `statistics_3` → `st_stats_healed` "Healed:" |
| 4 | u16 | +0x4E | **saved** (teammate rescues, event 8 [I]) | `statistics_4` → `st_stats_saved` "Saved:" |
| 5 | u16 | +0x50 | **headshot kills** | `statistics_5` → `st_stats_headshots` "Headshots:" |
| 6 | u16 | +0x52 | unused by the client. Probably silent kills [I] | — (`statistics_6` "Silent kills:" is **hard-coded 0**, 0x177931) |
| 7 | u16 | +0x54 | unused by the client. Probably melee kills [I] | — |
| 8 | u16 | +0x56 | **batteries found** | `statistics_8` → `st_stats_items_gathered` "Batteries found:" |
| 9 | u16 | +0x58 | **batteries defended** | `statistics_9` → `st_stats_items_defended` "Batteries defended:" |
| 10 | u16 | +0x5A | **artifacts collected** | `statistics_10` → `st_stats_artefacts_gathered` "Artifacts collected:" |
| 11 | u8 | +0x65 | **player level** (after the match) | `level` → `player_rank.set_rank` (≥100 shows the top-rank icon) |
| 12 | u16 | +0x5C | **batteries stolen × 100**. Shown as value/100 (integer division) | `statistics_7` → `st_stats_stolen_victory_items` "Batteries stolen:". Matches the ×0.01 scaling the client applies to victory-item events 6/0x11/0x12 (credit is split by carry share, 100 = one whole battery) [V 0x19C3A2; share split I] |
| 13 | u16 | +0x40 | **experience gained** | `statistics_exp` → score box 0 (icon 4) |
| 14 | u16 | +0x42 | **money gained** (sent to Flash as a float) | `statistics_money` → score box 2 (icon 1) |
| 15 | u16 | +0x44 | **rating before** the match | (only used for the delta) |
| 16 | u16 | +0x46 | **rating after** the match | `statistics_rating` = +0x46; `statistics_rating_delta` = +0x46 − +0x44 (signed) → score box 4 shows "rating (+delta)" (icon 10) |
| 17 | u8 | +0x66 | **reputation faction** (1..4; **0 = no reputation box**) | `statistics_reputation_faction` → icon frame faction+5 |
| 18 | u16 | +0x48 | **reputation gained** (for that faction). Also copied to P+0x9E0 (see §2) | `statistics_reputation` → score box 1, only if row 17 ≠ 0 |
| 19 | u16 | +0x5E | **max killstreak** | `statistics_11` → `st_stats_max_killstreak` "Killstreak:" |
| 20 | u8 | +0x64 | **accuracy %** | `statistics_12` = sprintf("%d %%") → `st_stats_accuracy` "Accuracy:" |
| 21 | u16 | +0x60 | **weapon 1 dict_id** (0 = none) | `weapon1_dict_id` → `inventory_list.create_item(id)` icon. **Use real items_dict ids** |
| 22 | u16 | +0x62 | **weapon 2 dict_id** (0 = none) | `weapon2_dict_id` |
| 23 | u8 nA (**≤ 4**) | +0x14 | list A count | — |
| 24 | nA × u32 | +0x04.. | list A. **Never read by the client** [V: only `deserialize` touches it]. Maybe completed quest ids or found-item ids [I]. **Send nA = 0** | — |
| 25 | u8 nD (**≤ 4**) | +0x38 | number of item drops | — |
| 26 | nD × {u32 dict_id, u32 condition_or_stack} | +0x18 + i·8 | **dropped items** | `items_drop[] = {dict_id, condition_or_stack}`. Flash calls `inventory_list.create_item(dict_id)` + `fillModifiersDescription`, so **dict_id must exist in items_dict** [I: an unknown id breaks the AS call] |
| 27 | u32 | +0x3C | **drop progress points** | `match_progress` → the "Found" progress bar (§3.1) |

Kills and deaths of the personal panel (`statistics_0` `st_stats_kills`, `statistics_1` `st_stats_deaths`) come from the **brief** row (+0x48 / +0x4A), not from `full`.

Score box details (AS3 `updatePremiumState`):
- For a **non-premium** player each box shows "(+X)" with X = 0.5 × value (`value × 1.5 − value`): the bonus premium would have given.
- For premium players (`PlayerInfo.premiumState ≠ 0`) no bonus text is shown.
- So the original server applied **premium = +50 %** to experience, reputation and money [I].

### 2.4 Event counters behind the stats [V structure, I mapping]

The client contains the stats code that the server shared:
- `shared_statistics` 0x1DD9xx (`player_shared_statistics`, stride 0xF0);
- `base_game_statistics_handler` 0x19C124.

Each player has 24 u16 counters indexed by `match_stats_events_dict_enum` (@41825f0):
- 0 kill, 1 assist, 2 death, 3 suicide, 4 team_kill, 5 artefact;
- 6 bring_victory_item (×100), 7 teammate_cure, 8 teammate_rescue, 9 revenge, 0xA vi_carry_kill;
- 0xB participating, 0xC won, 0xD melee_kill, 0xE silent_kill, 0xF headshot_kill;
- 0x10 winning_team_vi_bonus, 0x11 steal_vi (×100), 0x12 recover_vi (×100);
- 0x13 vi_found, 0x14 put_vi_in_container, 0x15 entrap_enemy, 0x16 bullet_fire, 0x17 bullet_hit.

How the client updates them:
- `shared_statistics::finish_match` 0x1DDB6F adds `participating` to everyone, `won` to the winners, and `winning_team_vi_bonus` × (batteries in the winner's container).
- Score = Σ floor(count × w[e] × k + 0.5), with weights from match_options C480 (k = 0.01 for events 6/0x11/0x12) [V 0x19C38A].

Natural mapping for srp, if it has these counters:
- kills = e0, assists = e1, deaths = e2, artifacts = e5;
- healed = e7, saved = e8, headshots = e15, silent = e14, melee = e13;
- stolen×100 = e17, found = e19, defended = e18 (recover) [I];
- shots = e22; accuracy = 100·e23/e22.

---

## 3. Reward rules in the 0.21d data

Data the client ships, and what the original server did with it. Only the bar thresholds (§3.1) are used by the client; everything else in this section was evaluated server-side.

### 3.1 Drops — `gameplay/drops/drops.options` [V data]

| key | value | used by |
|---|---|---|
| `slots_open` | [1000, 5000, 9000] | **client** [V 0x173207]: `root.set_drop_items_properties(slots_open, drop_amount·15/10)` → the 3 drop markers on the progress bar |
| `drop_amount` | [10000] | **client** [V 0x17326A]: bar maximum = 15000 |
| `slots_open1` | [1, 5, 9] | not used by the client [U] |
| `modifier_probabilities` | [50, 75, 90, 95, 99] | server. Cumulative % for 0..4+ modifications on a dropped item [I] |
| `item_level_probabilities` | 21 rows × 16 columns of % | server. Row = player level 0..20 [I], column = item level 0..15. Bell shape around the row index (row r: 5/25/40/25/5 centred on item level r) |
| `item_category_probabilities` | weighted tree | server. Weight **90** → {60: ammo categories 9, 18, 19, 20, 21; 40: consumables 10 meds, 11 traps, 24 grenades}. Weight **10** → {60: armour categories 1–7 (helmet, mask, torso, back, legs, gloves, boots); 40: weapons 8 rifles, 13 sniper, 14 shotguns, 15 SMG, 16, 17 pistols} |

How the progress bar and drops line up in the client [V AS3 `setDropUnlockPositions` / `setMatchItemsDrop`]:
- The marker for drop i sits at `slots_open[i] / 15000` of the bar.
- The bar animates to `match_progress / 15000` (capped at the maximum).
- `items_drop[i]` is attached to marker i and becomes visible when the bar passes it.
- Only the **first 3** drops can be displayed.
- So the original rule was probably: **number of drops = number of `slots_open` thresholds ≤ `match_progress`** [I].
- How progress is earned (per-match score, or carried over between matches) is unknown [U]. Whether it resets is [U].

Pick an item:
1. choose the category with the weights above;
2. choose the item level with the player-level row;
3. take a matching item from `items_dict` (`item_category`, `item_level`, `is_premium` false);
4. set condition 100, or a stack count for `is_stack` items [I].

The drop is **not added to the inventory by the screen**. srp must add it and push `0x36 03` [I]. The messaging important message type 5 "drop" (`#…`, logged "Drop arrived: %s") also exists [V 0x176F15]; it is optional.

### 3.2 Quests — `gameplay/quests/quests.options` [V data]

- **271 quests**, all `daily: true`, `ttl_minutes: 1440`, `priority: 10`.
- Ids look like `10100`; the faction is (id/100)%100.
- Each has `faction`, `type` (`quest_type` enum @4188c1c), `params {n, x, y, max_games_count}`, `ui` loc keys, `preconditions[]` and `awards[]`.

Quest types used:

| type id | affix | meaning |
|---|---|---|
| 28 | win_x_games | win |
| 29 | kd_ratio | win with K/D ≥ y |
| 6 | bring_x_items | bring n batteries |
| 33 | defend_items | win without a battery being stolen |
| 32 | win_score | win with ≥ n batteries in base |
| 15 | kill_enemy_x | kill n of faction x |
| 16 | headshots | n headshots |
| 19 | killstreak | streak of n |
| 30 | accuracy | win with accuracy ≥ y |
| 10 | find_artifacts | find n artifacts |
| 12 | kill_enemies | kill n |

All are checked against one match (`max_games_count: 1`).

Awards (`quest_award` enum @418d6c4: 0 experience, 1 reputation (param = faction), 2 money, 3 item, 4 open_slot):
- Only **money** and **reputation** are used.
- Money: 1000/2000 at quest level 1, rising by 200 per level to 4000 at level 11.
- Reputation: 100 at level 1, +10 per level to 200 (one outlier with 1200).

Preconditions (`quest_precondition` enum @418d53c):
- 0 `cooldown` n = 120 (on every quest; minutes [I]);
- 1 `minimum_reputation`, 2 `maximum_reputation`: x = faction, n = reputation level. These pick which faction-level band a quest is offered to.

The client also has `game_quest_progress_calculator` (`calculate_player_game_progress` 0x1B3859, `calculate_victory_item_progress` 0x1B378E; fed from `game_statistics_handler::on_event` 0x1A09CF), which computes progress from the same events in-match. That is useful as a reference for exact quest rules [V exists; not traced].

srp currently sends n = 0 quests. If it ever sends quests, it must evaluate them at match end and pay the awards [U: original assignment logic].

### 3.3 Reputation — `db_static_dictionaries` `factions_dict` [V data]

| faction | level thresholds (points) |
|---|---|
| 1 Scavengers, 2 Black Market | 0, 100, 420, 990, 1840, 3000 (levels 0..5) |
| 3 Renaissance, 4 Edge (`st_border_faction`) | 0, 1500, 3370, 5640, 8340, 11500, 15150, 19320, 24040, 29340, 35250 (levels 0..10) |
| 5 scientists, 6 mercenaries | placeholder (one level, 0) |

How the client uses them:
- It computes the level from the points it receives in `0x36 09` (lobby_spec §3.8).
- Crossing a threshold was announced with important message type 4 `#ru[<faction>:<level>]` (parsed at 0x176D45: `[`, `get_token(':')`, `atoi`). The client then re-queries `0x12` (if newly unlocked) and `0x09`.

Unknown [U]:
- how much reputation a match gives;
- which faction gets it. Likely the faction of the profile's equipment/seller, with quest awards on top [I].

### 3.4 Experience and level — **no table in the data** [V negative]

- There is no experience table in resources.db or the exe. The searched strings are only the Flash keys `experience_current` / `experience_next_level`.
- The server sends each profile's `exp_total`, `exp_level_start` and `exp_next_level` (`0x36 01`, `0x36 06`). It owns the curve [U].
- Level cap is **20** [I]: `db_skills_tree` has `skill_level_1..20` per skill, `level` = skill points unlocked, and drops have 21 level rows.
- Experience per match is unknown [U].

### 3.5 Rating — no data [U]

`0x36 0B` `{u32, u16, u16}` [P] feeds the ranking page. The victory screen shows only rating before and after (full+0x44 / +0x46). The formula is unknown.

### 3.6 What is data-driven and what srp must invent

| item | status |
|---|---|
| Drop bar thresholds and maximum | data + client [V] |
| Drop item choice (category / level / modification probabilities) | data [V]; algorithm [I] |
| Drop count from progress | [I] |
| Progress points per match | **[U]** |
| Quest list, conditions and awards | data [V]; assignment, rotation and progress storage [U] |
| Reputation levels | data [V] |
| Reputation per match | **[U]** |
| Experience curve and experience per match | **[U]** |
| Money per match | **[U]** |
| Premium bonus | +50 % [I from AS3] |
| Rating | **[U]** |
| Score weights (C480) | [U] (srp already uses guesses in `match_start.rs`) |

---

## 4. Lobby updates after results

`0x36 13` changes **nothing** in the client's account state (money, experience, level, inventory, reputation). It only fills the screen and the unused P+0x9E0 [V 0x19E6B1]. So srp must push the new state itself. Nothing in the client requires an order relative to `0x36 13`. The data dependencies are:

| what changed | push | notes |
|---|---|---|
| inventory (drops) | `0x36 03` (full list) | before any `0x36 02` that equips a dropped item (lobby_spec §3.2 Flash requirement) |
| experience / level of the played profile | **`0x36 06`** for that profile_id | updates P+0x9E3 / 9D0 / 9D4 / 9D8. Runs `fill_character_data` only for the **active** profile (status bar level, experience bar, skill points). Avoids the `0x36 01` side effects |
| profile selector experience bars (`setup_player_profiles`) | `0x36 01` | only if you want the profile list refreshed. The client then sends `21 02` for every profile; answer with `0x36 02` and **do not** answer the resulting `29 04` (R8) |
| money | `0x36 05` | after `0x36 0A` (nickname) has been sent once |
| reputation | `0x36 12`, then `0x36 09` | optionally preceded by the messaging `#ru[f:l]` important message (type 4) on a level-up |
| quests | `0x36 0D` (n = 0 today) | |
| rating | `0x36 0B` | optional |
| play state | `0x36 00` status 0 | the Play button unlocks only in state 0. srp already sends this when the match ends |

`0x36 13` itself must come after `0x30` (sign-in). Before that, any id other than 0x30 is ignored (lobby_spec §1.3).

---

## 5. Recommended srp implementation outline

1. **Match end (match server).** Decide the end, compute `winner` (0/1, else 3) and the team battery counts, and send 0x57 to each client (`match_start_spec.md` §6). Each client opens the victory screen and shows "Receiving match stats…" by itself.
2. **Collect per-player stats.** Use whatever srp can observe:
   - relayed inputs only: kills/deaths are not visible to a pure relay;
   - with snapshot options B/C: `shared_statistics` counters (§2.4);
   - otherwise: zeros.

   Build per player: id (= profile_id or account id), name, team, level, score, kills, deaths, assists, healed, saved, headshots, batteries found / defended / stolen×100, artifacts, max streak, accuracy %, weapon1/2 dict_ids (from the profile's slots 7 and 10).
3. **Titles.** These are the client's own categories, with srp-chosen rules [I]:
   - best player = max score;
   - shooter = max accuracy (needs shots > 0);
   - support = max saved+healed;
   - explorer = max artifacts+batteries found.

   Use an empty name to hide a title.
4. **Rewards** (srp-defined, §3.6):
   - experience and money from score / win;
   - reputation for one faction;
   - ×1.5 for premium;
   - drop progress and drops from `drops.options`.

   Apply them to the stored account/profile.
5. **Store** a `MatchResult { match_id, match_stats, per_player_full }` keyed by match_id, kept for at least the session (stats can be re-requested).
6. **Notify.** For each participant whose messaging session is signed in, send `C9 08 <u32 0> "" <u8 0> "#mstats [<match_id>]"`. If the player has no messaging session, push `36 13` directly, or wait for the 0x26 `pending` path.
7. **Answer the requests:**
   - `21 13 <id>` → `36 13 <match_stats><full of this account>`. If the match is unknown, send nothing (the screen keeps waiting until closed) or send a stub with n = 0 [I];
   - `21 0E <player_id> <match_id>` → `36 0E <full of player_id>`;
   - 0x26 with `pending_match_stats_id ≠ −1` → after `0x30` and the sign-in pushes, `36 13` for that id.
8. **Push the state** of §4: `0x36 03` (if drops), `0x36 06` (played profile), `0x36 05`, `0x36 12` + `0x36 09` (if reputation changed), `0x36 00` status 0.

Wire constraints:
- strings ≤ 63 bytes;
- n players ≤ 20;
- drops ≤ 4 (only 3 shown);
- list A ≤ 4 (send 0);
- every dict_id must exist (R6);
- include the recipient in the brief list, with `brief.id == full.id`.

Minimal valid `36 13` (one player, no rewards):

```
36 13
  <u32 match_id> <u8 winner> <u8 score_t0> <u8 score_t1>
  00 <u16 0>  00 <u8 0> <u16 0>  00 <u16 0> <u16 0>  00 <u16 0> <u16 0>   ; four empty titles
  01 <str name> <u32 id> <u16 score> <u16 kills> <u16 deaths> <u8 team> <u16 level>
  <u32 id> <9 × u16 0> <u8 level> <u16 0> <u16 exp> <u16 money> <u16 r0> <u16 r1>
  00 <u16 0> <u16 0> <u8 0> <u16 0> <u16 0> 00 00 <u32 0>
```

---

## 6. Open questions

- Formulas for experience, money, reputation, rating and drop progress per match: the original values are lost. Pick srp rules.
- Meaning of full+0x52 / +0x54 (probably silent and melee kills) and of list A (+0x04): the client never displays them.
- Which `shared_statistics` event the original fed into "batteries found" (0x13 found or 6 brought) and "defended" (0x12 recover).
- `drops.options` `slots_open1`, and whether drop progress carried over between matches.
- Whether `0x36 00` status 0 arriving while the victory screen is open (it calls `game::switch_to_lobby` when lobby_menu+0xD8 == 0) disturbs the screen. Test against the client.
- The format of the quest important messages (`#+q` new / `#!q` failed / `#q!` completed → `create_quest_ui_message`) and of the drop message (type 5) is not traced. Only the prefixes are known [V 0x176A7D..0x176BB1].
