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
| 21 | 0x200000 | use |
| 22 | 0x400000 | drop |
| 23 | 0x800000 | throw grenade |
| 25 / 26 | 0x2000000 / 0x4000000 | next / previous weapon: **never** (below) |
| 27 | 0x8000000 | melee |
| 28 | 0x10000000 | spot (key down, 3 s cooldown) |
| 24, 29, 30 | | unused |
| 31 | | a per-input flag the reader splits off |

PROVEN from the builder (0x241250) and the action table (0xA74AF0); the low
bits were checked against masks in real worlds. Whether a switch bit acts on
its level or on its edge is a GUESS: the bot sends key-down actions as one
tick pulses.

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
block has alive, pitch, yaw and the last mask (PROVEN, `players.rs`); the
position is being traced (LIKELY: a float triple near the end of a living
player's state). Between worlds the position can be carried forward from
the inputs (walk about 2.8 m/s, sprint about 4.5 m/s; LIKELY).

## 5. Map knowledge

There is no navigation data in the archive (the engine's loader is never
called; PROVEN). Each map project's `client_project` has the respawn
points, the battery spawners and stations, resupply and damage zones, and
the placed collision objects; the meshes (walker and bullet) are in the
models' `collision` / `walker_collision` entries. A navigation grid would be
built offline from the walker mesh.

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
   straight to the relay.
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
