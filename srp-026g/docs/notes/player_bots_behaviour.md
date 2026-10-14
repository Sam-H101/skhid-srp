# Player bots: behaviour design (0.26g)

What server-driven bots (`player_bots.md`) should do in a match, built on the data the server really has. How to build it: `player_bots_research.md`. A design, nothing of it is built yet. Tags as elsewhere: PROVEN, LIKELY, GUESS; "players' word" is what people who play the game told us.

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

From the client code since (`player_bots.md` §2):
- **Use** is bit 21, acting on the key going down: a pickup is seen in the inputs the moment it starts. PROVEN.
- **Grenade** is bit 23, held until the throw starts. PROVEN.
- **Pickup:** taking a battery is instant in code, with a LIKELY 0.6 s hands-up animation (`take_time`). Players feel it takes time; the bots plan with ~0.6 s and the recordings will say more.
- **Carrier speed:** ×0.95, 3.325 m/s walking (PROVEN in code); the saved worlds show carriers at 3.32–3.33 m/s, LIKELY unable to sprint.

Still to read from match recordings (`SRP_RECORD_DIR`): how long humans really stand at a battery, and how they throw grenades.

## 2. What a bot may know

The server knows everything; a bot that used it all would play like a wallhack. Every enemy fact passes one of these senses; the only exceptions are the small cheats listed below.

| Sense | Rule |
|---|---|
| Seen | Line of sight on the bullet mesh, inside a ~90° view, after a noticing delay of 0.2–0.5 s (longer far away, shorter if the enemy moves or fires) |
| Heard | Shot ~60 m, sprint ~15 m, reload or weapon switch ~10 m; the position error grows with distance |
| Told | What one bot senses, its bot teammates know ~1 s later (a callout) |
| Inferred | What the HUD and kill feed give a human: deaths and respawn timers, score, match clock, the battery markers (marked ones and the team's own) |
| Forgotten | An enemy not sensed for ~3 s becomes a last known position that fades |

**Batteries:** a fair bot knows what the HUD shows (marked batteries and its team's) and finds the others by checking the spawner positions; a battery a bot sees becomes known to its team. By default bots also know every battery (below).

**A little cheating** (players' word, 2026-10-14: "a bit, not too much"). The plays work better with a few things a human can't know, kept small and never in the aim:
- **Where batteries lie:** every battery's spawner and state, not only the HUD's. Saves the search, changes no fight.
- **The thief:** when a battery of ours is stolen, his position with ~10 m of error, refreshed every few seconds until he is seen or delivers.
- **A rough radar:** the nearest enemy's position with ~15 m of error every ~5 s while none is sensed (a human's map sense). It steers routes and where to look, never a shot.
- **Senses a notch better:** hearing ranges ×1.25, last known positions kept ~5 s instead of ~3 s.

Still never:
- shooting at, aiming at or pre-tracking an enemy who isn't seen;
- reacting faster than the noticing delay;
- perfect recoil and spread.

**Difficulty:** a setting may loosen or tighten this (for example "hunter" bots with a sharper radar, or "fair" bots with none). The default is the small set above.

## 3. Plays

Each play has a score from the situation, a precondition and steps. A commander per team hands them out every ~0.5 s: never two bots on one ambush, and no bot parked at its own container. In the saved worlds lives that stay in their own third are the worst in every mode (K/D 0.3–0.6, ~0.03 batteries a life), so a steal is answered by the 1–2 nearest bots (the thief cheat and the alarms in §4); the only guard is play 18.

1. **Battery ambush.** An enemy is heading to a battery no friend covers.
   1. Hide 10–25 m from it, with a line of sight to the pickup point and out of sight of his approach. The bot goes only if it gets there first; otherwise it intercepts (play 2). Saved worlds: the killers of players who died within 6 m of a battery spawner stood 10 / 17 / 27 m from it (quartiles), 45 % of them 10–25 m away.
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

    The mix follows how humans play (saved worlds, per life): about 55 % move steadily, 30 % mostly sprint, 10–15 % hold still. So a team of bots is mostly flankers and support, a third rushers, few campers. The losing team grows more aggressive as the gap grows.
15. **Hold the exits.** Hold forward chokes on the routes out of the enemy's base, as humans do: in the saved worlds the best kill spots of each side are mostly 30–60 m from the enemy base, and the enemy third of the map gives the best K/D (×1.3 overall, ×1.6 in Team Deathmatch). Owner's rule (2026-10-14): near the exits yes, **never inside a spawn area**.
    - **Team Deathmatch only, as a long hold.** In the battery modes the lives that reach the enemy third and keep moving bring the most (0.41 batteries a life, 0.60 in timed batteries), holding there brings 0.26 (0.00 in timed batteries): there a bot holds only briefly on its way.
    - **Never in the middle:** holding in the middle is poor in every mode (K/D 0.5–0.66).
    - **High ground, mid range:** among the allowed chokes, prefer spots above the exit with sight lines of ~15–30 m (the killer stood >2 m higher in 23 % of kills, lower in 14 %; median kill distance 19 m), not the longest open view.
    - **Move on:** after 1–2 kills or ~40 s, go to another allowed spot, never straight back to the last one; humans learn a spot fast.

16. **Late denial (battery modes).** Players' word: before the match ends, take batteries out of the enemy's container and just drop them there (no need to carry them home): the enemy's count falls (the statistics take one off the other team when a battery is taken out, `snapshot.rs` `StatisticsTail`), so a narrow lead turns. A bot does it in the last minutes when the score is close or behind and the enemy's container is reachable: steal (use, bit 21), then drop at once (bit 22), then steal the next one or fight off the defenders. Note: dropped batteries can be taken back, so it pays only near the end.
17. **Routes from humans.** For each map, mode and side, the paths humans took (saved worlds: each life's sideways offset at checkpoints along the base-to-base line, grouped) and what they gave (kills, assists, batteries found and put in). On most sides one or two routes clearly pay more (level_01 batteries, one side: K/D 2.9 on one route, 0.8 on another). Bots pick routes weighted by that reward, not always the best one, so they stay hard to read. What a bot knows then shifts the weights, and is checked again on the way (each ~0.5 s with the commander):
   - an enemy sensed, on the radar or last known on a route: that route weighs less, or more for a bot sent to fight (rusher, hunter);
   - a known battery, a thief's position or a carrier to escort: the route toward it weighs more, whatever humans did there;
   - a teammate killed on a route in the last ~20 s: it weighs less;
   - teammates already on a route: it weighs less, so the team spreads (one or two per route).
   A bot whose route turns bad on the way picks again from where it stands.

   **Sprinting on a route:** sprint out of spawn and behind the team's own lines; walk once within ~30 m of a known danger (an enemy sensed, last known or on the radar, a danger-map spot, a teammate's death). Lives that were mostly sprinting did far worse than steady movers reaching as far (K/D 0.2–1.0 against 0.6–2.3), but that is LIKELY skewed by short lives being mostly the run out of spawn, and sprinting right before a death was no commoner than before a kill. A cheap rule of thumb, not a finding.
18. **Guard the lead (battery modes).** The other side of play 16: in the last ~2 minutes, the leading side puts 1–2 bots 12–20 m from its own container, covering it, to stop a late denial. The only time bots guard their base. Not on top of it: in the saved worlds players within 12 m of their own container had K/D 0.74–0.81 of the mode's (0.46 within 6 m in timed batteries), 12–20 m off about even.
19. **Spot for the humans.** A bot that sees an enemy presses spot (bit 28, key down, 3 s cooldown), so its human teammates get what it sees: honest teamwork that humans notice. What a spot shows in 0.26g is to be checked in stage 0 before this is relied on.
20. **Grenade before the peek.** Against a held angle (play 8) or a camping spot from the danger map, throw a grenade (bit 23, held until the throw starts) and then peek or push. Rarely, so it stays a surprise, and never at an enemy no sense gave.
21. **Trade partner.** Bots leave spawn in pairs about 10 m apart. When one dies, the other gets ~3 s to trade on that spot (it was seen or heard), then breaks off (play 7). Not measured yet: the assists in the statistics can test how often humans trade.
22. **Heal behind cover.** A wounded bot out of every line of sight uses a medkit from a quick slot (bits 14–19), then goes back (play 10). Needs a kit that carries one: the lowest kit a bot spawns in is to be checked.
23. **Self-balancing difficulty.** Bots should make matches fun, not win them. The commander keeps the bots' K/D against humans: above ~1.5 over the last few minutes, their aim error and noticing delay grow a step; when humans stomp them, they shrink a step, inside set limits. It changes only aim and reaction, never what the bots know (§2).
24. **Raid.** In the battery modes, take a battery out of the enemy's container and bring it home (a swing of two) whenever their container is weakly held: some of them dead, none known by it, not in the first two minutes. Winners of 6+ player matches took batteries out about 2.6 times as often as losers, mostly in the middle and last thirds, not only at the end (play 16 drops them there instead).
25. **Keep with the partner.** Play 21's pairs move together: the follower keeps beside its leader (9 m, running back past 18 m) instead of roaming alone, and the leader waits up to 6 s for a follower left 25-90 m behind. Deaths with a teammate within 20 m were traded (the killer killed within 5 s) 20 % of the time, without one 3 %; winners died near a teammate more often.
26. **Cut the enemy's best way.** From the enemy side's human routes (play 17's table), weighed by lives times what a life gave: hold a side angle (off their line by 35 degrees or more, not on their side) 12-28 m from where the route comes into our half (60 % of the way from their base, out of every spawn area). Long holds only in Team Deathmatch; moved on after 1-2 kills. One bot per route.

What the paths also say (87 matches of 4+ players; `srp-worlds` tables):
- Lives that never get 30 % of the way out are the worst in every mode (K/D 0.3–0.6, almost no batteries): bots don't idle at base.
- Lives that reach the enemy third and then hold still are the longest (3–4 min) with K/D ~2: that is play 15.
- Sprinting is not what kills: players were sprinting about as often right before a death (21–24 %) as right before a kill (18–23 %).

**Never:**
- go into an enemy spawn area, or shoot into one from outside to camp it;
- aim perfectly from the first frame;
- react to anything no sense gave.

Bots never path back into their own spawn area.

What decides the winner (52 matches of 6+ players with a result, 10 of them recorded at every tick; a private study, 2026-10-14):
- With sides of equal size, being forward, crouching, sprinting, accuracy and height did not separate winners from losers; the bigger side is forward because it is bigger.
- Winners survived more and died alone less; their deaths were answered more often. At the tick level a trade needs a teammate within 20 m (play 25).
- The battery modes are won by the batteries: winners picked up more (20 of 25 matches), lost fewer on the way home (put in per pickup 1.00 against 0.67) and raided the enemy's container (play 24); more kills went with the win in only 64 % of them.
- Winners mostly won the gunfight itself (the first aimed shot won 85 % against 71 %): skill or gear, not a team pattern. Focus fire, crossfire and who saw first did not separate them. For both sides, firing back within 0.5 s and breaking the line of sight when losing paid.
- The commander keeps a slot (a battery, a post) with the bot that holds it unless another bot's claim beats it by the margin: two bots no longer swap one battery every half second.

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
- **Grenades and pickups in use:** when humans throw (bit 23) and how long they stand at a battery (bit 21).

**How the best players fight** (10 recorded matches of 6–10 players, 2026-10-14; every input of every player at its 50 ms tick, positions replayed between the worlds, sight on the bullet mesh as the bots' own; private research, aggregate numbers only). The three best players by kills a minute, K/D and kills+assists a life (players I, B and E; 20 matches between them, 1.4 / 1.4 / 0.8 kills a minute) against the 17 other humans; quartiles, n in brackets. The bots' defaults (`tuning.rs`, `reflex.rs`) are set to the top three's numbers where a bot can do the same:

| | top three | everyone else | bots |
|---|---|---|---|
| sight to first shot | 400 / 650 / 1050 ms (262) | 450 / 750 / 1300 ms (508) | 550 ms median at skill 1, 750 at 0.5, 950 at 0 (unchanged) |
| yaw off the chest while firing | 1.5° median (7,810 ticks), ~1.0° without the replay's error | 1.8° (12,551) | 0.9° at skill 1, 1.5° at 0.5, 2.0° at 0 (a slow wander added; was 0.3–0.8°) |
| automatic bursts | 500 / 750 / 1100 ms (658) | 150 / 350 / 800 ms (1,514) | 500–1000 ms (was 250–600) |
| pauses between bursts | 200 / 400 / 750 ms (139) | 100 / 200 / 450 ms (658) | 200–650 ms (was 300–700) |
| aim below the chest during a burst | 1.4° in the first 250 ms, 3.5° by 500 ms, 5.8° by 1 s (median) | 1.1 / 2.4 / 4.4° | pulled down 7° a second up to 5.5° (new) |
| down the sights while firing | 86 % within 10 m, 99 % beyond | 80 % / 98 % | always beyond 10 m, nearer with 85 % a target (new) |
| a side key held while firing (automatic) | 78 % (7,810) | 64 % (12,551) | side steps of 250–700 ms, a still moment of 150–450 ms after half of them (new) |
| one way of the side step, in a fight | 300 / 450 / 750 ms (920) | 250 / 450 / 750 ms (1,720) | as above; a hit turns it at once (73 % of the top three turn within 1 s) |
| standing still while shot at | 3 % (1,742 ticks) | 16 % (4,358) | side steps (bolt action excepted) |
| crouched while firing | automatic 12 %, bolt action 32 % (112) | 19 % / 28 % | a bolt action crouches with 30 % a target (new) |
| one-round weapons | press again at the weapon's cycle (SKS 200 ms at 200, TT-33 250 at 200, all players) | | at the cycle + 50 ms (was + 100), no burst pause (was one) |
| reload | 90 % before empty, half the magazine left (363) | 93 %, two thirds left (670) | tactical after 1 s with nothing seen (was 3 s) |
| sprint, share of moving time | 70 % with every enemy > 60 m, 49 % at 30–60 m, 30 % within 30 m unseen, 74 % of the first 10 s of a life | 57 / 49 / 36 %, 61 % | unchanged: run unless a known danger is within 30 m (play 17) |

PROVEN: the look pitch of a world is the sum of the inputs' turns (to 1e-5 rad over 1,275 pairs), so the weapon's kick is not in it. The kick lifts the shot, shown by the hits: 1,540 recorded hits (each victim's `m_recent_hits`, given to the enemy firing at him at that tick with him nearest the view) were landed with the attacker's view under the victim's chest by more the longer the trigger was held, 1.6° after 0.1-0.2 s, 2.7° at 0.3-0.4 s, 4.2° at 0.4-0.6 s, 5.1° at 0.8-1 s, 7.9° at 1-1.4 s (medians over the one-round weapons' hits, which sit at -0.4°; 1,427 automatic hits), while their yaw stays at 1.1-1.4°; and a round of 0.5-1 s into a burst still hits 85 % as often as one of the first 0.25 s (0.34 against 0.40 hits a round), which a shot along a view 4-5° under the chest could not. So the best players pull the mouse down against it, and the bots do the same (7° a second, up to 5.5°). In the 0.26g client the shot's direction was not traced (UNKNOWN in its code); 0.100b's code takes the bullets from the camera the frame drew, which the recoil clips move. The soaks' stand-in game lifts every shot by the same curve while the trigger is held (`bots/tests.rs`, `KICK_DEG_S`). Estimated: the replay's place error (~0.3 m) adds about 0.9° at 20 m to every angle above; reaction times are on the match clock from the first tick the enemy is in the view and in sight (the client's own display delay included, as it is for what a bot shows).

The worlds the server already saves (one every ~5 s, `SRP_SNAPSHOT_DIR`) give a first danger map now: `srp-worlds` tables them (positions, statistics, batteries), and a kill is placed where the killer stood in the world after his kill count grew, if he moved under 10 m in those 5 s. Per 8 m cell: time present, kills, kills per minute present (shrunk toward the level's rate), K/D against the level's average. Matches of fewer than 4 players are left out. The tables and maps stay private (game data of real matches).

## 6. Technical needs

- **Event delay:** events reach the server with the worlds, every ~5 s. In matches with bots: a world every 1 s (~8 KB each).
- **Positions between worlds:** carried forward from keys and turning (drift on collisions). A bot's own position is carried forward the same way and corrected at each world.
- **Path distances and visibility:** from a walk grid built from the walker mesh, and a ray tree on the bullet mesh (`player_bots.md` §5). Distance maps per target (enemy base, batteries, resupply) are cheaper than repeated path searches.
