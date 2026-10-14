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
//! (`item_params::MAX_CARRIED_WEIGHT`), and the lobby does not let a kit go
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
use super::item_params::{self, MAX_CARRIED_WEIGHT};

const TEST_PLAYER_PROFILES: &[u8] = include_bytes!(
    "../../../../resources/binary-config-examples/test_player_profiles_026g.options"
);

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
        self.weight() <= MAX_CARRIED_WEIGHT
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
    /// The loadouts the 0.26g client ships, parsed once.
    pub fn shared() -> &'static Self {
        static SHARED: std::sync::OnceLock<Loadouts> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| {
            Self::parse(TEST_PLAYER_PROFILES, Dictionary::shared())
                .expect("the embedded 0.26g test profiles parse")
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

/// The pockets: profile slots 13..=18, the only slots that take drugs,
/// boosters, traps and grenades (`slots_restrictions` of the dictionary:
/// categories 10, 11 and 24), and the six the client searches for a
/// grenade (its table at 0x925FC0). PROVEN.
pub const POCKET_SLOTS: [u8; 6] = [13, 14, 15, 16, 17, 18];

/// The order in which the client counts the slots for the quick-use keys
/// (its table at 0x925FD8, read by 0x2FE7F0): the pockets, the back, then
/// the other armour. PROVEN. Quick use `n` (1 on) is the `n`-th of these
/// that holds an item the client lets be used: every item class it was
/// read for (the base item, the grenade, the medkit, the oxygen tank)
/// passes the class test (`vtable+0x58` is `xor eax, eax`; the item that
/// fails it is LIKELY an artefact: the inventory fill 0x2FD1C8 starts the
/// artefacts at the first pocket that does), the other test (`item+0x24C`,
/// a constructor argument) was not read, so which items count is LIKELY
/// all but artefacts.
pub const QUICK_USE_ORDER: [u8; 13] = [13, 14, 15, 16, 17, 18, 3, 0, 1, 2, 4, 5, 6];

/// The input bit (`actions_mask`) of quick use 1; quick use `n` is this
/// bit + `n` - 1, up to quick use 6 at bit 19 (`docs/notes/player_bots.md`
/// §2; consumer 0x2D8D80..0x2D8DFF). PROVEN.
pub const QUICK_USE_FIRST_BIT: u8 = 14;

/// The input bit that throws a grenade, held until the throw starts: the
/// client throws from the first pocket ([`POCKET_SLOTS`] in order) with a
/// count above 0 whose item is a grenade set (0x2FE8B0: `vtable+0x64` is
/// `mov eax, ecx` only in `grenade_set_core`'s table 0xA14824). PROVEN.
pub const GRENADE_BIT: u8 = 23;

/// The input bit that takes weapon 1 (slot 7) in hand. PROVEN
/// (`player_bots.md` §2).
pub const WEAPON_1_BIT: u8 = 12;

/// One thing the server's bots wear ([`BOT_KIT`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BotKitItem {
    /// The dictionary name (`items_dict`).
    pub name: &'static str,
    /// The profile slot.
    pub slot: u8,
    /// Units of a stack; 1 for an item that is no stack.
    pub count: u32,
    /// The bit of the input's `actions_mask` that uses it, if a key does.
    pub input_bit: Option<u8>,
}

/// The starter pistol, the TT-33, in weapon slot 1 ([`STARTER`]).
pub const BOT_PISTOL: BotKitItem = BotKitItem {
    name: "tt_33",
    slot: 7,
    count: 1,
    input_bit: Some(WEAPON_1_BIT),
};

/// A clip of the TT-33's free ammo, 50 rounds (its `clip_size`), in the
/// weapon's first ammo slot ([`STARTER`]).
pub const BOT_PISTOL_AMMO: BotKitItem = BotKitItem {
    name: "ammo_7.62x25",
    slot: 8,
    count: 50,
    input_bit: None,
};

/// The back: Black Market's backpack `bm_back_1` (item level 3, 6,650
/// silver), the cheapest piece that adds a pocket: its default
/// modifications 1030 (`pocket_slots_modifier` +1) and 1170
/// (`pocket_slot_size_modifier` +1). See [`pockets`].
pub const BOT_BACK: BotKitItem = BotKitItem {
    name: "bm_back_1",
    slot: 3,
    count: 1,
    input_bit: None,
};

/// The legs: Black Market's trousers `bm_legs_5` (item level 4, 6,830
/// silver), the cheapest piece for another slot that adds a pocket
/// (default modification 1030, `pocket_slots_modifier` +1).
pub const BOT_LEGS: BotKitItem = BotKitItem {
    name: "bm_legs_5",
    slot: 4,
    count: 1,
    input_bit: None,
};

/// Two medkits ("Medikit", `st_base_medkit_name`; `drugs`, heals 1, item
/// level 3, 20 silver) in the first pocket: quick use 1, as the first
/// filled slot of [`QUICK_USE_ORDER`].
pub const BOT_MEDKIT: BotKitItem = BotKitItem {
    name: "medkit",
    slot: 13,
    count: 2,
    input_bit: Some(QUICK_USE_FIRST_BIT),
};

/// The radio (`respawn_booster`, "Radio", `st_respawn_booster_name`; a
/// `booster` of item level 0, 1,500 silver, `fills_whole_pocket`): its
/// default modification 1220 is `respawn_time_modifier` -0.2 (type 1).
/// In the second pocket. **It works by being carried; no key.** PROVEN in
/// the client: a `survarium::booster` (vtable 0xA1CA34, constructor
/// 0x33F690) makes a modifier entry for each non-zero one of its 68
/// parameters; at every spawn the player's modifier lists are cleared
/// (`base_player::respawn` 0x2DACE4) and the inventory fill (0x2FCB70,
/// called at 0x2DADBB) calls slot 14 of each item in the 23 slots
/// (0x2FD1A5), which for a booster (0x33F960 → 0x33F990) adds those
/// entries to the kit's lists. At a death the respawn rule (0x316AA0)
/// sets the delay to `respawn_time × max(0, 1 + modifier 60)` through the
/// getter 0x2D5AE0, which sums the lists: 8 s × 0.8 = 6.4 s, then up to
/// the 50 ms grid. That the -0.2 of the default modification reaches the
/// booster's parameters is LIKELY (the client applies modifications to
/// an item's config, `item_config_cook`; the defaults' path there was not
/// traced).
pub const BOT_RADIO: BotKitItem = BotKitItem {
    name: "respawn_booster",
    slot: 14,
    count: 1,
    input_bit: None,
};

/// Two RGD-5 frag grenades (`rgd-5`, `grenade`, item level 3, 30 silver;
/// the other grenade, `rdg-m`, is smoke at level 6) in the third pocket,
/// thrown with [`GRENADE_BIT`]. Last of the pockets, so the medkit's
/// quick-use key does not depend on whether a grenade or the radio counts
/// for quick use.
pub const BOT_GRENADE: BotKitItem = BotKitItem {
    name: "rgd-5",
    slot: 15,
    count: 2,
    input_bit: Some(GRENADE_BIT),
};

/// What the match server's own bots wear (`lobby-server` `play::server_bot`):
/// the starter kit ([`STARTER`]), and gear with three pockets of two for a
/// medkit, the radio and a grenade. The match server can find a bot's key
/// for each here.
pub const BOT_KIT: [BotKitItem; 7] = [
    BOT_PISTOL,
    BOT_PISTOL_AMMO,
    BOT_BACK,
    BOT_LEGS,
    BOT_MEDKIT,
    BOT_RADIO,
    BOT_GRENADE,
];

/// [`BOT_KIT`] as a loadout.
pub fn bot_kit() -> &'static Loadout {
    static SHARED: std::sync::OnceLock<Loadout> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        let mut loadout = Loadout::from_names(
            Dictionary::shared(),
            BOT_KIT.iter().map(|item| (item.slot, item.name)),
        )
        .expect("the bot kit's items exist and fit their slots");
        for (slot, item) in loadout.items.iter_mut().zip(BOT_KIT) {
            slot.count = item.count;
        }
        loadout
    })
}

/// What the type 1 (added) modifiers of the default modifications of
/// `dict_id` add to `parameter` (e.g. `pocket_slots_modifier`).
fn default_modifier(dict_id: u16, parameter: &str) -> f32 {
    let path = format!("parameters/{parameter}");
    let modifications = super::modifications::Modifications::shared();
    item_params::item_params(dict_id)
        .map_or(&[][..], |params| params.default_modifications)
        .iter()
        .filter_map(|&id| modifications.modification(id))
        .flat_map(|row| &row.modifiers)
        .filter(|modifier| modifier.kind == 1 && modifier.path == path)
        .map(|modifier| modifier.value)
        .sum()
}

/// The pockets a loadout opens and how many units each holds, with no
/// skills: the player's `pocket_slots_count` and `pocket_slot_size` (1 and
/// 1, `default.player`) plus what the worn items' default modifications
/// add, at most the six [`POCKET_SLOTS`].
///
/// LIKELY the client's rule: the lobby opens the first `item_slots` of
/// slots 13..=18 and empties and locks the others
/// (`PaperDollBase.handleQuickSlots`), a pocket takes `pocketSize` units
/// of an item, or one of an item that `fills_whole_pocket`
/// (`ItemRendererQSlot`, `MessageAsk`), and both numbers come from the
/// exe; pockets (15) and pocket size (17) are among the modifiers the
/// match's getter adds rather than multiplies (0x2D5AE0, table
/// 0x927E08). How the exe sums them was not read.
pub fn pockets(loadout: &Loadout) -> (u32, u32) {
    let added = |parameter| {
        loadout
            .items
            .iter()
            .map(|item| default_modifier(item.dict_id, parameter))
            .sum::<f32>()
            .max(0.0) as u32
    };
    let count = (item_params::POCKET_SLOTS_COUNT + added("pocket_slots_modifier"))
        .min(POCKET_SLOTS.len() as u32);
    let size = item_params::POCKET_SLOT_SIZE + added("pocket_slot_size_modifier");
    (count, size)
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
        // The bots' kit starts with the starter kit, as it is.
        assert_eq!(&bot_kit().items[..2], &starter.items[..]);
        // Something that does not fit its slot is refused.
        assert!(Loadout::from_names(dictionary, [(0, "tt_33")]).is_err());
        assert!(Loadout::from_names(dictionary, [(7, "no_such_item")]).is_err());
    }

    #[test]
    fn the_bots_kit_has_a_pocket_for_each_thing_it_uses() {
        let dictionary = Dictionary::shared();
        let kit = bot_kit();
        let params = |dict_id: u16| item_params::item_params(dict_id).unwrap();
        assert_eq!(kit.items.len(), BOT_KIT.len());
        for (item, wanted) in kit.items.iter().zip(BOT_KIT) {
            let entry = dictionary.item(item.dict_id).unwrap();
            assert_eq!((entry.name.as_str(), item.slot), (wanted.name, wanted.slot));
            assert!(dictionary.fits(item.dict_id, item.slot), "{}", wanted.name);
            assert!(!entry.is_premium, "{}", wanted.name);
            assert_eq!(item.count, wanted.count);
            if !entry.is_stack {
                assert_eq!(item.count, 1, "{}", wanted.name);
            }
        }
        // One slot each.
        let mut slots: Vec<u8> = BOT_KIT.iter().map(|item| item.slot).collect();
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), BOT_KIT.len());
        // The ammo is the TT-33's, a clip of it.
        let pistol = kit.slot(7).unwrap();
        let ammo = kit.slot(8).unwrap();
        assert!(dictionary.compatible(pistol.dict_id, ammo.dict_id));
        assert_eq!(ammo.count, item_params::shop_unit(ammo.dict_id));
        assert!(kit.fits_carry_limit(), "{}", kit.weight());

        // The gear opens three pockets of two; a pocket item sits in an
        // open pocket, at most a pocket's worth of it (one of an item that
        // fills a whole pocket).
        let (count, size) = pockets(kit);
        assert_eq!((count, size), (3, 2));
        assert_eq!(pockets(starter()), (1, 1));
        let open = &POCKET_SLOTS[..count as usize];
        for item in kit
            .items
            .iter()
            .filter(|item| POCKET_SLOTS.contains(&item.slot))
        {
            assert!(open.contains(&item.slot), "slot {}", item.slot);
            let limit = if params(item.dict_id).fills_whole_pocket {
                1
            } else {
                size
            };
            assert!(item.count >= 1 && item.count <= limit, "slot {}", item.slot);
        }

        // What each is.
        let kind = |item: BotKitItem| params(kit.slot(item.slot).unwrap().dict_id).kind;
        assert_eq!(kind(BOT_MEDKIT), "drugs");
        assert_eq!(kind(BOT_RADIO), "booster");
        assert_eq!(kind(BOT_GRENADE), "grenade");
        let radio = kit.slot(BOT_RADIO.slot).unwrap().dict_id;
        assert_eq!(params(radio).default_modifications, [1220]);
        assert!((default_modifier(radio, "respawn_time_modifier") + 0.2).abs() < 1e-6);
        // The frag grenade is the lowest one; the other is smoke.
        let grenades: Vec<(&str, u8)> = dictionary
            .items
            .iter()
            .filter(|item| params(item.dict_id).kind == "grenade")
            .map(|item| (item.name.as_str(), params(item.dict_id).item_level))
            .collect();
        assert_eq!(grenades, [("rgd-5", 3), ("rdg-m", 6)]);

        // The keys: quick use n is the n-th filled slot of the client's
        // order; the medkit is the first. The radio works carried and has
        // no key. The grenade is the first one in the pockets.
        let filled: Vec<u8> = QUICK_USE_ORDER
            .iter()
            .copied()
            .filter(|&slot| kit.slot(slot).is_some())
            .collect();
        let bit = BOT_MEDKIT.input_bit.unwrap();
        assert_eq!(
            filled[usize::from(bit - QUICK_USE_FIRST_BIT)],
            BOT_MEDKIT.slot
        );
        assert_eq!(BOT_RADIO.input_bit, None);
        let first_grenade = POCKET_SLOTS.iter().copied().find(|&slot| {
            kit.slot(slot)
                .is_some_and(|item| params(item.dict_id).kind == "grenade")
        });
        assert_eq!(first_grenade, Some(BOT_GRENADE.slot));
        assert_eq!(BOT_GRENADE.input_bit, Some(GRENADE_BIT));
        assert_eq!(BOT_PISTOL.input_bit, Some(WEAPON_1_BIT));

        // The gear is the cheapest that opens the two more pockets needed:
        // of every pair of pieces for two slots (or one piece) that sold
        // for silver.
        let pocket_gear: Vec<(u8, u16, u32, f32)> = dictionary
            .items
            .iter()
            .filter(|item| !item.is_premium)
            .filter_map(|item| {
                let slot = (0..7).find(|&slot| dictionary.fits(item.dict_id, slot))?;
                let pockets = default_modifier(item.dict_id, "pocket_slots_modifier");
                (pockets > 0.0).then_some((slot, item.dict_id, item.base_cost, pockets))
            })
            .collect();
        let mut cheapest = u32::MAX;
        for (i, a) in pocket_gear.iter().enumerate() {
            if a.3 >= 2.0 {
                cheapest = cheapest.min(a.2);
            }
            for b in &pocket_gear[i + 1..] {
                if a.0 != b.0 && a.3 + b.3 >= 2.0 {
                    cheapest = cheapest.min(a.2 + b.2);
                }
            }
        }
        let cost = |item: BotKitItem| {
            dictionary
                .item(kit.slot(item.slot).unwrap().dict_id)
                .unwrap()
                .base_cost
        };
        assert_eq!(cost(BOT_BACK) + cost(BOT_LEGS), cheapest);
        assert_eq!((cost(BOT_BACK), cost(BOT_LEGS)), (6650, 6830));
    }
}
