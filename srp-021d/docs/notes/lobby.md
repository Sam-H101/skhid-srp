# Lobby and messaging protocol: 0.100b (srp today) vs 0.21d (build 2010)

This is a read-only study. All 0.21d addresses are VAs with image base 0x10000. All 0.100b addresses use image base 0x400000.

**How the data was gathered**
- **Message ids:** taken from the PDB enum records (`scope021d/tools/pdbenums.py`, outputs `enums_new.txt` / `enums_old.txt`). The names are the developers' own.
- **Layouts:** read from the client's reader and writer functions. `scope021d/all_{new,old}.txt` holds every lobby_client, messaging_client, lobby_menu and network_client lobby function. `tools/f.sh new|old <label>` prints one of them without the logging noise.
- **Status flags:**
  - **verified** means the layout was read field by field from the disassembly.
  - **partial** means the byte layout is known but what some fields mean is not.
  - **unverified** means it is an inference.

## 0. Headline

1. **Framing is unchanged.** `tcp_packet_socket::send` 0x284EFF still frames with a u8 length, or 0 followed by a u16 length. Strings are still a u8 length followed by the bytes (`w_string` 0x19AD22, `r_string` 0x19ACEA). The server-to-client top-level ids 0x30..0x37 and the client-to-server ids 0x20..0x28 keep their numbers.
2. **The sign-in-and-query model is inverted.**
   - **0.100b:** after `connection_successful`, the client pulls its data itself (`lobby_menu::query_lobby_info` sends `0x21` queries 4, 5, 9, 0xA, prices ×4 and 0).
   - **0.21d:** the client sends no initial query at all. The only `query_info_from_server` callers are `process_shop_action`, the quest-permitted branch of `on_lobby_packet_received` and `important_message_arrived`. `query_profile_contents` is called only after a profile list arrives.
   - **Consequence:** the server must push profiles, inventory, prices, money, skills and the rest unsolicited after sign-in. srp's request/answer handler (`lobby_server/mod.rs::answer`) would sit idle.
3. **`query_info_types` was renumbered and lost its padding.**
   - **0.100b:** `[0x21][type u8][3 pad]` with types 0..0xB.
   - **0.21d:** `[0x21][type u8][args]`, no padding, types 0..0x13.
   - **Dropped:** `q_profile_slots_restrictions`, `q_items_compatibility` and `q_player_skills_tree`. The client now reads that data from `resources/gameplay/db_static_dictionaries` and `db_skills_tree` (data files).
4. **The profile and inventory wire formats are completely new.**
   - **0.100b:** a raw 0x1B8-byte struct with 19 slots.
   - **0.21d:** a 23-slot bitmask encoding plus item "modification" lists. Up to 8 profiles (was 3), each with its own experience and level.
5. **The text-tag notices are gone.** None of the 0.100b chat tags exist in the 0.21d exe, in either the ASCII or the UTF-16 form: `#pc:[`, `#e:[`, `Player [`, `#q:[`, `#+p:[`, `#-p:[`, `#t:[`, `#mc:[`. They were replaced as follows:
   - `online_stats` (0x38) carries the counters.
   - `client_state` with status 1 carries the match-making screen data.
   - `squad_status` (0x3A) carries the squad.
   - A stats-sender chat line containing `#mstats` and `[<match_id>]` makes the client query `q_match_stats`.
6. **0x38 `online_stats` is mandatory.**
   - `set_online_stats` 0x1768A2 stores its third u32 (minutes until maintenance) at `lobby_menu+0x640`.
   - `update_play_button_lock` 0x1769A7 unlocks Play only when status == 0 **and** that value > 30.
   - A value < 60 shows `st_server_maintenance`.
   - A value of 0 calls `exit_and_launch_launcher`.
7. **New subsystems** the client speaks to:
   - squads (`squad_action` 0x2A, `squad_status` 0x3A, `q_squad_member_profile_contents`)
   - quests (0x2B, `q_player_quest_list`)
   - Elo (`q_player_elo_rating`, `q_players_elo_list`)
   - account services: premium and faction loyalty (`q_player_services`)
   - shop offers (`q_shop_items`)
   - unlocked factions (`q_unlocked_factions_mask`)
   - selling items (`shop_action`/`item_sell`)
   - profile activate and autobuy (`profiles_action` 0x29)
   - important messages (0xC2/0xC6/0xCC on the messaging port)
   - match statistics (`match_stats`, `player_result_full`)

## 1. Lobby: client → server (`messages_enum@client@lobby`)

| id | name | 0.100b layout (srp parses) | 0.21d layout | srp status |
|---|---|---|---|---|
| 0x20 | set_status_ready_for_match | u32 profile_id | u32 profile_id (active profile). `play_button_clicked` 0x1715C1 sends it only if the profile count is > 0 | OK |
| 0x21 | query_client_status | u8 type, 3 pad (+u32 profile, or +u8 faction for prices) | u8 type, then args: `q_profile_contents`(2) u32 profile_id; `q_squad_member_profile_contents`(0x10) u32; `q_players_elo_list`(0xC) u32 + u8; `q_player_match_stats`(0xE) u32 + u32; `q_match_stats`(0x13) u32 match_id. No padding (`query_info_from_server` 0x19D532 writes 2 bytes). Types sent by the client: 2, 5, 0xD, 0xF, 0x10, 0xC, 0xE, 0x13 | CHG: parser and enum must change |
| 0x22 | — | unused | unused | — |
| 0x23 | inventory_action / item_moved_to_slot(0) | u8 0, u8 n, n × {u32 profile, u32 id, u16 dict, 2 pad, u8 from, 3 pad, u8 to, 3 pad, u16 amount} (22 B) | u8 0, **u32 profile_id once**, u8 n, n × {u32 id, u16 dict_id, u8 source_slot, u8 target_slot, u32 count, u32 count_in_inventory} (16 B). `move_item` 0x19DE6E; the Flash fields are id/dict_id/sourceSlot/targetSlot/count/count_in_inventory. Target slot 100 = inventory. Ammo targets are remapped through the 8-entry table at 0x7DB854 (8, 9, 0xB, 0xC, 0x13..0x16) | CHG (rewrite of the parser; new slot enum) |
| 0x24 | shop_action | 0 item_bought: u16 dict, u32 amount, u8 faction, u8 0 | 0 buy_item: u16 dict, u32 amount, u8 faction, u8 0, **u8 target slot (100 = inventory), u32 active profile_id** (0x19DF6B). **1 item_sell (new):** u32 item id, u32 amount. **2 buy_shop_item (new):** u32 shop item id | CHG + NEW |
| 0x25 | skills_tree_action | 0: u8 5, 5 × {u8 id, u8 pts}, u8 n, n perks; 1 reroll: nothing | 0: **u32 profile_id**, u8 n, n × {u8 id, u8 pts}, u8 m, m perks (0x19E0CF); 1 reroll: **u32 profile_id** (0x19E273) | CHG (small) |
| 0x26 | sign_in_info | u32 session_id | u32 session_id; u8 `just_left_match` (lobby_client+0x5A5C, set by `close_current_match` / `process_player_left_match`); u32 `pending_match_stats_id` (+0x5A58, −1 = none, set from `#mstats`); u8 `need_shop_items` (1 when the client has no shop items yet). See `on_connected` 0x19D04F | CHG (+10 bytes; these replace the separate 0x27-on-connect) |
| 0x27 | discard_playing_order | u32 order_id, sent on the first sign-in after a match | u32 order_id (+0x1CC), sent only when not in a match (+0x1C8 == −1) from a Flash callback. **It now means "leave the queue"** | CHG (semantics) |
| 0x28 | ping_server | u32 alive_ms | **not sent** (no sender in the exe; `read_ping_server_answer` still exists) | GONE |
| 0x29 | profiles_action (new) | — | 4 profile_activate: u32 profile_id (`set_active_profile`, sent on every profile switch); 3 profile_autobuy: u32 profile_id, u8 on. Ids 0..2 are `*_unused` | NEW |
| 0x2A | squad_action (new) | — | 2 add_squad_member: string name; 3 drop_squad_member: u32; 4 leave_squad: none; 1 destroy_squad: none. 0 create_squad is never sent | NEW |
| 0x2B | quest_action (new) | — | 0 switch_quest: u32, u32 | NEW |

## 2. Lobby: server → client (`messages_enum@server@lobby`, dispatch `network_client::on_lobby_packet_received` 0x1A1930)

| id | name | 0.100b layout (srp writes) | 0.21d layout | srp status |
|---|---|---|---|---|
| 0x30 | connection_successful | nothing | **u8 client_state** (stored at +0x1B0). Read by `sign_in_on_packet_received` 0x19D2F2 without a length check | CHG (add 1 byte) |
| 0x31/0x32 | invalid_session_id / invalid_password | type only | type only. Anything other than 0x30 is logged and ignored while signing in | OK |
| 0x33 | connect_to_match_server | string host, u16 port, u32 match_id, u8 team | identical: string host, u16 port, u32 match_id (+0x1C8), u8 team (+0x1D0). Sets status = 2 and connects with the new `base_match_client` (lockstep protocol 0x40..0x5F: a different area) | OK on the lobby side |
| 0x34 | operation_permitted | u8 op (+ shop: u8 0, u16 dict, u32 id, u32 cond) | u8 op. **op 0x24:** u8 event: 0 → u8 n, n × {u16 dict_id, u32 id, u32 cond_or_count} (adds to the inventory, then the client queries money); 1 → u32 id, u32 amount_sold, u32 n, n × u32 profile_id (the client re-queries money and each profile); 2 → nothing (the client queries money and services). **op 0x29:** u8 event; 3 → u32 profile_id, u8 autobuy. **op 0x2B:** the client queries `q_player_quest_list` and `q_player_services`. Other ops are ignored | CHG (buy gains a count prefix) |
| 0x35 | operation_denied | u8 op, u8 faction, string | u8 op, u8 (unused), string → `on_operation_denied_received` | OK |
| 0x36 | client_status | u8 type + body | u8 type + body (§3) | CHG (renumbered) |
| 0x37 | ping_server_answer | u32 | u32 echoed time. The client sends no ping, so this is moot | GONE in practice |
| 0x38 | online_stats (new) | — | u32 players_online, u32 games_online, u32 minutes_to_maintenance. **Mandatory:** Play stays locked unless the third value is > 30, and 0 quits the game | NEW, mandatory |
| 0x39 | connect_to_mtv_room (new) | — | **No handler.** The dispatch's default branch sends it to `read_squad_info`. Never send it | — |
| 0x3A | squad_status (new) | — | u32 squad_id (0 = none, end); u8 n; u8 (+0x5A2D, leader index?); n × {u32, u32, u32, string name, bool, bool}; bool all_ready (+0x5A30); u32 (+0x5A24). Squad id ≠ 0 adds the squad chat tab and resubscribes the chat (partial) | NEW, optional |

## 3. `client_status` sub-types (`query_info_types@lobby`)

| 0.100b | 0.21d | 0.21d body (server → client) | srp status |
|---|---|---|---|
| 0 q_client_state: u8 status (0 surf, 1 mm_order, 2 mm, 3 in_match, 4 unknown) [+u32, u32, u8 if 1..3] [+opt string] | 0 q_client_state | u8 status (**0 surf, 1 in_match_making, 2 in_match, 3 unknown**). Status 1 adds 5 u32: order (+0x1CC), +0x1C0, +0x1C4, +0x1B8, +0x1BC, passed to `set_match_making_status(1B8, 1C0, 1C4, 1BC)`, the match-making timer and "match found" screen (meaning partial). Status 2 adds u32 order, u32 match_id, u8 team. Then an optional string (status line). Status 0 calls `game::switch_to_lobby` (when `lobby_menu+0xD8` == 0); status 1 shows match making (`read_status_info` 0x19D7DD) | CHG (values shifted, new payload) |
| 1 q_enumerate_profiles: u8 n ≤ 3, n × {u32 id, string ≤ 31} | 1 | u8 n ≤ **8**, n × {u32 profile_id, string name (buffer 64), u8 ?(+0x9E2), u8 level, u32 exp_total, u32 exp_prev_level, u32 exp_next_level, u32 ?(+0x9DC)}. `fill_profiles` shows experience_current = total − prev, next = next − prev, skill_level = level. **The client then sends `0x21 02 <id>` for each profile** | CHG |
| 2 q_profile_contents: raw `player_profile` 0x1B8 | 2 | u32 profile_id (must match a listed one, else the rest is not consumed); `player_profile::deserialize_static` 0x1F5C52: u8 (+0x5AC), bool (+0x5B0), string name, u32 (+0x9C8), **u32 slot bitmask (23 bits) + per set slot one `inventory_item_descr` in the slot's mode** (table at 0x877168: slots 0..7 and 10 mode 0, others mode 2), u32 modifier bitmask (20 bits, `dispersion_modifier`..`recoil_modifier`, `stamina_spending_speed_modifier`) + f32 per set bit; then u32 (+0x9DC) | REWRITE |
| 3 q_enumerate_inventory: u32 n, n × 16 B instance | 3 | u32 n, n × `inventory_item_descr` mode 2 = {u16 dict_id, u32 id, u16 condition_or_stack, u32 amount, u8 k, k × u32 (modification ids? `modifications.options`, unverified)} | REWRITE |
| 4 slots restrictions | — | gone (now from data) | GONE |
| 5 items compatibility | — | gone (now from data) | GONE |
| 6 q_price_items: u8 faction, u16 n, n × {u16 dict, u16 cost, u8 rep_level, pad} | **4** q_price_items | u16 n, n × {u32 cost, u16 dict_id, u8 level, u8 faction_id}, one list for all factions (layout from `buy_ok_clicked` 0x1716F6 and 0x174D57) | CHG |
| 7 q_account_money: u32 gold, u32 premium, u8 skill_pts, string name | **5** q_account_money | u32 (+0x5928), u32 (+0x592C) only (inline in `on_lobby_packet_received`) | CHG |
| 8 q_player_skills: u32 total, next, prev, u8 n, n × 2 B, u8 perks | **6** q_player_skills | u32 profile_id; **if it matches a profile:** u8 level, u32 total, u32 next, u32 prev; then u8 n, n × {u8 id, u8 pts}, u8 m, m perk bytes. Calls `fill_character_data` | CHG (small) |
| 9 q_player_skills_tree | — | gone (`db_skills_tree`) | GONE |
| — | 7 q_player_profile_leveling | no client handler (logs "Unknown client state") | — |
| 0xA q_service_prices: 3 × u32 | **8** q_service_prices | 3 × u32 (+0x5A4C/50/54). The meaning of each u32 is unverified | OK body, new id |
| 0xB q_player_reputations: u8 n, n × 4 B | **9** | u8 n, n × 8 B `player_reputation` ({u32 (also used as faction index), u16, u16}: partial) | CHG |
| — | 0xA q_player_base_info | u32 total players (+0x5A68), string (read into lobby_client+0, probably the account name) | NEW |
| — | 0xB q_player_elo_rating | u32, u16, u16 | NEW, optional |
| — | 0xC q_players_elo_list | u8 n, n × {string name, u32, u16, u16} | NEW, optional |
| — | 0xD q_player_quest_list | u32 n, n × 20-byte `quest_instance` (memcpy; max 4 active) | NEW |
| — | 0xE q_player_match_stats | `player_result_full` | NEW, optional |
| — | 0xF q_player_services | u8 n, n × {u8 type, u8 pad, u32 a, u32 b}: type 0 premium_access (a = value), 1..4 faction loyalty (b low byte) | NEW |
| — | 0x10 q_squad_member_profile_contents | u32, `deserialize_static`, u32 | NEW, optional |
| — | 0x11 q_shop_items | u32 n, n × `shop_items_container` {u32, u8, u32, string, string, u8 k, u8, k × `shop_item` {6 × u32, u8, u8}} | NEW (content needs data) |
| — | 0x12 q_unlocked_factions_mask | u8 mask (+0x5A6C) | NEW |
| — | 0x13 q_match_stats | `match_stats` {u32, u8, u8, u8, string, u16, string, u8, u16, string, u16, u16, string, u16, u16, u8 n, n × `player_result_brief` (string, u32, u16, u16, u16, u8, u16)} + `player_result_full` (~25 fields) | NEW (match results) |

## 4. Messaging (chat) protocol

| id | dir | 0.100b | 0.21d | srp status |
|---|---|---|---|---|
| 0xC1 message | c→s | u32 channel_id, string receiver, u8 channel, string body | same (`on_message_typed` 0x19B1xx; it now takes `char*`, not `wchar_t*`) | OK |
| 0xC2 important_message | c→s | — | u32 0, string target name, u8 type (`important_message_types`: 0 friendship, 1 quest, 2 squad_invite, 3 squad_invite_accepted, 4 reputation_level_unlocked, 5 drop), string text. **`add_to_friend_list` sends type 0, not a 0xC4 action** | NEW |
| 0xC3 sign_in_info | c→s | u32 session, u8 5 | same | OK |
| 0xC4 friendship_action | c→s | action 0..7 (add_friend 0, remove 1, add_ign 2, remove_ign 3, find 4, q_friends 5, q_ignore 6, update 7) | **renumbered:** add_friend 0 (unused, see 0xC2), add_friend_by_name 1, remove_friend 2, add_ignorable 3, add_ignorable_by_name 4 (string), remove_ignorable 5, find_players 6 (string), query_friend_list 7, query_ignore_list 8, update_friends_status 9 | CHG (shift ids) |
| 0xC5 subscription | c→s | 9 × u32 | 9 × u32. [1] general = **language channel id (1 russian, 2 english)**, [2] = −1, [4] = 1, [5]/[6]/[7] match/team from the order, [8] squad id (+0x1A4) | OK (check that the router accepts id ≠ 0) |
| 0xC6 important_message_read | c→s | — | u32 message_id, u8 answer | NEW |
| 0xC8 | s→c | connection_successful (unused) | unused | — |
| 0xC9 message | s→c | u8 sender_type, u32 id, string name, u8 channel, string body | same. sender_type 8 (stats) with `#mstats` in the body → `on_stats_message_arrived` parses `[N]` → `query_match_stats(N)`; anything else goes to chat (filter/censor) | OK layout; notice texts must change |
| 0xCA sign_in_info | s→c | (0.100b used **0xCB**) | string local_name. The server must send **0xCA** (`sign_in_on_packet_received` 0x1B32xx); after it the client subscribes and queries friends and ignore list | CHG (id) |
| 0xCB friendship_status | s→c | (0.100b used **0xCC**) | u8 action (new numbering), then: 7 friends: u16 n, n × {u32, string, u8 online}; 9 statuses: u16 n, n × {u32, u8}; 8 ignore / 6 found: u16 n, n × {u32, string}; others: u8 result (`'4'` = accepted). Same item layouts as 0.100b | CHG (id + action ids) |
| 0xCC important_message | s→c | — (0xCC was friendship_status) | u8 sender_type, u32 sender_id, string sender_name, u8 type, string text, u32 message_id → `important_message_arrived`. A squad invite is auto-handled through `add_squad_member` | NEW (and a **collision**: srp's current 0xCC friendship packets would be misread as important messages) |

The client-type enum gained `quest_processor_server_client_type` = 9. The channel enum is unchanged (9 channels). The text-tag conventions in `docs/notes/server-notices.md` (`#pc`, `#e`, `Player [`, `#q`, `#+p`, `#-p`, `#t`) are dead in 0.21d. Sent anyway, they would show as raw chat text.

## 5. Shared structs and enums srp writes

| item | 0.100b | 0.21d |
|---|---|---|
| `profile_slot_enum` | 19 slots (0..0x12): ammo1/2 for each weapon, 6 quick slots | **23 slots:** 0..0x12 as before (renamed ammo_slot_1..4), **+ ammo_slot_5..8 = 0x13..0x16**, max 0x17, carried_item 0x18, inventory_slot_id 100, shop_slot_id 200 |
| factions | srp: 1 scavengers, 2 black_market, 3 army, 4 fringe (factions_count 6) | 0 neutral, 1 scavengers, 2 blackmarket, 3 renaissance, 4 edge, 5 scientists, 6 mercenaries (count 7); max_faction_reputation_level 16 |
| skill boosters | 11 × {u8, f32} | replaced by 20 `*_modifier` f32 entries (bitmask-encoded) |
| profile limits | 3 profiles, names 32 | 8 profiles, name buffer 64, max_player_quests 4, max_drop_items 4 |
| item instance | 16 B {cond u32, amount u32, id u32, dict u16} | `inventory_item_descr` (0x3C in memory): dict u16, id u32, cond u16, amount u32, k × u32 modifications |
| price item | 6 B per faction | 8 B, one list, with faction_id |
| item dict ids | from the 0.100b `db_static_dictionaries` | **data-dependent:** the 0.21d `db_static_dictionaries` must be read; the 0.100b ids in `player_profile.rs` will not match |

## 6. Minimum server traffic

### To reach a usable lobby screen

Reconstructed from the client code; Flash-side gating is not checked.
1. Messaging port: answer 0xC3 with **0xCA** + name. Answer 0xC4 7/8 with 0xCB lists, which may be empty.
2. Lobby: on 0x26, read 10 bytes more than today, then send `0x30 <state u8 = 0>`.
3. Push, unsolicited:
   - **`0x36 01` profiles (≥ 1).** The client then sends `0x21 02 id` for each profile; answer with `0x36 02`.
   - `0x36 06` skills for the active profile (`fill_character_data`).
   - `0x36 03` inventory
   - `0x36 04` prices
   - `0x36 05` money
   - `0x36 08` service prices
   - `0x36 09` reputations
   - `0x36 0F` services (premium and loyalty)
   - `0x36 12` faction mask
   - `0x36 0A` base info
   - `0x36 0D` quests (an empty list is fine)
   - `0x36 11` shop items when the sign-in flag asks for them
   - `0x36 00` status 0
   - **`0x38` with minutes_to_maintenance > 30** (≥ 60 also avoids the maintenance banner)
4. Which of these the Flash lobby actually requires before it leaves its loading state is **unverified**. It needs `resources/flash_movies/lobby_menu.swf`.

### To start a match

1. The client sends `0x29 04 <profile>` (activate) on profile selection, then `0x20 <profile>` on Play.
2. The server may send `0x36 00` status 1 with 5 u32 (the match-making screen; optional, unverified).
3. The server then sends **`0x33` host, port, match_id, team**. That alone sets the in-match state.
4. Afterwards everything depends on the 0.21d match protocol, which is lockstep and a different area.
5. On the next sign-in, the 0x26 flag says the client left a match. When the stats id is set, send `0x36 13` match stats.
6. To trigger stats live, send a messaging 0xC9 from sender_type 8 whose body contains `#mstats [<match_id>]`. The client then asks `0x21 13 <id>`.

## 7. Per-area estimate

| area | estimate | notes |
|---|---|---|
| TCP framing, strings, connection lifecycle | unchanged | same `tcp_packet` header rules |
| Lobby sign-in | small change | +10 bytes in, +1 byte out; sign-in now carries the left-match and stats flags |
| Query flow (0x21/0x36) | rewrite of the flow, small change in the parser | renumbered types, no padding, the server must push all initial data |
| Profiles, inventory, equip (0x23, q2, q3) | rewrite | new profile serializer (bitmask slots and modifiers), 23 slots, 8 profiles with per-profile XP and level, item modification lists, new move_item format, autobuy/activate. Item ids need the 0.21d data |
| Shop, prices, money (0x24, q4, q5, q8, q0x11, q0x12, q0xF) | small to medium change | buy gains slot+profile, selling is new, the price struct changed, money lost name and skill points, shop offers and premium/loyalty are new (contents data-dependent) |
| Matchmaking, ready, queue, match address (0x20, 0x27, 0x33, status) | small change | 0x20/0x33 are byte-identical; client_state values shifted and status 1 carries 5 u32; queue info no longer comes as chat tags; 0x27 now means "leave queue" |
| Match results and rewards | rewrite | structured `match_stats`/`player_result_*` via q 0x13/0x0E, triggered by `#mstats`; the `#e`/`Player [` notices are gone; reward logic depends on the 0.21d data (leveling/drops/quests options) |
| Skills and perks (0x25, q6) | small change | +profile_id, skill count variable; the tree comes from the `db_skills_tree` data |
| Friends | small change | message ids 0xCB→0xCA (sign-in), 0xCC→0xCB (friendship), action ids shifted, add-friend via important messages |
| Clans | unchanged (none) | neither build has a clan protocol; only the clan channel id exists |
| Chat channels | small change | language channel id in the subscription; same text layout |
| Stats notices | rewrite (simpler) | replaced by 0x38 (mandatory) plus the `#mstats` trigger; all current srp tag notices must go |
| Squads (0x2A, 0x3A, q 0x10, important msg type 2/3) | new subsystem, optional | client works without it (squad id 0) |
| Quests (0x2B, q 0xD) | new subsystem, needs data | an empty list is probably enough (unverified) |
| Elo / ratings (q 0xB, 0xC, 0xE) | new subsystem, optional | |
| Important messages (0xC2, 0xC6, 0xCC) | new subsystem | friend requests and squad invites depend on it |

## 8. Open points / data needed

The full install is at `E:\sur-res-pro\downloads\v021d\extracted\game\resources.db`. Request these files from the coordinator:
- `resources/flash_movies/lobby_menu.swf`: which pushes the lobby waits for, and the meaning of the profile and status fields.
- `resources/gameplay/db_static_dictionaries`: the item dictionary (dict ids, stack flags, slot categories, compatibility).
- `resources/gameplay/db_skills_tree`
- `resources/gameplay/items/modifications/modifications.options`
- `resources/gameplay/quests/quests.options`
- `resources/gameplay/drops/drops.options`
- `resources/gameplay/static_game_parameters.options`

Unverified points:
- the meaning of the profile fields +0x9E2, +0x9C8, +0x9DC and +0x5AC/+0x5B0
- the match-making status u32s
- the `player_reputation` fields
- the `shop_item` fields
- the `quest_instance` layout
- whether the u32 list in `inventory_item_descr` holds modification ids
- whether lobby_menu+0x640 is 0 before the first 0x38

Not checked here: how the 0.21d client gets the lobby and messaging addresses (login `connection_info`; the login enum moved to 0x08..0x16, with `sign_in_invalid_version` 0x14 and `sign_in_added_to_online_queue` 0x16 new). That belongs to the login area.
