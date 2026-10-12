# Late joins and world resyncs

What the stock client (0.100b) does with the messages that put players into
a client's world after its startup, and what srp sends. Addresses are
`survarium.exe`'s (symbols from its PDB); the vostok reconstruction was
checked against them.

## Who a client knows

A client learns its players only at startup: `match_options` (0x81) gives
`players_count`, then one `player_profile` (0x92) each. When all have come
it calls `network_client::query_players` (0x5C66D0) and numbers the players
by their place in that list. No message adds a player later, and every
per-player handler looks its player up without a check (`get_player` then
a dereference: 0x84 at 0x5C5C1B, 0x9C, 0x9E, the trap messages). So a
player who joins later stays invisible to earlier clients: srp keeps each
client's list (`Peer.roster`) and `ServerGameMessageKind::for_client`
renumbers every message for it and drops those about players it doesn't
have.

A newcomer sends `team_bases_initialize_info` (0x48) and then `join_match`
(0x42) once loaded (`on_players_ready`); srp shows it the other players on
0x48. A fresh client sends its first `time_synchronization_request` (0x45)
only once its own player is in the world; one that already played a match
in the same game process sends them every 4 s from the start.

## 0x84 `spawn_player`: the player as it is

`process_player_respawn` (0x5C5BF0) reads a u8 player id and calls
`player::deserialize` (0x5E52C0):

| Field | Type | Client |
|---|---|---|
| position | float3 | feet; `set_character_transform` |
| orientation | f32 | yaw (`create_rotation_y`) |
| look_pitch | f32 | |
| is_alive | u8 | `insert( is_alive )` |
| current active slot | u8 | `profile_slot_enum`; activated if it differs |
| target active slot | u8 | `profile_slot_enum` |
| stamina | f32 value, u32 last spending, u32 last tick, u8 lower threshold reached | `player_stamina::deserialize` (0x5AB030) |
| inventory | per item | `inventory::deserialize` (0x7004A0) |

The current active slot is compared with the item the player holds after
`insert` (`+0x40`, 0x5E5386); if it differs, `inventory::action` activates
it there and then (0x5E5394), which gives that weapon a logic state. Its
`weapon_core::deserialize` then reads a logic-state block (u8 at `+0x487`,
the hand IK, a u8 state index, the state's own body, the animation
selector; 0x5A4627 on) that no spawn carries, and with a garbage index
starts a null state (`set_initial_state`, 0x5A46C4): the client crashes.
So a spawn always names the slot `insert` leaves in hand (the first
weapon), and the weapon actually in hand as the target slot (`+0x44`, set
without activating), which the player switches to on its next tick.

Before reading the slots it takes the player out if it is in the world
(`remove` at 0x5E5315) and puts it back (`insert` at 0x5E533C).
`player::insert` gives a life from the client's copy of the profile
(`setup_from_profile`) and, alive, `insert_alive` resets the damage model
(0x5E3137) and the stamina (0x5E3142); the message then sets the amounts
in hand and the stamina. `player::remove` puts what the life left back in
that copy (`unload_to_profile`) and takes the player's placed traps out.

The inventory part goes over the profile's slots from the first weapon on;
the armour slots and the lifebone artefacts read nothing. A weapon
(`weapon_core::deserialize`, 0x5A4450): u16 condition, u32 `random32`
seed, u32 `normal_random` seed, u8 target, u32 old action mask, u16
magazine, u16 bullets in queue, u8 fire mode, u8 ammo slot, a u8 chamber
flag for weapons with one, then (only with a logic state: the weapon
`insert` activated, 0x5A4627 / 0x5E4DD5) u8 shown, u8 hands,
two u32 transition times, the sound state id (+ interval) and the sprint
state id (+ interval). The seeds set the generators outright, so the next
shot draws from them. Any other item: a u16 amount.

**srp.** A respawn sends the respawn point, full stamina, the life's
load and the life's seeds, as before. A player shown to a newcomer
(`live_spawn_message`) is sent as it is: its last update's position, yaw
and pitch (the spawn point until it moves), the weapon in hand or being
switched to, each weapon's magazine, chamber, current ammo slot, fire
mode and the generators' current states, the ammo carried, the drugs and
traps left, and its stamina. Weapons are shown idle, as at a spawn. A dead
player is not shown: `insert( false )` would stand it up somewhere, and
its own spawn comes at its respawn. Its traps are not shown either until
then (its respawn takes them away on both sides).

The stamina times are client clock times, and the client ticks every
player's stamina on its own clock (`player::tick`), so they are moved from
the player's clock to the newcomer's. srp knows a client's clock from its
updates (the mapping lag compensation already keeps) or, before its first
update, from its time sync requests. Without the newcomer's clock (a fresh
client's 0x48 comes before its first 0x45), the spawn's times (10 ms) go
instead; the first tick then regenerates to full.

## 0x9E `damage_model_state`

`damage_model_state_arrived` (0x5C48C0) reads a u8 player id and calls
`damage_model::deserialize` (0x6FFC10), which runs
`body_part_parameters::deserialize` (0x597B90) over the model's parts in
order. There is no count: the client reads one entry per part of its own
model.

| Field | Type |
|---|---|
| health | f32, absolute |
| last hit time | u32, ms on the receiving client's clock; 0 = never |
| affect count | u8 |
| per affect | u8 `hit_affects_type_enum`, u32 ms (`deserialize_affect`) |

Each affect goes through `apply_affect_by_force( type, applying, time )`
(0x597450), which skips one already applied and otherwise stores the
expiry `affects_durations[ type ] * 1000 + time` (durations at 0x9BCD10:
10, 10, 10, 100, 10, 120, 10, 10, 10 s). The time is therefore when the
affect started. Affects already applied and not listed stay; the message
is meant to follow a spawn, which cleared them.

The server side is linked in but never called:
`body_part_parameters::serialize( packet, client_offset )` (0x5971F0)
writes the health, `last_hit ? last_hit - client_offset : 0`, the count and
per affect `serialize_affect` (0xC9FF0): the type and `expiry -
client_offset`. Identical code folding names the u8 and u32 appends
`append(bool)` and `append(float)`; the reconstruction's u32 count is
wrong. The original server sent the expiry where the reader takes the
start, so its affects would have lasted twice as long; srp sends the start.

The parts are `fill_damage_model`'s: the `hit_params` entries in config
order (`add_body_part` pushes back). The only one in the game data is
`gameplay/hit_params/human_hit_params.options`, 15 parts: body, back,
head, face, right_leg, left_leg, right_foot, left_foot, right_arm,
left_arm, right_hand, left_hand, pain, infection, radiation. srp's model
loads the same file in the same order.

Every alive player's model is ticked on the receiving client's clock
(`player::tick` → `damage_model::tick`), regenerating each part once
`regeneration_timeout` has passed since its last hit. Only the local
player's model applies affects (`player_cook` gives it
`type_apply_directly`, the others `type_read_only`, 0x5DDCDE): on a copy
of another player, hits apply none (`hit_by_type`) and none ever runs out
(`regenerate` calls `update_affects` only when applying directly). Such a
copy changes affects only by `affect_damage_model` (0x8A).

**srp.** Right after each player's spawn to a newcomer, its model (times
moved to the newcomer's clock). Other players' clients get only the
affects srp announces with 0x8A (hand and leg damage), since nothing would
ever take another kind away there; the player's own client (in a resync)
gets all of them. While the newcomer's clock is unknown the models are
owed, and go out, as they are then, with its first time sync request.

## 0x9D `world_synchronization_request` / 0x4A confirmation

`on_world_sync_request` (0x5C5D60), no body:

1. empties its queue of updates to send;
2. for player ids 0–19 in the world: `player::remove` and, if it carries
   a battery, `set_victory_item( NULL )`;
3. empties every battery base and picks up every battery on the ground;
4. enqueues `world_synchronization_confirmation` (0x4A, no body,
   0x5C5F16).

It keeps the roster, the scores and the match status. A player taken out
is not ticked, so a client sends no updates of its own until its player
is put back.

**srp.** On 0x4A (`resend_world`): a spawn per living player the client
has, itself included (the late joiner's content), the battery state
(0x94, which the client places without clearing first, safe after a
clear), the traps of the players put back (0x9C), and their damage models
(0x9E). Its own spawn makes its client restart the player's weapons idle
from the spawn, so srp restarts its replay of them the same way
(`Arsenal::respawned`: same magazine, ammo, generators, fire mode; recoil
and stamina carried); a defuse in progress starts over.

From 0x9D to 0x4A nothing about the world goes to that client (`deliver`,
`is_world_state`): moves, spawns, hits, affects, batteries and traps would
be handled against players it took out (a trap placed for a player not in
the world would be inserted twice by the resend). Messages queued before
0x9D still arrive first. Kills still go (`process_player_kill` checks
that the victim is in the world and alive, and adds the kill feed line),
and so do scores, timers and the connected bitmask.

**Trigger.** The original server's reason to send 0x9D is unknown. srp's
transport is reliable and ordered: a client that stalls gets every
message after it, in order, and loses no state, so there is nothing to
repair automatically. Only the console asks: `resync [MATCH]` (srp's
terminal, or the standalone match-server's).

## Known gaps

- The console `resync` crashes the stock client when a player it took out
  had a weapon in hand: `player::remove` leaves the weapon's logic state,
  so the spawn that puts it back reads the block above. Not used
  automatically: a client that stopped reading (alt-tab) instead loses the
  moves and timers queued for it (`catch_up_stalled_peers`).

- A late joiner never sees players who joined after it.
- Weapons are shown idle: a player mid-reload or mid-burst is shown idle
  with its magazine as it is (on its own client after a resync too, and
  the replay restarts alike).
- After a resync, the player's own drug effects in progress (protectors)
  end on its client, which removed the item; srp keeps them, so its
  client's health shown may differ until the next spawn.
- The client's copy of a profile's stacks drifts in a resync (its remove
  and insert move a life back and forth); every spawn sets the amounts in
  hand, so nothing shown depends on it.
