//! What srp needs from each 0.26g item's own config
//! (`gameplay/items/**.options` in 0.26g's `resources.db`;
//! `docs/notes/data.md` §4): its level and matchmaking level,
//! faction, weight, rarity, wear and repair values, and for ammo its clip.
//! The table is generated from the game's files by
//! `scripts/gen-data-026g/gen_data.py`; an item without a field has 0.
//!
//! Against 0.21d: `drop_value` and `clips_in_bag` are gone (ammo has
//! `required_bags`), and `item_level` came here from the dictionary.

mod table;

pub use table::{
    AMMO_BAGS_COUNT, ITEM_PARAMS, MAX_CARRIED_WEIGHT, MIN_CARRIED_WEIGHT, POCKET_SLOT_SIZE,
    POCKET_SLOTS_COUNT,
};

/// One item's parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemParams {
    pub dict_id: u16,
    /// `type`: `wpn_aslt`, `arm_tors`, `ammo`, `drugs`, `trap`, `grenade`,
    /// `booster`, `drp_camo`, `vch_fixr`, ...; empty for artefacts, scopes
    /// and money.
    pub kind: &'static str,
    /// 0 none, 1..=4 the lobby factions.
    pub faction: u8,
    pub faction_affinity: u32,
    /// 0..=16.
    pub item_level: u8,
    pub match_making_level: u8,
    /// Cosmetics only.
    pub base_rarity_level: u8,
    /// `rarity` of an item that is no stack: its rarity levels (weapons and
    /// armour `[0, 1, 2, 3]`, cosmetics `[0]`).
    pub gear_rarity: &'static [u8],
    /// `rarity` of a stack: `(least, most)` units, one pair but for money
    /// (five, [`super::drops`]).
    pub stack_rarity: &'static [(u32, u32)],
    /// Kilograms; ammo has `clip_weight` instead.
    pub weight: f32,
    pub drop_weight: u32,
    /// Condition lost a use, a match and a death, as a fraction of 1.
    pub degrade_per_use: f32,
    pub degrade_per_match: f32,
    pub degrade_per_death: f32,
    pub reliability: f32,
    /// The share of the item's cost a full repair costs.
    pub repair_cost_ratio: f32,
    /// Ammo: the rounds of a clip and its weight, and the ammo bags it takes.
    pub clip_size: u32,
    pub clip_weight: f32,
    pub required_bags: u32,
    /// Weapons: the rounds a magazine holds.
    pub magazine_capacity: u32,
    pub double_handed: bool,
    pub fills_whole_pocket: bool,
    /// Cosmetics: the modification that puts it on an item.
    pub modification: u32,
    /// The modifications the item always has.
    pub default_modifications: &'static [u32],
}

/// The parameters of `dict_id`, if it is an item.
pub fn item_params(dict_id: u16) -> Option<&'static ItemParams> {
    ITEM_PARAMS
        .binary_search_by_key(&dict_id, |item| item.dict_id)
        .ok()
        .map(|index| &ITEM_PARAMS[index])
}

/// The units of `dict_id` one step of the shop's slider buys or sells: a
/// clip of ammo (`clip_size`), else one item. The 0.26g lobby shows a price
/// for such a step: `MessageAsk.onSliderUpdate` is
/// `slider / step * price` with `step = clip_size` (PROVEN,
/// `lobby_menu.swf`).
pub fn shop_unit(dict_id: u16) -> u32 {
    item_params(dict_id)
        .map(|params| params.clip_size)
        .filter(|&size| size > 0)
        .unwrap_or(1)
}

/// What `count` of `dict_id` weighs in a kit, as the lobby counts it:
/// ammo `count / clip_size * weight`, any other stack `weight * count`
/// (PROVEN, `lobby_menu.swf` `AmmoShareControll.as` 211, 217). That ammo's
/// `weight` there is the config's `clip_weight` is LIKELY: ammo configs have
/// no `weight`, and the exe's item descriptor fill reads `clip_weight` beside
/// `clip_size` (0x286BA1). An item that is no stack weighs its `weight`
/// whatever its condition: pass 1.
pub fn carried_weight(dict_id: u16, count: u32) -> f32 {
    let Some(params) = item_params(dict_id) else {
        return 0.0;
    };
    if params.clip_size > 0 {
        count as f32 / params.clip_size as f32 * params.clip_weight
    } else {
        params.weight * count as f32
    }
}

#[cfg(test)]
mod tests {
    use super::super::dictionary::Dictionary;
    use super::*;

    #[test]
    fn every_dictionary_item_has_parameters() {
        let dictionary = Dictionary::shared();
        assert_eq!(ITEM_PARAMS.len(), 384);
        assert!(ITEM_PARAMS.windows(2).all(|w| w[0].dict_id < w[1].dict_id));
        for item in &dictionary.items {
            let params = item_params(item.dict_id).unwrap();
            // A stack has counts for its rarity, anything else levels.
            if item.is_stack {
                assert!(params.gear_rarity.is_empty(), "{}", item.name);
            } else {
                assert!(params.stack_rarity.is_empty(), "{}", item.name);
            }
            assert!(params.item_level <= 16, "{}", item.name);
            assert!(params.faction <= 4, "{}", item.name);
            assert!(params.default_modifications.len() <= 8, "{}", item.name);
        }
        assert!(item_params(73).is_none());
    }

    #[test]
    fn the_carry_limit_is_the_players_config() {
        assert_eq!(MAX_CARRIED_WEIGHT, 30.0);
        assert_eq!(MIN_CARRIED_WEIGHT, 20.0);
        assert_eq!(AMMO_BAGS_COUNT, 3);
        assert_eq!((POCKET_SLOTS_COUNT, POCKET_SLOT_SIZE), (1, 1));
    }

    #[test]
    fn spot_checks_against_the_dump() {
        // item_params_026g.tsv.
        let ak = item_params(3).unwrap();
        assert_eq!(ak.kind, "wpn_aslt");
        assert_eq!((ak.faction, ak.faction_affinity), (3, 30));
        assert_eq!((ak.item_level, ak.match_making_level), (10, 10));
        assert_eq!(ak.gear_rarity, [0, 1, 2, 3]);
        assert_eq!((ak.weight, ak.drop_weight), (3.4, 60));
        assert_eq!(
            (
                ak.degrade_per_use,
                ak.degrade_per_match,
                ak.degrade_per_death
            ),
            (0.0005, 0.01, 0.0)
        );
        assert_eq!((ak.reliability, ak.repair_cost_ratio), (1.0, 0.1));
        assert_eq!(ak.magazine_capacity, 30);
        assert!(ak.double_handed);

        let ammo = item_params(7).unwrap();
        assert_eq!(ammo.kind, "ammo");
        assert_eq!(
            (ammo.clip_size, ammo.clip_weight, ammo.required_bags),
            (30, 0.25, 1)
        );
        assert_eq!(ammo.stack_rarity, [(60, 150)]);
        assert_eq!(ammo.item_level, 5);
        // The belt-fed 7.62x39 takes two bags.
        assert_eq!(item_params(147).unwrap().required_bags, 2);

        let oxygen = item_params(9).unwrap();
        assert_eq!(
            (oxygen.kind, oxygen.faction, oxygen.weight),
            ("arm_oxy", 1, 8.0)
        );
        assert_eq!(oxygen.degrade_per_use, 0.01);

        // Weights that changed against 0.21d.
        let dictionary = Dictionary::shared();
        let weight = |name: &str| {
            item_params(dictionary.item_named(name).unwrap().dict_id)
                .unwrap()
                .weight
        };
        assert_eq!(weight("ren_helmet_1"), 1.4);
        assert_eq!(weight("scav_back_1"), 3.0);
        assert_eq!(weight("ak_74u"), 2.7);

        // Money has five rarities, [`super::super::drops`].
        assert_eq!(item_params(214).unwrap().stack_rarity.len(), 5);
        assert_eq!(item_params(215).unwrap().stack_rarity[4], (50, 50));
        // Armour comes with its modifications.
        assert!(
            ITEM_PARAMS
                .iter()
                .filter(|params| !params.default_modifications.is_empty())
                .count()
                == 83
        );
    }

    #[test]
    fn a_shop_step_is_a_clip_and_weight_goes_by_clips() {
        assert_eq!(shop_unit(7), 30);
        assert_eq!(shop_unit(22), 15);
        assert_eq!(shop_unit(3), 1);
        assert_eq!(shop_unit(67), 1); // a medkit
        assert_eq!(shop_unit(9999), 1);
        // Two clips of 5.45, 0.25 kg each.
        assert_eq!(carried_weight(7, 60), 0.5);
        // Three RGD-5, 0.3 kg each.
        assert!((carried_weight(5, 3) - 0.9).abs() < 1e-6);
        assert_eq!(carried_weight(3, 1), 3.4);
        assert_eq!(carried_weight(9999, 5), 0.0);
    }
}
