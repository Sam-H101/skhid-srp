# 0.21d sprint: why Shift does nothing, and the fix

Tags: [V] verified in the client code (survarium.exe 0.21d, VAs, image base 0x10000), [I] inferred.

## TL;DR

The player is **overburdened**. `base_player::can_sprint` refuses to sprint when the inventory weight is
>= max carried weight. Max carried weight = `default.player` `stamina_params.max_carried_weight` = **25**, plus
modifier #5 from the 0x51 profile, which is **additive** and which we send as 0. The test loadout that
`Account::new` equips weighs **35.5 kg**: 25.3 kg of gear alone, plus 10.2 kg from 4 × 120 rounds. So
`35.5 >= 25`, and the player cannot sprint. [V]

**Fix:** in `write_profile_static`, send modifier mask bit 5 (`0x20`) followed by one f32 that lifts the limit
above the loadout weight, e.g. `+100.0`. Alternatively, give new accounts a loadout under 25 kg. [V]

## 1. Input and key binding

| what | value | where |
|---|---|---|
| player_input layout (0x45 payload) | `f32 dx, f32 dy, u32 actions, u32 time`; actions at +8 | [V] `is_sprinting@player_input` 0x1D2772 |
| action bits | move_forward 0x1, move_backward 0x2, strafe L/R 0x4/0x8, jump 0x10, shooting 0x20, aim 0x100, crouch 0x200, **sprint 0x400**, hold_breath 0x200000 | [V] PDB enum @41c5b58 |
| `player_input::is_sprinting` | `(a & 0x400) && (a & 0x1) && !(a & 0x222)`: sprint and forward, with no backward, shooting or crouch. Strafing and aim bits are allowed here | [V] 0x1D2772 |
| game action → bit | `process_first_person_mode` 0x17F268: if `action_present(accel=7)` **or** `action_present(sprint_toggle=8)`, then `actions |= 0x400` | [V] |
| accel vs sprint_toggle | Both set the same bit 0x400. `accel` is a hold action (held-actions circular buffer, `on_keyboard_action` 0x17EA6B). `sprint_toggle` is a toggle (toggle list +2A8, `process_toggle_action` 0x17EBF1). Accel does not need the toggle | [V] |
| toggle cancels (table 0x87ADE0, `process_dependent_actions` 0x17ECC7) | Pressing accel(7) clears crouch_toggle and sprint_toggle. Pressing crouch(5) or wpn_fire(37) clears sprint_toggle. aim, jump, wpn_1/2, melee and wpn_next clear wpn_aim_toggle | [V] |
| default bindings (`set_default_controls` 0x155843, table 0x87B9E8, stride 0x14) | `accel` = **kLSHIFT** (context mask 1), `sprint_toggle` = kX (toggle), `delay_breath` = kLSHIFT (context mask 4, aiming), `crouch` = kLCONTROL, `forward` = kW | [V] |

The user's user.cfg matches the defaults (`bind accel kLSHIFT`). So Shift does set 0x400. The server relays the
16-byte input unchanged (`game.rs` `relay`). **Input is not the cause.** [V]

## 2. Simulation gates

Movement speed: `set_physics_controller_walk_vector` 0x1D4B19 uses `sprint_fwd` only when
`is_sprinting(input) && active_object->vfunc58()` holds. The sprint state, animation, jump and stamina use
`weapon_user_animations_selector::is_trying_to_sprint` 0x1E0E4C, which requires
`is_sprinting(input) && base_player::can_sprint()`.

`base_player::can_sprint` 0x1D6747 (esi = player):

| # | check | reads | normally | addr |
|---|---|---|---|---|
| 1 | `damage_model[+748] == 0` | broken-limb counter (2 = broken legs; `on_broken_limb_affect` 0x1D8458) | 0 | 0x1D6754 [V] |
| 2 | `stamina.m_is_low_stamina == 0` | player+0x110E0 = player_stamina(+0x11078)+0x68. Set when value <= `start_low_stamina_value` (0), cleared when value > `stop_low_stamina_value` (30) (`tick` 0x1FFE2E) | false (the user sees a full bar) | 0x1D675D [V] |
| 3 | **`!is_overburdened(inventory.weight)`** | weight = `[player+10C]+0x184`. Limit = `apply_modifier(5 max_carried_weight, base 25, 1)` | **fails: 35.5 >= 25** | 0x1D677C → 0x1FFFE4 [V] |
| 4 | `active_object->can_sprint()` (vfunc +0x58) | weapon_core 0x1E4911: not aiming (`+4C0` = instant_aim flag) **and** weapon state is idle(3), reload(4), chamber_a_round(6) or throw_grenade(7) | true when holding the weapon, not aiming | 0x1D678D [V] |
| 5 | `char_controller[+1F2] != 0` | bt_character_controller flag (on ground / can stand) | true | 0x1D679C [V] |

There is no check for game status (warm-up or countdown), skills or +9C8 in any sprint path. [V]
(`+9C8` is read only by `new_player_damage_protector::reduce_damage` 0x204747 and the lobby UI.)

## 3. Where the values come from

**Stamina params** (`stamina_base_parameters::load` 0x1FFB4D, embedded at player_stamina+0), from
`gameplay/players/default.player` → `stamina_params`: max_value 100, sprint_spending_speed 8, regeneration_speed 5,
amount_to_jump 10, start_low 0, stop_low 30, min_carried_weight 15, **max_carried_weight 25** (+0x1C),
movement_speed_mod -0.15, spending_speed_mod 1.5. [V]

**Modifiers.** `deserialize_static` 0x1F5C52 zeroes the 20 floats at profile+0x5B8 and then adds one f32 per set
mask bit. `apply_modifier` 0x1DEBA7 computes `sum = float[e] + dynamic list`:
- e in {5 max_carried_weight, 10 engineer_success, 14 artefact_slots, 15 device_slots, 16 ammo_bags,
  17 quick_slot_size} (table 0x73ECC4) → `sum*scale + base` (**additive**)
- any other e → `(sum*scale + 1.0) * base` (multiplicative; 0x876F40 = 1.0)

So mask 0 leaves every base as it is: stamina, regeneration and speed are unchanged. The limit stays at **25**. [V]
Every item config has `max_carried_weight_modifier: 0`. [V]

**Weight** (`inventory::update_weight` 0x1FAB64): the sum over 23 slots of `amount(+17C) × weight(+158)`, plus the
weapon's loaded rounds × ammo weight. Item weight is the config's `weight`, or `clip_weight / clip_size` for ammo
(`dictionary_item_params::load` 0x1C5D31). Armour and weapons get amount 1 (`inventory_cook` 0x1C5784, 0x1C5871).
Ammo slots get `min(descr u16 count, descr u32 amount)` (`setup_from_profile` 0x1FAE33). [V]

Test loadout (`Account::new`, test_player_profiles profile 0):

| item | kg |
|---|---|
| ren_helmet_1 1.7, scav_resp_1 0.5, ren_torso_1 10, scav_back_1 4, ren_legs_1 2.1, scav_gloves_1 0.2, ren_boots_1 1.2 | 19.7 |
| aps 1.2, spas_12 4.4 | 5.6 |
| 120 × 9x18 makarov (0.01), 120 × 9x18 pbm (0.01), 120 × 12mm buck (0.03), 120 × 12mm slug (0.035) | 10.2 |
| **total** | **35.5** (gear alone 25.3, still >= 25) |

Skills (`db_skills_tree`): booster id N is modifier N − 1 (`fill_character_data` 0x1759DF: `dec eax`) [V]. skill_2
(physical) has booster 6 = max_carried_weight at +1 kg per level, so +20 kg at 20 points; its booster 5 is movement
speed. srp sends the booster sums in this mask (`v021d/skills.rs`), the multiplicative ones / 100, as every match
call passes scale 1 [V].

## 4. Fix

| field (0x51 / 36 02 / 36 10 profile) | now | send |
|---|---|---|
| modifier mask (u32 after the slots) | 0 | **0x20** (bit 5) [V] |
| f32 for bit 5 | — | **100.0** (limit becomes 125 kg; any value > weight − 25 works, e.g. ≥ 11 for this loadout) [V] |
| u32 @+9C8 | 10 | unchanged; it does not affect sprint [V] |
| team, bool, name, slots | — | unchanged |

Layout: `... slots, u32 0x00000020, f32 100.0`. The floats come in ascending bit order, one per set bit. [V]

Alternatives: send fewer rounds per ammo slot (min(u16 count, u32 amount) is what counts), or use a default
loadout under 25 kg (e.g. a scavenger torso at 3 kg instead of ren_torso_1). Below 25 kg but above 15
(`min_carried_weight`) the player can sprint, but `max_carried_weight_*` probably scales stamina spending and
movement speed [I].

Notes:
- The same `write_profile_static` feeds the lobby (36 02 / 36 10), so the lobby's player-params panel
  (`lobby_menu::create_player_params`, which reads the modifiers container) will show the extra carry weight too [I].
- All clients must get identical profiles (deterministic sim). The 0x51 already goes to everyone [V].
