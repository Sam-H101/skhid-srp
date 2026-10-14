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

**Movement (measured 2026-10-14 on 163,369 player rows of 165 saved
matches; private analysis, numbers below):**

| What | Value | How sure |
|---|---|---|
| Forward at yaw `a`, map frame (x, z) | `(-sin a, cos a)`; a positive yaw turns left | PROVEN (median error 1.6° over 4,456 pairs of worlds 50 ms apart; aim at enemies agrees) |
| LEFT / RIGHT | yaw + 90° / yaw − 90°; diagonals ±45° at the forward speed | PROVEN |
| Yaw, pitch | the yaw is never wrapped (seen from −66 to +55 rad); pitch up-positive, ±1 | PROVEN; the clamp LIKELY |
| `m_linear_horizontal_speed` | m/s, equal to the moves between worlds | PROVEN (ratio 0.997-0.999) |
| Walk | 3.5 m/s; each life at ×0.95, ×1 or ×1.05 of it | PROVEN (the cause, kit modifiers, LIKELY) |
| Sprint | about 5.2 m/s (also 4.95-5.0; never the config's 5.5) | PROVEN as seen |
| Strafe, crouch / aimed / aimed strafe / back | 0.75 / 0.9 / 0.6 / 0.85 of walk | PROVEN; back LIKELY |
| Getting to speed | about +0.6 m/s a tick, −0.5 slowing (0 to 3.5 in 0.3 s) | LIKELY |
| A world's keys | not yet in its own position: a tick's keys move the body from the next | LIKELY |
| Respawn | about 7.9 s after death, a median 72 m from it | LIKELY / PROVEN |

The bots' movement model (`v026g::bots::kin`) is this table. Between worlds
a bot's position is carried forward from its own inputs (`bots::body`).

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
     bots' kit, session and account 0, no seat.
   - **The bots' kit** (`survarium::v026g::loadouts::BOT_KIT`: item, slot,
     count, input bit; `bot_kit()`, `pockets()`): the starter kit (TT-33
     in slot 7, bit 12, and a clip in slot 8), `bm_back_1` (slot 3) and
     `bm_legs_5` (slot 4), and in the pockets two `medkit` (slot 13, quick
     use 1, bit 14), the radio `respawn_booster` (slot 14, no key: it
     works carried; default modification 1220, `respawn_time_modifier`
     -0.2) and
     two `rgd-5` (slot 15, bit 23).
     - Pockets: drugs, boosters, traps and grenades go only in slots
       13..18 (the dictionary's `slots_restrictions`), and the client
       throws from the first of them holding a grenade set (0x2FE8B0,
       `grenade_set_core` alone answers `vtable+0x64`). PROVEN. Quick use
       `n` is the `n`-th filled slot of 13..18, 3, 0, 1, 2, 4, 5, 6
       (0x925FD8). PROVEN; that every item but an artefact counts is
       LIKELY (the item's `+0x24C` was not read), so the grenade is the
       last pocket.
     - A player has 1 pocket of 1 unit (`default.player`); gear adds them
       (modification 1030: a pocket, 1170: a unit more in each). The
       lobby opens that many of 13..18 and empties the rest
       (`PaperDollBase.handleQuickSlots`); a `fills_whole_pocket` item
       takes one. LIKELY the client's rule (the exe's sum was not read).
       The backpack (+1 pocket, +1 unit, level 3, 6,650 silver) and the
       trousers (+1 pocket, level 4, 6,830) are the cheapest pair of
       non-premium pieces that open three: 3 pockets of 2. Tested
       (`loadouts` test `the_bots_kit_has_a_pocket_for_each_thing_it_uses`).
     - The radio works **by being carried**; a bot need not press
       anything. PROVEN (client, 2026-10-14): a `survarium::booster`
       (vtable 0xA1CA34, constructor 0x33F690) makes a modifier entry for
       each non-zero one of its 68 parameters. At every spawn
       `base_player::respawn` clears the player's 68 modifier lists
       (0x2DACE4) and the inventory fill 0x2FCB70 (called at 0x2DADBB)
       calls slot 14 of every item in the 23 slots, pockets included
       (0x2FD1A5); a booster's (0x33F960 → 0x33F990) adds its entries to
       the kit's lists. At a death the respawn rule (0x316AA0) sets the
       delay through the getter 0x2D5AE0 (index 0x3C, which sums the
       lists): `respawn_time × max(0, 1 + modifier)`, rounded up to the
       50 ms grid; 8 s × 0.8 = 6.4 s. No use, no count check. LIKELY only:
       that the -0.2 of default modification 1220 is in the booster's
       parameters (the client applies modifications to an item's config,
       `item_config_cook`; the defaults' path there was not traced). Other
       passive adders of the same kind: `armor` (0x33F3A0) and the
       artefacts (0x311AD0..); the medkit's adds (0x33D892, 0x33DDC1) run
       only while it is active.
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
     `the_servers_bots_take_the_places_after_the_players` and
     `a_server_bot_wears_the_bots_kit_and_a_client_reads_it`, rewards
     `a_match_with_a_server_bot_moves_no_rating_and_leaves_nothing_for_account_0`.
   - Not yet: a bot leaving to make room for a late joiner; any choice of
     what to do (the script is fixed); pressing the kit's keys (nothing
     sends bits 14 or 23 yet); a real game with the kit.
2. **Bots that walk the map.** Built 2026-10-14, not run with a real game:
   - `SRP_MAPDATA_DIR` (match server): the folder of `mapdata-dump`'s files
     (`player_bots_mapdata.md`). With it, the server's bots
     (`v026g::bots`) walk; without it, or for a map with no file, they run
     the stage 0 script. The map's walk grid is built on a thread of its
     own (0.3-0.8 s, 60-150 MB) when the match is made; the bots stand
     until it is there.
   - **Where its body is** (`bots::body`): the trusted worlds show it
     (`Game::observe` → `observe_driven`: the controller's position, yaw,
     keys, speed, alive, spawn time). Between worlds it is carried forward
     tick by tick with the bot's own inputs and the movement model above,
     kept on the walk grid (walls stop it, it slides along them); each
     world puts it back and the inputs since are played again. While bots
     walk, the clients are asked for a world every 2 s (1 s while a bot
     waits to see its body: just entered, or dead), not 5 s.
   - **Routes** (`bots::path`): A* on the grid, pulled straight to the few
     real corners; a jump or a drop is a corner of its own.
   - **Smooth movement** (`bots::motor`): the view turns with an eased
     speed (most of a turn in about 0.15 s, at most 5 rad/s, its speed
     changing by at most 45 rad/s²), never past the heading; it walks while
     turning and lets go of forward only for a turn over 80°; it steers at
     a point 1.6 m + 0.35 s ahead on the route (shortened until the line
     there is walkable, so it never cuts a corner into a wall); it sprints
     only with 10 m of straight route ahead, after facing the way for
     0.3 s, and stops 3.5 m before a sharp corner; its heading sways by
     about a degree; standing, it looks about every 1-2 s.
   - **Stuck** (`bots::stuck`): two worlds 1.5 s apart that show the body
     moved under 0.4 m while it walked. First a jump and a step back and
     to a side, and the same route again; caught again at the same spot
     within 20 s, a jump and a sidestep and another goal; a third time, it
     stands a few seconds and goes elsewhere. A spot that caught it twice
     is kept clear of for 45 s (routes past it are not taken, unless every
     route passes it).
   - **What it does** (the one play of this stage): walks to one of the
     map's objectives (a container, a battery's place, a station) or a
     floor 20-60 m off, never within 25 m of the other side's spawns (its
     side: the spawns it first came out of), looks about there 1.5-5 s,
     and picks the next.
   - Each bot's random numbers are its own, seeded by the match and its
     place (`bots::rng`), so a replay with the same worlds repeats it.
   - Tests: `bots` (movement model, encoder, the body's reckoning, routes,
     motor smoothness, stuck), scenario tests against a stand-in game
     that moves the body by the bot's inputs (`bots/tests.rs`: goal to goal
     round a wall, a post the grid lacks), and the match end to end
     (`tests/driven.rs`: a client's worlds show the body, the bot walks
     with eased turns, steps back when the worlds show it not moving, and
     the worlds are asked for every 2 s).
   - Not yet: a real game; what the bot does about other players (it does
     not see, aim or shoot); jumps up ledges are pressed but their reach is
     not checked; the cost of the extra uploads for the clients (two
     worlds every 2 s) is not measured.
3. Senses and combat: seeing and hearing (`player_bots_behaviour.md`), aim
   with error and reaction time, the plays of the battery modes.
4. Difficulty, cover, grenades; bots fill short queues and give their
   place up to late joiners.

Bots and late joiners: a roster place is never reused, so a joiner cannot
take the bot's own place; the bot leaves (0xEA) and the joiner comes on a
new place. Every place ever used counts toward the 20 of a match, so bots
should fill a match to a modest size.

## 7. Behaviour

What bots should do in a match, built on this data (senses, plays such as
battery ambushes, carrier interception and sneaking into an empty base,
reactions to events such as a stolen battery): `player_bots_behaviour.md`.

How to build them (what shipped games did, and a module layout for this
server: layers, utility scoring with rank bands, runtime modifiers from the
match state, self-balancing, tuning files, decision traces and tests), from
web research with its sources: `player_bots_research.md`.
