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
//! 2. the account's is the highest of its kits that can be played (not the
//!    premium kit without a premium account, and only a kit that passes the
//!    Play check);
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
//! **The map** is the one the match's players have played least ("the
//! history of players' matches is taken into account now. The system
//! selects the less played map", patch notes 0.25a): a map is a level
//! (`level_01`..`level_08`), whatever its mode and weather; the mode is
//! drawn among the modes allowed for it, then the weather.

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
/// (`maps::MapMode::level`, 1..=8).
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

/// The `maps_dict` id of a match, of the ids in `pool` (playable ones; the
/// others are passed over): the map its players played least, `plays`
/// being all of theirs on a map; of several such maps one is drawn, then a
/// mode the pool has for it, then one of that mode's ids (the weather).
/// `below(n)` draws a number under `n`. `None` when the pool has no
/// playable id.
pub fn choose_map(
    pool: &[u8],
    plays: impl Fn(u8) -> u64,
    mut below: impl FnMut(u32) -> u32,
) -> Option<u8> {
    let pool: Vec<&maps::MapMode> = pool
        .iter()
        .filter_map(|&id| maps::map_mode(id))
        .filter(|map| map.is_playable())
        .collect();
    let mut draw = |count: usize| below(count as u32) as usize % count.max(1);
    let mut levels: Vec<u8> = pool.iter().map(|map| map.level).collect();
    levels.sort_unstable();
    levels.dedup();
    let least = levels.iter().map(|&level| plays(level)).min()?;
    levels.retain(|&level| plays(level) == least);
    let level = levels[draw(levels.len())];
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
        // The premium kit counts only with a premium account.
        put(&mut account, 2, 7, "ak_74m", 1);
        put(&mut account, 2, 8, "ammo_5.45x39_7n10", 30);
        assert_eq!(gear_level(&account, 100), 0);
        account.premium_until = 1000;
        assert_eq!(gear_level(&account, 100), 10);
        assert_eq!(gear_level(&account, 2000), 0);
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

    #[test]
    fn the_least_played_map_is_chosen() {
        let pool = all_maps();
        // Every map played but the Chemical Plant (level 4).
        let plays = |level: u8| if level == 4 { 0 } else { 3 };
        for draw in 0..40 {
            let id = choose_map(&pool, plays, |bound| draw % bound.max(1)).unwrap();
            assert_eq!(maps::map_mode(id).unwrap().level, 4, "draw {draw}");
        }
        // Two maps played equally little: either, by the draw.
        let plays = |level: u8| if level == 2 || level == 7 { 1 } else { 9 };
        let mut seen = std::collections::BTreeSet::new();
        for draw in 0..40 {
            let id = choose_map(&pool, plays, |bound| draw % bound.max(1)).unwrap();
            seen.insert(maps::map_mode(id).unwrap().level);
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), [2, 7]);
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
            seen.insert(choose_map(&pool, |_| 0, &mut next).unwrap());
        }
        assert_eq!(seen.len(), pool.len());

        // The operator's pool: only Team Deathmatch on Rudnya and School.
        let pool: Vec<u8> = maps::playable_in(maps::mode::TEAM_DEATHMATCH)
            .filter(|map| map.level <= 2)
            .map(|map| map.id)
            .collect();
        for draw in 0..50 {
            // School was played less, also counting maps outside the pool.
            let plays = |level: u8| u64::from(level != 2 && level != 5);
            let id = choose_map(&pool, plays, |bound| draw % bound.max(1)).unwrap();
            let map = maps::map_mode(id).unwrap();
            assert_eq!((map.level, map.mode), (2, maps::mode::TEAM_DEATHMATCH));
            assert!(pool.contains(&id));
        }
    }

    #[test]
    fn a_pool_with_no_playable_map_gives_none() {
        assert_eq!(choose_map(&[], |_| 0, |_| 0), None);
        // 107 is a night config without a project; 141 the loopback one.
        assert_eq!(choose_map(&[107, 141], |_| 0, |_| 0), None);
        assert!(choose_map(&[107, 61], |_| 0, |_| 0).is_some());
    }
}
