# 0.21d progression: experience, reputation, drops, daily quests

What the 0.21d client and its data fix, and what srp chooses where the data
is silent. Tags: **[V]** checked in the exe (VAs, image base 0x10000) or the
data; **[I]** inferred; **[W]** from the web (wiki changelogs, 2014 forum);
**[C]** srp's choice. Builds on `results_spec.md`.

## Data srp embeds

- `gameplay/quests/quests.options` (271 quests) and `gameplay/drops/drops.options`
  from 0.21d's `resources.db`, in `resources/binary-config-examples/`.
- `crates/survarium/src/v021d/item_params.rs`: each item's `faction`,
  `faction_affinity`, `drop_value`, `drop_weight`, and for ammo `clip_size`
  and `clips_in_bag`, generated from the items' own configs
  (`gameplay/items/**.options`).

## Experience and money

- The client and data have no experience values: no level table, no per-event
  amounts, and the match score weights come only from the server [V].
- The 0.22a changelog cut a kill "from 10 to 3" experience and money "from 50
  to 15", so 0.21d paid **10 experience and 50 money a kill** [W].
- srp: experience = score / 10 (a kill is weighted 100, so 10), + 10 for
  staying to the end, + 20 for a win [C]; money = 1.5 × experience [C, the
  15 a kill of 0.22a; 5 × (0.21d's 50) made too much against the shop]. Batteries count through their score weights.
- All that × `SRP_EXPERIENCE_RATE` (default 3) [C]: at 0.21d's rates the 20
  levels would take about a thousand matches. Money is not multiplied.
- Levels: 500 for level 2 and 250 more for each next, to 20 (the 2015 wiki) [W].

## Reputation

- Ranks (`factions_dict`) [V]: Scavengers and Black Market 0/100/420/990/1840/
  3000 (levels 0..5); Renaissance and Edge 0/1500/3370/5640/8340/11500/15150/
  19320/24040/29340/35250 (0..10). Level 0 is "Невідомий" (Unknown).
- The results screen shows reputation for `player_result_full` +0x66 (faction;
  0 hides the box) and +0x48 (amount) [V].
- The **gear faction**: `get_profile_faction_affinity` (0x1DEC3A) sums each
  equipped item's `faction_affinity` by its `faction` over 7 factions and
  takes the largest (a tie goes to the lower id) [V]. Torso and weapons 30,
  helmet and legs 20, back 15, mask 10, gloves and boots 5.
- The per-match amount is lost. srp: the experience, credited to the gear
  faction; none without one [C]. The 0.25a changelog says reputation "now"
  goes to the faction a player cooperates with [W].
- Faction loyalty adds up to 25 % to it (`loyalty.md`).
- Wire: `36 12` (unlock mask), then `36 09` {u32 faction, u16 0, u16 points}.
  A new level is announced by important message type 4 `#ru[<faction>:<level>]`;
  the client re-queries `12` and `09` [V 0x176D45].

## Drops (the "Знайдено" bar)

- `drops.options` [V]: `slots_open` 1000/5000/9000 (the bar's notches),
  `drop_amount` 10000 (the bar's top is ×1.5 = 15000, the premium factor),
  `modifier_probabilities`, a category tree (90: ammo 60 / consumables 40;
  10: armour 60 / weapons 40) and `item_level_probabilities` (21 rows × 16
  levels).
- The screen [V `fill_match_statistic` 0x1776FB, `VictoryScreen.as`]: drop i
  sits on notch i and shows only if `match_progress` ≥ `slots_open[i]`; the
  bar animates from 0 each time (no carry-over). At most 3 are shown.
- The screen changes nothing: the items must also reach the inventory (`36 03`).
- How progress was earned is lost; 2014 forum posts tie good drops to the
  score (about 400) [W]. srp [C]:
  - progress = min(score × 10, 10000);
  - one drop per notch reached;
  - each: a category from the tree, a level from the level table (row = the
    highest item level the profile wears), then an item of that category and
    level (else the nearest level) whose `drop_value` ≤ progress, by
    `drop_weight`; premium items and `drop_weight` 0 never drop;
  - ammo: one pack (`clip_size` × `clips_in_bag`); other stacks 1; anything
    else at full condition. No modifications (srp's items carry none).

## Daily quests (the "Завдання" window)

- `quests.options` [V]: 271 daily quests (ttl 1440 min, priority 10, one game),
  each with a faction, a type, `params {n, x, y, max_games_count}`,
  `faction_representative`, preconditions and awards.
  - Preconditions: type 0 = a cooldown of n minutes (120); 1 `minimum_reputation`
    and 2 `maximum_reputation` with n = level, x = faction. Each faction has a
    band of 7 or 8 quests per reputation level.
  - Awards: type 2 money, type 1 reputation (param = faction). Money 1000 + 200
    a level (Scavengers, Black Market) or 2000 + 200 (Renaissance, Edge);
    reputation 100 + 10 a level (quest 30301: 1200, a data typo).
  - Quests above level 0 have `faction_representative` = their faction: the
    client tracks them only when the gear faction matches [V 0x1B358A].
- Goals (texts in the localization) and how the client's HUD counts them
  (`calculate_player_game_progress` 0x1B3859, `calculate_victory_item_progress`
  0x1B378E) [V]:

  | type | goal | needs a win |
  |---|---|---|
  | 28 win_x_games | win `max_games_count` battles | yes |
  | 29 kd_ratio | kills / deaths > y | yes |
  | 6 bring_x_items | deliver n batteries (event 20) | no |
  | 33 defend_items | no battery stolen from the base | yes |
  | 32 win_score | ≥ n batteries home | yes |
  | 15 kill_enemy_x | n kills of players in faction x's gear | no |
  | 16 headshots | n headshots | no |
  | 19 killstreak | a streak of n | no |
  | 30 accuracy | accuracy ≥ 100·y % | yes |
  | 10 find_artifacts | n artefacts | no |
  | 12 kill_enemies | n kills | no |

- Lobby wire [V]: `36 0D` = u32 n, n × {u32 instance, u32 quest id, u32 games
  done, i32 minutes left, u8 state (1 active, else a proposal), 3 pad}. Changing
  a quest sends `2B 00 <u32 active instance> <u32 chosen instance>`; the client
  re-queries `0D` after `34 2B`. Results go as important messages type 1:
  `#q![<instance>][<quest>]` done, `#!q[<instance>][<quest>]` failed, `#+q`
  new; the client re-queries money, reputation and quests.
- srp [C]: each unlocked faction offers one active quest from the band of the
  player's level with it (skipping those still cooling down), the rest of the
  band as proposals; a quest lives 24 h from when it is given; a swap is free.
  At a match's end every active quest the gear faction allows is checked;
  a completed one pays its awards, cools down for 120 min and is replaced.
