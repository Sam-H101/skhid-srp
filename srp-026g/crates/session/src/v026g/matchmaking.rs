//! What the 0.26g matchmaker knows of an account: its level, and the maps
//! it has played (`docs/notes/matchmaking.md`).
//!
//! **The level** is the client's own (it shows it in the lobby as the
//! "equipment level" and looks for a match around it; PROVEN, 0x2CA9B0 and
//! 0x2F79A0):
//!
//! 1. a kit's level is the highest `match_making_level` of what its 23
//!    slots hold, each times its slot's `slots_weight` (all 1 in the data),
//!    rounded; a stack counts only while it has units;
//! 2. the account's is the highest of its kits that can be played (the
//!    client skips a kit of state 2, the premium kit, which srp sends for
//!    none; only a kit that passes the Play check);
//! 3. the rating adds the `modifier_value` of the last `elo_modifiers` row
//!    whose `elo_min` it reaches;
//! 4. the sum is a row of `items_levels_modifiers`: its `item_level_min`
//!    is the level, and `item_level_min..=item_level_max` the levels the
//!    search starts with (the range the client shows grows from there by
//!    one at each end every search step it has waited, 0x225655).
//!
//! **The match's level** is an index into `match_levels_dict`, the reward
//! multipliers: [choice] the mean of its players' levels, rounded (the
//! client shows each player the multipliers of its own level, 0x2CAB60;
//! what the original server took for a match of mixed levels is not in the
//! client).
//!
//! **The map** is drawn, more likely the less the match's players have
//! played it ("the history of players' matches is taken into account now.
//! The system selects the less played map", patch notes 0.25a): a map is
//! a level (`level_01`..`level_09`), whatever its mode and weather.
//! [choice] Not always the least played one, or a new map would be in
//! every match until the players' counts caught up: each level of the pool
//! has a weight from its players' average matches on it ([`map_weight`]), a
//! level is drawn by weight, then a mode allowed for it, then the weather.

use survarium::v026g::dictionary::Dictionary;
use survarium::v026g::item_params::item_params;
use survarium::v026g::maps;

use super::account::{Account, Kit};
use super::rules;
use crate::Level;

/// The level of a kit: see the module, step 1.
pub fn kit_level(account: &Account, kit: &Kit) -> u8 {
    let dictionary = Dictionary::shared();
    let weights = &dictionary.match_maker.slots_weight;
    let highest = kit
        .slots
        .iter()
        .enumerate()
        .filter_map(|(slot, held)| {
            let held = held.as_ref()?;
            let item = account.item(held.item_id)?;
            // A stack with nothing in the slot or left of it is no gear
            // (0x2F7A26..0x2F7A3A).
            if dictionary.is_stack(item.dict_id)
                && (held.count == 0 || item.condition_or_stack == 0)
            {
                return None;
            }
            let level = item_params(item.dict_id)?.match_making_level;
            Some(f32::from(level) * weights.get(slot).copied().unwrap_or(1.0))
        })
        .fold(0.0_f32, f32::max);
    // The client adds 0.5 and floors (0x2F7A9E).
    (highest + 0.5).floor().clamp(0.0, 255.0) as u8
}

/// The highest level of the account's kits that can be played at `now`
/// (step 2); 0 with none.
pub fn gear_level(account: &Account, now: u64) -> u8 {
    account
        .kits
        .iter()
        .filter(|kit| rules::check_play(account, kit.id, now).is_ok())
        .map(|kit| kit_level(account, kit))
        .max()
        .unwrap_or(0)
}

/// What a rating adds to the level (step 3).
pub fn elo_modifier(rating: u16) -> u8 {
    let mut modifier = 0;
    for row in &Dictionary::shared().match_maker.elo_modifiers {
        if u32::from(rating) < row.elo_min {
            break;
        }
        modifier = row.modifier_value;
    }
    modifier.min(u32::from(u8::MAX)) as u8
}

/// The level the account is looked for a match at (step 4).
pub fn level(account: &Account, now: u64) -> Level {
    level_of(gear_level(account, now), account.rating)
}

/// [`level`] of a gear level and a rating.
pub fn level_of(gear_level: u8, rating: u16) -> Level {
    let rows = &Dictionary::shared().match_maker.items_levels_modifiers;
    let index = usize::from(gear_level) + usize::from(elo_modifier(rating));
    // The client indexes the rows unchecked; the data's 21 take the
    // highest gear (15) with the highest rating's modifier (5).
    match rows.get(index).or(rows.last()) {
        Some(row) => Level {
            min: row.item_level_min.min(u32::from(u8::MAX)) as u8,
            max: row.item_level_max.min(u32::from(u8::MAX)) as u8,
        },
        None => Level::default(),
    }
}

/// The level of a match of players of these levels: their mean, rounded,
/// as an index into `match_levels_dict`. 0 with no players.
pub fn match_level(levels: &[u8]) -> u16 {
    if levels.is_empty() {
        return 0;
    }
    let sum: usize = levels.iter().map(|&level| usize::from(level)).sum();
    let mean = (2 * sum + levels.len()) / (2 * levels.len());
    let last = Dictionary::shared().match_levels.len().saturating_sub(1);
    mean.min(last) as u16
}

/// How many matches the account was put in on the map `level`
/// (`maps::MapMode::level`, a number of the game data's).
pub fn map_plays(account: &Account, level: u8) -> u32 {
    account
        .map_plays
        .iter()
        .find(|(map, _)| *map == level)
        .map_or(0, |(_, plays)| *plays)
}

/// The account was put in a match on the map `level`.
pub fn count_map_play(account: &mut Account, level: u8) {
    match account.map_plays.iter_mut().find(|(map, _)| *map == level) {
        Some((_, plays)) => *plays = plays.saturating_add(1),
        None => {
            account.map_plays.push((level, 1));
            account.map_plays.sort_unstable_by_key(|(map, _)| *map);
        }
    }
}

/// `SRP_MATCH_MAP_SKEW` when unset: a map is at most twice as likely as
/// the one the match's players played most.
pub const DEFAULT_MAP_SKEW: f64 = 1.0;
/// The most `SRP_MATCH_MAP_SKEW` can be.
pub const MAX_MAP_SKEW: f64 = 4.0;
/// The matches per player behind the most played level at which a level's
/// weight is the most it gets.
const SKEW_RAMP: f64 = 5.0;
/// The weights are drawn as whole numbers of this many parts each.
const WEIGHT_SCALE: f64 = 1000.0;

/// The averages of the levels of a pool, each a level's matches per
/// player of a match: the least, the most and the mean of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoolPlays {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
}

/// How likely a level is drawn for a match, against the others of the
/// pool; `average` is its players' matches on it per player. The level
/// they played most has 1, one they played `k` fewer times each has
/// `1 + skew * k / 5`, and `1 + skew` from 5 behind on (`skew`:
/// `SRP_MATCH_MAP_SKEW`; 0: all alike).
pub fn map_weight(average: f64, pool: &PoolPlays, skew: f64) -> f64 {
    let behind = (pool.max - average).clamp(0.0, SKEW_RAMP);
    1.0 + skew.max(0.0) * behind / SKEW_RAMP
}

/// The weights of `levels` ([`map_weight`]) as whole numbers, each 1 or
/// more: `plays` is all the matches of a match's `players` on a level.
fn level_weights(levels: &[u8], plays: impl Fn(u8) -> u64, players: usize, skew: f64) -> Vec<u32> {
    let players = players.max(1) as f64;
    let averages: Vec<f64> = levels
        .iter()
        .map(|&level| plays(level) as f64 / players)
        .collect();
    let stats = PoolPlays {
        min: averages.iter().copied().fold(f64::INFINITY, f64::min),
        max: averages.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        mean: averages.iter().sum::<f64>() / averages.len().max(1) as f64,
    };
    averages
        .iter()
        .map(|&average| {
            // A weight under one part, or of no number, still leaves the
            // level a chance.
            let scaled = (map_weight(average, &stats, skew) * WEIGHT_SCALE).round();
            if scaled >= 1.0 {
                scaled.min(1e6) as u32
            } else {
                1
            }
        })
        .collect()
}

/// The `maps_dict` id of a match, of the ids in `pool` (playable ones; the
/// others are passed over): a level drawn by weight ([`map_weight`];
/// `plays` is all the matches of the match's `players` on a level), then a
/// mode the pool has for it, then one of that mode's ids (the weather).
/// `below(n)` draws a number under `n`. `None` when the pool has no
/// playable id. The ids are those of the game data ([`maps::all`]).
pub fn choose_map(
    pool: &[u8],
    plays: impl Fn(u8) -> u64,
    players: usize,
    skew: f64,
    below: impl FnMut(u32) -> u32,
) -> Option<u8> {
    choose_map_in(maps::all(), pool, plays, players, skew, below)
}

/// [`choose_map`] of the ids of `table` (a game data's,
/// [`maps::derive`]).
pub fn choose_map_in(
    table: &[maps::MapMode],
    pool: &[u8],
    plays: impl Fn(u8) -> u64,
    players: usize,
    skew: f64,
    mut below: impl FnMut(u32) -> u32,
) -> Option<u8> {
    let pool: Vec<&maps::MapMode> = pool
        .iter()
        .filter_map(|&id| table.iter().find(|map| map.id == id))
        .filter(|map| map.is_playable())
        .collect();
    let mut levels: Vec<u8> = pool.iter().map(|map| map.level).collect();
    levels.sort_unstable();
    levels.dedup();
    if levels.is_empty() {
        return None;
    }
    let weights = level_weights(&levels, plays, players, skew);
    let total: u32 = weights.iter().sum();
    let mut left = below(total) % total;
    let mut level = levels[levels.len() - 1];
    for (&each, &weight) in levels.iter().zip(&weights) {
        if left < weight {
            level = each;
            break;
        }
        left -= weight;
    }
    let mut draw = |count: usize| below(count as u32) as usize % count.max(1);
    let mut modes: Vec<u8> = pool
        .iter()
        .filter(|map| map.level == level)
        .map(|map| map.mode)
        .collect();
    modes.sort_unstable();
    modes.dedup();
    let mode = modes[draw(modes.len())];
    let ids: Vec<u8> = pool
        .iter()
        .filter(|map| (map.level, map.mode) == (level, mode))
        .map(|map| map.id)
        .collect();
    Some(ids[draw(ids.len())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v026g::account::KitSlot;
    use std::collections::BTreeMap;

    /// A dictionary item by its name.
    fn dict_id(name: &str) -> u16 {
        Dictionary::shared()
            .items
            .iter()
            .find(|item| item.name == name)
            .unwrap_or_else(|| panic!("no item {name}"))
            .dict_id
    }

    fn put(account: &mut Account, kit: usize, slot: usize, name: &str, count: u32) -> u32 {
        let item_id = account.add_item(dict_id(name), count);
        account.kits[kit].slots[slot] = Some(KitSlot { item_id, count });
        item_id
    }

    #[test]
    fn a_new_account_is_level_0() {
        // The TT-33 and the Mosin carbine "have level zero" (KB 317).
        let account = Account::new("Alice");
        assert_eq!(gear_level(&account, 0), 0);
        assert_eq!(level(&account, 0), Level { min: 0, max: 1 });
    }

    #[test]
    fn a_kit_is_as_high_as_its_highest_item() {
        let mut account = Account::new("Alice");
        assert_eq!(
            item_params(dict_id("ak_74m")).unwrap().match_making_level,
            10
        );
        // A second weapon of level 10 beside the starter's of level 0.
        put(&mut account, 0, 10, "ak_74m", 1);
        put(&mut account, 0, 11, "ammo_5.45x39_7n10", 30);
        assert_eq!(kit_level(&account, &account.kits[0]), 10);
        assert_eq!(gear_level(&account, 0), 10);
        assert_eq!(level(&account, 0), Level { min: 10, max: 11 });
    }

    #[test]
    fn ammo_counts_while_the_slot_has_rounds() {
        let mut account = Account::new("Alice");
        let level_0 = kit_level(&account, &account.kits[0]);
        assert_eq!(level_0, 0);
        // The Mosin's second ammo slot: ammo of level 5.
        assert_eq!(
            item_params(dict_id("ammo_5.45x39_7n6m"))
                .unwrap()
                .match_making_level,
            5
        );
        let ammo = put(&mut account, 0, 9, "ammo_5.45x39_7n6m", 30);
        assert_eq!(kit_level(&account, &account.kits[0]), 5);
        // None in the slot, or none left of the stack: no gear.
        account.kits[0].slots[9].as_mut().unwrap().count = 0;
        assert_eq!(kit_level(&account, &account.kits[0]), 0);
        account.kits[0].slots[9].as_mut().unwrap().count = 30;
        account.item_mut(ammo).unwrap().condition_or_stack = 0;
        assert_eq!(kit_level(&account, &account.kits[0]), 0);
    }

    #[test]
    fn the_highest_kit_that_can_be_played_is_the_accounts() {
        let mut account = Account::new("Alice");
        // The second kit: a level 10 rifle with rounds. The first stays 0.
        put(&mut account, 1, 7, "ak_74m", 1);
        put(&mut account, 1, 8, "ammo_5.45x39_7n10", 30);
        assert_eq!(kit_level(&account, &account.kits[0]), 0);
        assert_eq!(gear_level(&account, 0), 10);
        // Without rounds it can not be played, and does not count.
        account.kits[1].slots[8] = None;
        assert!(rules::check_play(&account, account.kits[1].id, 0).is_err());
        assert_eq!(gear_level(&account, 0), 0);
        // The third kit counts without a premium account.
        put(&mut account, 2, 7, "ak_74m", 1);
        put(&mut account, 2, 8, "ammo_5.45x39_7n10", 30);
        assert_eq!(gear_level(&account, 100), 10);
    }

    #[test]
    fn the_rating_moves_the_level_up_by_the_datas_rows() {
        assert_eq!(elo_modifier(0), 0);
        assert_eq!(elo_modifier(100), 0);
        assert_eq!(elo_modifier(10_000), 0);
        assert_eq!(elo_modifier(10_001), 1);
        assert_eq!(elo_modifier(12_500), 3);
        assert_eq!(elo_modifier(14_001), 5);
        assert_eq!(elo_modifier(u16::MAX), 5);

        assert_eq!(level_of(0, 100), Level { min: 0, max: 1 });
        assert_eq!(level_of(3, 11_500), Level { min: 5, max: 6 });
        assert_eq!(level_of(14, 100), Level { min: 14, max: 15 });
        // The rows past 15 are all of item level 15.
        assert_eq!(level_of(15, 100), Level { min: 15, max: 15 });
        assert_eq!(level_of(13, 14_500), Level { min: 15, max: 15 });
        assert_eq!(level_of(15, 15_000), Level { min: 15, max: 15 });
        assert_eq!(level_of(200, 15_000), Level { min: 15, max: 15 });
    }

    #[test]
    fn a_match_is_of_its_players_mean_level() {
        assert_eq!(match_level(&[]), 0);
        assert_eq!(match_level(&[0, 0]), 0);
        assert_eq!(match_level(&[7]), 7);
        assert_eq!(match_level(&[0, 15]), 8);
        assert_eq!(match_level(&[0, 0, 1]), 0);
        assert_eq!(match_level(&[0, 1, 1]), 1);
        assert_eq!(match_level(&[15; 16]), 15);
        // Never past the multipliers' last row.
        assert_eq!(match_level(&[200, 200]), 20);
        assert!(
            Dictionary::shared()
                .match_level(usize::from(match_level(&[15, 15])))
                .is_some()
        );
    }

    #[test]
    fn map_plays_are_counted_by_map() {
        let mut account = Account::new("Alice");
        assert_eq!(map_plays(&account, 3), 0);
        count_map_play(&mut account, 3);
        count_map_play(&mut account, 1);
        count_map_play(&mut account, 3);
        assert_eq!(account.map_plays, [(1, 1), (3, 2)]);
        assert_eq!((map_plays(&account, 3), map_plays(&account, 8)), (2, 0));
    }

    fn all_maps() -> Vec<u8> {
        maps::playable().map(|map| map.id).collect()
    }

    /// The levels of `pool`, as `choose_map` draws them.
    fn levels_of(pool: &[u8]) -> Vec<u8> {
        let mut levels: Vec<u8> = pool
            .iter()
            .filter_map(|&id| maps::map_mode(id))
            .map(|map| map.level)
            .collect();
        levels.sort_unstable();
        levels.dedup();
        levels
    }

    /// How often `choose_map` gives each level over every number the
    /// level's draw can take (the mode's and weather's take 0).
    fn level_counts(
        pool: &[u8],
        plays: &dyn Fn(u8) -> u64,
        players: usize,
        skew: f64,
    ) -> BTreeMap<u8, u32> {
        let mut total = 0;
        choose_map(pool, plays, players, skew, |bound| {
            total = total.max(bound);
            0
        });
        let mut counts = BTreeMap::new();
        for number in 0..total {
            let mut first = true;
            let below = |_| {
                if std::mem::take(&mut first) {
                    number
                } else {
                    0
                }
            };
            let id = choose_map(pool, plays, players, skew, below).unwrap();
            *counts.entry(maps::map_mode(id).unwrap().level).or_default() += 1;
        }
        counts
    }

    /// A level's share of the draw, from the weights of `levels`.
    fn share(levels: &[u8], plays: &dyn Fn(u8) -> u64, players: usize, skew: f64, of: u8) -> f64 {
        let weights = level_weights(levels, plays, players, skew);
        let total: u32 = weights.iter().sum();
        let at = levels.iter().position(|&level| level == of).unwrap();
        f64::from(weights[at]) / f64::from(total)
    }

    const LONDON: u8 = 9;
    const NINE_LEVELS: [u8; 9] = [1, 2, 3, 4, 5, 6, 7, 8, LONDON];

    #[test]
    fn a_weight_grows_over_five_plays_behind_the_most_played_level() {
        let pool = PoolPlays {
            min: 0.0,
            max: 10.0,
            mean: 8.0,
        };
        let weight = |average: f64, skew: f64| map_weight(average, &pool, skew);
        assert_eq!(weight(10.0, DEFAULT_MAP_SKEW), 1.0);
        assert!((weight(9.0, DEFAULT_MAP_SKEW) - 1.2).abs() < 1e-9);
        assert!((weight(9.5, DEFAULT_MAP_SKEW) - 1.1).abs() < 1e-9);
        assert_eq!(weight(5.0, DEFAULT_MAP_SKEW), 2.0);
        assert_eq!(weight(0.0, DEFAULT_MAP_SKEW), 2.0);
        assert_eq!(weight(0.0, MAX_MAP_SKEW), 5.0);
        assert_eq!(weight(0.0, 0.0), 1.0);
    }

    #[test]
    fn levels_played_alike_are_drawn_alike() {
        let pool = all_maps();
        for skew in [0.0, DEFAULT_MAP_SKEW, MAX_MAP_SKEW] {
            let counts = level_counts(&pool, &|_| 12, 2, skew);
            assert_eq!(counts.keys().copied().collect::<Vec<_>>(), levels_of(&pool));
            assert!(
                counts.values().all(|&count| count == 1000),
                "{skew}: {counts:?}"
            );
        }
        assert!(
            (share(&NINE_LEVELS, &|_| 5, 1, DEFAULT_MAP_SKEW, LONDON) - 1.0 / 9.0).abs() < 1e-9
        );
    }

    #[test]
    fn a_level_far_behind_is_twice_as_likely() {
        // The newest level: 10 plays behind for each of 2 players.
        let pool = all_maps();
        let levels = levels_of(&pool);
        let new = *levels.last().unwrap();
        let plays = move |level: u8| if level == new { 0 } else { 20 };
        let counts = level_counts(&pool, &plays, 2, DEFAULT_MAP_SKEW);
        for (&level, &count) in &counts {
            assert_eq!(
                count,
                if level == new { 2000 } else { 1000 },
                "level {level}"
            );
        }
        // With London the ninth: 2 / 10, from 5 behind on.
        let london = |behind: u64| {
            move |level: u8| {
                if level == LONDON {
                    4 * (6 - behind)
                } else {
                    24
                }
            }
        };
        for behind in [5, 6] {
            let share = share(&NINE_LEVELS, &london(behind), 4, DEFAULT_MAP_SKEW, LONDON);
            assert!((share - 0.2).abs() < 1e-9, "{behind} behind: {share}");
        }
        // 1 behind: 1.2 / 9.2; 3 behind: 1.6 / 9.6.
        let one = share(&NINE_LEVELS, &london(1), 4, DEFAULT_MAP_SKEW, LONDON);
        assert!((one - 1.2 / 9.2).abs() < 1e-9, "{one}");
        let three = share(&NINE_LEVELS, &london(3), 4, DEFAULT_MAP_SKEW, LONDON);
        assert!((three - 1.6 / 9.6).abs() < 1e-9, "{three}");
        // The most skew: 5 / 13.
        let most = share(&NINE_LEVELS, &london(5), 4, MAX_MAP_SKEW, LONDON);
        assert!((most - 5.0 / 13.0).abs() < 1e-9, "{most}");
    }

    #[test]
    fn averages_are_per_player_and_fractional() {
        // 3 players, 2 matches fewer among them: 2/3 behind, 1 + 2/15.
        let plays = |level: u8| if level == LONDON { 28 } else { 30 };
        let weights = level_weights(&NINE_LEVELS, plays, 3, DEFAULT_MAP_SKEW);
        assert_eq!(weights[..8], [1000; 8]);
        assert_eq!(weights[8], 1133);
        // The same counts of 30 players are hardly a difference.
        let weights = level_weights(&NINE_LEVELS, plays, 30, DEFAULT_MAP_SKEW);
        assert_eq!(weights[8], 1013);
    }

    #[test]
    fn no_skew_is_a_uniform_draw() {
        let pool = all_maps();
        let plays = |level: u8| u64::from(level) * 100;
        let counts = level_counts(&pool, &plays, 1, 0.0);
        assert!(counts.values().all(|&count| count == 1000), "{counts:?}");
    }

    #[test]
    fn the_share_of_a_new_map_over_many_draws() {
        // London unplayed, the others 6 matches a player ahead.
        let plays = |level: u8| if level == LONDON { 0 } else { 60 };
        let mut state = 7_u32;
        let mut next = |bound: u32| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 8) % bound.max(1)
        };
        let weights = level_weights(&NINE_LEVELS, plays, 10, DEFAULT_MAP_SKEW);
        let total: u32 = weights.iter().sum();
        let draws = 20_000;
        let mut london = 0;
        for _ in 0..draws {
            let mut left = next(total);
            let at = weights.iter().position(|&weight| {
                let hit = left < weight;
                left = left.saturating_sub(weight);
                hit
            });
            london += u32::from(at == Some(8));
        }
        let share = f64::from(london) / f64::from(draws);
        assert!((share - 0.2).abs() < 0.015, "{share}");
        // And through choose_map, on the pool's own newest level.
        let pool = all_maps();
        let new = *levels_of(&pool).last().unwrap();
        let plays = move |level: u8| if level == new { 0 } else { 60 };
        let draws = 20_000;
        let picked = (0..draws)
            .filter(|_| {
                let id = choose_map(&pool, plays, 10, DEFAULT_MAP_SKEW, &mut next).unwrap();
                maps::map_mode(id).unwrap().level == new
            })
            .count();
        let expected = 2.0 / (2.0 + (levels_of(&pool).len() - 1) as f64);
        let share = picked as f64 / f64::from(draws);
        assert!(
            (share - expected).abs() < 0.015,
            "{share} against {expected}"
        );
    }

    #[test]
    fn no_level_is_ever_impossible() {
        let pool = all_maps();
        let levels = levels_of(&pool);
        for skew in [0.0, DEFAULT_MAP_SKEW, MAX_MAP_SKEW] {
            // One level played far more than all the others.
            let plays = |level: u8| if level == levels[0] { 1_000_000 } else { 0 };
            let counts = level_counts(&pool, &plays, 1, skew);
            assert_eq!(counts.keys().copied().collect::<Vec<_>>(), levels, "{skew}");
            assert!(
                counts.values().all(|&count| count >= 1000),
                "{skew}: {counts:?}"
            );
        }
        // No players, or no matches: still every level.
        let counts = level_counts(&pool, &|_| 0, 0, DEFAULT_MAP_SKEW);
        assert_eq!(counts.len(), levels.len());
    }

    #[test]
    fn the_mode_and_weather_are_drawn_within_the_pool() {
        // Nothing played: every map, mode and weather of the pool comes up.
        let pool = all_maps();
        let mut state = 7_u32;
        let mut next = |bound: u32| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 8) % bound.max(1)
        };
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..4000 {
            seen.insert(choose_map(&pool, |_| 0, 1, DEFAULT_MAP_SKEW, &mut next).unwrap());
        }
        assert_eq!(seen.len(), pool.len());

        // The operator's pool: only Team Deathmatch on Rudnya and School.
        let pool: Vec<u8> = maps::playable_in(maps::mode::TEAM_DEATHMATCH)
            .filter(|map| map.level <= 2)
            .map(|map| map.id)
            .collect();
        // School was played less, also counting maps outside the pool.
        let plays = |level: u8| if level == 2 || level == 5 { 0 } else { 5 };
        for _ in 0..200 {
            let id = choose_map(&pool, plays, 1, DEFAULT_MAP_SKEW, &mut next).unwrap();
            let map = maps::map_mode(id).unwrap();
            assert_eq!(map.mode, maps::mode::TEAM_DEATHMATCH);
            assert!(pool.contains(&id));
        }
        let counts = level_counts(&pool, &plays, 1, DEFAULT_MAP_SKEW);
        assert_eq!(
            counts.into_iter().collect::<Vec<_>>(),
            [(1, 1000), (2, 2000)]
        );
    }

    #[test]
    fn a_map_of_a_patch_is_drawn_like_any_other() {
        // Game data with a Team Deathmatch map of a level 12, which no code
        // names (`maps::fixtures`).
        use maps::fixtures::{AddedMap, table_of, with_map};
        let added = AddedMap::level_12_tdm();
        let table = table_of(&with_map(&added)).unwrap();
        let in_mode = |game_mode: u8| -> Vec<u8> {
            table
                .iter()
                .filter(|map| map.is_playable() && map.mode == game_mode)
                .map(|map| map.id)
                .collect()
        };
        let tdm = in_mode(maps::mode::TEAM_DEATHMATCH);
        assert!(tdm.contains(&added.id));
        // Its players never played it: it is drawn, and the likeliest
        // level, all others played as much.
        let plays = |level: u8| if level == 12 { 0 } else { 10 };
        let mut total = 0;
        choose_map_in(&table, &tdm, plays, 1, DEFAULT_MAP_SKEW, |bound| {
            total = total.max(bound);
            0
        });
        let mut drawn = BTreeMap::<u8, u32>::new();
        for number in 0..total {
            let mut first = true;
            let below = |_| {
                if std::mem::take(&mut first) {
                    number
                } else {
                    0
                }
            };
            let id = choose_map_in(&table, &tdm, plays, 1, DEFAULT_MAP_SKEW, below).unwrap();
            let map = table.iter().find(|map| map.id == id).unwrap();
            *drawn.entry(map.level).or_default() += 1;
        }
        assert_eq!(drawn.len(), 9, "{drawn:?}");
        assert_eq!(drawn[&12], 2000);
        assert!(
            drawn
                .iter()
                .all(|(&level, &count)| level == 12 || count == 1000)
        );
        // A pool of the batteries has no level 12: it is Team Deathmatch only.
        let batteries = in_mode(maps::mode::BATTERIES);
        assert!(!batteries.contains(&added.id));
        for number in 0..100 {
            let id =
                choose_map_in(&table, &batteries, |_| 0, 1, 0.0, |bound| number % bound).unwrap();
            assert_ne!(table.iter().find(|map| map.id == id).unwrap().level, 12);
        }
        // The data without the map does not know its id.
        assert_eq!(
            choose_map(&[added.id], |_| 0, 1, DEFAULT_MAP_SKEW, |_| 0),
            None
        );
    }

    #[test]
    fn a_pool_with_no_playable_map_gives_none() {
        let choose = |pool: &[u8]| choose_map(pool, |_| 0, 1, DEFAULT_MAP_SKEW, |_| 0);
        assert_eq!(choose(&[]), None);
        // 107 is a night config without a project; 141 the loopback one.
        assert_eq!(choose(&[107, 141]), None);
        assert_eq!(choose(&[107, 61]), Some(61));
    }
}
