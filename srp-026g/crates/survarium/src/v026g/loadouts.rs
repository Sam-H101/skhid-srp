//! Starting loadouts: the game's own test loadouts,
//! `gameplay/test_player_profiles.options` of 0.26g's `resources.db`
//! (`docs/notes/data.md` §4).
//!
//! The file has ten profiles of thirteen item names, the name at index `i`
//! for profile slot `i` (as in 0.21d): helmet, mask, torso, back, legs,
//! gloves, boots, the first weapon and two ammo for it, the second weapon
//! and two ammo for it. Every name is a 0.26g item and fits its slot.
//!
//! **The carry limit.** 0.26g's player carries 30 kg
//! (`item_params::player().max_carried_weight`), and the lobby does not let a kit go
//! over it. With one clip of each ammo the ten profiles weigh 32.35, 32.3,
//! 38.4, 26.9, 19.3, 26.3, 22.3, 26.3, 26.3 and 26.3 kg: **the first three
//! are over**. (The gear alone is 30.5, 30.0 and 36.1 kg: profiles 0 and 2
//! are over even with no ammo.) They are the three with the heavy
//! Renaissance and Edge torsos. srp **drops those three** rather than trim
//! them [choice: which piece to take off a test profile is nobody's rule]:
//! [`Loadouts::usable`] gives the seven that fit. All ten stay readable in
//! [`Loadouts::profiles`].
//!
//! **What a new account has.** "One of the starting weapons available to
//! all players when they first enter the world, along with the TT-33 and
//! Mosin carbine" (the wiki's `PPSH-41` page of 2015-02-14, revision 9600,
//! and `Mosin carbine` of 2015-02-24, revision 10011; 0.26i). PROVEN (wiki
//! of the time). 0.26g's data agrees: the three weapons and their two
//! kinds of ammo (7.62x25 for the TT-33 and the PPSh-41, 7.62x54R for the
//! Mosin) are the only weapons and ammo with `base_cost` 0 and item
//! level 0. [`STARTER`] is what of it the first kit wears and
//! [`STARTER_STASH`] what lies in the inventory.
//!
//! [choice] Which of the three a new player wore is not known. srp's first
//! kit wears what it always did, the TT-33 and a clip in weapon slot 1, and
//! the Mosin, a clip for it and the PPSh-41 lie in the inventory for the
//! player to put on. A kit with one kind of ammo is also the one whose
//! ammo use the match counters prove (`session::v026g::autobuy`). The free
//! ammo is one clip each; more of it costs nothing
//! (`session::v026g::rules`).

use binary_config_parser::tree::{ConfigError, ConfigValue};

use super::dictionary::{Dictionary, PROFILE_SLOTS};
use super::item_params;

/// Its path in the archive.
pub const PATH: &str = "gameplay/test_player_profiles.options";

/// What a new account's first kit wears, `(slot, item)`: the TT-33 in
/// weapon slot 1 and a clip of its free ammo in that weapon's first ammo
/// slot. See the module.
pub const STARTER: [(u8, &str); 2] = [(7, "tt_33"), (8, "ammo_7.62x25")];

/// What a new account owns besides what it wears ([`STARTER`]): the Mosin
/// carbine with a clip of its free ammo, and the PPSh-41, which fires the
/// TT-33's ammo.
pub const STARTER_STASH: [&str; 3] = ["mosin_nagant", "ammo_7.62x54r_old", "ppsh"];

/// What a slot of a loadout holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotItem {
    pub slot: u8,
    pub dict_id: u16,
    /// Units for a stack (one clip of ammo, one of anything else); 1 for an
    /// item that is no stack.
    pub count: u32,
}

/// One loadout: its items by slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loadout {
    pub items: Vec<SlotItem>,
}

impl Loadout {
    /// From `(slot, item name)`; an item that is unknown or does not fit its
    /// slot is an error.
    pub fn from_names<'a>(
        dictionary: &Dictionary,
        names: impl IntoIterator<Item = (u8, &'a str)>,
    ) -> Result<Self, ConfigError> {
        let items = names
            .into_iter()
            .map(|(slot, name)| {
                let item = dictionary
                    .item_named(name)
                    .ok_or_else(|| ConfigError(format!("no item `{name}`")))?;
                if usize::from(slot) >= PROFILE_SLOTS || !dictionary.fits(item.dict_id, slot) {
                    return Err(ConfigError(format!("`{name}` does not fit slot {slot}")));
                }
                let count = if item.is_stack {
                    item_params::shop_unit(item.dict_id)
                } else {
                    1
                };
                Ok(SlotItem {
                    slot,
                    dict_id: item.dict_id,
                    count,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { items })
    }

    /// What the loadout weighs, as the lobby counts it.
    pub fn weight(&self) -> f32 {
        self.items
            .iter()
            .map(|item| item_params::carried_weight(item.dict_id, item.count))
            .sum()
    }

    /// Whether a player can carry it.
    pub fn fits_carry_limit(&self) -> bool {
        self.weight() <= item_params::player().max_carried_weight
    }

    pub fn slot(&self, slot: u8) -> Option<&SlotItem> {
        self.items.iter().find(|item| item.slot == slot)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loadouts {
    /// The file's profiles, in its order.
    pub profiles: Vec<Loadout>,
}

impl Loadouts {
    /// The players' test profiles ([`super::game_data`]), parsed once.
    pub fn shared() -> &'static Self {
        static SHARED: std::sync::OnceLock<Loadouts> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| {
            Self::parse(super::game_data::file(PATH), Dictionary::shared())
                .expect("the test profiles parse")
        })
    }

    pub fn parse(bytes: &[u8], dictionary: &Dictionary) -> Result<Self, ConfigError> {
        let profiles = ConfigValue::parse(bytes)?
            .req("profiles")?
            .children()
            .into_iter()
            .map(|profile| {
                let names = profile
                    .children()
                    .into_iter()
                    .map(|name| {
                        name.as_str()
                            .ok_or_else(|| ConfigError("a test profile item is not a name".into()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Loadout::from_names(
                    dictionary,
                    names
                        .into_iter()
                        .enumerate()
                        .map(|(slot, name)| (slot as u8, name)),
                )
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { profiles })
    }

    /// The profiles a player can carry: all but the overweight ones.
    pub fn usable(&self) -> impl Iterator<Item = &Loadout> {
        self.profiles
            .iter()
            .filter(|loadout| loadout.fits_carry_limit())
    }
}

/// What a new account wears ([`STARTER`]).
pub fn starter() -> &'static Loadout {
    static SHARED: std::sync::OnceLock<Loadout> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        Loadout::from_names(Dictionary::shared(), STARTER).expect("the starter items exist")
    })
}

/// What a new account owns unworn ([`STARTER_STASH`]): `(dict id, units)`,
/// a clip of a stack, 1 of an item that is no stack.
pub fn starter_stash() -> &'static [(u16, u32)] {
    static SHARED: std::sync::OnceLock<Vec<(u16, u32)>> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        let dictionary = Dictionary::shared();
        STARTER_STASH
            .iter()
            .map(|name| {
                let item = dictionary
                    .item_named(name)
                    .expect("the starter items exist");
                let units = if item.is_stack {
                    item_params::shop_unit(item.dict_id)
                } else {
                    1
                };
                (item.dict_id, units)
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_026g_test_profiles() {
        let dictionary = Dictionary::shared();
        let loadouts = Loadouts::shared();
        assert_eq!(loadouts.profiles.len(), 10);
        for loadout in &loadouts.profiles {
            assert_eq!(loadout.items.len(), 13);
            for (slot, item) in loadout.items.iter().enumerate() {
                assert_eq!(usize::from(item.slot), slot);
                assert!(dictionary.fits(item.dict_id, item.slot));
            }
        }
        let name = |profile: usize, slot: u8| {
            let item = loadouts.profiles[profile].slot(slot).unwrap();
            dictionary.item(item.dict_id).unwrap().name.as_str()
        };
        assert_eq!(name(0, 0), "ren_helmet_1");
        assert_eq!(name(0, 2), "edge_torso_5");
        assert_eq!(name(0, 7), "ak_74u");
        assert_eq!(name(0, 8), "ammo_5.45x39_7n10");
        assert_eq!(name(0, 10), "spas_12");
        assert_eq!(name(2, 2), "ren_torso_6");
        assert_eq!(name(4, 2), "scav_torso_1");
        // A clip of each ammo, one of each piece of gear.
        assert_eq!(loadouts.profiles[0].slot(8).unwrap().count, 30);
        assert_eq!(loadouts.profiles[0].slot(11).unwrap().count, 15);
        assert_eq!(loadouts.profiles[0].slot(7).unwrap().count, 1);
        // Each ammo fits the weapon before it.
        for loadout in &loadouts.profiles {
            for (weapon, ammo) in [(7, 8), (7, 9), (10, 11), (10, 12)] {
                let (weapon, ammo) = (loadout.slot(weapon).unwrap(), loadout.slot(ammo).unwrap());
                assert!(dictionary.compatible(weapon.dict_id, ammo.dict_id));
            }
        }
    }

    #[test]
    fn three_profiles_are_over_the_carry_limit_and_dropped() {
        let loadouts = Loadouts::shared();
        let weights: Vec<f32> = loadouts.profiles.iter().map(Loadout::weight).collect();
        let expected = [32.35, 32.3, 38.4, 26.9, 19.3, 26.3, 22.3, 26.3, 26.3, 26.3];
        for (weight, expected) in weights.iter().zip(expected) {
            assert!((weight - expected).abs() < 0.01, "{weights:?}");
        }
        let fits: Vec<bool> = loadouts
            .profiles
            .iter()
            .map(Loadout::fits_carry_limit)
            .collect();
        assert_eq!(
            fits,
            [
                false, false, false, true, true, true, true, true, true, true
            ]
        );
        assert_eq!(loadouts.usable().count(), 7);
        assert!(loadouts.usable().all(|loadout| loadout.weight() <= 30.0));
    }

    #[test]
    fn the_starter_is_the_three_free_weapons_and_their_ammo() {
        let dictionary = Dictionary::shared();
        let name = |dict_id: u16| dictionary.item(dict_id).unwrap().name.as_str();
        let starter = starter();
        // Worn: the pistol and a clip, where they always were.
        assert_eq!(starter.items.len(), 2);
        let pistol = starter.slot(7).unwrap();
        let ammo = starter.slot(8).unwrap();
        assert_eq!(name(pistol.dict_id), "tt_33");
        assert_eq!((pistol.count, ammo.count), (1, 50));
        assert!(dictionary.compatible(pistol.dict_id, ammo.dict_id));
        // In the inventory: the Mosin and a clip, and the PPSh-41, which
        // fires the pistol's ammo. Each weapon fits both weapon slots.
        let stash = starter_stash();
        let stashed: Vec<(&str, u32)> = stash
            .iter()
            .map(|&(dict_id, units)| (name(dict_id), units))
            .collect();
        assert_eq!(
            stashed,
            [("mosin_nagant", 1), ("ammo_7.62x54r_old", 10), ("ppsh", 1)]
        );
        assert!(dictionary.compatible(stash[0].0, stash[1].0));
        assert!(dictionary.compatible(stash[2].0, ammo.dict_id));
        for weapon in [stash[0].0, stash[2].0] {
            assert!(dictionary.fits(weapon, 7) && dictionary.fits(weapon, 10));
        }
        assert!(dictionary.fits(stash[1].0, 8) && dictionary.fits(stash[1].0, 11));
        // The heaviest kit of them (the PPSh-41, the Mosin and a clip for
        // each) is far under the carry limit.
        let heaviest = Loadout::from_names(
            dictionary,
            [
                (7, "ppsh"),
                (8, "ammo_7.62x25"),
                (10, "mosin_nagant"),
                (11, "ammo_7.62x54r_old"),
            ],
        )
        .unwrap();
        assert!(heaviest.fits_carry_limit());
        // All are free and of level 0 in 0.26g's data, and they are all the
        // weapons and ammo that are.
        let mut owned: Vec<u16> = starter.items.iter().map(|item| item.dict_id).collect();
        owned.extend(stash.iter().map(|&(dict_id, _)| dict_id));
        owned.sort_unstable();
        for &dict_id in &owned {
            assert_eq!(dictionary.item(dict_id).unwrap().base_cost, 0);
            assert_eq!(item_params::item_params(dict_id).unwrap().item_level, 0);
        }
        let free: Vec<u16> = dictionary
            .items
            .iter()
            .filter(|item| matches!(item.category, 8 | 13..=21) && !item.is_premium)
            .filter(|item| item.base_cost == 0)
            .filter(|item| {
                item_params::item_params(item.dict_id)
                    .is_some_and(|params| params.item_level == 0 && !params.kind.is_empty())
            })
            .map(|item| item.dict_id)
            .collect();
        assert_eq!(
            free,
            owned,
            "{:?}",
            free.iter().map(|&id| name(id)).collect::<Vec<_>>()
        );
        assert!(starter.fits_carry_limit());
        // Something that does not fit its slot is refused.
        assert!(Loadout::from_names(dictionary, [(0, "tt_33")]).is_err());
        assert!(Loadout::from_names(dictionary, [(7, "no_such_item")]).is_err());
    }
}
