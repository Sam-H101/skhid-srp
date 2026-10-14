# Player bots driven by the server (0.26g)

Status 2026-10-14: **planned**. Stage 0 (a test bot that sends inputs,
`srp-bot --drive`) is built and not yet run against a real client. Tags:
PROVEN (code, bytes or data read), LIKELY (strong inference or a few real
samples), GUESS.

## 1. Why no client change is needed

In lockstep every client simulates every player from the inputs the match
server relays (0xEB). A relayed input is applied unchecked, as a real
player's is (0x1FAB50 → 0x2D86E0; `match.md` §20). PROVEN. So a place whose
inputs the server makes up walks, turns and fires in every client's world.

The clients' respawn rule spawns an entered player with no input, in the
lowest loaded kit, and again 8 s after a death (`match.md` §19). PROVEN. A
bot never sends a kit choice (0xDE).

## 2. An input

0xDC from a client, 0xEB to the others: tick (`time / 50 mod 256`), place,
two IEEE half floats (rotation), u32 `actions_mask` (`match_msgs.rs`).

| bit | mask | action |
|---|---|---|
| 0-3 | 0x1 / 0x2 / 0x4 / 0x8 | forward / back / strafe left / strafe right |
| 4 | 0x10 | jump |
| 5 | 0x20 | fire |
| 6 | 0x40 | reload |
| 7 | 0x80 | aim (or aim toggle) |
| 8 | 0x100 | crouch (or toggle) |
| 9 | 0x200 | sprint (or toggle) |
| 10 / 11 | 0x400 / 0x800 | next fire mode / next ammo (key down) |
| 12 / 13 | 0x1000 / 0x2000 | weapon 1 (slot 7) / weapon 2 (slot 10) |
| 14-19 | 0x4000 .. 0x80000 | quick use 1..6 |
| 20 | 0x100000 | hold breath |
| 21 | 0x200000 | use: acts on the key going down only, if no use is running |
| 22 | 0x400000 | drop: acts only while carrying a battery |
| 23 | 0x800000 | throw grenade: held until the throw starts |
| 25 / 26 | 0x2000000 / 0x4000000 | next / previous weapon: **never** (below) |
| 27 | 0x8000000 | melee |
| 28 | 0x10000000 | spot (key down, 3 s cooldown) |
| 24 | 0x1000000 | never set by the game, yet tested (0x2D8BC1, on a path naming "brain"): **never** send it |
| 29, 30 | | unused |
| 31 | | a per-input flag the reader splits off |

PROVEN from the builder (0x241250) and the action table (0xA74AF0); the low
bits were checked against masks in real worlds. The consumer is
`base_player` 0x2D86E0:
- **Use** (0x2D88C3..0x2D88DF) starts on the edge, 0 then 1; holding 1 starts
  nothing new. PROVEN.
- **Grenade** (0x2D8E05..0x2D8E6E): pressing marks the grenade slot as
  wanted (0x309DC0), releasing clears it; the throw starts once the weapon is
  in state 6 (0x2E4090). So a bot holds bit 23 until the throw begins. PROVEN;
  the shortest hold that throws is not known.
- **Quick use 1..6** (0x2D8D7D..0x2D8DFF): key down calls 0x2FCAA0(slot, 1),
  key up (slot, 0); the slot is the n-th filled one in the order 13..18, 3,
  0, 1, 2, 4, 5, 6, then pocket items. PROVEN.
- **Drop** (0x2D8E73..0x2D8EC8): bit 22, or a new fire press, lets go of a
  carried battery. PROVEN that it runs; "a carrier who fires drops it" is
  LIKELY, not seen in a game.

Whether the other switch bits act on their level or their edge is a GUESS:
the bot sends key-down actions as one-tick pulses.

Batteries (code): taking one from the ground on use is instant (0x3007E0),
and so are putting one in and stealing from a container (0x31CC40); the
battery config's `take_time: 0.6` is LIKELY the hands-up animation before it
is in hand. Only resupply zones (`resupply_time_in_ms`), artefact
containers (`artefacts_search_time_sec`) and defusing a trap need use held.
A carrier's speed is ×0.95 (`movement_speed_modifier: -0.05`, read at
0x30016D; speed = max(0, 1 + sum of modifiers), 0x2D5AE0): 3.325 m/s
walking. The worlds agree: carriers' horizontal speed piles up at 3.32–3.33
m/s, and of 20 carriers seen with the sprint key and a move key down most
stay at 3.32–3.4 (LIKELY a carrier cannot sprint; the few at 4.5–4.75 m/s
are not explained).

Rotation: the first half is a change of yaw, the second of pitch, in
radians, made once at the input's tick (LIKELY: applied at 0x2D86E0, yaw
`+= x` at 0x2D8AD0, the builder zeroes it every frame 0x2410D1). Forward
moves along (−sin yaw, −cos yaw) in x/z with y up (LIKELY, from worlds).
Which way a positive yaw turns is to be seen in stage 0.

## 3. What a bot must never do (every client would crash or diverge)

- A kit with no weapon in slots 7 / 10: every client crashes at the spawn
  (0x1F25BF). PROVEN.
- Bits 25 / 26 while holding something other than the two weapons: every
  client reads past the inventory (0x2D8D0A). PROVEN. Use bits 12 / 13.
- A rotation that is no number or huge stays in every world; the server
  refuses those already (`PlayerInput::rotation_is_sane`). Keep each turn
  under π, one input a tick.
- An input that reaches a client after its horizon (500 ms) is moved and
  that client's world diverges: send for a tick 50-100 ms ahead of the
  clock, never behind it; more than 2.5 s ahead is pulled back.
- Two inputs for one tick are merged with their rotations added: one a
  tick, on the 50 ms grid (never the precise 0xDD).

## 4. What the server knows

All inputs at once (kept 60 s), and the worlds the patched clients upload
(about 0.5 s old, every 3-5 s today; `snapshot_rules.md` §7). Each player's
block head has alive, pitch, yaw and the last mask (PROVEN, `players.rs`).

**Position (PROVEN, 2026-10-14):** `base_player::serialize` (0x2DAF60)
writes `bullet_character_controller::serialize` (0x7F7F0, 67 bytes) for a
living player only (the skip at 0x2DB303). It starts `84 + 8n` bytes before
the end of the player's state, `n` being the recent hits counted just
before the end (`u32 n`, then `n × {u32 time, f32 speed}`, then one byte).
Its first 12 bytes are `m_last_pos` in physics space: the map frame is
`(x, y, -z)`, the capsule's centre, 0.9 m above the feet standing (checked
against the walker mesh of level_05: 0.90 ± 0.03 m at 9 of 12 standing
samples). Then the jumping and crouch flags, normals, and at +66 sprinting.
Earlier in the tail: the horizontal speed (about 3.1-3.6 walking, 5.2-5.3
sprinting; unit LIKELY m/s), the spawn time, and per body part a health
(part 0: 1.0 unhurt; LIKELY the player's health). Decoded on the four
worlds of a live match, every player read with no ambiguity. Not yet seen:
a crouched sample, the sign of A/D. `players.rs`'s "No 0.26g snapshot was
seen yet" is stale.

Between worlds the position can be carried forward from the inputs (walk
about 2.8 m/s, sprint about 4.5 m/s measured from worlds; LIKELY).

## 5. Map knowledge

There is no navigation data in the archive (the engine's loader is never
called; PROVEN). Each map project's `client_project` has the respawn
points, the battery spawners, containers and stations, resupply and damage
zones, and the placed collision objects (`collision_objects[]`: library
name, position, rotation, scale, group and mask); the meshes are in the
models' `collision` / `walker_collision` entries (vertices, triangles, a
u16 per face whose top 2 bits are the walker behaviour: 0 ground up to
60°, 1 always ground, 2 never ground). Many walker meshes are hard links in
the archive's index: a loader must follow them.

- The walking body (group 4, mask 2) meets objects with `cgroup & 2`;
  bullets (16, 8) meet `cgroup & 8`. Walls come from the walker mesh; the
  bullet mesh is for line of sight only. PROVEN.
- Placement: world point = (local point × the `#[sx][sy][sz]` scale in the
  library name) × `Rx·Ry·Rz`(rotation) + position, row vectors; the
  object's own `scale` field is not applied again. All 43 spawns of
  level_01/day_4 land within -0.04..+0.12 m of a walking surface this way.
  (The 0.100b srp's `combat/collision.rs` applies the scale twice.)
- Body: capsule 0.76 m wide, 1.8 m standing, 1.1 m crouched; walk 3.5,
  sprint 5.5 m/s (`default.player`); step 0.6 m (PROVEN). Jump height:
  GUESS.
- A walk grid from the walker mesh (Recast-style, 0.25-0.5 m cells) builds
  in 0.2-1.4 s per map; a ray tree on the bullet mesh in 0.2 s, 1-2 µs a
  ray. On level_01/day_4 all 43 spawns connect with jumps; on
  level_05/evening_2 the spawn areas are left by drops and never re-entered
  (players' word: spawns can be one-way, with nothing to go back for).
- Built: the `mapdata` crate (dump per project, walk grid, paths, line of
  sight), with its file format, API and timings on all 72 projects:
  `player_bots_mapdata.md`.

## 6. Stages

0. **A test bot that moves** (`srp::drive`, `srp-bot --drive`): no server
   change. After its entry it waits 2 s, then walks 3 s, turns +π/2 once,
   walks 2 s, sprints 3 s, walks crouched 3 s, fires 1 s, reloads, takes
   weapon 2, turns back, takes weapon 1; again after 5 s. Each input is
   printed with the sum of the turns sent, which the bot player's yaw in
   the worlds must equal. To check with one real game: it walks, turns a
   quarter, fires; no "rotation" warning, no resync, no input of the bot
   dropped (0xF1) or moved (0xEF) in its lines; positions and yaw in the
   dumped worlds (`SRP_SNAPSHOT_DIR`) as predicted.
1. Bot places made by the server (no account, no session, never timed out,
   never a voter), entered once a real client is live; the inputs go
   straight to the relay. **Built 2026-10-14, not run with a real game:**
   - `SRP_SERVER_BOTS=<n>` (lobby, default 0, testing only) puts `n` bots
     into each match made, after its players, each on the side with fewer
     places (`play::server_bot`): name `Bot-<n>`, clan tag `BOT`, the
     starter kit (TT-33 in slot 7), session and account 0, no seat.
   - `MatchPlayer::driven` (match server): `game/driven.rs` enters the
     place once a real client simulates (0xE9 300 ms ahead) and relays the
     stage 0 script's inputs (`v026g::drive`, moved there from `srp`) as a
     player's, kept in the history for clients that start later. No
     witness of the world (`is_bot`); a match left to bots ends as one
     left by everyone.
   - The queue's roster (`Matchmaking::add_bots`) has the bots' seats, so
     a late joiner's place counts from past them; the rewards move no
     rating in a match with one and keep nothing for account 0.
   - Tests: `match-server` `tests/driven.rs`, lobby
     `the_servers_bots_take_the_places_after_the_players`, rewards
     `a_match_with_a_server_bot_moves_no_rating_and_leaves_nothing_for_account_0`.
   - Not yet: a bot leaving to make room for a late joiner; any choice of
     what to do (the script is fixed).
2. Positions from the worlds, worlds more often in matches with bots, a
   simple state machine (go, turn, shoot at short range, reload).
3. Map data on the server: collision, line of sight, a navigation grid and
   paths; objectives of the battery modes.
4. Difficulty, aim error, cover, grenades; bots fill short queues and give
   their place up to late joiners.

Bots and late joiners: a roster place is never reused, so a joiner cannot
take the bot's own place; the bot leaves (0xEA) and the joiner comes on a
new place. Every place ever used counts toward the 20 of a match, so bots
should fill a match to a modest size.

## 7. Behaviour

What bots should do in a match, built on this data (senses, plays such as
battery ambushes, carrier interception and sneaking into an empty base,
reactions to events such as a stolen battery): `player_bots_behaviour.md`.
