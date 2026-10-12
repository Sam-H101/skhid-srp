# 0.26g matchmaking: levels, teams, maps

**Version scope:** client 0.26g build 2777 (no PDB; VAs are image VAs, base 0x10000) and srp's
0.26g lobby. Written 2026-10-12.

Tags: **PROVEN** = read in the exe (VA cited), the data or the lobby SWF. **LIKELY** / **GUESS**
= inference. **choice** = srp's own rule, where the client and the sources of the time say
nothing.

Code: `crates/session/src/v026g/matchmaking.rs` (level, match level, map choice, map counts),
`crates/session/src/matchmaking.rs` (`LevelRules`, `queue_leveled`, `poll_with`: the queue, the
range, the sides), `servers/lobby-server/src/v026g/play.rs` (Play, queue screen, making the
match), `crates/session/src/v026g/persist/map_plays.rs` (`g26_map_plays`, schema step 5).

Sources of the time: `progression_sources.md` §9.

---

## 1. A player's level: the client computes it itself — PROVEN

The lobby shows an "equipment level" and looks for a match around it. The client computes both
from its own account and the static dictionary, so the server must compute the same or the
search screen lies.

`0x228E60` (lobby UI) calls `0x2CA9B0(account, dictionary, u16 rating, &min, &max)`, stores the
two results at ui+0x312C / +0x3130 and calls `root.set_equipment_level(min)` (string 0xA02150).
The rating is the u16 at ui+0x3128, written by the setter at 0x2274B0 (LIKELY from the account's
own ranking row, `90`).

`0x2CA9B0`:

1. For each kit of the account (count at account+0x57, stride 0x704): skip a kit whose state is
   2 (the premium kit without premium) and a kit that fails the Play check `0x2F7DA0` (the same
   check as before Play: `rules::check_play`). Of the rest take the **highest**
   `0x2F79A0(kit)` (0x2CA9C7..0x2CAA42).
2. `match_maker_defines.elo_modifiers`: walk the rows in order while `rating >= elo_min`; the
   modifier is the `modifier_value` of the last row reached, 0 if none (0x2CAA44..0x2CAACD).
3. Row `kit level + modifier` of `match_maker_defines.items_levels_modifiers` (no bounds check):
   `min = item_level_min`, `max = item_level_max` (0x2CAACF..0x2CAB4B).

`0x2F79A0` (a kit's level): for each of the 23 slots (0x18 × 23 = 0x228) that holds an item:
if the item is a stack (dictionary entry +0x22A), skip it when the slot's count or the item's
own count is 0; else value = `match_making_level` of the item (dictionary entry byte +0x385)
× `slots_weight[slot]`; keep the highest; return `floor(highest + 0.5)` (0x2F7A9E, the constant
0.5 at 0x927A44).

The data (`gameplay/db_static_dictionaries`):

- `match_making_level` equals `item_level` for all 384 items (0..=16; 16 only for two decals,
  which no slot holds).
- `slots_weight`: 23 × 1. So a kit's level is its highest item's, **ammo, grenades and drugs
  included** (patch notes 0.23g: the ammo's level counts).
- `elo_modifiers`: 100..10000 → 0, 10001..11000 → 1, … 14001..15000 → 5. A rating under 10001
  changes nothing; srp's ratings start at 100.
- `items_levels_modifiers`: row *n* for *n* ≤ 14 is `min n, max n + 1`; rows 15..20 are all
  `min 15, max 15`. So the level is 0..=15, and a player starts its search at its own level and
  the next ("Reduced starting level range in matchmaking", 0.25b, fits a start of one level).

This is what "если вы создадите профили с разным уровнем экипировки, то противники будут
искаться по максимальному уровню экипировки" (preview of 0.26) describes.

srp: `matchmaking::kit_level`, `gear_level`, `elo_modifier`, `level` / `level_of` → `Level {
min, max }`. A new account (TT-33, Mosin carbine) is level 0, range 0..=1.

## 2. The search screen — PROVEN (what it shows), GUESS (the columns' meaning)

`0x85` status 1 carries `u16 search step` and `20 × u16 search properties` (lobby.md §2.4).

- `0x22564D..0x225748`: `w = current_wait / search_step`;
  `root.set_mm_search_range(max(0, min − w), max + w)` with the `min` / `max` of §1. The SWF
  (`LobbyGameSearch.setSearchRange`) writes "search range: a–b" and lights the columns in it.
  **So in the original a waiting player's range grew by one level at each end every search
  step.** The step's value is the server's; not in the client's data.
- `root.set_mm_search_properties([{level: i, value}; 20])` (0x2254FE..0x225648; SWF
  `setSearchProperties`): 20 columns over levels 0..19, heights relative to the highest value
  (at least 30), the value shown on hover. GUESS: the players searching at each level. srp
  sends that count (a level over 19 would count as 19; there is none).
- `0x222D1F` calls `0x2CAB60(level = min)`: the three multipliers of `match_levels_dict[min]`
  (`money_k`, `experience_k`, `reputation_k`), shown in the lobby. So the client tells a player
  the multipliers of **its own** level.

## 3. The queue's rules in srp

When a match starts is 0.21d's rule (the owner's: no blocking on player counts) but for squads
(below): two parties, the gather window from the second (`SRP_MATCH_GATHER_SECONDS`, 60), sooner
with `SRP_MATCH_FILL_PLAYERS` (10) players waiting and nobody new for `SRP_MATCH_QUIET_SECONDS`
(10), at once when full. A player alone waits, and so does a squad alone.

What is new:

| Rule | Tag |
|---|---|
| A match is two sides of at most `SRP_MATCH_TEAM_SIZE` players (8; 1..=10) | 8: wiki of Feb 2015, "two teams of up to 8 players" |
| A squad stays on one side; the sides are as even as the squads allow, first in players, then in the sum of levels | choice |
| **A squad is one party, and a match needs two parties**: two players alone, or a squad and at least one player who is not of it (alone, or of another squad). The gather window starts with the second party | the owner's rule, 2026-10-12 |
| A squad is looked for by its highest member | LIKELY (as the kits of one account are) |
| A match is formed around a waiting player (the longest waiting first) from the players whose level is in its range: `min − w ..= max + w`, `w = its wait / step` | the range: PROVEN as what the client shows; that the server matched by it: LIKELY |
| The step is `SRP_MATCH_SEARCH_STEP_SECONDS`, default 4 | choice |
| The start conditions above are counted over the players in range | choice |
| With more in range than a match holds, the nearest levels go, then who came first | choice |

**Nobody waits for ever.** The range grows without limit, so after `14 × step` seconds (56 s by
default) every level is in the range of whoever waits. With the default step that is inside the
default gather window: a server with a few players of any levels starts its matches exactly when
it did before levels counted. An operator of a full server raises the step (60 would show the
range growing by one a minute).

Only the range of the player the match is formed around is looked at; two others in it may be
further from each other than from it.

**A squad alone is not given a match** (since 2026-10-12). Until then the count was of players,
as in 0.21d: two squadmates alone in the queue were "two players waiting", got a match after the
60 s with both on one team and nobody against them, and left at once (live, matches 0x7f58348f
and 0x5438c90e). Now a squad counts once (`Group::parties`), so it waits as a player alone does,
for as long as it takes; one more player in its range of levels, alone or of another squad, is
enough, and the window of 60 s starts when that player joins. It does not wait for a second
squad (the owner's word: that might never come). The rule is the queue's, so
0.21d's entry in this tree (`Matchmaking::queue`, without levels) has it too, and
`srp-021d` got the same change the same day (its `WORK.md`). Tests in
`matchmaking.rs`: `a_squad_alone_waits_for_somebody_who_is_not_of_it`,
`a_squad_is_left_waiting_when_the_other_side_leaves`,
`a_squad_alone_gets_no_match_by_levels_either`,
`a_squad_is_not_matched_with_itself_while_another_is_out_of_its_levels`; the 0.21d
lobby's `a_squad_queues_when_its_last_member_plays`, which said a squad of two is
placed alone, is rewritten.

What the squad's clients see while they wait is the search screen, as for anybody: nothing in
the client tells a squad from a player there (§2).

`N` (static info's roster size) is the players really put in the match, as before: the client
starts its countdown at 80 % of it (modes_maps.md §10).

Clan matches (`seat_teams`) are not touched: their sides are the clan teams
(`SRP_CLAN_TEAM_MAX`).

## 4. The match's level — choice

`MatchSetup::match_level` (→ static info's `u16 match_level`, which the client only logs; the
rewards' row of `match_levels_dict`, results.md) was 0. It is now **the mean of the levels
(§1's `min`) of the players put in the match, rounded half up**, 0..=15 (never past the dict's
last row, 20).

What the original took for a match of mixed levels is not in the client. The mean keeps the
client's promise (§2: each player is shown the multipliers of its own level) exactly when the
players are of one level, which is what the original's narrow range gave.

## 5. The map — official source, the counting is srp's

"The history of players' matches is taken into account now. The system selects the less played
map" (patch notes 0.25a). No mode choice in 0.26 (added in 0.27a).

- A map is a level (`match_making_params.map_id`, 1..=8), whatever the mode and weather.
- Each account counts the matches it was **put in** on each map (`Account::map_plays`, table
  `g26_map_plays`), counted when the match is made at the match server. [choice: not when it
  ends; a player who leaves early has still seen the map.]
- The match's map is the one with the smallest sum over its players, among the maps of the
  operator's pool (`SRP_MATCH_MODES`, `SRP_MATCH_MAPS`); of several, one at random. Then a mode
  at random among the pool's modes for that map, then one of that mode's ids (the weather).

So two players who play on see all eight maps before one comes up again.

## 6. Database

Schema step 5 of the `g26_*` tables (`g26_meta`): `g26_map_plays (account_id, map, plays)`,
primary key `(account_id, map)`. Additive; an account of before has no rows (has played
nothing). Tested from version 4.

## 7. Not done / open

- The original's step and whether its range was symmetric in practice: unknown; the client only
  shows what §2 says.
- Whether `match_levels_dict` was taken per match or per player by the original server: unknown
  (§4).
- New-player protection in the matchmaker (the account's `protection` byte): not used.
- Nothing here was seen in a live client: the search screen's range and columns want a look.

## Correction (2026-10-12): a seat is kept for a game that broke, not for a player who left

Where this note says a seat is kept for the whole match and Play sends a player back to it, read: for a player whose game or link broke. A player who left by its own will (0xE0 on the match link, match.md §25) has given the seat up and is queued like anybody (`Matchmaking::give_up_seat`).
