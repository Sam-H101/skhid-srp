# Player bots: behaviour design (0.26g)

What server-driven bots (`player_bots.md`) should do in a match, built on the data the server really has. A design, nothing of it is built yet. Tags as elsewhere: PROVEN, LIKELY, GUESS; "players' word" is what people who play the game told us.

## 1. Game facts this rests on

Players' word (2026-10-14):
- Picking up a battery takes time.
- A battery carrier is slower and cannot shoot.
- The HUD shows only marked batteries and the batteries last picked up by the player's own team.
- A battery the team picked up and did not deliver, when stolen, just disappears from the minimap: the thief is not shown.
- Spawn areas can be one-way (left by drops), and there is nothing in them to go back for.

From the code and data (`player_bots.md` §4, §5):
- **Inputs:** every player's reach the server at once: keys (walk, sprint, crouch, aim, fire, reload, weapon switch) and turning. PROVEN.
- **Worlds, every ~5 s:**
  - alive, pitch, yaw and position (PROVEN);
  - crouch, jump and sprint flags, horizontal speed and recent hits (PROVEN);
  - health (LIKELY).
- **Statistics in the worlds:** kills, deaths, streaks, battery bring (event 24) and steal (event 26) counts. PROVEN.
- **Loadouts:** each player's weapons and armour, from the match's start info.
- **Map:** spawns, battery spawners, containers, stations, resupply zones, the walker mesh and the bullet mesh (`player_bots.md` §5).

Unknown, to read from match recordings (`SRP_RECORD_DIR`):
- the "use" key bit: a pickup would then be seen the moment it starts;
- the grenade throw bit: an unknown key bit followed by that player's grenade count dropping in the next world;
- the real carrier speed;
- the pickup time.

## 2. What a bot may know

The server knows everything; a bot that used it all would play like a wallhack. Every enemy fact passes one of these senses, and nothing else is used:

| Sense | Rule |
|---|---|
| Seen | Line of sight on the bullet mesh, inside a ~90° view, after a noticing delay of 0.2–0.5 s (longer far away, shorter if the enemy moves or fires) |
| Heard | Shot ~60 m, sprint ~15 m, reload or weapon switch ~10 m; the position error grows with distance |
| Told | What one bot senses, its bot teammates know ~1 s later (a callout) |
| Inferred | What the HUD and kill feed give a human: deaths and respawn timers, score, match clock, the battery markers (marked ones and the team's own) |
| Forgotten | An enemy not sensed for ~3 s becomes a last known position that fades |

**Batteries:** a bot knows what the HUD shows (marked batteries and its team's). Other batteries must be found by checking the spawner positions; a battery a bot sees becomes known to its team.

**Difficulty:** a setting may loosen the senses (for example "hunter" bots). The default plays fair.

## 3. Plays

Each play has a score from the situation, a precondition and steps. A commander per team hands them out every ~0.5 s: never two bots on one ambush, and one defender per base in the battery modes.

1. **Battery ambush.** An enemy is heading to a battery no friend covers.
   1. Hide 10–25 m from it, with a line of sight to the pickup point and out of sight of his approach. The bot goes only if it gets there first; otherwise it intercepts (play 2).
   2. Crouch, stand still, aim at the pickup point.
   3. Let him finish the pickup: he then cannot shoot back and is slow. Kill him, and take the battery he drops.
   4. Shoot during the pickup only if his teammates are close.
   5. Give up after 30–40 s.
2. **Carrier interception.** An enemy carries a battery: slow, unable to shoot, bound for his container. Meet him at a choke he reaches 1–2 s after the bot; otherwise chase him.
3. **Escort.** Our carrier, bot or human, cannot shoot. One or two bots go along, one ahead and one behind looking back.
4. **Sneak into their base.** Do it when several enemies are dead (respawn timers), last known far away, or heard fighting elsewhere.
   1. Take a route around their last known positions and the routes humans use most.
   2. Crouch-walk the last 20 m.
   3. Steal, then leave by another route.
   4. Abort on contact.
5. **Punish a reload.** An enemy heard reloading or switching weapons in a duel: push or peek in that window.
6. **Turn to a sound.** A shot heard: turn toward the estimated spot, with the error of its distance.
7. **Break sight and re-peek.** After taking hits: leave the line of sight, wait 1–2 s, come back from another angle, never the same one twice in a row.
8. **Read a held angle.** An enemy crouched, aiming and still is holding an angle: flank him or wait.
9. **Crossfire.** Two bots cover one choke from angles at least 60° apart.
10. **Health.** Push a wounded enemy. A wounded bot retreats to resupply by a covered route.
11. **Weapon matchup.** From the loadouts: keep a shotgun at range; close in on a sniper through cover.
12. **Cover a human.** Follow a human teammate at 5–10 m and look where he isn't looking.
13. **The top player.** Normal bots don't duel the best enemy alone; they pair up. An optional hunter squad goes after top players on purpose.
14. **Personalities.**
    - camper: ambushes, holds;
    - rusher: intercepts, pushes;
    - flanker: sneaks, wide routes;
    - support: escorts, crossfires, covering humans.

    The losing team grows more aggressive as the gap grows.

**Never:**
- camp an enemy spawn;
- aim perfectly from the first frame;
- react to anything no sense gave.

Bots never path back into their own spawn area.

## 4. Event reactions

| Event | How bots learn it | Reaction |
|---|---|---|
| A battery the team picked up disappears from the minimap (stolen) | HUD marker gone | An enemy has it near where it last was: that spot is the thief's last known position. The 1–2 nearest bots cut off his way to his container (play 2); his exact place is known only once seen or heard |
| An enemy steals from our container | The steal count (event 26) in a world, up to ~5 s late | Same alarm; the rest keep their posts |
| An enemy picking up at a spawner, seen | Sight; he stands still while picking up | Ambush or interrupt (play 1) |
| Our carrier killed | Kill feed and the battery's marker | The nearest bot goes for the dropped battery, others cover it |
| We killed an enemy carrier | Kill feed and sight | The nearest bot picks it up, one escorts |
| A battery delivered, either side | The bring count (event 24) | All bots re-plan |
| Several enemies dead at once | Kill feed | Push or sneak window (play 4) |
| A teammate killed at a spot | Kill feed | That spot is dangerous for ~20 s: avoid it, or go in to trade |
| An enemy on a kill streak | Statistics | Pair up against him |
| Little time left | Match clock | Losing team all in, winning team defends |

## 5. What match recordings give

- **Danger map per map:** where humans kill and die. Bots pre-aim known camping spots as they pass.
- **Real routes:** where humans approach batteries from (play 1), and which routes are busy (play 4).
- **Real timings:** spawn to each battery, pickup time, carrier speed.
- **Human numbers:** reaction time (an enemy appears, then the first turn toward him and the first shot), burst lengths, how often humans re-peek. These set the bots' parameters.
- **The unknown key bits** (use, grenade).

## 6. Technical needs

- **Event delay:** events reach the server with the worlds, every ~5 s. In matches with bots: a world every 1 s (~8 KB each).
- **Positions between worlds:** carried forward from keys and turning (drift on collisions). A bot's own position is carried forward the same way and corrected at each world.
- **Path distances and visibility:** from a walk grid built from the walker mesh, and a ray tree on the bullet mesh (`player_bots.md` §5). Distance maps per target (enemy base, batteries, resupply) are cheaper than repeated path searches.
