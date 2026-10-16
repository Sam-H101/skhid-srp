# Player bots: research on what to build and how (0.26g)

Two rounds of web research (2026-10-14) for the server-driven player bots
(`player_bots.md`), done by agents and recorded here with their sources:
- **Part A, what to build:** the smartest bots for the least effort.
- **Part B, how to build it:** a structure that is easy to read, test and
  maintain, and that changes how bots play while a match runs.

The behaviours themselves are in `player_bots_behaviour.md` (senses, cheats,
plays 1–23, event reactions).

Tags:
- **[E]** established practice, from a source read for this file (URL given);
- **[S]** an inference for our setup;
- **checked**: a claim re-read in its source by a second person.

Quotes are kept under 15 words. Nothing here is built yet beyond stage 1
(`player_bots.md` §6).

## Part A. What to build

**Bottom line:** a hand-built "classic" bot in the style of Counter-Strike
1.6 and Killzone 3, not a learning one.
- The pieces that make bots feel smart are documented and cheap.
- Having no server-side simulation rules out reinforcement learning.
- Imitation learning is at most a later add-on.
- Recorded human play is best used to fit the hand-built bot's numbers:
  reaction times, aim error by range, burst lengths, map heat maps.

1. **A self-model and a layer that turns intent into inputs.** Killzone 3
   bots drive a "virtual controller", the same interface a human uses:
   perception → world database → planner → tasks → skills → controller
   [E] (Killzone 3, GAP1 ch. 29, below).
   - [S] We know a bot's yaw and pitch exactly, because we send the turns.
   - [S] Its position can be carried forward from its own inputs and
     corrected at each world.
   - [S] To move one way while aiming another, take the key combination
     closest to (wanted direction − yaw), out of 8.
   - [S] A stuck detector replans.
2. **Navigation.** Recast is the industry standard for navmeshes (zlib,
   https://github.com/recastnavigation/recastnavigation). Rust options are
   `rerecast` (young), `polyanya` and `landmass` [E].
   - [S] Our own walk grid with A* and floods (`player_bots_mapdata.md`)
     already serves; a navmesh is optional.
3. **Visibility.**
   - The Source SDK precomputes visibility between nav-area pairs [E] (read
     for the design only: Valve licence).
   - It keeps a decaying danger value per area for each team [E].
   - It takes a hiding spot as one where at least 8 of 16 rays hit within
     100 units [E].
   - [S] Our ray tree (1–2 µs a ray) is enough for 20 players.
4. **Combat: the biggest "human" effect.** The CS 1.6 bot code is readable
   as ReGameDLL_CS (https://github.com/s1lentq/ReGameDLL_CS; MIT,
   reverse-engineered, so reimplement and never copy):
   - five body points checked for sight;
   - a chance to notice by visible parts, distance, speed, crouch and skill;
   - reaction as a queue of threat snapshots read "reaction time" ago;
   - the view on a spring-damper;
   - aim error that shrinks while still (focus);
   - fire only inside a cone;
   - bursts by range [E].

   Reaction is ~0.2 s for a simple case, ~0.4 s with a friend-or-foe choice,
   and more for far targets and turns (Rabin, GAP2 ch. 5:
   http://www.gameaipro.com/GameAIPro2/GameAIPro2_Chapter05_Agent_Reaction_Time_How_Fast_Should_An_AI_React.pdf)
   [E].

   Quake III's skill knobs (reaction time, aim accuracy and skill, view
   factor, attack skill, camper, aggression:
   https://icculus.org/gtkradiant/documentation/bot_manual/chars.html) are
   a model for per-bot profiles [E].

   [S] Lockstep gives bots no lag, so the delay must be added on purpose.
5. **Decisions:** a priority list with utility scores, no planner at first.
   - Halo 2 uses a priority behaviour tree [E].
   - F.E.A.R. plans over 3 states [E].
   - Killzone uses HTN, but its smarts lie in picking positions and squad
     paths [E].
6. **Tactical positions.** Two references [E]:
   - Killzone scores a spot by how near it is, its line of fire to the main
     threat, its cover from others, and its range.
   - Crytek's Tactical Position Selection does cheap filters first, then
     weights, then costly rays lazily from the best down, spread over ticks
     (GAP1 ch. 26:
     http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter26_Tactical_Position_Selection.pdf).
7. **Team AI.** A commander per team [E]:
   - with weighted objectives and squads;
   - an assignment that keeps earlier choices;
   - an influence map of friends, enemies and recent deaths.

   Players notice bots that move as a group [E] (Verweij thesis:
   https://www.guerrilla-games.com/media/News/Files/VUA07_Verweij_Hierarchically-Layered-MP-Bot_System.pdf).
   Hand annotations per map are fine; Killzone 3 had them.
8. **Memory.** No omniscience: every enemy fact passes sight, view and a
   noticing check [E]. Keep a target 2–3 s after losing it, then its last
   known place, then shared search spots (GAP2 ch. 27:
   http://www.gameaipro.com/GameAIPro2/GameAIPro2_Chapter27_Looking_for_Trouble_Making_NPCs_Search_Realistically.pdf)
   [E].
9. **Difficulty.** Per-bot profiles with the CS and Quake III knobs [E].
   Condition Zero's templates run from 0.25 s reaction at skill 75 to 0.5 s
   at skill 0 [E].
10. **Learning, last.**
    - Valorant-like imitation (modl.ai / Riot,
      https://arxiv.org/html/2501.00078v1) took 48.6 h of 28 players' play
      [E].
    - Observers judged only 30 % of those bots human [E].
    - Reinforcement learning needs fast simulators, which we do not have.
    - LLMs are far too slow to control a player (seconds a step) [E].

Risks found:
1. **Own-position drift:** correct it at each world and detect a stuck bot.
2. **Walking parameters:** they must match the character: radius, height,
   step, slope, and the mirrored axis.
3. **Omniscience reads as cheating:** perception gates and a deliberate
   reaction delay prevent it.
4. **Late feedback:** hits and kills are seen only in the worlds, so aim is
   tuned by observed kill rates.
5. **Licences:** read the Valve SDK only; reimplement ReGameDLL.

## Part B. How to build it

### Bottom line

1. **Seven layers, each a plain function over owned data:**
   - `view`: a copy of the match state;
   - `sense`: percepts;
   - `know`: memory, every fact tagged with its source;
   - `team`: the commander hands out plays every ~0.5 s;
   - `play`: one play at a time, as a small enum state machine;
   - `tactics`: spot and route queries;
   - `motor`: aim, move, fire, reaction delay;
   - `encode`: today's `drive` tick rules.

   Orders go down and reports come up, as in Killzone 2 and 3.
2. **Two tracks per bot.** The *play* decides where to be and why. A
   separate *combat reflex* handles target, aim, fire, breaking sight,
   punishing a reload, turning to a sound and grenades. It runs every think
   and may override movement to survive. Left 4 Dead's survivor bots have a
   "Main" and "Legs"; Killzone 3 bots may "temporarily ignore orders".
3. **Decide with utility, not a behaviour tree or a planner.**
   - Each play is scored by a few considerations: input → bookends →
     response curve → multiply, stopping at the first 0 (Guild Wars 2,
     Infinite Axis Utility System).
   - Plays sit in **rank bands**: alarm > objective > fight > roam.
   - In the top band, take the best. For routes, pick at random among the
     near-best (dual utility).
4. **Inside a play, a plain `enum` of steps.** Each step returns `Continue`,
   `ChangeTo(..)` or `Done`, with a reason string (Left 4 Dead's NextBot).
   There is no generic tree interpreter.
5. **Runtime adjustment goes through one `Mods` struct per team**, set each
   commander tick:
   - its inputs are match signals: score gap, time left, players alive,
     deaths, thefts, K/D against humans;
   - it is computed through curves and smoothed;
   - considerations multiply by it;
   - the tuning file can tie any play to any signal without code.
6. **Difficulty changes only motor numbers**:
   - only aim error and noticing delay change;
   - in damped steps (Unreal Tournament's `AdjustSkill`);
   - applied **at respawn**, never mid-fight.
7. **Tuning** lives in typed `serde` structs, with unknown fields refused
   and values validated at load.
   - A new file is taken up by the *next match*.
   - Each trace names the tuning's hash, so any match can be replayed.
8. **Determinism and explanation:**
   - one seeded random stream per bot (match id + place), and no wall clock
     inside decisions;
   - every decision writes a trace line: the candidates, each
     consideration's score, the winner and the reason.
9. **Budget:**
   - motor and encoding every 50 ms tick;
   - sight rays from a per-tick budget, stalest pair first;
   - decisions staggered at 2–5 Hz (CS: full AI at 10 Hz, commands at 30 Hz,
     checked; Killzone 2: plans at 5 Hz);
   - bots no human can see think more slowly.

### B1. Layers and boundaries

#### Killzone 3 [E]

Source: GAP1 ch. 29,
http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter29_Hierarchical_AI_for_Multiplayer_Bots_in_Killzone_3.pdf.
- **Three layers:** commander, squads and individual bots, with orders
  moving downward and information moving up.
- **Inside each bot:**
  - perception, messages and daemons fill a world database;
  - an HTN planner makes plans;
  - tasks drive skills, and skills drive a virtual controller shared with
    human players;
  - that controller strictly separates the AI from the engine.
- **Orders:** each squad and bot has a command queue, and a new order
  replaces the old one.
- **Assignment:** the commander keeps earlier assignments unless its rules
  force a change.

#### Killzone 2 [E]

Source: slides, https://www.slideshare.net/guerrillagames/killzone-2-multiplayer-bots.
- **Orders down:** strategy → squad (defend, advance) → bot (move to).
- **Reports up:** "order failed", combat information, requests to be
  reassigned.
- **Rates:** plans at 5 Hz, with guards against twitchy replanning.
- **Lesson:** create information by hand first, automate later.

#### Left 4 Dead NextBot [E]

Source: Booth 2009,
https://steamcdn-a.akamaihd.net/apps/valve/2009/ai_systems_of_l4d_mike_booth.pdf.
- **Parts:** an actor owns Locomotion, Body, Vision and Intention.
  Intention is concurrent behaviours, each a stack of actions.
- **Actions:**
  - they return `Continue`, `ChangeTo(next, reason)`, `SuspendFor(next,
    reason)` or `Done(reason)`;
  - only the running action may change itself;
  - reason strings are "invaluable for realtime debug output".
- **Events and queries:** events and questions such as "should I hurry"
  pass from the innermost action outward.

#### Counter-Strike (ReGameDLL) [E]

- `Upkeep()` is light and runs often; `Update()` is heavy and runs less
  often.
- Commands go at 1/30 s, "just like human clients", and the full AI runs
  at 1/10 s (`bot.h`; checked 2026-10-14).
- States are classes with enter, update and exit: Idle, Hunt, Attack,
  InvestigateNoise, MoveTo, Hide, Follow.

#### Knowledge (Brewer) [E]

Source: GAP Online 2021 ch. 4,
http://www.gameaipro.com/GameAIProOnlineEdition2021/GameAIProOnlineEdition2021_Chapter04_Knowledge_is_Power_an_Overview_of_AI_Knowledge_Representation_in_Games.pdf.
- Keep perception apart from behaviour.
- Store the last known position and the time since it was seen, not
  "visible".
- Flag first-hand against second-hand facts: a bot told of an enemy must not
  act as if it saw him.
- Cache "unreachable" so failing path searches are not repeated.

#### No generic blackboard first [E]

Source: Francis, GAP3 ch. 9,
http://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter09_Overcoming_Pitfalls_in_Behavior_Tree_Design.pdf.
Tying the logic to a blackboard API cost "a month or more" of rewiring,
twice. A plain struct often suffices.

#### Smart objects and option stacks (Dill) [E]

Source: GAP1 ch. 5,
http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter05_Structural_Architecture_Common_Tricks_of_the_Trade.pdf.
- Places advertise what can be done there.
- One option can suspend another without dropping it.
- Inertia stops flip-flopping between options.

#### What this means for us [S]

- **The source tag is the backbone of the fairness rules:**
  - `Source = Seen | Heard | Told | Hud | Cheat`.
  - Motor accepts only a `SeenTrack`. That newtype is built only by `know`,
    from a sight older than the noticing delay. "Never aim at what no sense
    gave" is then a type rule.
- **Map facts are smart objects:**
  - battery spawners, containers, exits and chokes advertise **slots**: an
    ambush 10–25 m off, a guard spot 12–20 m off, an exit hold 30–60 m from
    the enemy base (numbers from the saved worlds,
    `player_bots_behaviour.md`);
  - "never two bots on one ambush" is slot ownership.

What crosses each boundary [S]:

| From → to | Data | Rate |
|---|---|---|
| `game` → bots | `MatchView`: an owned copy (clock, mode, score; each place's estimated pose, flags, health, alive; the newest statistics; sounds from inputs: fire, reload, switch, use) | each poll |
| `sense` → `know` | `Vec<Percept>` (who, what, estimated place, error, `Source`, time) | per bot, budgeted |
| `know` → `team` | `TeamKnowledge` (read only), `BotStatus` (alive, health, play, slot, progress), `Report`s | 2 Hz |
| `team` → bot | `Assignment { play, target, slot, reason, since }` | 2 Hz, kept unless a rule forces a change |
| bot → `tactics` | `SpotQuery` / `RouteQuery` → `Spot` / `Route` | on demand, budgeted |
| play + reflex → `motor` | `Intent { move, stance, look, fire, pulses }` | 2–10 Hz |
| `motor` → `encode` | keys and (yaw, pitch) change per tick | 20 Hz |
| bot → `team` | `Report::{Done, Failed(reason), Contact(track), Died(at)}` | on the event |

### B2. Adjustable while a match runs

#### What the sources say [E]

**The Infinite Axis Utility System, as Guild Wars 2 used it.** Source:
Lewis, GAP3 ch. 13,
http://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter13_Choosing_Effective_Utility-Based_Considerations.pdf;
GDC talks https://www.gdcvault.com/play/1018040/Architecture-Tricks-Managing-Behaviors-in
and https://www.gdcvault.com/play/1021848/Building-a-Better-Centaur-AI, of
which only the description pages were read.
- **Scoring:**
  - each consideration maps an input to [0, 1] by bookends, then through a
    response curve;
  - the considerations are multiplied, and any 0 stops the evaluation;
  - so cheap checks go first and costly rays last.
- **Curves:** a palette of preset curves.
- **Standard considerations:** "runtime" (stop after n repeats) and
  "cooldown".
- **Against oscillation:** a commitment bonus.

**Many multiplied considerations shrink a score.** The named fixes are a
compensation factor (Mark) or a geometric mean (Graham)
(https://uintel-go.utilityworlds.com/Documentation/UtilityIntelligence/Considerations/).
- The formula `m = 1 − 1/n; s' = s + (1 − s)·m·s` was given from memory of
  the talk.
- It matches that page's table: 0.5 with 2 considerations gives 0.391, with
  3 gives 0.296.

**Curves and choice (Graham).** Source: GAP1 ch. 9,
http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter09_An_Introduction_to_Utility_Theory.pdf.
- Curves are linear, polynomial, logistic or piecewise linear (The Sims).
- Inertia comes from a bonus to the running action.
- Choice can be weighted random among the near-best, or by buckets.

**Dual utility (Dill).** Source: GAP2 ch. 3,
http://www.gameaipro.com/GameAIPro2/GameAIPro2_Chapter03_Dual-Utility_Reasoning.pdf.
- Each option has a rank and a weight.
- Keep the top rank, drop options far below the best, then pick weighted
  random.
- Zoo Tycoon 2's ranks: needs 0, shows ~100, death 1,000,000.

**Dragon Age Inquisition.** Source: GAP3 ch. 31,
http://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter31_Behavior_Decision_System_Dragon_Age_Inquisition’s_Utility_Scoring_Architecture.pdf.
- Designers score in bands: basic 10, offensive 20–40, support 25–45,
  reaction 50–70.
- An evaluation returns a score and a target together.

**Visible thresholds (Johnson).** Source: GAP Online 2021 ch. 5,
http://www.gameaipro.com/GameAIProOnlineEdition2021/GameAIProOnlineEdition2021_Chapter05_Taming_Spatial_Queries_Tips_for_Natural_Position_Selection.pdf.
- Players learn fixed timers, so use continuous utility and hysteresis.
- Beware relative normalisation: the best candidate is then always 1.0.
- Aggression can itself be a curve over the match's progress.

**The Left 4 Dead director** (Booth, above).
- Each survivor's intensity rises with damage and nearby kills, and decays
  when not fighting.
- The phases are build-up → peak (3–5 s) → fade → relax (30–45 s).
- It changes the pacing, not the difficulty.

**Hunicke and Chapman.** Source:
https://users.cs.northwestern.edu/~hunicke/pubs/Hamlet.pdf.
- Changes to what is already in a fight are hard for the player to read.
- "Proactive" changes, made out of sight, keep disbelief suspended.
- Each intervention should cost more the more often it is made.

**Unreal Tournament's `AdjustSkill`** (`bAdjustSkill=True` by default;
checked 2026-10-14 at
https://project-archives.etc.cmu.edu/2001/fall/coyote210/Docs/undox/Botpack.ChallengeBotInfo.html).
- When a bot wins against the player, `AdjustedDifficulty − 2/Min(PlayerKills, 10)`,
  never below 0.
- When it loses, `+ FMin(7, 2/Min(PlayerDeaths, 10))`.
- So the steps shrink from 2 to 0.2: a damped controller.

**Damage control by accuracy.** Source: GAP3 ch. 33,
http://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter33_Using_Your_Combat_AI_Accuracy_to_Balance_Difficulty.pdf.
- Decide *when* a shot may hit, with a delay by distance, stance and
  facing.
- Miss believably.

**Pitfalls of dynamic difficulty.** These come from essays and a 2024
paper's review, seen in search summaries only:
- swings that are large or frequent read as rubber-banding;
- players who notice it exploit it.

#### What this means for us [S]

**Three tiers of runtime levers:**
1. **Signals** (`MatchSignals`, per team, each commander tick):
   - score gap (normalised by mode) and time left;
   - players alive on each side, enemies respawning;
   - team deaths at a spot in the last 20 s;
   - age of a battery theft, carriers of each side;
   - K/D against humans (sliding 5 minutes) and the number of humans.
2. **Mods** (`Mods`, per team):
   - `aggression`, `risk`, `objective_urgency`, `spread`, and each bot's
     `skill`;
   - each is a curve of one or two signals, clamped and smoothed (`m += (t − m)·0.2`
     a tick), so one goal does not flip the team in one tick.
3. **Considerations** read signals or mods. A play's tuning may add
   `{ signal = "score_gap", curve = … }` with no code change.

**"The losing team grows more aggressive".**
- `aggression = logistic(−score gap, mid = 1 battery) × poly(1 − time left / total, k = 0.5)`.
- Aggressive plays carry a `Mod(Aggression)` consideration: sneak,
  intercept, push the wounded, late denial.
- Camping plays carry its inverse.
- It changes *what* bots try, never their aim.

**Self-balancing (play 23).**
- Every 60 s, look at the bots' K/D against humans over ~5 minutes.
- Nothing changes inside the dead band of 0.67–1.5.
- Outside it, move `skill` one step of `base / min(n, 10)` (as in Unreal
  Tournament), clamped.
- A bot takes the new skill at its next respawn.
- Knowledge and cheats never change.
- Every change is logged with its inputs.

**Personalities (play 14) are tuning presets, not code.**
- Each preset is a set of multipliers per play family, plus motor habits:
  sprinting, hold time, peek style.
- They are drawn per bot at match start by the human mix: 55 % steady, 30 %
  sprinting, 10–15 % holding.

### B3. Easy to read, test and maintain

#### What the sources say [E]

**Comparing the methods (Dawe et al.).** Source: GAP1 ch. 4,
http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter04_Behavior_Selection_Algorithms.pdf.
- State machines suffer "transition overload" as states grow.
- Behaviour trees are simple, and their behaviours know nothing of each
  other.
- Utility suits many options with no single right one and recovers well,
  but is harder to tune and less predictable.
- GOAP removes the designer's control.
- HTN reads well but must be authored, and its search grows with it.

**Analog choices.** Behaviour trees are poor at choosing among several valid
options; a utility selector node fixes that (Merrill, GAP1 ch. 10).

**HTN at scale.**
- Killzone 2's HTN grew to 360 methods and 1,048 branches, debugged with
  text dumps of plans (slides above).
- HTN needs a world-state vector, effects and replanning (Humphreys, GAP1
  ch. 12).

**Debugging tools.**
- Treyarch records every frame in a ring buffer, including the branches
  taken, and scrubs back and forth (GAP3 ch. 6:
  http://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter06_Debugging_AI_with_Instant_In-Game_Scrubbing.pdf).
- Final Fantasy XV logs events with time, position and agent through a
  non-blocking logger. It draws them on top-down map screenshots with heat
  maps per event type (GAP3 ch. 3:
  http://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter03_Logging_Visualization_in_FINAL_FANTASY_XV.pdf).

#### For our case [S]

Our case is 23 plays, one programmer, no designer tools and a headless
server:

| | Fit | Upkeep | Debugging | Verdict |
|---|---|---|---|---|
| Flat state machine | the event table maps well | transitions explode past ~10 states | easy | only inside a play |
| Behaviour tree (`bonsai-bt`) | good for step sequences | needs a blackboard; priorities fixed by order | tree dump | not worth a dependency |
| HTN | Killzone 2 and 3 proved it for these bots | a domain language and replanning | plan dumps | too much for 23 plays |
| GOAP | new plans | hard to steer | hard | no |
| **Utility with rank bands, an enum state machine per play** | the plays already have "a score, a precondition and steps" | a new play = one file and a tuning entry; no transitions to edit | every score is a number in the trace | **yes** |

Rules against spaghetti [S]:
1. A play never reads another play's state; effects between plays go
   through `TeamKnowledge` or the commander.
2. One file per play, with `candidates` (scoring), `start`, `step` and its
   tests.
3. Shared considerations live in one catalogue: health, path distance, time
   left, cooldown, runtime, danger at a spot.
4. Steps return transitions with a reason; they never set the bot's play
   themselves.
5. Every number comes from the tuning.

### B4. Rust without an engine

**The ECS keynote (Catherine West, RustConf 2018) [E].** Source:
https://kyren.github.io/2018/09/14/rustconf-talk.html.
- Objects that change themselves and also take `&mut World` fight the
  borrow checker.
- Keep state as plain data and systems as functions.
- Refer to things by index.

**Crates [E], as seen 2026-10-14:**
- `bonsai-bt` 0.14.0 (MIT, no required dependencies);
- `big-brain` 0.22.0, which needs Bevy;
- arewegameyet lists no engine-free utility or HTN crate
  (https://arewegameyet.rs/ecosystem/ai/), and the HTN crates found are
  Bevy-bound or stale;
- `toml` (MIT/Apache), `arc-swap`, and `insta` (Apache-2.0, snapshot
  tests).

**Recommendation [S]: no AI crate.**
- Utility scoring, curves and an enum state machine are ~400 lines.
- At most add `toml` for commented tuning files (or stay with `serde_json`),
  and `insta` as a dev-dependency.
- A 15-line SplitMix64 replaces `rand`, and its stream never changes under
  us.
- A tuning file is read when a match is made; no hot-reload crate.

**Snapshot in, commands out [S].**
- `Bots` owns everything on the bot side.
- `Game` builds a `MatchView` (owned, small: 20 places) and gets back
  `Vec<(place, input)>`.
- There are no references from bots into `Game` and no `Rc<RefCell>`.
- Players are `Place(u8)`.

**Plays as data and functions [S].**
- `enum PlayKind` (Copy) keys the tuning, the logs and the tests.
- Each play is a module of free functions.
- `enum Running` holds each play's step state, and one `match` dispatches.
- Exhaustive matching makes "a play added and the dispatcher forgotten" a
  compile error.

**Types for safety [S].**
- `Ms`, `Meters` and `Score` (built only by clamping to [0, 1]).
- `SeenTrack`.
- `Keys`, which refuses the bits a bot never sends, as `Drive::new` does
  now.

### B5. Performance budget

**What the sources say [E]:**
- CS gives commands at 30 Hz and full thinking at 10 Hz; Killzone 2 plans
  at 5 Hz.
- Killzone 3 squads find paths incrementally.
- Unreal's sight sense is budgeted per tick (traces per tick, time slice
  per tick) and treats sight as an n² problem
  (https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/AISense_Sight).
- The LOD Trader picks detail per character by importance under a fixed
  budget, not by distance thresholds (GAP1 ch. 14).

**Numbers for one match of 19 bots [S]:**
- **Sight:**
  - ~361 pairs, culled by view and range first;
  - at most 5 rays a pair at 1–2 µs, so a full sweep is ≤ 3.6 ms;
  - every 200 ms that is ≤ 18 ms a second, ~2 % of a core.
- **Utility:** 23 plays × a few targets × ~8 considerations is ~1–2 k
  evaluations, ~50 µs a commander tick.
- **Spots:**
  - up to 200 candidates from map slots and grid samples;
  - cheap filters first, then rays lazily from the best down;
  - at most 2 queries a tick.
- **Distance maps:** one flood per fixed target at match start; A* only
  when replanning.

**Scheduling inside `poll_driven` [S]:**
- motor and encoding every tick;
- each bot thinks every 200–400 ms in round robin, and events ("hit",
  "enemy seen") bring it forward;
- the commander runs every 10 ticks, staggered between the teams;
- a tick budget moves the rest to the next tick;
- a bot no human can see, with no enemy track, thinks at half rate.
- Measure `bots_ms` per match (p50 and p99 per minute) before optimising
  anything.

### B6. Module layout (`servers/match-server/src/v026g/bots/`) [S]

```text
bots/
  mod.rs      Bots: all bot state of one match; poll(now, &MatchView) -> Vec<(Place, Sent)>
  view.rs     MatchView, PlaceView, SoundEvent (built by game/driven.rs: the only Game -> bots boundary)
  sense.rs    sense(bot, &MatchView, &Map, &mut RayBudget, &mut Rng) -> Vec<Percept>
  cheats.rs   cheat_percepts(team, &MatchView, &Tuning.cheats, &mut Rng) -> Vec<Percept>  (Source::Cheat only)
  know.rs     Memory, Track, SeenTrack, Source, TeamKnowledge (callouts 1 s, batteries, danger spots, deaths)
  signals.rs  MatchSignals::from(..); Mods::update(&mut self, &MatchSignals, &Tuning)
  curve.rs    Curve, Consideration, Input, score(&[Consideration], &Ctx) -> (Score, Explain)
  team.rs     Commander::assign(..) -> Vec<Assignment>; slots
  plays/      mod.rs (PlayKind, Running, dispatch); one file per play
  reflex.rs   combat track: target, fire policy, break sight, punish reload, turn to sound, grenade, spot
  tactics.rs  SpotQuery, RouteQuery, RouteBook (human routes + runtime weights)
  motor.rs    reaction queue, aim spring and error, path following, stance, bursts -> Keys, turn
  encode.rs   today's Drive tick rules (one input a tick, 50-100 ms ahead, key-downs as one-tick pulses)
  tuning.rs   Tuning (serde, deny_unknown_fields), Personality, Skill, validate()
  trace.rs    decision trace records (JSONL), tuning hash
  rng.rs      Rng (SplitMix64), Rng::for_bot(match_id, place)
```

Main types (sketch):

```rust
pub enum Source { Seen, Heard { error_m: f32 }, Told { by: Place }, Hud, Cheat(CheatKind) }
pub struct Track { pub who: Place, pub pos: Vec3, pub at: Ms, pub source: Source, pub confidence: f32 }
pub struct SeenTrack(Track); // built only by know.rs: seen, and older than the noticing delay
pub enum Step { Continue(Intent), ChangeTo(PlayKind, &'static str), Done(&'static str), Failed(&'static str) }
pub struct Candidate { pub kind: PlayKind, pub target: TargetRef, pub rank: u8, pub score: Score, pub why: Explain }
pub struct Assignment { pub bot: Place, pub kind: PlayKind, pub target: TargetRef, pub slot: Option<SlotId>, pub reason: &'static str }
pub enum Curve { Linear, Poly(f32), Logistic { k: f32, mid: f32 }, Piecewise(Vec<(f32, f32)>), Step(f32) }
pub struct Consideration { pub input: Input, pub lo: f32, pub hi: f32, pub curve: Curve, pub invert: bool }
pub enum Input { TimeLeft, ScoreGap, PathDist(TargetRef), Health, DefendersNear(TargetRef),
                 Signal(SignalId), Mod(ModId), SinceStarted, SinceLast(PlayKind) }
```

Where each play and rule lives:

| Plays and rules (`player_bots_behaviour.md`) | Module |
|---|---|
| 1 ambush, 2 interception, 3 escort, 4 sneak, 8 read a held angle, 10 retreat to resupply, 12 cover a human, 15 hold exits, 16 late denial, 18 guard the lead, 22 heal | `plays/` (scored per bot and target; slots from `team.rs`) |
| 9 crossfire, 13 pair against the top player, 21 trade partners, "one bot per ambush", "1–2 nearest answer a theft", the event alarms | `team.rs` (commander rules and slots; they raise the matching play's rank) |
| 5 punish a reload, 6 turn to a sound, 7 break sight and re-peek, 11 weapon matchup, 19 spot, 20 grenade before the peek | `reflex.rs` (combat track, every think) |
| 17 routes from humans and their runtime weights; the sprint rule | `tactics.rs::RouteBook` (a data file per map, mode and side) |
| 14 personalities | `tuning.rs` presets, drawn per bot |
| 23 self-balancing | `signals.rs` → `Skill` → `motor.rs` at respawn |
| The cheats | `cheats.rs` and `Tuning.cheats`; they give only `Source::Cheat` percepts, which `motor` cannot aim at by type |
| The "never" rules | spawn areas filtered in `tactics.rs`, `SeenTrack`, the reaction queue in `motor.rs`, and debug asserts |

#### Worked example: play 16, late denial [S]

```rust
// plays/late_denial.rs
pub fn candidates(cx: &Ctx, out: &mut Vec<Candidate>) {
    if !cx.sig.mode.has_containers() { return; }
    let target = TargetRef::Container(cx.team.enemy_container);
    let (score, why) = curve::score(&cx.tuning.late_denial.considerations, cx, target);
    if score.is_zero() { return; }
    out.push(Candidate { kind: PlayKind::LateDenial, target, rank: RANK_OBJECTIVE, score, why });
}
// Default considerations (tuning file; multiplied, with compensation):
//  TimeLeft         0..240 s, piecewise (0,1) (0.5,1) (0.75,0.3) (1,0)  -> 0 above 4 min
//  ScoreGap         -3..+3,   logistic                                  -> high when behind or level
//  PathDist(cont.)  0..120 m, inverted
//  DefendersNear    0..3,     inverted (known defenders, not cheated)
//  Health           step at 0.35
//  Mod(Aggression)  0.5..1
//  SinceLast(LateDenial) cooldown 20 s; SinceStarted runtime 60 s
pub enum State { Approach { route: RouteId }, Steal { tries: u8 }, Drop, Next }
pub fn step(s: &mut State, cx: &Ctx) -> Step {
    match s {
        State::Approach { .. } if cx.at_container(30.0) => { *s = State::Steal { tries: 0 }; Step::Continue(Intent::crouch_walk_to(..)) }
        State::Steal { .. } if cx.me.carrying => { *s = State::Drop; Step::Continue(Intent::pulse(Keys::DROP)) }
        State::Steal { tries } if *tries > 3 => Step::Failed("container empty or blocked"),
        State::Steal { tries } => { *tries += 1; Step::Continue(Intent::pulse(Keys::USE)) }
        State::Drop => { *s = State::Next; Step::Continue(Intent::hold()) }
        State::Next if cx.team.container_has_batteries(..) => { *s = State::Steal { tries: 0 }; Step::Continue(Intent::hold()) }
        State::Next => Step::Done("container emptied"),
        _ => Step::Continue(Intent::follow_route(..)),
    }
}
```

Its tests: a scenario builder and a check of the trace.

```rust
#[test]
fn a_bot_behind_by_one_with_90_s_left_goes_for_late_denial_and_says_why() {
    let mut s = Scenario::map("level_01_day_4").mode(Mode::Batteries)
        .clock_left_s(90).score(2, 3).bot(0, Side::A).at_path_m_from_enemy_container(50.0).seed(7);
    let d = s.commander_tick();
    assert_eq!(d.assignment(Place(0)).kind, PlayKind::LateDenial);
    insta::assert_yaml_snapshot!(d.trace(Place(0))); // every consideration's input, output and the product
}
#[test] fn leading_by_two_never_denies() { /* score(4, 2): the score gap gives 0 */ }
#[test] fn the_play_never_takes_more_than_four_use_presses() { /* step() over every State */ }
```

#### Worked example: a runtime modifier [S]

```rust
// signals.rs: once per commander tick, per team
pub fn update(&mut self, sig: &MatchSignals, t: &ModTuning) {
    let target = t.aggression_gap.eval(-sig.score_gap_norm) * t.aggression_time.eval(sig.match_progress);
    self.aggression += (target.clamp(t.aggression_min, t.aggression_max) - self.aggression) * t.smoothing;
}
// Self-balancing: every 60 s
pub fn balance(&mut self, kd_vs_humans: f32, t: &SkillTuning) {
    if kd_vs_humans > t.kd_high { self.steps_down += 1; self.skill -= t.step / self.steps_down.min(10) as f32; }
    else if kd_vs_humans < t.kd_low { self.steps_up += 1; self.skill += t.step / self.steps_up.min(10) as f32; }
    self.skill = self.skill.clamp(t.min, t.max); // motor takes it only when a bot respawns
}
```

### B7. Tuning parameters [S]

| Parameter | In the tuning file | Changed while playing, by |
|---|---|---|
| Sight field, noticing delay, hearing ranges, callout delay, memory | yes, with the cheat multipliers | noticing delay and aim error scale with `skill` (K/D against humans) |
| Aim spring, aim error, focus, bursts, reaction | per `Skill` level | `skill`, at respawn |
| Cheats on or off, their errors (thief ±10 m, radar ±15 m / 5 s) | yes, per difficulty preset | nothing: never adaptive |
| Rank bands, considerations, curves, cooldowns, runtimes per play | yes | considerations read `Mods` and signals |
| `aggression` | curves, bounds, smoothing | score gap, time left |
| `objective_urgency` | curves | time left, age of a theft, a carrier alive |
| `risk` | curves | alive on each side, enemies respawning, recent team deaths |
| `spread` | weight | teammates on a route |
| Route weights | a file per map, mode and side, from the recordings | enemies sensed, deaths on a route (20 s), battery and thief positions |
| Give-up times of holds and ambushes | ranges, drawn | personality |
| Commander and think periods, ray budget, the "nobody sees it" radius | yes | match load (`bots_ms`) |
| Personality mix and presets | yes | drawn at match start |
| Self-balancing: K/D band, step, bounds, window | yes | K/D against humans |

### B8. Testing and debugging [S]

1. **Unit tests:**
   - curves (monotone, endpoints);
   - scoring (stop at 0, compensation);
   - commander rules (exclusive slots, assignments kept);
   - encoding (today's `drive` tests).
2. **A scenario test per play:** a builder over `MatchView`, with a small
   test map or a real `.mapdata` when one is given (skipped otherwise).
   Check the play, the target and the reason, and snapshot the trace.
3. **Invariants, over every scenario and replay:**
   - no aim toward a place without a `SeenTrack`;
   - no first shot sooner than the noticing delay;
   - no path through a spawn area;
   - no bit a bot never sends;
   - one input a tick, and each turn under π.
4. **Replays of recorded matches:** feed worlds and inputs through `Bots`
   with a fixed seed and tuning, and snapshot the decision trace. A change
   in decisions shows as a reviewable diff.
5. **Soak runs:** matches of only bots at an accelerated clock in a test
   harness, as Left 4 Dead stress-tested. They catch stuck bots, oscillation
   and budget overruns.
6. **The decision trace:** JSONL per match and bot, written on a change and
   every N s. Each line has:
   - time, pose, play and step;
   - the top 5 candidates with every consideration;
   - the reason, the mods, the skill and the tuning hash.

   One line answers "why did it do that".
7. **Map drawings:** top-down pictures from the walk grid of:
   - positions over time;
   - tracks with their source and error circle;
   - slots, chosen spots with candidate scores as a heat map;
   - routes with their weights.

   This is Final Fantasy XV's approach. The `mapdata-layout` top views
   already exist; drawings of real matches stay private.
8. **Counters in the log each minute:**
   - plays chosen and changes per bot-minute (oscillation);
   - give-ups and stuck replans;
   - `bots_ms` p50 and p99;
   - the skill of each team and the K/D against humans.

### B9. Risks and anti-patterns

| Risk | Remedy | Source |
|---|---|---|
| Oscillation between near-equal plays or spots | commitment bonus, cooldown and runtime considerations, hysteresis, change only by a margin | Lewis; Graham; Johnson; Dill |
| Fixed timers that players learn ("gives up at 30 s") | draw from ranges, continuous utility | Johnson |
| Relative normalisation in spot scoring | clamped bookends | Johnson |
| Many multiplied considerations sink long plays | compensation or geometric mean | uintel page; Mark and Lewis |
| A generic blackboard first | typed structs | Francis |
| Planner overreach | GOAP and HTN take control away or need a domain language | Dawe et al.; Killzone 2 |
| Omniscience leaking through team knowledge | tag second-hand facts; cheats in one module | Brewer |
| Difficulty swings seen as rubber-banding | change at respawn, damped steps, a dead band, aim and reaction only | Hunicke; UT `AdjustSkill`; the essays |
| Perception starved by a fixed ray budget | prioritise by staleness and threat; measure the delay | Unreal's settings; a forum report seen only in a search snippet |
| Reloading tuning mid-match breaks replays | switch only between matches; log the hash | [S] |
| Wall clock or `HashMap` order inside decisions breaks replays | match clock, `Vec` / `BTreeMap`, seeded random | [S] |

### Sources of part B, and how they were read

**Read in full or in the parts used:**
- GAP1 chapters 4, 5, 9, 29;
- GAP2 ch. 3;
- GAP3 chapters 3, 6, 9, 13, 31;
- GAP Online 2021 chapters 4 and 5;
- the Left 4 Dead talk (Booth 2009);
- the Killzone 2 slides;
- ReGameDLL `bot.h`, `bot.cpp`, `cs_bot.h`;
- Hunicke and Chapman;
- Unreal Tournament `ChallengeBotInfo`;
- the uintel considerations page;
- Unreal's `AISense_Sight` property list;
- the RustConf 2018 keynote;
- the crate pages.

**Read only in part:** GAP1 ch. 10, 12 and 14 (introductions); GAP3 ch. 33
(skimmed).

**Not read:**
- The GDC talks (Mark and Lewis 2013, 2015): only their pages were read.
  The compensation formula is from memory of them, and it agrees with the
  uintel table.
- Search summaries only: the rubber-banding essays and paper, the HTN
  crates, the Unreal forum report on sight lag.

**Checked by a second reader (2026-10-14):** Unreal Tournament's
`AdjustSkill` steps and default, and the CS bot's command and think
intervals.
