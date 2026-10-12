# 0.21d mode 3: batteries that never appear

Mode 3 (`match_time_vi_spawn`, `match_start_spec.md` §game modes) should put a
battery into the world about every 30 s (± 10 s) until all of them are out.
The 0.21d client never keeps one. Tags as elsewhere: **[V]** from the exe or
data, **[I]** inferred, **[L]** seen live.

## What srp saw [L]

Three mode 3 matches (level_04 twice, level_06): every uploaded snapshot, from
50 ms to the end, has `m_available_victory_items_count` 0. The rule's other
fields are right: start 50, spawn time 30000, variation 10000.

## The rule [V]

`match_time_vi_spawn_rule` (vtable 0x820D2C) is `gather_victory_items_rule`
with its own tick (slot 7), on_match_ready, serialize and deserialize.

| offset | field | serialized |
|---|---|---|
| +110/+114 | the batteries (`victory_item_core*`), begin/end | first A only |
| +120 | the spawner of each battery (position at +0, angle at +10) | — |
| +140 | LCG (`* 0x8088405 + 1`), also the spawner seed | yes |
| +160 | `m_match_start_time_ms` | yes |
| +164 | the time of the next battery | **no** |
| +168 | `m_available_victory_items_count` = A | yes |
| +16C / +170 | spawn time, variation (ms; config s × 1000) | yes |

- `on_match_ready` 0x1FC6D8: start = t, the two times from the first battery's
  `victory_items` config (+1CC, +1D0), +164 = t + spawn time.
- `tick(u32, t)` 0x1FC388, once a tick for every rule
  (`game_world_core::atomic_tick` 0x1CC04F):
  1. nothing once A = the number of batteries;
  2. at each t with `(variation + t − start) mod spawn_time = 0` (start −
     variation + k·spawn_time) the LCG steps and +164 = t + hi(random ×
     2·variation): the k-th battery's time, start + k·spawn_time ± variation;
  3. when t ≥ +164: battery A is inserted at spawner A
     (`victory_item::insert` 0x186301: physics and render model), A += 1,
     **+164 += spawn_time**.
- serialize 0x1FC42F / deserialize 0x1FC5AA: the fields above; the items mask
  and the items' own state cover only the first A batteries.
- Battery state (`victory_item_core` serialize 0x1E7A74): +1B4
  `m_spotted_mask`, +8 `m_user` (the carrier), +1B8 the container, then the
  position. `+1D4` = inserted. `remove` (vtable +0x94) does nothing when it
  is not inserted; `set_transform` is +0x98.
- The HUD (`fill_match_hud_state` 0x1FBDF4) marks every battery whose
  spotted mask has the player's team, whatever A is. There is no "a battery
  appeared" message: a new battery shows only as its HUD marker once spotted.

## Why none stays [V + I]

The client simulates up to its own clock and rolls back whenever an input
arrives for a tick it has already simulated (`game_world_core::move`
0x1CCCA3: nearest state ≤ C7FC, deserialize, re-tick), which with another
player is nearly every tick.

- A battery placed at T is followed by a rollback to before T. The restored
  state has A − 1, but +164, not part of the state, is already T +
  spawn_time, so the re-simulation passes T without placing it. Until the
  next re-roll sets a new time, and the same happens again.
- The placed battery itself is not taken back: deserialize skips items past
  A, so it stays in the physics world and on the renderer with the rule
  counting it unplaced.
- A model of both (`client-patches/spawn_model.py`: the tick as above, a rollback
  of 50–500 ms after 80 % of ticks) keeps 0–1 of 5 batteries a match, as seen.

The bug is the client's own; srp never sees +164. Mode 3 has no name in the
localization and the lobby offers no mode choice, so it most likely never
shipped in 0.21d [I].

## The patch (`client-patches/battery_spawn.asm`)

1. **A new tick in place of the old one** (135 of its 167 bytes). It keeps
   the re-roll and its draw, and derives the rest from serialized fields:
   k = (variation + t − start) div spawn_time re-rolls have happened; battery
   A is due when A < k and t ≥ the last re-roll + hi(random × 2·variation).
   Without rollbacks this places the same batteries at the same times as the
   original (the model checks 2000 seeds); with them, a re-simulation places
   them again. A battery still inserted is moved (+0x98), not inserted twice.
2. **A hook in deserialize** (0x1FC663, 9 bytes, into 75 bytes at 0x6A1F90,
   the end of the padding after .text): after the rule's fields, every
   battery from A on is removed and its user, spotted mask and container are
   cleared, as deserialize does for "none". A rollback thus leaves exactly the
   batteries its state has.

The snapshot format and hash are unchanged. All clients in a mode 3 match must
run the patched exe: a stock client keeps no battery and would disagree with
the others as soon as one appears.

Status: built and checked with `ndisasm` [V]; not yet run in a match.
