# Survarium v0.100b — local-player weapon FSM: exact timing spec

Source root: `E:\sur-res-pro\tools\vostok\sources\vostok\` (all paths relative to it).
Short names: `wc.cpp` = `game_core/sources/weapon_core.cpp`; `pt.cpp` = `game/sources/player_tick.cpp`;
`tree.cpp` = `animation/sources/mixing_n_ary_tree.cpp`; `aei.cpp` = `animation/sources/mixing_n_ary_tree_animation_event_iterator.cpp`;
`ttc.cpp` = `animation/sources/mixing_n_ary_tree_transition_tree_constructor.cpp`.
Clip data: the per-weapon table (`bbr1gztb4.txt`) for item clips, plus hands/sprint/jump/shotgun-substate clips
read from `E:\sur-res-pro\Survarium\resources.db` with the same B-spline parser
(length = max_frame − min_frame, as `animation/cubic_spline_skeleton_animation.h:51`). 30 fps everywhere.

Map of the requested topics → sections: (1) §1, (2) §0+§2, (3) §3, (4) §5, (5) §6, (6) §7, (7) §8, (8) §0+§9. Numbers: §10. Unresolved: §11.

---------------------------------------------------------------------------------------------------

## 0. Execution model — WHEN the weapon logic runs (most important section)

`weapon_core::tick()` is **not** called once per frame. Its only caller chain is
`player::select_animations(t)` → `base_player::tick_active_object()` → `current->tick()` (`game/sources/player.cpp:1062-1065`, `game_core/sources/base_player.cpp:36-48`, `game/sources/weapon.cpp:166-169`). A "SELECT(t)" is:

1. `tick_active_object()`: `current->tick()`; then if `current != target_active_object && current->is_ready_to_be_deactivated()`: `on_before_active_object_changed`, `current->deactivate()`, swap, `new->activate()`, `new->tick()` (`base_player.cpp:38-48`). (Then a broken-hands auto-switch check, `:50-60`.)
2. Build the expression from the (post-tick) weapon state (`player.cpp:1070`).
3. `m_target.animation_player.set_target_and_tick(expr, t)` and `m_current.animation_player.set_target_and_tick(expr, t)` (`player.cpp:1072-1074`): tick to t, set target, tick to t again (`animation/sources/animation_player.cpp:321-335`). Callbacks fired inside these ticks run, but their interrupt flags are discarded. Only `m_current` has real callbacks (`m_target` gets `empty_callback`, `player.cpp:1180-1213`).

Per local frame k with game time `t_k` (ms, `game.cpp:629`, passed unchanged to `player::tick`) — `pt.cpp:297-441`:

| step | what | source |
|---|---|---|
| a | `prev = t_{k-1}` (= `m_current_time_in_ms`), `m_current_time_in_ms = t_k` | `pt.cpp:305-307` |
| b | `previous_input = m_input; m_input = local_input()` (mask_k) | `pt.cpp:327-328` |
| c | `stamina.tick(t_k, stamina.can_be_spent() && active_object->is_sprinting())` — uses the selector state from the last SELECT | `pt.cpp:339-340` |
| d | weapon-slot keys: `process_quick_slots_for_current_player()` → maybe `inventory().action(slot)` → `set_new_active_item` sets `m_target_active_object` and forces selection | `pt.cpp:344`, `player.cpp:973-1001, 844-853` |
| e | loop: `time = t_{k-1}`; **if `time != t_{k-1}` OR `mask_k != previous mask` OR forced → SELECT(time)**; `m_current.tick_to_nearest_user_handled_callback(t_k)`; `time = last_tick_time`; tick `m_target` to `time`; `previous_input = m_input`; repeat while `time != t_k` | `pt.cpp:351-384` |
| f | if the last sub-step stopped on an interrupting callback (at exactly `t_k`) → SELECT(t_k) | `pt.cpp:386-387` |
| g | send `(mask_k, t_k)` to server; `render(t_k)` → `weapon_core::update_bones_matrices` (dispersion tick, `set_fire_bullet_transform`) | `pt.cpp:389-400`, `wc.cpp:879-941` |

`tick_to_nearest_user_handled_callback(t_k)` repeatedly ticks to `min(t_k, nearest event time)` and stops after the first event time at which any callback set `interrupt_animation_player_tick` (`animation_player.cpp:304-319`). Within one `tick()` all events with time ≤ target are processed in time order: states are advanced to the event time, then its callbacks run synchronously (`tree.cpp:1145-1187`).

Therefore the weapon FSM is evaluated ("weapon tick") exactly at:
* **W1** `t_{k-1}` — the *previous* frame's time — when mask_k differs from mask_{k-1}, or a selection was forced (weapon switch `player.cpp:849`, sprint stamina depletion `player_logic_sprint_state.cpp:103-106`, broken limb `weapon_user_animations_selector.cpp:320-325`, spawn/insert `player.cpp:236`, camera-mode switch `player.cpp:1137`, detach `:1157`, weapon deserialize `wc.cpp:1064`). So the input sent with time `t_k` takes effect at `t_{k-1}`.
* **W2** the exact ms `e` of every **interrupting** animation callback in `(t_{k-1}, t_k]` (SELECT(e) in the next loop pass, or step f if `e == t_k`). Interrupting callbacks for the local player: weapon `"shoot"` (`weapon_core_fire_state_base.cpp:56`, `weapon_core_aimed_fire_state_base.cpp:73`); end of the waited weapon clip in fire/aim_fire/show/hide/reload/chamber/chamber_aimed (`weapon_core_animation_end_aware_state.cpp:31-45` + each `on_animation_end_impl` sets it true); shotgun start/one_round/finish clip ends (`weapon_core_shotgun_reload_*_substate.cpp:73/61/69`); sprint lexeme end (`wc.cpp:563`); jump start: **any** interval end on the user object while in jump-start (`jump_logic_state_start.cpp:159-169`); jump landing interval end (`jump_logic_state_landing.cpp:143-150`). (`grep` of game/game_core finds no other player-side setter; `animations_selector.cpp:92` is NPC-only.)
* Nowhere else. In particular in idle/aimed idle with constant input there are no weapon ticks at all, and predicates that change silently (stamina regenerating, physics `can_jump`) are only re-evaluated at the next W1/W2.

Inside a weapon tick the input is always the current frame's mask_k; `m_old_actions_mask` = mask seen by the previous weapon tick (`wc.cpp:404`).

Animation event times (exact ms, independent of frames) — `animation/sources/mixing_n_ary_tree_time_in_ms_calculator.cpp:66-90,186-196`, `aei.cpp:63-96`:
* An event at clip time `x` s (knot `F`/30, clip end `L`/30) of a lexeme whose time-scale anchor is `(t0 ms, a0)` fires at `t0 + floor((x − a0)·1000/T)`. A lexeme created by SELECT(s) is anchored at `(s, 0)` unless it inherits a phase (below). Callback `params.callback_time_in_ms` is that exact ms (`tree.cpp:1019-1024`); `instant_fire` receives it.
* Cyclic lexemes: at each wrap the anchor is reset to `(wrap_ms, 0)` (`tree.cpp:611-636`) ⇒ cycle `n` starts at `s + n·C`, `C = floor(L·1000/(30T))`, and its knot fires at `s + n·C + floor(F·1000/(30T))`.
* The anchor is also reset when a weight transition (fade-in, 0.3 s) ends (`tree.cpp:690-708`) — mathematically neutral, can move a boundary value by 1 ms (§11).
* Float model: Master Gold is built `/arch:SSE2 /fp:fast` (`game_core/sources/game_core.vcproj`, `animation/sources/animation.vcproj` EnableEnhancedInstructionSet=2, FloatingPointModel=2) and every thread sets x87 to 24-bit precision (`core/sources/math_functions_win_xbox360.cpp:21`) ⇒ emulate every operation in IEEE float32. Values that are within 1e-4 of an integer are marked `*` in §10 (±1 ms risk).

Lexeme identity across SELECTs (`animation/sources/mixing_n_ary_tree_comparer.cpp:48-110`, `_transition_tree_constructor_impl.h:57-83`): weight-sync group, time-sync group, animated object, bones mask, **playback type**, weight-driving animation, unique id, `can_generate_events`, intervals (animation pointer/start/length), override flag. **Not** the identifier string, **not** the time scale. Matched lexemes continue untouched; if the whole tree is equal `set_target` is a no-op (`animation_player.cpp:207-209`).
A *new* lexeme starts at phase 0, **unless** it is the time driver of a sync group (group ≠ −1) and the old tree had a non-frozen driver of that group: then it inherits the old driver's interval id and `time × new_len/old_len` (same normalized phase) (`ttc.cpp:98-136, 251-252`). A frozen (once-clip that reached its end) old driver does not pass its phase.
Fading-out lexemes raise no user/end callbacks; the weapon's event lexeme has an instant weight interpolator so it is removed at once (`weapon_lexeme_pair.cpp:54-67`, `animation/sources/mixing_binary_tree_animation_node.cpp:26-33`, `tree.cpp:989-993`). Only the weapon "offset" lexeme raises events; its twin has `can_generate_events(false)` (`weapon_lexeme_pair.cpp:38-40`).

---------------------------------------------------------------------------------------------------

## 1. Generic `ai::fsm` (weapon `m_logic`, selector `m_logic`, `jump_logic`, shotgun reload sub-fsm)

| Rule | Source |
|---|---|
| States in `add_state` order; each state's transitions in `add_transition` order (append). | `ai/sources/fsm.cpp:146-158`, `intrusive_list_inline.h:50-70` |
| `set_initial_state(s)`: no-op if `s == current`; else `current->finalize()`, `current = s`, `s->initialize()` (NULL allowed). | `fsm.cpp:165-177` |
| `tick()`: if `current->is_ready_for_transition()`, scan its transitions in order; the **first** true predicate: `finalize()` old, switch, `initialize()` new, **break**. Then **always** `current->execute()` (the new state's, if switched). | `fsm.cpp:179-198` |
| ⇒ max ONE transition per tick; the new state's initialize + execute run in the same tick; its own transitions wait for the next tick (i.e. the next W1/W2). | same |

Initial states: weapon = **inactive** (`activate`: `set_initial_state(states.front())` + one `tick()`, `wc.cpp:786-787`, `add_state` order `wc.cpp:170-181`); selector = **stand** (`weapon_user_animations_selector.cpp:71,111`); shotgun sub-fsm = **start** (`weapon_core_shotgun_reload_state.cpp:39-43`); jump_logic = inactive → start (true) → landing (`jump_logic.cpp:95-111`).

---------------------------------------------------------------------------------------------------

## 2. `weapon_core::tick()` (`wc.cpp:368-405`)

1. `user_animations_selector().tick()` (`:373`) — §8.
2. If `current_state && is_idle() && !(mask & 0x20)` (`:375-383`): `mask&0x400` → `set_next_fire_queue_type()` (`:642-648`, no queue reset this tick); else `mask&0x800` → `set_next_ammo_type()` (`:650-668`); else `reset_fire_queue()`.
   `is_idle()` = `m_is_idle || (m_aimed && !m_is_firing)` (`weapon_core.h:157`). `m_is_idle` only in idle states (`weapon_core_idle_state_base.cpp:14-21`, `wc.cpp:1303-1313`); `m_aimed` true in aimed idle, aimed fire (but firing) and chamber_a_round_aimed.
3. Target, first match (`:385-394`):
   1. `this != user.target_active_object() && selector.is_ready_to_be_deactivated()` (always true: `player_logic_base_state.cpp:16`, never cleared) → `set_target(inactive)`;
   2. `!could_be_used` (2 broken hands && double-handed, `wc.cpp:1078-1082`) → `set_target(idle)`;
   3. `mask&0x20` → `set_target(is_trying_to_aim() ? aim_fire : fire)`;
   4. `mask&0x40` → `set_target(reload)`;
   5. else `set_target(is_trying_to_aim() ? aim : idle)`.
4. `m_logic->tick()` (`:396`).
5. If `m_aiming_state_transition` (set by `instant_aim_start/end`, `:491-504`): selector `tick()` again, clear flag (`:398-402`).
6. `m_old_actions_mask = mask` (`:404`).

`set_target(t)` (`wc.cpp:574-621`; game override `game/sources/weapon.cpp:442-469` only toggles a muzzle light):
1. `t ∈ {fire, aim_fire}`: `mag + chambered == 0` → `t = reload`; else `!is_ready_to_shoot()` → fire→idle, aim_fire→aim. `is_ready_to_shoot` = `(has chamber state ? chambered : mag > 0) && bullets_in_queue != 0 && m_ready_for_fire` (`:1221-1224`).
2. Sprint latch: `if (!m_is_in_sprint_transition && selector state == sprint)` → `m_is_in_sprint_transition = true` + subscribe `on_sprint_animation_ended` to `channel_id_on_animation_lexeme_end` of sprint clip 0 (1st- and 3rd-view) on the user (`:589-596`). Cleared only by that callback (`:561-570`) or `activate` (`:759`).
3. `m_is_in_sprint_transition || selector state == jump` → fire→idle, aim_fire→aim (`:598-604`).
4. `t == reload && !ready_to_reload()` → `on_ammo_empty()` if no ammo item or amount 0 (UI only), `t = (old m_target ∈ {aim_fire, aim}) ? aim : idle` (`:606-618`).
   `ready_to_reload` = `mag+chambered != max_ammo && ammo && ammo.amount != 0 && !m_is_in_sprint_transition && selector state != jump` (`:1248-1256`); `max_ammo = capacity + (chamber state && !chamber_a_round_on_reload)` (`:1281-1285`) = capacity for all 11 configs.
5. `m_target = t`.

`is_trying_to_aim()` (`:1226-1235`) = `broken_hands != 2 && (mask&0x80) && !(input.is_sprinting() && ((~old_mask & mask) & 0x200)) && selector state != jump`.
`reset_fire_queue()` (`:623-640`): `L = fire_queue_types[type]` (config value cast `(s8)` → u8, so `-1` = 0xFF, `weapon_core_cook.cpp:69-76`); `L == 0xFF` → `bullets_in_queue = mag + chambered`, else `min(L, mag + chambered)`. Other resets: `instant_idle_start` if fire up (`:1303-1308`), `instant_aim_start` if `!m_is_firing && !(mask&0x20)` (`:491-498`), `instant_reload` (`:464`), `activate` (`:813`).
`set_next_ammo_type()` unloads magazine+chamber to the old stack (`:443-455`), switches ammo, writes `m_target = reload` (`:667`) — but step 3 overwrites `m_target` in the same tick, and idle has no auto-reload transition ⇒ the weapon stays empty in idle until reload or fire (empty→reload) is pressed. (Literal source behaviour.)

Other periodic work, NOT inside weapon tick: `update_dispersion(is_moving, t_k)` + `set_fire_bullet_transform(head)` once per frame from `render` (`wc.cpp:893, 937`; is_moving is true if any of 0x1/0x2/0x4/0x8 is set — it compares masked bits, `wc.cpp:893`); recoil/breath `computed_*_recoil_time` delegates run as lexeme time-calculators during animation ticks (`wc.cpp:1093-1145`, only stand/crouch states have recoil lexemes). `instant_fire` uses the bullet transform of the last render (previous frame).

---------------------------------------------------------------------------------------------------

## 3. State classes, clips, playback, time scales

### 3.1 Registry (`resources_classes.h:161-189`; cook `weapon_core_cook.cpp:127-172`)

| id | class | weapons |
|---|---|---|
| 300 | inactive (`weapon_core_inactive_state.h`, implicit) | all |
| 301 / 302 / 303 | show / hide / idle | ak_74u, magnum, rem_700, rem_870, toz_122, toz_34, uzi, vityaz |
| 304 | reload (magazine) | ak_74u, magnum, toz_122, uzi, vityaz |
| 305 | chamber_a_round | rem_700, rem_870, toz_122 |
| 306 | chamber_a_round_aimed | rem_870 |
| 307 / 308 / 309 | fire / aimed idle / aimed fire | ak_74u, magnum, rem_700, rem_870, toz_122, toz_34, uzi, vityaz |
| 310 (+311 start, 312 one_round, 313 finish) | shotgun one-round reload | rem_700, rem_870 |
| 314-320 | double-barrel show/hide/idle/reload/fire/aimed/aimed_fire | toz_66 (all); toz_34 uses only 317 |
| 321-327 | pistol show/hide/idle/reload/fire/aimed/aimed_fire | fort_17, tt_33 |

Client cooks: states with `sounds` are `weapon_sound_events_handler_state<T>` (only adds a `"sound_events"` subscription, `game/sources/weapon_sound_events_handler_state_inline.h:71-86`); time scales from the same formulas applied to `animations[0]` (`game/sources/weapon_sound_events_handler_state_cook.cpp:20-70`, `_cook_inline.h:180`, show/hide `_cook_specializations.h:19-118`). Array order: config `animations[]` (item clips) then `user_animations[]` (hands) (`_cook_inline.h:58-69`; same in `weapon_core_shotgun_reload_state_cook.cpp:64-95`). (The game_core template `weapon_core_state_cook_template_inline.h:142` reading `cfg["animations"][0..N)` is a decompilation artefact.)

### 3.2 Clip index (config `animations[i]`)
* generic (301-310 family): `i = 2·view + crouch`, `view = is_third_view`, `crouch = (selector state == crouch)`; sprint/jump use stand clips (`weapon_core_fire_state.cpp:82-102` and siblings).
* pistol 321/322/323/326/324: `i = 4·view + 2·crouch + (mag == 0)`, re-evaluated every SELECT (`pistol_weapon_core_show_state.cpp:85`, `_hide:87`, `_idle:79`, `_aimed_idle:72`, `_reload:87`). Pistol fire/aimed fire 325/327: last-shot flag fixed at initialize: `bullets_in_queue ? (mag == 1) : (mag == 0)` (`pistol_weapon_core_fire_state.cpp:67-76`, `pistol_weapon_core_aimed_fire_state.cpp:67-76`).
* DB 314/315/316/319: `i = 6·view + 3·crouch + mag` (every SELECT). DB fire/aimed fire 318/320: `i = 4·view + 2·crouch + (mag != 2)` fixed at initialize (`double_barreled_weapon_core_fire_state.cpp:68-75`). DB reload 317: `i = 4·view + 2·crouch + (mag != 1)` every SELECT (0 = first barrel, 1 = both) (`double_barreled_weapon_core_reload_state.cpp:84`).
* Hands clips: `2·view + crouch` (pistol/DB reload `4·view + 2·crouch + e`). Logic never waits on hands clips: `set_animation_to_wait` always gets the item clip and callbacks are filtered on `animated_object == &weapon` (`weapon_core_animation_end_aware_state.cpp:31-45`).
* `view` for the local first-person player is 0 (`player.cpp:1066-1070`: third view only in third-person camera mode).

### 3.3 Per-state behaviour
`grp` = time-sync group (`weapon_lexeme_pair.cpp:10-68`); `ready` = `is_ready_for_transition()`; `E` = `has_animation_ended()` = `m_animation_has_been_ended` (`weapon_core_base_state.h:33`).

| state | playback | time scale T (from anim[0]) | grp | ready | effects |
|---|---|---|---|---|---|
| inactive 300 | — | — | — | true | `m_is_ready_to_be_deactivated = true` (only state with it) (`weapon_core_inactive_state.h:12-16`) |
| show 301/314/321 | once+freeze | `len(item[0]) / len(hands[0])` = 36/18 = **2.0** (`weapon_core_show_state.cpp:125-133`; pistol uses anim[8] `pistol_weapon_core_show_state.cpp:142`; DB anim[12] `double_barreled_weapon_core_show_state.cpp:147`) | 6 | **E** (`weapon_core_show_state_base.h:23`) | init: E=false, end cb, `instant_toggle_start`; fin: `instant_toggle_end`; end: `m_is_shown = true` (`weapon_core_show_state_base.cpp:16-33`) |
| hide 302/315/322 | once+freeze | 2.0 | 7 | **E** (`weapon_core_hide_state_base.h:29`) | same, end: `m_is_shown = false` (`weapon_core_hide_state_base.cpp:16-33`) |
| idle 303/316/323 | cyclic | 1.0 | −1 | true | init `instant_idle_start` (m_is_idle, queue reset if fire up); fin `instant_idle_end` |
| aimed 308/319/326 | cyclic | 1.0 | −1 | true | init `instant_aim_start`; fin `instant_aim_end` (`weapon_core_aimed_state_base.cpp:14-23`) |
| reload 304/317/324 | once+freeze | `(len(item[0])/30)/reload_time` (`weapon_animations_timescale_inline.h:10-14`) | 2 | true | init: E=false; if `chamber_a_round_on_reload && chambered` → `unload_chambered_round()` (`weapon_core_reload_state_base.cpp:18-27`); end: `instant_reload()` (`:29-33`) |
| chamber 305 | once+freeze | **1.0** (`weapon_core_chamber_a_round_state.cpp:137`) | 8 | true | end: `instant_chamber_a_round()` (`weapon_core_chamber_a_round_state_base.cpp:23-27`) |
| chamber_aimed 306 | once+freeze | **1.0** (`weapon_core_chamber_a_round_aimed_state.cpp:133`) | 8 | true | init `instant_aim_start`; fin `instant_aim_end`; end `instant_chamber_a_round()` (`weapon_core_chamber_a_round_aimed_state_base.cpp:18-35`) |
| fire 307/318/325 | 307: chosen at every SELECT: `bullets_in_queue <= 1 ? once+freeze : cyclic` (`weapon_core_fire_state.cpp:99`); 318: always once (`double_barreled_weapon_core_fire_state.cpp:92-110`); 325: `m_playback_type` fixed at init (`weapon_core_fire_state_base.cpp:26`, used `pistol_weapon_core_fire_state.cpp:98-114`) | `rps / (n_shoot / (len(item[0])/30))` (`weapon_animations_timescale_inline.h:21-39`) ⇒ one clip pass = `n_shoot/rps` | 1 | true | init: E=false, end cb + `"shoot"` cb, `*m_is_firing=true` (`weapon_core_fire_state_base.cpp:16-29`); **execute: E=false every tick** (`:35-39`); fin: unsubscribe, `*m_is_firing=false` (`:41-48`); end: interrupt only (`:76-79`) |
| aimed_fire 309/320/327 | as fire | same formula, own anim[0] | 1 | true | init: `*m_is_firing=true` (`weapon_core_aimed_fire_state_base.cpp:23`) then end cb, shoot cb, `instant_aim_start` (no queue reset since firing), `"aiming"` cb, playback type (`:21-42`); execute E=false (`:48-52`); fin `instant_aim_end`, `*m_is_firing=false` (`:54-65`) |
| shotgun 310 | delegates to substate | — | — | true | init: sub-fsm → start, E=false (`weapon_core_shotgun_reload_state.cpp:39-43`); execute: sub-fsm `tick()` only (`:150-153`); fin: playback reset, `set_initial_state(0)` (current substate's finalize) (`:45-49`) |
| ↳ start 311 | once+freeze | 1.0 (`weapon_core_shotgun_reload_state_cook.cpp:155`) | 3 | `m_animation_ended` | init: end cb; `chamber_a_round_on_reload && chambered` → `unload_chambered_round()` (`weapon_core_shotgun_reload_start_substate.cpp:34-48`); end: `m_animation_ended = true` (`:64-77`) |
| ↳ one_round 312 | **cyclic** | `(len(item[0])/30)·capacity/reload_time` (`weapon_animations_timescale_inline.h:41-45`, cook `:152`) | 4 | true | **every** end of its clip (each loop): `reload_one_round()` (`weapon_core_shotgun_reload_one_round_substate.cpp:52-64`) |
| ↳ finish 313 | once+freeze | 1.0 (cook `:161`) | 5 | true (no exits) | end: owner `E = true` (`weapon_core_shotgun_reload_finish_substate.cpp:60-72`); **finalize: `chamber_a_round_on_reload && mag != 0` → `instant_chamber_a_round()`** (`:45-58`) |

Base `execute()` only resets the saved playback state (`weapon_core_base_state.h:38`): for show/hide/reload/chamber/shotgun E stays true once set until the state is left; for fire/aim_fire E is true only in the weapon tick that is triggered by the end callback itself (W2), and cleared by that tick's `execute`.
End callbacks: once+freeze raises `channel_id_on_animation_end` once at clip end, then freezes and raises nothing more (no user events either) (`aei.cpp:165-172`, `tree.cpp:582-591`); cyclic raises it at **every** wrap (`aei.cpp:67-80, 266-270`, `tree.cpp:611-637`).

Data effects: `instant_reload` (`wc.cpp:457-465`): `m_aimed=false`; `load_magazine()` = move `min(stack, capacity − mag)` into the magazine, then `chamber_a_round()` (`--mag; chambered=true`) if `chamber_a_round_on_reload` (`:420-439`); recoil reset; dispersion `+= reload_dispersion_amount` (`weapon_dispersion_calculator.cpp:62-65`); `reset_fire_queue()`. `instant_chamber_a_round` (`:467-476`): `m_aimed=false; --mag; chambered=true;` recoil reset. `reload_one_round` (`:480-489`): if `mag != capacity && stack != 0`: `++mag; --stack` (nothing else). `unload_chambered_round` (`:1270-1279`): `++mag; chambered=false`. `instant_fire(t)` (`:526-553`): `--bullets_in_queue`; `buck_shot` pellets; chamber-state weapon → `chambered=false`, else `--mag`; recoil + dispersion `+= one_shoot_dispersion_amount`.

---------------------------------------------------------------------------------------------------

## 4. Weapon transitions (`wc.cpp:183-270`, in this order)

`T(x)` = `m_target == x`; `E` as above; `MC` = `!chambered && mag != 0 && selector != jump` (`:1199-1203`); `AIM` = `is_trying_to_aim()`; `CMR` = `ready_to_reload() && mag == 0 && !chambered` (`:1259-1262`); `II` = `selector.sprint_predicate() || selector == jump` (`:1070-1073`). chamber rows only for weapons with 305/306; `reload→chamber` only if `!chamber_a_round_on_reload` (never here, `:215-216`).

| from (ready) | ordered transitions |
|---|---|
| inactive (true) | show: target ≠ inactive (five rows) |
| show (E) | hide T(inactive); chamber_aimed MC∧AIM; chamber MC; reload CMR; idle T(idle); reload T(reload); fire T(fire); aim T(aim); aim_fire T(aim_fire) |
| hide (E) | inactive T(inactive); show: any other target |
| idle (true) | hide T(inactive); chamber_aimed MC∧AIM; chamber MC; reload T(reload); fire T(fire); aim T(aim); aim_fire T(aim_fire) |
| reload (true) | hide T(inactive); [chamber MC∧E]; idle T(idle)∧E; idle II; fire T(fire)∧E; aim T(aim)∧E; aim_fire T(aim_fire)∧E |
| fire (true) | hide T(inactive)∧E; chamber_aimed MC∧E∧AIM; chamber MC∧E; reload E∧CMR; idle T(idle)∧E; reload T(reload)∧E; aim T(aim)∧E; aim_fire T(aim_fire)∧E; **aim_fire AIM** |
| aim (true) | hide T(inactive); idle T(idle); idle II; reload T(reload); fire T(fire); aim_fire T(aim_fire) |
| aim_fire (true) | hide T(inactive)∧E; chamber_aimed MC∧E∧AIM; chamber MC∧E; reload E∧CMR; idle T(idle)∧E; reload T(reload)∧E; fire T(fire)∧E; aim T(aim)∧E; **fire ¬AIM** |
| chamber (true) | hide T(inactive); idle T(idle)∧E; idle II; fire T(fire)∧E; aim T(aim)∧E; aim_fire T(aim_fire)∧E; chamber_aimed AIM |
| chamber_aimed (true) | hide T(inactive); idle T(idle)∧E; idle II; fire T(fire)∧E; aim T(aim)∧E; aim_fire T(aim_fire)∧E; chamber ¬AIM |

Notes: no `fire→fire` (cyclic fire just continues while target stays fire). `II` exits abort without the end effect (no ammo moved / no round chambered). Idle has no `II` exit, so with sprint active and an unchambered round each weapon tick alternates idle→chamber, chamber→idle (literal). Reload 304/317/324 can be interrupted only by `hide` and `II` (fire/aim targets need E).

---------------------------------------------------------------------------------------------------

## 5. Shots

1. Fire/aim_fire `initialize` subscribes `"shoot"` on the weapon object (`weapon_core_fire_state_base.cpp:16-24`, `wc.cpp:724-730`). `on_shot_event` (`weapon_core_fire_state_base.cpp:54-74`, `weapon_core_aimed_fire_state_base.cpp:71-91`): interrupt = true; `bullets_in_queue == 0` → LOG_ERROR, no shot; else `instant_fire(callback_time_in_ms)`.
2. Every fire clip of all 11 weapons has exactly one `"shoot"` knot (domain 7) ⇒ **one shot per clip pass**. Knot frames (F) and lengths (L) in §10.
3. Fire state entered by SELECT(s) (s = W1 time `t_{k-1}` of the trigger frame, or the W2 time at which show/chamber/reload ended with target fire): the event lexeme is new (idle/aimed use group −1) ⇒ phase 0 ⇒ **shot at `s + floor(F·1000/(30T))`**, clip end at `s + floor(L·1000/(30T))`; cyclic: shots at `s + n·C + floor(F·1000/(30T))`, ends at `s + (n+1)·C`.
   Exception (derived from `ttc.cpp:402-428`): if a previous non-frozen fire lexeme of group 1 is still fading (< 300 ms since it was replaced, i.e. re-entering fire within 0.3 s after leaving a *cyclic* fire at a loop end), the new one inherits its phase instead of starting at 0.
4. Queue behaviour:
   * Semi (queue 1; all pistols, shotguns, rifles, DB, and mode "1" of auto weapons): bullets 1 → once clip → shot → bullets 0 → W2 at the shot: `set_target(fire)` → idle/aim (not ready); state stays until the clip-end W2 (`T(idle)∧E`) → idle (`instant_idle_start` resets the queue only if the trigger is up) ⇒ next shot needs trigger up + down (each a W1).
   * Auto (0xFF) / burst (vityaz 3): cyclic while `bullets_in_queue > 1`. Releasing the trigger changes the target at that W1 but the state only leaves at the next loop-end W2 (`T(idle)∧E`); shots whose knot comes first still fire (bursts always complete unless E coincides).
   * Last-round switch (307/309 only, derived — verify): the shot that drops `bullets_in_queue` from 2 to 1 triggers SELECT at that ms; the rebuilt expression now uses once+freeze → different lexeme identity (playback type) → the new lexeme inherits the exact phase of the old group-1 driver (= the knot time) and a new lexeme may fire an event at its start phase (`aei.cpp:107-113, 261-264`) ⇒ the `"shoot"` knot fires **again at the same ms** inside `set_target_and_tick` → the final round is fired simultaneously with the previous one (for vityaz burst: s+8, s+88, s+88 ms instead of s+168). This is what the source implies; confirm with a client capture before relying on it.
   * Empty: last round → `mag+chambered == 0` → target reload (if `ready_to_reload`) → at the clip-end W2 `fire→reload` via `E∧CMR`.
5. fire ↔ aim_fire (`wc.cpp:232, 249`, no E needed): old finalize/new initialize (E=false, shoot re-subscribed, `m_is_firing` stays true at the end, queue **not** reset). Both use group 1, so the new clip **continues at the same normalized phase** (old driver not frozen; different clip pointer ⇒ new lexeme with inherited phase) — **no restart, no extra shot**; a knot not yet reached fires once in the new state, the cycle/clip end keeps its original ms. (A frozen old clip would restart the new one at 0, but a frozen once-clip means its end W2 already left the fire state.) Exception: a switch whose W1 coincides with the shot ms can re-fire the knot as in the last-round case. Pistol/DB re-pick the last-shot/barrel index at the new initialize (clip pointer only).
6. `"shoot"` with `bullets_in_queue == 0` (cyclic clip continuing after the queue emptied) is ignored.

---------------------------------------------------------------------------------------------------

## 6. Reload

### 6.1 304 magazine (ak_74u, magnum, toz_122, uzi, vityaz)
Entry from idle/aim/show on `T(reload)` (needs `ready_to_reload`), show on `CMR`, fire/aim_fire on `E∧CMR` / `T(reload)∧E`. Init: toz_122 unloads its chambered round into the magazine count. Duration from entry SELECT `s`: end at `s + floor(L_i·1000/(30T))`, T from item[0]; all indices have equal length ⇒ ≈ `reload_time`. **All ammo moves at the end callback** (`instant_reload`, which also resets the fire queue), i.e. *before* the W2 weapon tick at that ms; that tick leaves via E (target idle/aim/fire/aim_fire) — so a trigger held through a 304/317/324 reload goes straight `reload→fire` at the end ms (contrast: shotgun 310 chambers after the decision, §6.4). Aborted (hide/II) → nothing loaded.

### 6.2 324 pistol (fort_17, tt_33)
`e = (mag == 0)` each SELECT (`pistol_weapon_core_reload_state.cpp:79-99`), item clips both `reload_*` 48 f; T = (48/30)/1.6 ⇒ 1599* ms. After `instant_reload` the index flips, but the state is left at that same W2.

### 6.3 317 double barrel (toz_34, toz_66)
`r = (mag != 1)`: 1 round left → first-barrel clip, 0 left → two-barrels clip; T from the first-barrel clip. toz_34: 80 f / 110 f, reload_time 2.5 → 2500 / 3437 ms. toz_66: 80 f / 104 f, 2.0 → 2000 / 2600 ms. Loads `min(stack, 2 − mag)` at the end.

### 6.4 310 shotgun (rem_700, rem_870)
Sub-fsm start → one_round (true, gated by start's `m_animation_ended`) → finish (`finish_reload_predicate`) (`weapon_core_shotgun_reload_state.cpp:115-131`); sub-fsm ticks once per weapon tick (outer `execute`).
* start: init unloads the chambered round (`++mag`). Clip `reload_start`, T=1: rem_870 35 f → 1166 ms, rem_700 19 f → 633 ms. End W2 → (same weapon tick) sub-transition to one_round; one_round's lexeme starts at that ms.
* one_round: cyclic `reload_1` 10 f, T = (10/30)·cap/reload_time ⇒ cycle 500 ms (rem_870, T 0.6667) / 600 ms* (rem_700, T 0.5556). **Each loop-end W2: `reload_one_round()` (+1 round if mag<cap && stack>0)**, then in the same weapon tick `finish_reload_predicate` is checked: `mag == cap || stack == 0 || (mag+chambered != 0 && target ∈ {fire, aim_fire})` (`:142-148`). The fire clause never triggers on rem_700/rem_870 (chamber unloaded ⇒ `is_ready_to_shoot()` false ⇒ `set_target` turns fire into idle).
* finish: `reload_end`, T=1: rem_870 18 f → 600 ms, rem_700 31 f → 1033 ms. Its end callback sets the outer E during the animation tick; the W2 weapon tick at that ms sees E and leaves reload (`T(idle|fire|aim|aim_fire)∧E`) → outer finalize → `finish.finalize` → **round chambered at that moment** (`mag != 0` → `instant_chamber_a_round`). Because the chamber happens *after* the transition decision, a held trigger yields target idle (not ready) ⇒ reload→idle; with the trigger still held there is no further weapon tick until the next W1 (input change) — the player must re-press.
* Interrupted during start/one_round (hide or II): loaded rounds stay, nothing chambered; afterwards `MC` sends idle → chamber_a_round (305).
* Total (N rounds to load, uninterrupted): rem_870 1166 + 500·N + 600 ms; rem_700 633 + 600·N + 1033 ms.

---------------------------------------------------------------------------------------------------

## 7. Chamber, show/hide, switch

### 7.1 Chamber 305/306
Entry `MC` from show/idle, `MC∧E` from fire/aim_fire (= at the shot clip's end W2); aimed variant first if `AIM`. T = 1: rem_870 30 f → 1000 ms (aimed clip also 30 f); rem_700 / toz_122 40 f → 1333 ms. End W2 → `instant_chamber_a_round`, leaves in the same tick (E). chamber ↔ chamber_aimed on AIM change (no E; group 8 ⇒ phase continues, E re-cleared). In chamber_aimed `is_idle()` is true ⇒ trigger-up there resets the queue.

### 7.2 Show / hide
Wait for the **item** clip end; item idle clip 36 f at T = 36/18 = 2 ⇒ **600 ms**, equal to the 18-frame hands show/hide clips (all `…/hands_only/show|hide` = 18 f in resources.db, all weapons, both views). Not cancellable (ready = E). `is_toggling()` = `m_is_toggling || !m_is_shown` (`weapon_core.h:86`) blocks sprint/jump predicates during show/hide and until the show end.

### 7.3 Switch A → B
1. Frame k with 0x1000/0x2000 (toggle bit, set on the key-down frame only): step d sets `target_active_object = B` + force (`player.cpp:973-1001, 844-853`); W1 at `t_{k-1}`: A's tick → target inactive → hide immediately from idle/aim/reload/chamber/chamber_aimed; from fire/aim_fire at their next end W2; from show only after show ends.
2. Hide end W2 at `h = start + 600`: A hide→inactive; same SELECT: A ready → `A.deactivate()`, swap, `B.activate()`, `B.tick()` (`base_player.cpp:40-48`).
3. `B.activate()` (`wc.cpp:750-823`): `m_is_shown=false`; `m_ready_for_fire=false` immediately overwritten to true by `set_fire_bullet_transform` (`:758, 768, 555-559`) ⇒ never gates firing; `m_is_in_sprint_transition=false`; selector restarts at stand (jump/sprint state lost); fsm := inactive + one tick with B's stale target (first activation: `idle` → show inside activate; later: `inactive` → stays); first activation only: `load_ammo()` (fill + chamber); `reset_fire_queue()`; `instant_show()`. `B.tick()` → inactive→show at `h`.
4. Show end at `h + 600` → show→(chamber | reload | idle | fire | aim | aim_fire).
⇒ from an idle A: hide `[t_{k-1}, t_{k-1}+600]`, show `[t_{k-1}+600, t_{k-1}+1200]`, first possible shot `t_{k-1}+1200+shot_ms`.

---------------------------------------------------------------------------------------------------

## 8. Selector (stand / crouch / sprint / jump), sprint transition, jump, stamina

Selector fsm (`weapon_user_animations_selector.cpp:40-72`), transitions in order:
* stand → crouch (crouch_pred), → sprint (sprint_pred), → jump (jump_pred)
* crouch → stand (stand_pred), → sprint (sprint_pred)
* sprint → crouch (crouch_pred), → stand (stand_pred), → jump (jump_pred)
* jump (ready only when `jump_logic.is_jump_finished() || legs == 2 || dead`, `player_logic_jump_state.cpp:26-31`) → crouch (broken_legs_pred), → stand (stand_pred, always true here), → sprint (unreachable)

Predicates:
| predicate | definition | source |
|---|---|---|
| sprint_pred | `!weapon.m_is_firing && !weapon.is_toggling() && broken_legs == 0 && !m_forced_not_to_sprint && input.is_sprinting() && stamina.can_be_spent()` | `:189-196` |
| stand_pred | crouch state: `legs != 2 && !(mask&0x100) && physics.can_stand()`; sprint state: `!sprint_pred`; otherwise true | `:198-209` |
| crouch_pred | `broken_legs_pred || ((mask&0x100) && physics.can_crouch())` | `:231-234` |
| broken_legs_pred | `broken_legs == 2` | `:241-244` |
| jump_pred | `!firing && !toggling && legs == 0 && (mask&0x10) && physics.can_jump() && stamina.value >= max·factor/5` (= 20) | `:246-256`, `player_stamina.h:47` |
| `input.is_trying_to_sprint()` | `mask & 0x200` | `player_input.h:25` |
| `input.is_sprinting()` | `(mask&0x200) && (mask&0x1) && !(mask & 0x16E)` (no back/left/right/fire/reload/crouch; jump/aim allowed) | `player_input_inline.h:8-13` |
| `m_forced_not_to_sprint` (selector tick, before fsm tick) | `!(mask&0x200)` → false; else if `!(input.is_sprinting() && weapon.is_idle()) && state == sprint` → true | `:80-88` |

`can_jump` = not crouched, on ground (|vertical velocity| < 1e-3), not on a steep slope (`physics/sources/bullet_character_controller.cpp:448-450, 467-469`) — physics state at the W1/W2 moment.

Sprint transition (`wc.cpp:561-604`): latched in the first weapon tick where the selector is in sprint; cleared by `channel_id_on_animation_lexeme_end` of the sprint clip (sprint_hud[0]/sprint[0] = `…/sprint/sprint` or `sprint_pistol`, 22 f). While sprinting the lexeme is matched every SELECT and never ends. When a SELECT builds an expression without it (selector left sprint), the sprint lexeme fades with the new weight-group-0 driver's interpolator (stand/crouch/jump movement lexemes: linear 0.3 s, `player_logic_stand_state.cpp:90-92`, `player_logic_crouch_state.cpp:96-98`, `jump_logic_state_start.cpp:85-86`) and `lexeme_end` fires at fade start + floor(0.3f·1000) = **300 ms** (`animation/sources/mixing_n_ary_tree_weight_event_iterator.cpp:63-72`) → interrupt → W2 → `m_is_in_sprint_transition = false`. ⇒ fire/aim_fire/reload are blocked from the first weapon tick in sprint until 300 ms after the weapon tick that left sprint. (The 22-frame clip length itself does not matter.)

Jump (all weapons use the ak jump clips, 1st view; `weapon_user_animations_container_cook.cpp:83-84`):
* W1 with 0x10 → stand/sprint→jump; jump state init `jump_logic.activate()`; execute ticks jump_logic: inactive→start (`jump_logic.cpp:108`); start lexeme starts at that SELECT time `j`.
* `"jump"` event (domain 4, not interrupting): stamina `spend(20)` + physics `jump()` (`jump_logic_state_start.cpp:171-181`).
* Start ends at its waited interval end (interrupting W2) → jump_logic start→landing (landing init: `end_jump()`) (`jump_logic.cpp:109`, `jump_logic_state_landing.cpp:129-137`).
* Landing interval end (W2) → `is_jump_finished` → in that same weapon tick the selector goes jump→stand (or crouch with 2 broken legs). No physics landing check, no timer.
* On-site jump: start `on_site` 24 f (jump event at f7 = 233 ms) → 800 ms; landing `on_site_landing` 12 f → 400 ms ⇒ selector in jump during `[j, j+1200]`.
* Moving jumps: start clip 23 f (fwd/back/diagonals, event f6) or 20 f (left/right, event f2), landing 13 f / 6 f; the start lexeme has two intervals (move-clip "preface" + jump clip, order by leg) in time group 0, so it inherits the movement cycle phase (`jump_logic_state_start.cpp:58-90`) — exact moving-jump duration therefore depends on the movement phase; not resolved (§11).
* While in jump: fire→idle/aim_fire→aim (`set_target`), `II` aborts reload/chamber/aim, `MC` false, `ready_to_reload` false, `is_trying_to_aim` false.

Stamina (`game_core/sources/player_stamina.cpp`; config `gameplay__players__default.player.txt` `stamina_params`: max_value 100, spending_speed 20, regeneration_speed 10, spending_threshold 20, regeneration_threshold 0, max_carried_weight 30; factors 1.0 by default `:11-19`):
* Ticked once per frame at `t_k` before any SELECT: `tick(t_k, can_be_spent() && selector==sprint)` (`pt.cpp:339-340`).
* Sprinting: `value -= 20·factor·(t_k − last_tick)/1000`, `last_spending = t_k` (`:151-156`). `decrease_value` clamps to [0, max]; if `value <= regeneration_threshold (0)` → `lower_threshold_was_reached = true` and depletion subscribers fire (sprint state → `force_animation_selection`, `player_logic_sprint_state.cpp:103-118`) (`:112-121`).
* Regeneration: if `value < max` and (`value == 0` or (`last_spending != 0 && last_spending + 1000 < t_k`)): `value += 10·factor·(t_k − last_tick)/1000` (only if `last_tick != 0`) (`:123-143`); `increase_value` clears the flag when `value > spending_threshold (20)` (`:97-102`).
* `can_be_spent()` = `value > 20 ? true : !lower_threshold_was_reached` (`:158-162`) ⇒ after hitting 0 you cannot sprint again until value > 20.
* Jump: needs `value >= 20`; spends 20 at the `"jump"` event (`spend` sets `last_spending = last stamina tick time`, `:145-149`).

---------------------------------------------------------------------------------------------------

## 9. Time-source subtleties (summary)

* Weapon logic time = W1/W2 instants only (§0); there is no per-frame weapon tick. The client clock is `game::m_timer.get_elapsed_msec()` (u32 ms, variable frame delta) (`game.cpp:629`); the same `t_k` is sent to the server with mask_k (`pt.cpp:392-393`).
* Input of frame k is applied at `t_{k-1}` (W1). Animation-end and shot events are observed at their exact ms (W2), not at frame boundaries — a state with an end callback leaves at the end ms itself.
* The only per-frame things are stamina (`t_k`), dispersion/bullet transform (`t_k`, after the loop) and slot switching (before the loop).
* Events fired inside `set_target_and_tick` (knot exactly at a new lexeme's start phase, e.g. §5.4 last-round case) execute their effects but do not trigger a weapon tick.
* `is_replaying_history` (server-correction re-simulation) only re-simulates `m_target` physics; it never re-runs weapon logic (`pt.cpp:114-152, 196-217`).
* Lags ≥ 10 s skip animation events (`animation_player.cpp:271-288`).
* Client mask generation (for reading captures): the mask is rebuilt every frame from key events; hold bindings keep their bit set on the release frame too; 0x400/0x800/0x1000/0x2000 are toggle bindings set on the key-down frame only; while aiming, LSHIFT produces 0x8000000 (hold breath) instead of 0x200 (key context switch in `game/sources/weapon.cpp:151,162`) (`player_input_handler.cpp:77-81,150-324`, `key_binder.cpp:84-130`).

---------------------------------------------------------------------------------------------------

## 10. Numbers (float32 model; `*` = within 1e-4 of an integer, may be ±1 ms; first value = float32 emulation, exact rational in brackets)

Fire (1st view stand/crouch item clips; 3rd-view differs only for magnum):
| weapon | queues | fire ids | L (f) | F | rpm | T | shot after entry (ms) | clip pass / cycle C (ms) |
|---|---|---|---|---|---|---|---|---|
| ak_74u | auto, 1 | 307/309 | 10 | 1 | 650 | 3.611111 | 9 [9.23] | 92 [92.31] |
| uzi | auto, 1 | 307/309 | 10 | 1 | 600 | 3.333333 | 10* | 100* |
| vityaz | auto, 1, 3 | 307/309 | 10 | 1 | 750 | 4.166667 | 8* | 80* |
| rem_870 | 1 | 307/309 | 10 | 1 | 250 | 1.388889 | 24* | 240* |
| rem_700 | 1 | 307/309 | 10 | 1 | 120 | 0.666667 | 50* | 500* |
| toz_122 | 1 | 307/309 | 10 | 1 | 200 | 1.111111 | 30* | 300* |
| toz_34 | 1 | 307/309 | 25 | 1 | 100 | 1.388889 | 24* | 599* [600] |
| toz_66 | 1 | 318/320 | 32 | 4 | 80 | 1.422222 | 93 [93.75] | 750* |
| magnum | 1 | 307/309 | 30 | fire 7 (3rd view 5); aimed 8 (3rd 5) | 100 | 1.666667 | fire 140*, aimed 160*, 3rd view 100* | 600* |
| fort_17, tt_33 | 1 | 325/327 | 10 | 1 | 250 | 1.388889 | 24* | 240* |

Cycle C = `floor(L·1000/(30T))`; shot = `floor(F·1000/(30T))`; T = `(rpm/60) · (L/30)` for one shoot knot. All clip indices of a weapon's fire/aimed_fire have the same L and F except magnum (above).

Other durations from the entering SELECT (end-callback ms offset):
| item | clip | T | ms |
|---|---|---|---|
| show / hide, all weapons | item idle 36 f (hands 18 f) | 2.0 | 600* |
| reload 304 ak_74u, vityaz | 70 f, reload_time 2.5 | 0.933333 | 2499* [2500] |
| reload 304 uzi | 60 f, 2.5 | 0.8 | 2500* |
| reload 304 magnum | 105 f, 3.0 | 1.166667 | 2999* [3000] |
| reload 304 toz_122 | 120 f, 3.0 | 1.333333 | 3000* |
| reload 324 fort_17, tt_33 | 48 f, 1.6 | 1.0000001 | 1599* [1600] |
| reload 317 toz_34 first / both | 80 f / 110 f, 2.5 | 1.066667 | 2500* / 3437 [3437.5] |
| reload 317 toz_66 first / both | 80 f / 104 f, 2.0 | 1.333333 | 2000* / 2600* |
| chamber 305/306 rem_870 | 30 f | 1 | 1000* |
| chamber 305 rem_700, toz_122 | 40 f | 1 | 1333 |
| shotgun rem_870 start / per round / finish | 35 f / 10 f / 18 f | 1 / 0.666667 / 1 | 1166 / 500* / 600* |
| shotgun rem_700 start / per round / finish | 19 f / 10 f / 31 f | 1 / 0.555556 / 1 | 633 / 600* / 1033 |
| sprint-transition tail after the SELECT that left sprint | weight fade | — | 300 |
| jump on-site start / landing (jump event at 233) | 24 f / 12 f | 1 | 800* / 400* |
| moving jump start (fwd/back/diag, left/right) / landing | 23 f, 20 f / 13 f, 6 f | 1 | 766, 666 / 433, 200 (+ phase-dependent preface, §11) |
| any weight fade (all interpolators used here) | — | — | 300 |

---------------------------------------------------------------------------------------------------

## 11. Not determined / caveats

1. **Last-round double shot** (§5.4) and **phase inheritance on re-entering fire within 0.3 s of a cyclic fire** (§5.3): derived by reading `ttc.cpp:98-136, 246-252, 402-428` and `aei.cpp:107-113, 261-264` together; not runtime-verified. Check with a client capture (vityaz burst, ak full-auto last two rounds).
2. **±1 ms boundaries**: values marked `*` sit on integers; `/fp:fast` may reorder `x·1000/T` and the anchor resets at weight-transition ends (`tree.cpp:690-708`) can shift sub-ms rounding. Model: float32 per operation, floor.
3. **Moving-jump duration**: depends on the movement-lexeme phase inherited via time group 0 and on `is_jump_from_right_leg` (`jump_logic.cpp:211-215`, `jump_logic_state_start.cpp:58-90`); interval mapping not resolved.
4. **Server stepping**: no server-side simulation source exists in this tree (`game_server/sources` is an app shell). The spec describes the client; a server must reproduce W1 at the *previous* update's time and W2 at event times.
5. **3rd-person camera** changes `view` to 1 (different clip indices; matters only for magnum's shoot knot 5 vs 7). The client update does not carry the view mode.
6. `weapon_core_inactive_state::has_animation_ended` is declared virtual returning true but the base accessor is non-virtual (`weapon_core_inactive_state.h:16` vs `weapon_core_base_state.h:33`); irrelevant because no inactive transition uses E.
7. Physics predicates (`can_jump`, `can_stand`, `can_crouch`, on-ground) were not traced into Bullet; Bullet's internal stepping was not inspected.
