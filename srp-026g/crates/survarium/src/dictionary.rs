//! The 0.21d item dictionary and test loadouts, read from the game's own
//! files (`gameplay/db_static_dictionaries` and
//! `gameplay/test_player_profiles.options` in 0.21d's `resources.db`).
//!
//! In 0.21d the client knows each item's category and which categories fit
//! which profile slot from this data, so the server serves the same ids and
//! checks equipping against the same rules.

use binary_config_parser::tree::{ConfigError, ConfigValue};

const STATIC_DICTIONARIES: &[u8] =
    include_bytes!("../../../resources/binary-config-examples/db_static_dictionaries_021d");
const TEST_PLAYER_PROFILES: &[u8] =
    include_bytes!("../../../resources/binary-config-examples/test_player_profiles_021d.options");

/// Profile slots a 0.21d loadout has (`profile_slot_id` 0..=22).
pub const PROFILE_SLOTS: usize = 23;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub dict_id: u16,
    pub name: String,
    pub category: u8,
    pub is_stack: bool,
    pub is_premium: bool,
    pub level: u8,
    /// The item's own config, e.g. `gameplay/items/weapons/wpn_spas_12.options`.
    pub cfg_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Category {
    pub id: u8,
    pub minimum_count: u32,
    pub optimal_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactionLevel {
    /// Reputation needed for this level.
    pub value: u32,
    pub name: String,
    pub unlock_message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Faction {
    pub id: u8,
    pub name: String,
    pub levels: Vec<FactionLevel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dictionary {
    /// By `dict_id`.
    pub items: Vec<Item>,
    /// `(first_item, second_item)`: e.g. a weapon and an ammo it fires.
    pub compatibility: Vec<(u16, u16)>,
    pub categories: Vec<Category>,
    /// `(category_id, profile_slot_id)`: the categories each slot accepts.
    pub slot_restrictions: Vec<(u8, u8)>,
    /// By `id`.
    pub factions: Vec<Faction>,
    /// The game's test loadouts, as item names.
    pub test_profiles: Vec<Vec<String>>,
    /// The ids of `skills_dict` and `perks_dict`.
    pub skills: Vec<u8>,
    pub perks: Vec<u8>,
}

fn int<T: TryFrom<i32>>(value: &ConfigValue, key: &str) -> Result<T, ConfigError> {
    let number = value.i32_at(key)?;
    T::try_from(number).map_err(|_| ConfigError(format!("`{key}` = {number} is out of range")))
}

fn flag(value: &ConfigValue, key: &str) -> Result<bool, ConfigError> {
    value
        .req(key)?
        .as_bool()
        .ok_or_else(|| ConfigError(format!("`{key}` is not a boolean")))
}

impl Dictionary {
    /// The dictionary the 0.21d client ships, parsed once.
    pub fn shared() -> &'static Self {
        static SHARED: std::sync::OnceLock<Dictionary> = std::sync::OnceLock::new();
        SHARED.get_or_init(Self::v021d)
    }

    /// The dictionary the 0.21d client ships.
    pub fn v021d() -> Self {
        Self::parse(STATIC_DICTIONARIES, TEST_PLAYER_PROFILES)
            .expect("the embedded 0.21d dictionaries parse")
    }

    pub fn parse(dictionaries: &[u8], test_profiles: &[u8]) -> Result<Self, ConfigError> {
        let root = ConfigValue::parse(dictionaries)?;

        let mut items = root
            .req("items_dict")?
            .children()
            .into_iter()
            .map(|item| {
                Ok(Item {
                    dict_id: int(item, "dict_id")?,
                    name: item.str_at("name")?.to_owned(),
                    category: int(item, "item_category")?,
                    is_stack: flag(item, "is_stack")?,
                    is_premium: flag(item, "is_premium")?,
                    level: int(item, "item_level")?,
                    cfg_name: item.str_at("cfg_name")?.to_owned(),
                })
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        items.sort_by_key(|item| item.dict_id);

        let compatibility = root
            .req("items_compatibility")?
            .children()
            .into_iter()
            .map(|pair| Ok((int(pair, "first_item")?, int(pair, "second_item")?)))
            .collect::<Result<Vec<_>, ConfigError>>()?;

        let mut categories = root
            .req("categories_dict")?
            .children()
            .into_iter()
            .map(|category| {
                Ok(Category {
                    id: int(category, "id")?,
                    minimum_count: int(category, "minimum_count")?,
                    optimal_count: int(category, "optimal_count")?,
                })
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        categories.sort_by_key(|category| category.id);

        let slot_restrictions = root
            .req("slots_restrictions")?
            .children()
            .into_iter()
            .map(|rule| Ok((int(rule, "category_id")?, int(rule, "profile_slot_id")?)))
            .collect::<Result<Vec<_>, ConfigError>>()?;

        let mut factions = root
            .req("factions_dict")?
            .children()
            .into_iter()
            .map(|faction| {
                Ok(Faction {
                    id: int(faction, "id")?,
                    name: faction.str_at("name")?.to_owned(),
                    levels: faction
                        .req("levels")?
                        .children()
                        .into_iter()
                        .map(|level| {
                            Ok(FactionLevel {
                                value: int(level, "value")?,
                                name: level.str_at("name")?.to_owned(),
                                unlock_message: level.str_at("unlock_message")?.to_owned(),
                            })
                        })
                        .collect::<Result<Vec<_>, ConfigError>>()?,
                })
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        factions.sort_by_key(|faction| faction.id);

        let test_profiles = ConfigValue::parse(test_profiles)?
            .req("profiles")?
            .children()
            .into_iter()
            .map(|profile| {
                profile
                    .children()
                    .into_iter()
                    .map(|name| {
                        name.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| ConfigError("a test profile item is not a name".into()))
                    })
                    .collect()
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;

        let ids = |key: &str| {
            root.req(key)?
                .children()
                .into_iter()
                .map(|entry| int::<u8>(entry, "id"))
                .collect::<Result<Vec<_>, ConfigError>>()
        };
        let (mut skills, mut perks) = (ids("skills_dict")?, ids("perks_dict")?);
        skills.sort();
        perks.sort();

        Ok(Self {
            skills,
            perks,
            items,
            compatibility,
            categories,
            slot_restrictions,
            factions,
            test_profiles,
        })
    }

    pub fn item(&self, dict_id: u16) -> Option<&Item> {
        self.items.iter().find(|item| item.dict_id == dict_id)
    }

    pub fn item_named(&self, name: &str) -> Option<&Item> {
        self.items.iter().find(|item| item.name == name)
    }

    /// Whether an item may go into a profile slot.
    pub fn fits(&self, dict_id: u16, profile_slot_id: u8) -> bool {
        self.item(dict_id).is_some_and(|item| {
            self.slot_restrictions
                .contains(&(item.category, profile_slot_id))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_021d_dictionary() {
        let dictionary = Dictionary::v021d();
        assert_eq!(dictionary.items.len(), 83);
        assert_eq!(dictionary.compatibility.len(), 41);
        assert_eq!(dictionary.categories.len(), 24);
        assert_eq!(dictionary.slot_restrictions.len(), 77);
        assert_eq!(dictionary.factions.len(), 6);

        let spas = dictionary.item(124).unwrap();
        assert_eq!(spas.name, "spas_12");
        assert_eq!(spas.category, 14);
        assert_eq!((spas.is_stack, spas.level), (false, 7));
        assert!(dictionary.item_named("ammo_7.62x54r_old").unwrap().is_stack);
        // The SPAS-12 fires both kinds of 12 gauge.
        assert!(dictionary.compatibility.contains(&(72, 124)));

        // Every slot takes something; the shotgun goes in a weapon slot.
        assert!(
            (0..PROFILE_SLOTS as u8)
                .all(|slot| dictionary.slot_restrictions.iter().any(|&(_, s)| s == slot))
        );
        assert!(dictionary.fits(124, 7));
        assert!(!dictionary.fits(124, 0));

        assert_eq!(dictionary.skills, [1, 2, 3, 4, 5]);
        assert_eq!(dictionary.perks.len(), 36);
        let scavengers = dictionary
            .factions
            .iter()
            .find(|faction| faction.id == 1)
            .unwrap();
        assert_eq!(scavengers.name, "st_scavengers_faction");
        assert_eq!(scavengers.levels[1].value, 100);
    }

    #[test]
    fn test_loadouts_name_real_items() {
        let dictionary = Dictionary::v021d();
        assert_eq!(dictionary.test_profiles.len(), 10);
        assert_eq!(dictionary.test_profiles[0][0], "ren_helmet_1");
        assert_eq!(dictionary.test_profiles[0][10], "spas_12");
        for name in dictionary.test_profiles.iter().flatten() {
            assert!(dictionary.item_named(name).is_some(), "{name}");
        }
    }
}
