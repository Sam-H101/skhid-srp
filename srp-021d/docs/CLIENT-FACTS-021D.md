# Verified facts about the 0.21d client

> **Version scope: client 0.21d** (build 2010), except where a line says
> 0.100b. Addresses, data values (skills tree, item configs) and message
> formats differ in other builds: re-check them there.

Addresses are virtual addresses in `survarium.exe` 0.21d (image base 0x10000;
file offset = VA − 0x10C00). Function names come from the PDB (see
RE-TOOLKIT.md). Everything here was checked in the disassembly unless marked
otherwise. More: `srp-021d/docs/notes/` (`lobby_spec.md`,
`match.md`, `sprint.md`, `option_b.md`, `clock_sync.md`, `ping.md`).

## Player stats: modifiers, skills, perks, gear

A profile carries **20 player modifiers** (`player_params_modifiers_enum`):

| Bit | Modifier | Bit | Modifier |
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

- **Where they come from:** `player_profile::deserialize_static` 0x1F5C52 reads
  a u32 mask, then one f32 per set bit in bit order (profile +0x5B8). The
  server sends this in 0x51 (match) and 36 02 / 36 10 (lobby).
- **How they apply in a match:** `player_params_modifiers_container::apply_modifier`
  0x1DEBA7 = profile float + a runtime list. Additive for bits 5, 10, 14, 15,
  16 and 17 (table 0x73ECC4): `sum·scale + base`. Otherwise multiplicative:
  `(sum·scale + 1)·base`. **Every gameplay call passes scale 1**, so
  multiplicative modifiers are fractions (an artefact's 0.15 movement speed =
  +15 %). The runtime list holds only artefacts, medkits, weapons, the battery
  and stamina overload.
- **Skills:** `gameplay/db_skills_tree` has 5 skills × 20 levels. Each level
  has boosters {id, value}; **booster id N = modifier N − 1** (0x1759DF
  `dec eax`), and points p give levels 1..p. The lobby sums them only for its
  stats panel (`fill_character_data` 0x175809 → `create_player_params`
  0x175C81). They never reach the profile, so **the server must send the
  sums**. The tree's values are percents: srp divides the multiplicative ones
  by 100. Per point:

  | Skill | Gives |
  |---|---|
  | 1 sniper | dispersion −2.5 %, aiming speed +5 % |
  | 2 physical | stamina regeneration +5 %, movement speed +0.5 %, **carried weight +1 kg** |
  | 3 engineering | device damage +2.5 %, splash radius +1 % |
  | 4 medical | health regeneration +5 %, pain +2 % |
  | 5 knowledge | artefact search time −5 %, anomaly damage −2.5 % |

  srp sends them since 2026-10-11 20:47 (`crates/survarium/src/v021d/skills.rs`).
- **Perks do nothing in 0.21d.** `perks_dict` has only names and icons. The
  exe reads perks only in `lobby_menu::fill_skills_tree` and
  `fill_character_data`, and the match profile has no perk field.
- **Gear:**
  - Armour's per-body-part protection (`hit_params`: armor and reduce for
    each of 8 hit types) is added to the player's damage model by
    `armor::on_holder_assigned` 0x20B9A9, from the item config of each slot
    0x51 names. It works with no server help.
  - Gear models, sounds and materials load the same way
    (`player_cook::query_for_equipment_configs`, slots 6, 1, 2, 4, 3).
  - Every armour config's `*_modifier` is 0 in 0.21d. Only the lobby's free
    `apply_modifier` 0x1DE93C adds item modifiers (slots 3, 0, 1, 2, 4, 5,
    6); nothing does in a match.
- **Sprint and weight:** `base_player::can_sprint` 0x1D6747 refuses when
  inventory weight ≥ `apply_modifier(5, 25 kg)`
  (`player_stamina::is_overburdened` 0x1FFFE4). The base is
  `default.player` `max_carried_weight` 25. Sprint input is 0x400 (Shift =
  `accel`, X = `sprint_toggle`).

## Lobby protocol pitfalls (`lobby_spec.md`)

- The lobby server pushes everything; the client asks for little.
- The order matters: 36 01 (profiles) before 36 06 (skills), or the skill bytes
  are misparsed. 0x0A before 0x05, or the nickname is empty. The inventory
  (36 03) must hold every item a profile's slots name before that profile's
  36 02, or the paper doll stays empty.
- Unknown modification or item ids crash the client.
- At most 8 profiles; names ≤ 63 bytes.

## Matches

See MATCH-SYNC-021D.md. In short:
- the stock client stays frozen until its first snapshot
  (`game_world_core::move` 0x1CCCA3 checks +0xC80E);
- its server mode is hard-coded off (0x151603);
- it never sends a world on its own, which is why the client patch uploads
  one.

## Crashes and limits not caused by srp

- **Lobby UI crashes** (ACCESS_VIOLATION in Scaleform AS3, garbage call
  stacks), found in the **0.100b** client (not yet checked in 0.21d, which
  uses the same Scaleform): each Flash movie's memory pool is created
  without thread safety (`or eax,3` at 0.100b's 0x402948). The game-logic thread (running
  AS3) and the render thread use it with no lock. Found from 7 minidumps.
  Candidate one-byte patch (3 → 2) **not applied**; it needs a yes and a
  backup.
- **"Not enough memory; try -max_video_memory"** on level_02: the
  `pvp_match_core` ctor (0x1DC90A) allocates 11 × 0xB9954 bytes of history
  from the 104 MB unmanaged arena. Fix: `-minimum_resources_memory_size=768`
  (default 448; `-managed_to_unmanaged_share`, default 3.5). It is in the
  local shortcuts; check what the players' zip launches with.
- **One client per Windows session**, through the mutex
  `survarium_or_its_installer_is_already_running`. The srp patch removes the
  check.
- After alt-tab in fullscreen, the window can come back invisible (seen with
  0.100b); windowed mode avoids it. Alt-tabbed 0.21d clients lag about 1 s.
- The HUD once showed a ping of 525 ms on localhost (real round trip 3 ms);
  the research is in `ping.md`.
