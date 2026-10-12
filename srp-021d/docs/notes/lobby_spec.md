# Survarium 0.21d (build 2010) lobby and messaging wire spec

This spec is for writing a server. Every layout was read from the 0.21d client's reader and writer functions. Sources:
- `v021d_disasm/survarium_disasm.txt`. Addresses are VAs with image base 0x10000.
- `lobby_menu.swf`, exported with ffdec to `scope021d/spec_work/lobby_swf/scripts`.
- The data dumps in `scope021d/txt` and `scope021d/spec_work`.

**Tags:**
- **[V]** means verified instruction by instruction.
- **[U]** means unverified (meaning inferred, or not traced).
- **[P]** means partial: the byte layout is verified but the meaning is not.

Work files are under `scope021d/spec_work/`:
- `g.sh` prints one function.
- `xref.sh` does a fast cross-reference.
- `py/cfg.py` parses the config dumps.
- `py/examples.py` produces the example bytes.

---

## 0. Ground rules (read first: these decide whether the client crashes)

| # | Rule | Evidence |
|---|---|---|
| R1 | **Framing.** One message per frame. Short frame: `u8 len` (1..255) followed by the payload. Long frame: `00`, then `u16 len` (LE), then the payload. The payload starts with the message id. The client sends the short form when len < 256 and never sends len 0. It receives either form, and its buffer is resized per frame, so there is no cap below 65535. | send 0x284EFF, receive 0x285249 / 0x28589A [V] |
| R2 | **The reader has no bounds check.** Every `r<T>` is a raw `movs` that advances a cursor. Reading past the end of a frame returns stale bytes from the buffer. A short message does not error; it corrupts state silently. The only length check in the lobby is the optional status string of 0x36 00. | 0x19ACEA, 0x19F14A, 0x155E6E [V] |
| R3 | **Strings are `u8 len` + bytes, with no NUL on the wire.** The reader copies `len` bytes and appends a NUL, with no capacity check (0x19ACEA, 0x19C628). Each string's maximum length is therefore its destination buffer minus 1. The limits are listed per field. Overflowing one corrupts the struct after it. | [V] |
| R4 | **Booleans written by the client are `00` / `FF`.** `w<bool>` at 0x155EF9 writes `(v!=0) ? 0xFF : 0`. Parse them as `!= 0`. Booleans the client reads are `!= 0` (0x155E6E). | [V] |
| R5 | **After sign-in, every unknown lobby id goes to `read_squad_info`.** `on_lobby_packet_received` 0x1A1930 handles 0x33..0x38 explicitly. Every other id, including 0x30, 0x31, 0x32 and 0x39, falls into the default branch, which calls `read_squad_info`. After 0x30 the server may send only 0x33, 0x34, 0x35, 0x36, 0x37, 0x38 and 0x3A. | [V] |
| R6 | **Dictionary lookups have no bounds check.** `items_dictionary::item_by_id` 0x15FDAA and `item_modification_by_id` 0x1707FD search linearly with no end check. An unknown `dict_id` or modification id runs off the array and **crashes**. Use only ids from the 0.21d data (§7). `get_quest_descriptor` 0x1C6EAF logs, then returns the element one past the end: garbage. | [V] |
| R7 | **Fixed capacities:**<br>• profiles ≤ 8<br>• modifications per item ≤ 8 (otherwise a fatal "buffer overflow" assert)<br>• squad members ≤ 3<br>• shop items per container ≤ 5<br>• loyalty services ≤ 8<br>• match players ≤ 20<br>• drops ≤ 4 | per message [V] |
| R8 | **Feedback loops to avoid:**<br>• Do not answer `0x29 04` (activate profile) with a 0x36 02 for that profile. The 0x36 02 calls `on_profile_changed`, which sends `0x29 04` again.<br>• Do not answer `0x21 02` with 0x36 01. 0x36 01 makes the client send `0x21 02` for every profile. | [V] |

Message processing order: frames are queued as `receive_response` objects (0x238040) and handled on the main thread in arrival order [V].

---

## 1. Connection, sign-in and the two sockets

### 1.1 Telling messaging from lobby on one port [V]

The client asks the browser over HTTP for each address separately: `…&type=2&…` returns the lobby, `…&type=4&…` returns messaging. Format string 0x80D320: `%s&type=%d&local_ip=%s&login_ip=%s&ver=%s`, ver = `0.21d` (0x16DA1A). The server can therefore give each its own port.

On one shared port, sniff the first payload byte that the client sends. The client sends first; the server never sends a greeting.

| Socket | First frame (bytes on the wire) | Source |
|---|---|---|
| lobby | `0B 26 <u32 session> <u8 just_left_match 00/FF> <u32 pending_stats_match_id> <u8 need_shop_items 00/FF>` | `lobby_client::on_connected` 0x19D04F |
| messaging | `06 C3 <u32 session> 05` | `messaging_client::on_connected` 0x1A00E4 |

Both session ids come from the login reply. The lobby's copy is at lobby_client+0x44, copied from the same value the messaging client reads (login +0x188, 0x16DBAA / 0x16DC57) [V].

Messaging does not connect when the host is `"x"` or the port is 0 (0x1A007A) [V].

### 1.2 Lobby 0x26 sign_in_info (client → server), payload 11 bytes [V]

| off | type | field | note |
|---|---|---|---|
| 0 | u8 | 0x26 | |
| 1 | u32 | session_id | lobby_client+0x44 |
| 5 | bool(00/FF) | just_left_match | +0x5A5C; reset to 0 after sending |
| 6 | u32 | pending_match_stats_id | +0x5A58, 0xFFFFFFFF = none. Set by an `#mstats [N]` chat line received while disconnected; reset to −1 after sending. If set, answer with 0x36 13 for that match (optional) |
| 10 | bool(00/FF) | need_shop_items | FF when the client has no shop container array yet (+0x5A60 == NULL, i.e. 0x36 11 has never been received in this process) |

### 1.3 0x30 connection_successful (server → client) [V] `sign_in_on_packet_received` 0x19D2F2

| off | type | field |
|---|---|---|
| 0 | u8 | 0x30 |
| 1 | u8 | client_state (`client_state_enum`) → lobby_client+0x1B0 |

`client_state_enum`, the same enum as status in 0x36 00:
- 0 `surf_lobby_menu`
- 1 `in_match_making`
- 2 `in_match`
- 3 `unknown` (the constructor's default)

What the client does with it:
- Resets the reconnect counter (+0xBC = 0).
- Swaps in the main packet handler.
- Runs the signed-in callback.
- **Changes no UI.**

The byte matters as follows:
- **Play button.** `update_play_button_lock` (0x1769A7) enables Play only when this state is 0.
- **Reconnects.** `on_error` (0x19D189) reconnects automatically only when the state is 0 or 1.

**Send 0.**

Any id other than 0x30 while signing in is logged and ignored, and the client keeps waiting.

### 1.4 0x31 / 0x32 [V]

There is no handler. While signing in they are logged and ignored. After sign-in they would be parsed as squad_status (R5). **Never send them.** To reject a session, close the socket. The client then calls `on_error`, sets +0xBC++, and retries the HTTP query every 5 s. After more than 3 failures it signs out to the login screen (`network_client::tick` 0x1A2A82).

### 1.5 Messaging sign-in [V] `messaging_client::sign_in_on_packet_received` 0x1B32A0

**Client → server, 0xC3:**

| off | type | field |
|---|---|---|
| 0 | u8 | 0xC3 |
| 1 | u32 | session_id |
| 5 | u8 | 5 (`account_client_type`) |

**Server → client, 0xCA:**

| off | type | field |
|---|---|---|
| 0 | u8 | 0xCA |
| 1 | string | local player name → messaging+0x118, **buffer 64 (≤ 63 chars)**. Used as the chat name |

Any other id is logged and ignored, and the client keeps waiting. After 0xCA the client sends, in order:
1. `0xC5` subscription (the language id, messaging+0x1A0, is 1 from the constructor 0x19FEE8 and set to 2 at 0x1B341A when `strcmp(localization, "english") == 0`; so `russian`, `ukrainian`, `original` and anything else are all 1. srp shares the general channel across both ids)
2. `0xC4 07` (query the friend list)
3. `0xC4 08` (query the ignore list)

After sign-in, 0xCA is "unknown" and is ignored.

---

## 2. Lobby server → client

The dispatcher is `network_client::on_lobby_packet_received` 0x1A1930. Each row is [V] unless marked otherwise.

### 2.1 0x33 connect_to_match_server

| type | field | note |
|---|---|---|
| string | host | stack buffer at esp+0xB0, next local at esp+0xF0 → **≤ 63 chars** |
| u16 | port | |
| u32 | match_id | → +0x1C8 (also used as the chat match channel id) |
| u8 | team | → +0x1D0 (`game_team_id`: 0 team_1, 1 team_2, 2 neutral, 3 undefined) |

Effects:
- Sets client_state to 2.
- Opens the match connection.
- Calls `messaging.assign_match_channel_order(match_id, team)`, which re-sends 0xC5.

### 2.2 0x34 operation_permitted

Layout: `u8 op`, then a body that depends on op. op = the client message id being confirmed.

**op 0x24 shop_action**, `process_shop_action` 0x1A2179. Next field is `u8 event`:

| event | body | client action |
|---|---|---|
| 0 buy_item | `u8 n`, n × {`u16 dict_id`, `u32 item_id`, `u32 value`} | For each item:<br>• if item_id is already in the inventory: `condition_or_stack += value`<br>• otherwise: append {dict, id, cond=value, amount=0, no mods}<br>Then `fill_inventory_contents` and sends `0x21 05`. Bought items do **not** go into a slot locally |
| 1 item_sell | `u32 item_id`, `u32 amount`, `u32 n`, n × `u32 profile_id` | Finds the item:<br>• if the dict `is_stack`: `cond -= amount`, erased when it reaches 0<br>• otherwise: erased<br>Then `fill_inventory_contents`, sends `0x21 05`, then `0x21 02 <pid>` for each listed profile. The loop counter is u8, so n ≤ 255 |
| 2 buy_shop_item | — | sends `0x21 05` and `0x21 0F` |
| other | — | ignored |

**op 0x29 profiles_action**, `process_profiles_action` 0x1A212B. Next field is `u8 event`:
- 3 (autobuy): `u32 profile_id`, `u8 on`. Calls `set_autobuy_checkbox(profile_id, on)` → Flash `set_autobuy`.
- Any other event reads nothing more.

**op 0x2B quest_action:** no body. When connected (+0x1B4), the client sends `0x21 0D` and `0x21 0F`.

**Any other op:** nothing more is read, and the message is ignored. In particular, 0x23 (move item) has **no** permitted handler.

### 2.3 0x35 operation_denied, `on_operation_denied_received` 0x16F29F

| type | field |
|---|---|
| u8 | op (client message id) |
| u8 | unused [V] |
| string | text key (translated). Stack buffer is 0x200 bytes, so any length ≤ 255 |

Behaviour by op:

| op | behaviour |
|---|---|
| 0x20 | If text == `st_not_logged_id`: signs out, disconnects lobby and messaging, returns to login. Otherwise: `root.show_system_message(99, "debug", text)` |
| 0x23, 0x24, 0x25, 0x29 | `show_system_message(99, "debug", text)` |
| 0x2A | silently ignored (srp answers a change with the next `3A`) |
| other | logged |

### 2.4 0x36 client_status

Layout: `u8 type`, then the body (§3). After the body, `lobby_menu::on_client_status_received(type)` (0x16F090) runs for every type.

### 2.5 0x37 ping_server_answer [P]

Body: `u32`. The client converts it and calls `root.set_ping`. The client never sends 0x28, so do not send this.

### 2.6 0x38 online_stats, `read_online_stats` 0x19E651 / `set_online_stats` 0x1768A2 — **mandatory**

| type | field | UI |
|---|---|---|
| u32 | players_online | `root.set_players_online` |
| u32 | games_online | `root.set_games_online` |
| u32 | minutes_to_maintenance | stored at lobby_menu+0x640 |

Effects of minutes_to_maintenance:
- **< 60:** `root.set_warning_text("<st_server_maintenance> :%dm")`.
- **0:** calls `game::exit_and_launch_launcher` (**quits the game**).
- **Play:** `update_play_button_lock` enables Play (and, through Flash, unlocks the inventory window) only when status()==0 **and** this value > 30.

The value starts at 0 (lobby_menu ctor 0x16E9FA). **Send ≥ 60.**

### 2.7 0x3A squad_status (optional), `read_squad_info` 0x19EE6D

| type | field | mem |
|---|---|---|
| u32 | squad_id | +0x5A28. **If 0, nothing else is read** (message = `3A 00 00 00 00`) |
| u8 | n (**≤ 3**) | +0x5A2C |
| u8 | ? [P] | +0x5A2D |
| n × member: u32 | account/player id [P] | m+0 |
| u32 | profile_id | m+4 (when not one of the player's own profiles, the client sends `0x21 10 <profile_id>`) |
| u32 | ? [P] (compared with the cached character) | m+8 |
| string | name (**≤ 63**) | m+0x10 |
| bool | online | m+0xC |
| bool | ready | m+0xD |
| bool | in_squad flag | +0x5A30. Gates Play: Play then requires ready_count == n−1 |
| u32 | revision | +0x5A24 (log only) |

Effects:
- A squad_id different from the chat's squad channel updates the 0xC5 subscription.
- squad_id ≠ 0 adds the squad chat tab; 0 removes it.
- Then `fill_squad_info` runs.

When the squad id is 0, the trailing bool and u32 are **not** read.

**How the client uses it [V]** (`fill_squad_info` 0x1780FE, `update_play_button_lock` 0x1769A7, `read_status_info` 0x17635B):
- **The member list includes the player itself.**
- A member whose profile_id is one of the player's own profiles is the player. For anyone else the client asks `21 10 <profile_id>` and shows that member's character next to its own. m+8 is compared with the shown character's revision; a different value makes it ask again.
- **The trailing bool is commander mode** (`set_squad_commander_mode`).
  - A commander's Play is unlocked only when ready_count == n − 1.
  - A non-commander's Play is not gated.
- `set_squad_player(index, name, m+0, ready)`. `2A 03` drops by m+0.
- On the queue screen (status 1), the client shows `st_squad_not_ready` while n ≠ ready_count. ready_count counts every member, the player included.
- From this, Play is a member's "ready" [I]:
  - Members press Play and see the queue screen saying the squad isn't ready.
  - The commander's Play, unlocked once the others are ready, completes it.

**The flow srp serves:**
1. The inviter sends `C2` type 2 to the invited player. It is passed on only to a player in the chat who doesn't ignore the sender.
2. The invited player answers `C6 id 00`.
3. Its client then sends `C2` type 3 ("membership accepted") to the inviter. It is passed on only after that accept.
4. The inviter's client answers it by itself and sends `2A 02 <name>`. The lobby adds the player only for that accepted invite, at most 3 members.
5. Every member's lobby connection sends the new `3A` on its next wake-up (at most 1 s).

A membership change unreadies everyone; a member waiting in the queue goes back to the lobby state. A commander who leaves hands the squad to the next member, and a squad of one ends.

---

## 3. 0x36 client_status sub-types (`query_info_types`)

| type | name | handled? |
|---|---|---|
| 0x00 | q_client_state | yes |
| 0x01 | q_enumerate_profiles | yes |
| 0x02 | q_profile_contents | yes |
| 0x03 | q_enumerate_inventory | yes |
| 0x04 | q_price_items | yes |
| 0x05 | q_account_money | yes |
| 0x06 | q_player_skills | yes |
| 0x07 | q_player_profile_leveling | **no handler**: only the type byte is consumed. Do not send |
| 0x08 | q_service_prices | yes |
| 0x09 | q_player_reputations | yes |
| 0x0A | q_player_base_info | yes |
| 0x0B | q_player_elo_rating | yes |
| 0x0C | q_players_elo_list | yes |
| 0x0D | q_player_quest_list | yes |
| 0x0E | q_player_match_stats | yes |
| 0x0F | q_player_services | yes |
| 0x10 | q_squad_member_profile_contents | yes |
| 0x11 | q_shop_items | yes |
| 0x12 | q_unlocked_factions_mask | yes |
| 0x13 | q_match_stats | yes |

Notation used below:
- `P(i)` = lobby_client + 0x9C0 + i·0x9E8 (`lobby_player_profile`, i < 8).
- `player_profile` is the first 0x9D0 bytes of it.

### 3.0 type 0x00 client state, `read_status_info` 0x19D7DD [V]

| type | field | condition |
|---|---|---|
| u8 | status → +0x1B0 | always |
| u32 | order_id → +0x1CC | status 1 or 2 |
| u32 | place → +0x1C0 (0 = "match found") | status 1 |
| u32 | queue_total → +0x1C4 [P] | status 1 |
| u32 | current_wait_seconds → +0x1B8 | status 1 |
| u32 | average_wait_seconds → +0x1BC | status 1 |
| u32 | match_id → +0x1C8 | status 2 |
| u8 | team → +0x1D0 | status 2 |
| string | status text → buffer_string +0x92C, **capacity 128 (≤ 127)**; overflow hits the profile count at +0x9B8 | **only if bytes remain in the frame** |

**Status 0:**
- order, match and team are set to −1, −1, 3.
- 0x1B8..0x1C4 are zeroed.
- UI: if lobby_menu+0xD8 == 0, calls `game::switch_to_lobby`; then `show_match_making(false)`; then `update_play_button_lock`.

**Status 1:**
- match_id is set to −1 and team to 3.
- UI: `show_match_making(true)`, then `set_match_making_status(place, queue_total, avg_wait, cur_wait)`:
  - place == 0 → `set_mm_sync_state("st_mm_match_found")` (latched by lobby_menu+0x6AC);
  - in a squad that is not all ready → `st_squad_not_ready`;
  - otherwise → `set_mm_current_time(cur_wait as mm:ss / hh:mm:ss)`, `set_mm_place(place, queue_total)`, `set_mm_average_time(avg_wait)`.

**Status 2:**
- 0x1B8..0x1C4 are zeroed.
- UI: only `update_play_button_lock`.

**Status 3 or any other value:** only +0x1B0 is set, plus `update_play_button_lock`.

After any status, `messaging.assign_match_channel_order(match_id, team)` runs; it ignores match_id −1.

### 3.1 type 0x01 profile list, `read_enumerate_profiles_info` 0x19D942 [V]

Layout: `u8 n` (→ +0x9B8), then n × the record below. **n ≤ 8.** There is no check: n > 8 overwrites the inventory vector at +0x5900, which **crashes**.

| type | field | mem | used by |
|---|---|---|---|
| u32 | profile_id | P+4 | id used in every later message. **Use non-zero, distinct ids** |
| string | name (**≤ 63**) | P+8 (64-byte buffer) | Flash `nickname` |
| u8 | autobuy flag | P+0x9E2 | `set_autobuy_checkbox(profile_id, flag)` in `player_parameters_ready` 0x174C7D |
| u8 | level | P+0x9E3 | Flash `skill_level`. Also the number of skill points unlocked (`points_unlocked`; `points_available` = level − Σ skill points) |
| u32 | exp_total | P+0x9D0 | `experience_current` = total − level_start (**u32 subtraction: keep total ≥ level_start**) |
| u32 | exp_level_start | P+0x9D8 | |
| u32 | exp_next_level | P+0x9D4 | `experience_next_level` = next − level_start |
| u32 | revision | P+0x9DC | `lobby_character::setup_profile` 0x1A481B skips rebuilding the 3D character when profile_id **and** revision equal the cached ones. **Bump it whenever the equipment changes** [V logic, name inferred] |

Then `on_client_status_received(1)`:
1. sends `0x21 02 <P(i).profile_id>` for i = 0..n−1;
2. `fill_profiles` → `root.setup_player_profiles([{nickname, icon:1, experience_current, skill_level, experience_next_level}])` and `root.set_add_profile_button_enabled(n < 8)`.

### 3.2 type 0x02 profile contents, `read_profile` 0x19DA56 [V]

| type | field |
|---|---|
| u32 | profile_id. Searched among **all 8** slots' P+4. **If none matches, nothing more is read** (the remaining bytes are left unparsed) |
| … | `player_profile::deserialize_static` 0x1F5C52 (below) into P(i) |
| u32 | revision → P+0x9DC (overwrites the value from 0x01) |

After the read, P+0x5AC (team) is forced to 0. If i == lobby_menu+0x698 (the active profile index, initially 0), `on_profile_changed(i)` runs (0x1745C0):
- `root.player_profile.selectProfile(i)`;
- `lobby_character::setup_profile` builds the 3D model from the slots;
- `root.fill_player_profile_items({items:[{itemId, slotId, condition_or_stack}], name, profileId, icon:1})` for every slot whose **item id ≠ 0**;
- `set_active_profile` → **the client sends `0x29 04 <profile_id>`**;
- `player_parameters_ready`: weight, slot counts, autobuy;
- `root.set_active_faction(faction, seller name)`.

**Flash requirement [V from SWF]:** `PlayerProfile.fillProfileItems` looks up every `itemId` in the inventory dictionary and dereferences the result without a null check (`_loc12_.is_stack`). Every item id in a slot must therefore already be in the inventory (0x36 03) **before** the 0x36 02 of the active profile arrives. Otherwise the paper doll stays empty; the AS3 error aborts that call.

**`deserialize_static`** (P = the profile):

| type | field | mem | note |
|---|---|---|---|
| u8 | team | P+0x5AC (u32) | overwritten with 0 by read_profile; `game_team_id` in a match |
| bool | is_local? [U name, from the 0.100b struct] | P+0x5B0 | not used in the lobby (lobby_character forces 0). Send 0 |
| string | name (**≤ 63**) | P+8 | |
| u32 | new-player protection counter [P] | P+0x9C8 | Only use: `new_player_damage_protector::reduce_damage` 0x204718, factor = max(0, 1 − 0.1·value). Send 0 or ≥ 10 [U semantics] |
| u32 | slot mask (bits 0..22) | | bit k set → one `inventory_item_descr` follows for slot k, in **ascending** k, with mode = `MODE[k]` |
| … | items | P+0x48 + k·0x3C | clear bit → the slot is zeroed |
| u32 | modifier mask (bits 0..19) | | |
| f32 × popcount | modifier values | P+0x5B8 + 4·bit | in bit order. The skills' boosters, summed (`v021d/skills.rs`): in a match the player reads its modifiers only from here [V] |

**Slot table `MODE[k]`** at 0x877168 (`slot_serialize_mode_enum`). Read-only; its only other users are `base_player::(de)serialize` [V]:

| k | slot | mode | k | slot | mode |
|---|---|---|---|---|---|
| 0 | helmet_slot | 0 | 12 | ammo_slot_4 | 2 |
| 1 | mask_slot | 0 | 13 | quick_slot1 | 2 |
| 2 | torso_slot | 0 | 14 | quick_slot2 | 2 |
| 3 | back_slot | 0 | 15 | quick_slot3 | 2 |
| 4 | pants_slot | 0 | 16 | quick_slot4 | 2 |
| 5 | gloves_slot | 0 | 17 | quick_slot5 | 2 |
| 6 | boots_slot | 0 | 18 | quick_slot6 | 2 |
| 7 | weapon1_slot | 0 | 19 | ammo_slot_5 | 2 |
| 8 | ammo_slot_1 | 2 | 20 | ammo_slot_6 | 2 |
| 9 | ammo_slot_2 | 2 | 21 | ammo_slot_7 | 2 |
| 10 | weapon2_slot | 0 | 22 | ammo_slot_8 | 2 |
| 11 | ammo_slot_3 | 2 | | | |

Modes: 0 = `serialize_just_condition_stack_values`, 1 = `serialize_just_amount_values` (unused by the lobby), 2 = `serialize_both_values`.

**`inventory_item_descr::deserialize(mode)`** 0x1F5B53. The struct is 0x3C bytes in memory:

| type | field | mem | modes |
|---|---|---|---|
| u16 | dict_id | +0xC | all. **Must exist in items_dict (R6)** |
| u32 | item_id (instance id) | +8 | all. **0 means an empty slot** to `on_profile_changed` and `query_inventory_item` |
| u16 | condition_or_stack (zero-extended to u32) | +0 | 0 and 2 |
| u32 | amount_in_inventory | +4 | 1 and 2. The name comes from the debug string "slots[ i ].amount_in_inventory" at 0x1D5EC2. Not shown by the lobby UI [P] |
| u8 | k (**≤ 8**: inline `buffer_vector<u32>` capacity 0x20 B; k > 8 is a fatal assert) | +0x10 | all |
| k × u32 | modification ids (`modifications.options` → `modifications[].id`; **must exist, R6**) | +0x1C | all |

The 36 modification ids are:
- 0
- 1000, 1010, 1020, 1030, 1031, 1040, 1041, 1050, 1051
- 1060, 1070, 1080, 1090, 1100, 1110, 1120, 1130, 1140, 1150, 1160, 1170, 1180, 1190
- 2000, 2010, 2020, 2030, 2040, 2050, 2060, 2070, 2080, 2090, 2091, 2092

Id 0 is a real modification (`st_ppsh_name`, faction 3), not "none". Send k = 0. The items' own `default_modifications` come from the data.

The value of `condition_or_stack`:
- **Stack items** (`is_stack`: ammo, drugs, devices, grenades): a count. In an ammo slot it is the rounds loaded; the Flash ammo UI divides it by `clip_size`.
- **Other items:** a condition. Its range is not checked by the exe or by Flash; 100 is a safe value [U].

**Modifier bit order** (`player_params_modifiers_enum`, PDB) [V]:

| bit | modifier | bit | modifier |
|---|---|---|---|
| 0 | dispersion | 10 | engineer_success_chance |
| 1 | aiming_speed | 11 | device_damage |
| 2 | health_regeneration_speed | 12 | splash_radius |
| 3 | stamina_regenation_speed | 13 | reload_speed |
| 4 | movement_speed | 14 | artefact_slots |
| 5 | max_carried_weight | 15 | device_slots |
| 6 | pain_health | 16 | ammo_bags |
| 7 | artefact_search_time | 17 | quick_slot_size |
| 8 | anomaly_damage | 18 | recoil |
| 9 | engineer_use_time | 19 | stamina_spending_speed |

The names above drop the `_modifier` suffix.

### 3.3 type 0x03 inventory, `read_enumerate_inventory_info` 0x19DB20 [V]

Layout: `u32 n`, then n × `inventory_item_descr` in **mode 2** (always). n is not capped (heap vector).

Then `fill_inventory_contents` (0x173DD2) → `root.inventory_list.setupInventory([{id, dictId, faction_id, condition, condition_or_stack, modifiers:[{icon, name, props}]}])`:
- `faction_id` = the dict item's faction, overridden by any modification with faction ≠ 7.
- `amount_in_inventory` is **not** passed to Flash.
- A modification id is looked up in `items_dictionary`. An unknown id **crashes** (R6).

### 3.4 type 0x04 prices, `read_price_items` 0x19DBDF [V]

Layout: `u16 n`, then n × 8-byte `price_item` (memcpy):

| off | type | field |
|---|---|---|
| 0 | u32 | cost |
| 4 | u16 | dict_id |
| 6 | u8 | level (reputation level needed) |
| 7 | u8 | faction_id (seller) |

Then `on_price_items_arrived` → `root.setup_shop_items([{faction_id, level, dictId, cost}])`. The client's `buy_ok_clicked` also uses this list to find the seller faction when Flash passes faction 0 (0x17171D).

### 3.5 type 0x05 money (inline in the dispatcher at 0x1A1B25) [V]

Layout: `u32 generic_money` (+0x5928), `u32 premium_money` (+0x592C).

Then `reset_account_money` → `root.setPlayerInfo({nickname: lobby_client+0 (from 0x0A), generic_money, premium_money})`. **Send 0x0A before 0x05**, or the nickname is empty.

### 3.6 type 0x06 skills, `read_player_skills` 0x19DC4D [V]

| type | field | condition |
|---|---|---|
| u32 | profile_id | always. Searched among the **n listed** profiles only |
| u8 | level → P+0x9E3 | **only if found** |
| u32 | exp_total → P+0x9D0 | only if found |
| u32 | exp_next_level → P+0x9D4 | only if found (**note the order differs from 0x01**) |
| u32 | exp_level_start → P+0x9D8 | only if found |
| u8 | n_skills (+0x5914) | always |
| n × {u8 skill_id, u8 points} | skills (+0x5910) | always |
| u8 | n_perks (+0x5A38) | always |
| n × u8 | perk ids (+0x5A34) | always |

**If profile_id is not listed, the 13 profile bytes are not consumed**, and the skill list is misparsed. Always send 0x01 first.

Then `fill_character_data` (0x175809) runs for the **active** profile:
- For each skill it reads `db_skills_tree` → `skill_<id>` → `levels.skill_level_1..points` → `boosters`. **Use skill ids 1..5 and points ≤ 20.** A missing key goes through `binary_config_value::operator[]` with an unknown result [U]: avoid it.
- Calls `root.set_char_stats(params)` and `root.fill_char_info({trees:[{id, points}], points_available = level − Σpoints, points_unlocked = level, experience_current, experience_next_level = max(0, next − start), profile: P+0x9E0 (u16, set only by 0x13), perks:[…], nickname})`.

### 3.7 type 0x08 service prices, `read_service_prices` 0x19E304 [V]

Layout: `u32 a` (+0x5A4C), `u32 b` (+0x5A50), `u32 c` (+0x5A54).

`fill_service_prices` sends `root.set_reroll_cost(a)` and `root.set_profile_cost(b)`. `c` is stored and **never read** by the exe.

### 3.8 type 0x09 reputations, `read_player_reputations` 0x19E4CD [V]

Layout: `u8 n` (+0x591C), then n × 8-byte `player_reputation` (memcpy, heap, no cap):

| off | type | field |
|---|---|---|
| 0 | u32 | faction_id. Only 1..4 are displayed; others are skipped |
| 4 | u16 | not read anywhere in the exe (only memcpy'd) |
| 6 | u16 | reputation points |

Then `on_player_reputations_arrived` (0x17673A), per faction 1..4:
- level = the number of `factions_dict[faction].levels[].value` thresholds ≤ points, minus 1, computed from the dictionary;
- calls `root.setup_player_progress(faction, level, points, lockText)`;
- lockText = "" if bit (faction−1) of the 0x12 mask is set, otherwise `blocked_factions_text[faction]`.

**Send 0x12 before 0x09.**

### 3.9 type 0x0A base info (inline at 0x1A1BE9) [V]

Layout: `u32 ranked_players_total` (+0x5A68, shown by `root.ranking_set_players_total_count`), then `string account_nickname`.

The nickname goes to lobby_client+0, **a 64-byte buffer (≤ 63)**. lobby_client+0x40 holds the game pointer, so overflow **crashes**. It is used by `setPlayerInfo`.

### 3.10 type 0x0B Elo rating (optional), `read_player_elo_rating` 0x19E88D

Layout: `u32`, `u16`, `u16` → `set_player_elo_rating` → `root.ranking_set_player`. [P: meanings]

### 3.11 type 0x0C Elo list (optional), `read_players_elo_list` 0x19EA34

Layout: `u8 n`, then n × {`string name (≤63)`, `u32`, `u16`, `u16`} → `root.ranking_set_data`. [P]

### 3.12 type 0x0D quests, `read_quest_list` 0x19E8D7 [V layout]

Layout: `u32 n`, then n × 20-byte `quest_instance` (memcpy):

| off | type | field |
|---|---|---|
| 0 | u32 | instance id (Flash `id`) |
| 4 | u32 | quest id in `quests.options`. **Must exist (R6)** |
| 8 | u32 | ? [U]. Not used by the lobby |
| 0xC | i32 | time_limit |
| 0x10 | u8 | state (1 = active; anything else → `proposal: true`) |
| 0x11 | 3 B | padding [U] |

The list is sorted by `(quest_id/100)%100`. Then `root.clear_quests()` and `root.add_quest(...)` for each. **Send n = 0.**

### 3.13 type 0x0E player match stats (optional)

Body: `player_result_full` (layout in §3.17) → `fill_player_statistic`.

### 3.14 type 0x0F account services, `read_account_services` 0x19E358 [V]

Layout: `u8 n`, then n × 10 bytes:

| off | type | field |
|---|---|---|
| 0 | u8 | type: 0 premium_access, 1 scavengers, 2 blackmarket, 3 renaissance, 4 edge loyalty |
| 1 | u8 | skipped |
| 2 | u32 | a |
| 6 | u32 | b |

**Type 0:** has_premium = true and hours = a. Calls `root.set_premium_state(state, text)`:
- state 0 = none, 1 = ≥ 24 h, 2 = < 24 h;
- text is `st_less_then_hour` / `%d <st_hour_short>` / `%d <st_day_short>`.

**Types 1..4:** add {faction_id = type, bonus = low byte of b} → `root.set_faction_bonus([...])`. The stack array holds **≤ 8** entries. Extra entries overwrite locals: the 13th overwrites the premium flag, the 15th the loop counter.

**Types > 4:** ignored.

`set_player_premium_access_status` and `set_factions_loyalties` are always called, even when n = 0.

### 3.15 type 0x10 squad member profile (optional), `read_squad_member_profile` 0x19DAD7

Layout: `u32 profile_id`, then `deserialize_static` (as in §3.2), then `u32 revision`. Then `on_squad_member_profile_arrived`.

### 3.16 type 0x11 shop items, `read_shop_items` 0x19E40E [V layout, P meaning]

Layout: `u32 n` (+0x5A64), then n × `shop_items_container` (0x108 bytes in memory, heap):

| type | field | mem |
|---|---|---|
| u32 | id (sent back in `0x24 02`) | +0 |
| u8 | shelf (only 0 = `premium_shelf` is displayed) | +4 |
| u32 | price | +8 |
| string | label key (**≤ 31**) | +0xC |
| string | value2 key (**≤ 31**) | +0x2C |
| u8 | k (**≤ 5**) | +0x4C |
| u8 | money_type (0 generic, 1 premium) | +0x104 |
| k × shop_item | 6 × u32 (→ +0, +4, +0xC, +0x10, +0x14, +0x18), then u8 (+0x1C), u8 (+0x20) | +0x50 + j·0x24 |

Then `on_shop_items_arrived` → `root.set_premium_store([{id, value, money_type, label, value2}])`. The shop_item fields are never read by the lobby [U meaning].

**Send n = 0.** It also clears the 0x26 need_shop_items flag on the next connect.

### 3.17 type 0x13 match stats (optional), `read_match_stats` 0x19E6B4 [V layout, U meaning]

Body: `match_stats` (0x19C78F), then `player_result_full` (0x19C93C).

**`match_stats`:**
- `u32` (+0x740)
- `u8` (+0x752), `u8` (+0x753), `u8` (+0x754)
- `string(≤63)` (+0)
- `u16` (+0x744)
- `string(≤63)` (+0x40)
- `u8` (+0x750)
- `u16` (+0x746)
- `string(≤63)` (+0x80)
- `u16` (+0x74A), `u16` (+0x748)
- `string(≤63)` (+0xC0)
- `u16` (+0x74C), `u16` (+0x74E)
- `u8 n` (+0x751, **≤ 20**), then n × `player_result_brief`

**`player_result_brief`:**
- `string(≤63)`
- `u32` (+0x40)
- `u16` (+0x44), `u16` (+0x48), `u16` (+0x4A)
- `u8` (+0x4C)
- `u16` (+0x46)

**`player_result_full`:**
- `u32` (+0)
- 9 × `u16` (+0x4A..+0x5A)
- `u8` (+0x65)
- `u16` (+0x5C)
- 4 × `u16` (+0x40..+0x46)
- `u8` (+0x66)
- `u16` (+0x48), `u16` (+0x5E)
- `u8` (+0x64)
- `u16` (+0x60), `u16` (+0x62)
- `u8 n1` (**≤ 4**), then n1 × `u32` (drops)
- `u8 n2` (**≤ 4**), then n2 × {`u32`, `u32`}
- `u32` (+0x3C)

The u16 at +0x48 of player_result_full is copied to P+0x9E0 of the profile whose P+**0** equals result+0. P+0 is never set from the wire [U].

Then `show_match_statistic` and `fill_match_statistic` / `fill_player_statistic`.

### 3.18 type 0x12 unlocked factions mask (inline) [V]

Layout: `u8 mask` (+0x5A6C). Bit (f−1) set means faction f (1..4) is unlocked.

There is no UI call on arrival; the mask is used by later 0x09 and important messages. **Send 0x0F.**

---

## 4. Lobby client → server

Senders live in `lobby_client` (0x19D46E..0x19EE6D). Most action senders do nothing while lobby_client+0x1C8 (match_id) ≠ −1, i.e. while in a match. That guard covers 0x23, 0x24, 0x25, 0x27 and the squad actions; the squad actions use the order (+0x1CC) instead.

| id | payload after the id byte | sender / guard |
|---|---|---|
| 0x20 | `u32 profile_id` (active profile) | `play_button_clicked` 0x17158B. Only when the profile count > 0 [V] |
| 0x21 | `u8 type`, then args: see the list below | 0x19D532.. [V] |
| 0x23 | `u8 0` (item_moved_to_slot), `u32 profile_id`, `u8 n`, then n × 16 B `relocate_item_descr` {`u32 item_id`, `u16 dict_id`, `u8 source_slot`, `u8 target_slot`, `u32 count`, `u32 count_in_inventory`} | `move_item` 0x19DE6E. Slot 100 = inventory. Not in a match [V layout; P semantics of count/count_in_inventory] |
| 0x24 00 | `u16 dict_id`, `u32 count`, `u8 faction_id`, `u8 00` (always false), `u8 target_slot` (100 from the shop button), `u32 active profile_id` | `buy_item` 0x19DF6A [V] |
| 0x24 01 | `u32 item_id`, `u32 amount` | `sell_item` 0x19EB19 [V] |
| 0x24 02 | `u32 shop_container_id` | `buy_shop_item` 0x19E057 (Flash `accept_premium`) [V] |
| 0x25 00 | `u32 active profile_id` (not sent when it is 0), `u8 n`, n × {`u8 skill_id`, `u8 points`}, `u8 m`, m × `u8 perk` | `set_player_skills` 0x19E0CE [V] |
| 0x25 01 | `u32 active profile_id` | `reroll_player_skills` 0x19E272 [V] |
| 0x26 | see §1.2 | on connect |
| 0x27 | `u32 order_id` (+0x1CC) | `discard_playing_order` 0x19E562 (Flash `leave_queue`) [V] |
| 0x28 | — | never sent |
| 0x29 03 | `u32 profile_id`, `u8 on` (raw Flash bool, 0/1) | `set_autobuy_for_profile` 0x19EBA2 [V] |
| 0x29 04 | `u32 profile_id` | `set_active_profile` 0x19D46E. Sent on **every** `on_profile_changed`: an active-profile 0x36 02, or the user picking a profile [V] |
| 0x2A 01 / 02 / 03 / 04 | destroy (—) / add (`string name`) / drop (`u32 member_id`) / leave (—) | only when order == −1. Add also needs the lobby connected. Add is also sent automatically on an important message of type 3 [V] |
| 0x2B 00 | `u32 caller_quest_id`, `u32 selected_quest_id` | `switch_quest` 0x19EC23, from Flash `accept_change_quest(selectedId, callerId)`. The wire order is callerId first [V] |

The 0x21 query arguments, as the client sends them (no padding):

| query | args | sent when |
|---|---|---|
| `0x21 02` | `u32 profile_id` | after 0x36 01 and after a sell |
| `0x21 05` | — | after 0x34 0x24 |
| `0x21 09` | — | important messages |
| `0x21 0C` | `u32 a`, `u8 b` | ranking page range (Flash `request_players_range`) [P] |
| `0x21 0D` | — | after 0x34 0x2B and on quest messages |
| `0x21 0E` | `u32 a`, `u32 b` | victory screen (Flash passes player id and match id) [P order] |
| `0x21 0F` | — | after 0x34 0x24/02, after 0x34 0x2B, on quest messages |
| `0x21 10` | `u32 profile_id` | squad member profile |
| `0x21 12` | — | reputation level unlock |
| `0x21 13` | `u32 match_id` | `#mstats [N]` |

The client **never** sends 0x21 with types 0, 1, 3, 4, 6, 7, 8, 0x0A, 0x0B or 0x11. All of those must be pushed by the server.

Server replies the client expects [U where there is no client handler]:

| client message | server reply |
|---|---|
| 0x23 | 0x36 03, then 0x36 02 (bump the revision), or 0x35 23 |
| 0x24 00 | 0x34 24 00 … (if bought into a slot, also 0x36 02) |
| 0x24 01 | 0x34 24 01 … |
| 0x24 02 | 0x34 24 02 |
| 0x25 | 0x36 06 for the profile |
| 0x29 03 | 0x34 29 03 pid on |
| 0x29 04 | nothing (R8) |
| 0x2B | 0x34 2B |
| 0x20 | 0x36 00 status 1 …, later 0x33 |
| 0x27 | 0x36 00 status 0 |

---

## 5. Messaging (chat)

Dispatcher: `messaging_client::on_packet_received` 0x19AD5A, after sign-in.

| id | handling |
|---|---|
| 0xC9 | text message |
| 0xCB | friendship status |
| 0xCC | important message |
| anything else, including 0xC8 and 0xCA | logged as unknown and ignored |

### 5.1 0xC9 message (server → client), `process_incoming_text_message` 0x19B858 [V]

| type | field | note |
|---|---|---|
| u8 | sender_type (`client_type`) | 5 = account (player); 8 = stats_processor |
| u32 | sender_id | If sender_type == 5 and sender_id is in the ignore list, **nothing more is read** (the message is dropped) |
| string | sender_name (**≤ 63**) | |
| u8 | channel (`message_channel_enum` 0..8) | |
| string | body (≤ 255) | |

Handling:
- **Type 8:** if the body contains `#mstats` (0x7DB8C4), the client parses `[N]` (`on_stats_message_arrived` 0x1765A5). When connected it sends `0x21 13 N`; otherwise N is stored for the next 0x26. Any other type-8 body is dropped.
- **Other types:** the body is censored (option) and `add_message(channel, body, name)` / `add_to_recent_list(name)` are called.

### 5.2 0xCB friendship_status (server → client) [V]

Layout: `u8 action`, then a body that depends on action.

| action | body | client |
|---|---|---|
| 7 query_friend_list | `u16 n`, n × {`u32 id`, `string name (≤63)`, `u8 status`} | Replaces the friend list. Status is `lobby_state 0 / ingame 1 / offline 2 / away 3` [P] |
| 9 update_friends_status | `u16 n`, n × {`u32 id`, `u8 status`} | **n must equal the current friend count**, otherwise the client re-queries (`0xC4 07`) without reading. An unknown id also triggers a re-query |
| 8 query_ignore_list | `u16 n`, n × {`u32 id`, `string name (≤63)`} | replaces the ignore list |
| 6 find_players | `u16 n`, n × {`u32 id`, `string name (≤63)`} | search results |
| 0, 1, 2 | `u8 result` | `'4'` (0x34) means OK → re-query the friend list; anything else is logged |
| 3, 4, 5 and anything else | `u8 result` | `'4'` → re-query the ignore list |

`lobby_menu::on_friendship_status_recivied(action)` then runs for every action.

### 5.3 0xCC important_message (server → client), 0x19B964 [V]

| type | field |
|---|---|
| u8 | sender_type |
| u32 | sender_id (unused) |
| string | sender_name (≤63) |
| u8 | type (`important_message_types`) |
| string | text (≤255) |
| u32 | message_id |

A message whose message_id is already pending is ignored. Otherwise the id is stored and `important_message_arrived(id, type, text, name)` runs.

| type | meaning | client action |
|---|---|---|
| 0 | friendship request | Flash `add_important_message` (`st_add_to_friends_request`); waits for the user |
| 1 | quest | prefixes `#+q` / `#!q` / `#q!`; system message plus queries 0x0F / 0x12 / 0x09 / 0x0D; auto `0xC6 id 00` |
| 2 | squad invite | Flash `add_important_message` (`st_add_to_squad_request`); waits for the user |
| 3 | squad_invite_accepted | auto `0xC6 id 00`, then `0x2A 02 <sender_name>` |
| 4 | reputation level unlocked | `#ru…`; queries 0x12 (if newly unlocked) and 0x09; auto `0xC6 id 00` |
| 5 | drop | log; auto `0xC6 id 00` |

### 5.4 Client → server [V]

| id | payload |
|---|---|
| 0xC1 | `u32 channel_id`, `string receiver`, `u8 channel`, `string body (≤255)` |
| 0xC2 | `u32 0`, `string target_name`, `u8 type`, `string text` |
| 0xC3 | §1.5 |
| 0xC4 | `u8 action`, then by action:<br>• 2 remove_friend, 3 add_ignorable, 5 remove_ignorable: `u32 id`<br>• 4 add_ignorable_by_name, 6 find_players: `string`<br>• 7, 8, 9: nothing |
| 0xC5 | 9 × u32 = [`0`, `language (1 ru / 2 en)`, `0xFFFFFFFF`, `0`, `1`, `match_id or 0`, `team1 slot`, `team2 slot`, `squad_id`] |
| 0xC6 | `u32 message_id`, `u8 answer` (0 accept, 1 decline) |

**0xC1 channel_id by channel:**
- 1 general → the language id (0x19B1F1, messaging+0x1A0)
- 4 private → 0
- 5 match → match_id
- 6/7 team → match_id (the channel byte becomes 6 + (team ≠ 0))
- 8 squad → squad_id
- 3 clan, an unset match, or squad 0 → **not sent**
- otherwise → 0

**Text encoding:** the line comes from the Flash input as `GFx::Value::GetString` (0x1B5729, Scaleform UTF-8), is trimmed (0x163FE1) and copied as bytes (`strcpy_s` 256, 0x45DAE) into 0xC1. An incoming 0xC9's name and body go back to Flash with `SetString` (0x1B5608) unchanged (0x19B858 → 0x1633B7); only the channel byte picks where it shows, and the channel_id is not in 0xC9. So chat is UTF-8 in every localization; the C locale the client sets (`.1251` for `original`/`russian`, `.1252` for `english`, `.ACP` otherwise, 0xB0E19) does not touch it.

**0xC2** is sent by `add_friend` (type 0), `invite_to_squad` (type 2) and accepting a squad invite (type 3, text `membership accepted`).

**0xC4** actions 0 and 1 (add_friend) are never sent as 0xC4; adding a friend goes through 0xC2 type 0.

**0xC5 team slots:** the match_id goes into slot 6 when team == 0 and slot 7 when team == 1. Both stay 0 when team == 3 or there is no match.

---

## 6. Sign-in and loading: what the client needs

The lobby scene is shown as soon as login succeeds (`on_connected_to_login` 0x16D7E3 → `game::switch_to_lobby`), before any lobby traffic. Its Flash movie and item dictionary are loaded asynchronously when the game starts (`lobby_menu` ctor → `query_scene_resources`; `on_render_scenes_ready` sets the movie pointer +0x630 and fills the dictionary, restrictions, skills tree and reputation levels).

The Flash side has **no** "waiting for data" state (`LobbyMenu.as`). Each push updates its own widget.

**Gating visible from the exe and the SWF [V]:**
1. **Play enabled** (`lock_play_button(true)`, which also calls `topMenu.lockInventory(false)`). All of these must hold:
   - status()==0 (0x30 byte 0, or a 0x36 00 status 0);
   - minutes_to_maintenance > 30 (0x38);
   - no squad, or the squad is ready.
2. **Pressing Play** sends 0x20 only when at least one profile is listed.
3. **Paper doll:** the inventory (0x36 03) must contain every slot item before the active profile's 0x36 02.
4. **Skills:** 0x36 06 must follow 0x36 01.
5. **Nickname in the money bar:** 0x36 0A before 0x36 05.
6. **Faction lock texts:** 0x36 12 before 0x36 09.

**Recommended push order after 0x30 [V order constraints, U completeness]:**

| step | message | why |
|---|---|---|
| 1 | `30 00` | |
| 2 | `36 0A` | nickname, ranking total |
| 3 | `36 05` | money |
| 4 | `36 12` | faction mask |
| 5 | `36 09` | reputations (4 entries) |
| 6 | `36 08` | reroll and profile costs |
| 7 | `36 0F` | n = 0, or premium/loyalty |
| 8 | `36 04` | prices, for the shop |
| 9 | `36 11` | n = 0 (whenever the 0x26 flag is FF, or always) |
| 10 | `36 03` | inventory |
| 11 | `36 01` | profiles; the client then sends `21 02 <id>` per profile, answered with `36 02 <id>`. Profile index 0 → client sends `29 04 <id>`: do not reply |
| 12 | `36 06` | active profile (index 0) |
| 13 | `36 0D` | n = 0 |
| 14 | `36 00 00` | surf |
| 15 | `38` | players, games, minutes ≥ 60 |

**Messaging (separate socket):** `CA name` after C3. Then answer `C4 07` with `CB 07 0000` and `C4 08` with `CB 08 0000`.

Example bytes, frame included, for the minimal sequence with loadout A: `scope021d/spec_work/examples_out.txt`, generated by `py/examples.py`. Short excerpts:

| message | bytes |
|---|---|
| 0x30 | `02 30 00` |
| 0x38 (1440 min) | `0D 38 01 00 00 00 00 00 00 00 A0 05 00 00` |
| 0x36 00 | `03 36 00 00` |
| 0x36 01 | `21 36 01 01 01 00 00 00 07 'Default' 00 01 00000000 00000000 E8030000 01000000` |

**Timing risk [U]:** a 0x36 that arrives before `on_render_scenes_ready` meets a NULL movie pointer (lobby_menu+0x630, 0 in the ctor) and would crash. Normally login takes long enough. A server that pushes immediately after a very fast auto-login could hit this.

---

## 7. Item identity and slots (0.21d data)

### 7.1 Dictionary ids [V]

`dict_id` is the explicit `dict_id` field of each `items_dict` entry, not an index. The cooker copies `items_dict[i].dict_id` to `dictionary_item+0` (0x1C6954..0x1C69AD), and `item_by_id` matches on it. The other fields come from the same entry:
- `item_category` → +0x228
- `is_premium` → +0x229
- `is_stack` → +0x22A
- `item_level` → +0x22C
- `name`, and `cfg_name` (the per-item config, which provides `default_modifications` and the faction)

There are 83 items. Full list: `scope021d/txt/gameplay__db_static_dictionaries.txt`; a sorted table can be printed with `spec_work/py/cfg.py`.

### 7.2 Slot rules [V]

`items_dictionary::can_move_item_to_slot(category, slot)` 0x1C6F53:
- slot 100 (inventory) → always allowed;
- otherwise allowed only when a `slots_restrictions` entry has `profile_slot_id == slot` and `category_id == item_category`.

The client checks this before it sends 0x23 (scaleform_call 0x170C4B), and Flash gets the same table (`set_profile_slots_restrictions`). The server should enforce it too.

Weapon/ammo pairs come from `items_compatibility` (first_item = ammo, second_item = weapon). The client checks them when it auto-fills ammo (`check_items_compatibility` 0x1C6E6C).

| category | slots |
|---|---|
| 1 helmet | 0 |
| 2 mask | 1 |
| 3 torso | 2 |
| 4 back | 3 |
| 5 pants | 4 |
| 6 gloves | 5 |
| 7 boots | 6 |
| 8, 13, 14, 15, 16, 17 weapons | 7, 10 |
| 9, 18, 19, 20, 21 ammo | 8, 9, 11, 12, 19, 20, 21, 22 |
| 10 drugs, 11 devices/traps, 24 grenades | 13..18 |
| 12 artefacts, 22 scopes, 23 | no slot |

The ammo slot list [8, 9, 11, 12, 19, 20, 21, 22] is the table at 0x7DB854 [V].

Pairing [U, inferred from enum order and client auto-fill]:
- ammo_slot_1/2 (8, 9) belong to weapon1;
- ammo_slot_3/4 (11, 12) belong to weapon2;
- 19..22 are the extra ammo-bag slots.

### 7.3 Example loadouts

Instance ids are free choices (here 1000 + dict_id). Every one of them must also appear in 0x36 03. Each loadout satisfies `slots_restrictions` and `items_compatibility`.

**Loadout A.** `test_player_profiles.options` profile 0. All 10 test profiles are identical. The index of each entry in the options list is the slot index [U: the client never reads this file; the 13 entries line up with slots 0..12]. A full byte dump is in `examples_out.txt`.

| slot | name | dict |
|---|---|---|
| 0 helmet | ren_helmet_1 | 60 |
| 1 mask | scav_resp_1 | 43 |
| 2 torso | ren_torso_1 | 58 |
| 3 back | scav_back_1 | 49 |
| 4 pants | ren_legs_1 | 59 |
| 5 gloves | scav_gloves_1 | 41 |
| 6 boots | ren_boots_1 | 62 |
| 7 weapon1 | aps (cat 17) | 126 |
| 8 ammo1 | ammo_9x18_makarov | 70 |
| 9 ammo2 | ammo_9x18_pbm | 103 |
| 10 weapon2 | spas_12 (cat 14) | 124 |
| 11 ammo3 | ammo_12mm_buck | 22 |
| 12 ammo4 | ammo_12mm_slug | 72 |

Slot mask 0x00001FFF.

**Loadout B (black market).** Slot mask 0x0000EDFF.

| slot | dict |
|---|---|
| 0 helmet | bm_helmet_1 27 |
| 1 mask | scav_resp_1 43 |
| 2 torso | bm_torso_1 29 |
| 3 back | scav_back_1 49 |
| 4 pants | bm_legs_1 28 |
| 5 gloves | bm_gloves_1 25 |
| 6 boots | bm_boots_1 24 |
| 7 weapon1 | tt_33 18 |
| 8 ammo1 | ammo_7.62x25 50 |
| 10 weapon2 | ak_74u 13 |
| 11 ammo3 | ammo_5.45x39_7n6m 7 |
| 13 quick1 | bandages 66 |
| 14 quick2 | rgd-5 5 |
| 15 quick3 | medkit 67 |

**Loadout C (Edge).** Slot mask 0x00006DFF.

| slot | dict |
|---|---|
| 0 helmet | edge_hunter_helmet_1 86 |
| 1 mask | edge_resp_1 102 |
| 2 torso | edge_hunter_torso_1 83 |
| 3 back | edge_hunter_back_1 87 |
| 4 pants | edge_hunter_legs_1 84 |
| 5 gloves | scav_gloves_2 42 |
| 6 boots | edge_hunter_boots_1 85 |
| 7 weapon1 | akmn 80 |
| 8 ammo1 | ammo_7.62x39 81 |
| 10 weapon2 | mosin_nagant 107 |
| 11 ammo3 | ammo_7.62x54r_old 104 |
| 13 quick1 | painkiller 65 |
| 14 quick2 | base_trap 68 |

Encoding notes for these loadouts:
- In the profile, slots 0..7 and 10 carry `u16 dict, u32 id, u16 cond, u8 0`.
- Slots 8, 9, 11..22 carry `u16 dict, u32 id, u16 count, u32 amount, u8 0`.
- In the inventory, every entry uses the second form.

---

## 8. Verification status and open points

**Verified [V]:**
- every server→client layout in §1–§5;
- every client→server layout in §4–§5;
- the slot table, both enums, all buffer sizes and caps;
- the gating conditions;
- the dict_id identity;
- the restriction check.

**Partial or unverified:**
1. **Meanings:**
   - the 0x3A member u32s and the byte at +0x5A2D;
   - the 0x0B/0x0C/0x0E/0x13 numeric fields;
   - shop_item fields;
   - quest_instance +8 and its padding;
   - player_reputation +4 (unused by the exe);
   - P+0x9C8 (new-player protection count);
   - P+0x5B0 (is_local?);
   - amount_in_inventory;
   - relocate count / count_in_inventory;
   - the condition range;
   - the 0x21 0C/0E argument meanings.
2. What the server must answer to 0x23/0x24/0x25/0x2A/0x27/0x20: the client has no permitted handler for these, so the replies in §4 are inferred.
3. Behaviour of `binary_config_value::operator[]` on a missing skill id or level.
4. The slot ownership of ammo slots 19..22, and test_player_profiles index == slot.
5. A lobby push arriving before `on_render_scenes_ready` (NULL movie) is a possible crash (§6).
6. The Flash/AS3 behaviour was read from the ffdec export. Scaleform's handling of an uncaught AS3 error, observed in practice, is that the call aborts, not the process [U].
