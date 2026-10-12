# Why the trap slot shows x10 (srp gives 2)

Short answer: the client reads 10 from the spawn message. The 10 is not a trap count. It is the low half of weapon 2's `start_transition_time_in_ms_rhs` (srp sends 10). srp sends a weapon logic-state block for **both** weapons. The client reads that block only for a weapon whose FSM has a current state, and at a spawn that is only the weapon `player::insert` activated (weapon 1). So every u16 after weapon 2's chamber byte is read 12 bytes early.

## 1. Where the trap set's amount and trap objects come from

- **Cook (once per player object).** `inventory_cook::translate_query` (0x765330, trap branch at 0x76566C) builds `booby_trap_set_cook_data` = { profile `is_local` (+0x1B4), **(u8) slot.condition_or_stack** (slot +0) }.
  - `booby_trap_set_core_cook::on_config_ready` (0x7624C0) calls `set_amount(byte)` at 0x76257A, then `load` (0x6FDC50). `load` sizes the trap buffer (+0x118/+0x144) to `amount()` (0x6FDC9B).
  - It then queries that many `booby_trap` sub-resources (loop 0x762618–0x762634).
  - So the number of trap objects is the profile's `condition_or_stack` as a u8. No config value clamps it, and nothing in `base_trap.load` reads a count.
- **Each life.** `player::insert` (0x5E4CE1) calls `inventory::setup_from_profile` (0x700800). For a quick slot, if the dictionary item's `is_stack` (+0x11B) is set:
  - `amount = min(condition_or_stack, amount_in_inventory)`, and `amount_in_inventory -= amount` (0x7009B3–0x7009E2).
  - Otherwise `amount = (u16) condition_or_stack`.
- **The spawn message overrides it.** `player::deserialize` (0x5E52C0) calls `insert`, then the stamina, then `inventory::deserialize` (0x7004A0).
  - `call_item_deserialize` (0x700270) skips slots 0–6 (table 0x8A2E84) and calls vtable +0x4C.
  - For `booby_trap_set` (vtable 0x96F744) that is `inventory_item::deserialize` (0x5A0810): a u16 into `m_amount` (+0x114).
- **Placing a trap.** `on_trap_placed_message` (0x740280) inserts `traps[index]`, with the index taken from the message and not checked. It also lowers `m_amount` by 1 (0x740303).

## 2. What the HUD shows

- `game_world_ui::update_quick_slot` (0x5D2B30) and `fill_quick_slots` (0x5D2E00) call vtable +0x6C. That is `inventory_item::get_item_props` (0x5A07E0, the base version; no override for the trap set). It returns `{dict = +0x116, count = +0x114}`.
- `create_slot_value` (0x5D1E20) sets `count = m_amount` and `enabled = m_amount > 0`.
- `QuickItemRenderer.as` draws `"x" + count` only when `count > 1 && enabled`.
- So the HUD shows the trap set's `m_amount`.

## 3. What the client allows

- Online, `booby_trap_set::action` (0x740770) does not place traps itself. `try_place_trap` runs only when the world's vfunc +0x14 is false. Otherwise it only toggles the ghost model, and only while `m_amount != 0` (0x7407DD). So the client keeps asking for placements while its own amount is above 0. That is why srp logged repeated "none left" refusals.
- **Per life:** the client allows `m_amount` placements, the value the spawn's u16 sets.
- **At once:** the number of trap objects, which is (u8) `condition_or_stack` from the profile in `PlayerProfile`. `try_place_trap`'s `find_if` at 0x6FE98D is srp's `AllOut`. A `trap_placed` index at or above this number would index past the buffer.
- **srp's numbers are already right:**
  - `TrapSet` objects = `condition_or_stack as u8`.
  - amount = `min(condition_or_stack, life amount_in_inventory)`.
  - The spawn u16 = that same amount (`runtime_amount` or `kit.amounts`).
  - No change is needed here.

## 4. Where 10 comes from

- `weapon_core::deserialize` (0x5A4450) reads the logic-state block only if `fsm(+0x414)->current (+0x10) != 0` (0x5A460D–0x5A4627). The block is: u8 shown, the hand IK (u8 + 2×u32), the sound state, and the selector.
- `weapon_core::activate` sets the current state (call at 005A68D2). `deactivate` clears it (0x5A422D).
- `player::insert` activates only weapon 1, or weapon 2 if there is no weapon 1 (0x5E4DD5 / 0x5E4DF2). Weapon 2 has never been activated, so it reads no block.
- srp's `game::runtime_weapon` always sends `weapon_core_state: Some(..)`, 12 bytes for an inactive weapon: 00 | 00 | 0A000000 | 0A000000 | 00 | 00.
- After the Remington's chamber byte, the client therefore reads:

  | Slot | Client reads | Value |
  |---|---|---|
  | `ammo1_weapon2` | shown, hands | 0 |
  | `ammo2_weapon2` | lhs low | 10 |
  | `quick_slot1` (bandages) | lhs high | 0 |
  | `quick_slot2` (traps) | rhs low | **10** |

- The 12 bytes left over are skipped because the messages are length-framed, so nothing crashes.
- Ways to confirm this in the client:
  - The bandage slot shows no count and is greyed.
  - The Remington has 0 spare rounds on the client, while srp thinks it has 5.

## Recommended srp change

In `servers/match-server/src/game.rs` `runtime_weapon`:

```rust
weapon_core_state: is_active.then(|| weapon_core_state { .. }),
```

Send the block only for the weapon that `insert` activates. `active_slot` already is that weapon (see the `SpawnState` doc). This also fixes `live_spawn_message`, which builds weapons with the same function. Update any test that expects `Some` for both weapons.

**Still to check live:** a respawn after dying with weapon 2 in hand. It is not yet known whether death or `insert`'s `action(7)` deactivates weapon 2. If weapon 2 keeps its state, it reads the block, and srp would need to track it.
