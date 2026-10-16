//! Every table srp derives from the game's files, built once from given
//! files and checked ([`check`]): what [`super::game_data::init`] runs
//! before the files become the process's, so that whatever is wrong with
//! them stops `srp` at start, not at a player's first use.

use binary_config_parser::tree::ConfigValue;

use super::awards::Awards;
use super::dictionary::{self, Dictionary};
use super::drops::{self, Drops};
use super::game_data::{GameData, GameDataError};
use super::item_params::{self, PlayerLimits};
use super::loadouts::{self, Loadouts};
use super::maps;
use super::modifications::{self, Modifications};
use super::quests::{self, Quests};
use super::skills::{self, SkillsTree};

/// The file at `path` of `data`, which a table needs.
fn needed<'a>(data: &'a GameData, path: &str) -> Result<&'a [u8], GameDataError> {
    data.file(path)
        .ok_or_else(|| GameDataError(format!("{}: no {path}", data.origin)))
}

/// `what` of `path`, or why not.
fn parsed<T>(
    path: &str,
    what: Result<T, binary_config_parser::tree::ConfigError>,
) -> Result<T, GameDataError> {
    what.map_err(|cause| GameDataError(format!("{path}: {}", cause.0)))
}

/// Builds every table of `data` and checks what one file says of another:
/// each item's config is there, each default modification is one of
/// `modifications.options`, ids are once in the dictionary, and every item
/// the code names is in it ([`named_items`]).
pub fn check(data: &GameData) -> Result<(), GameDataError> {
    let dictionary = parsed(
        dictionary::PATH,
        Dictionary::parse(needed(data, dictionary::PATH)?),
    )?;
    check_named(data, &dictionary)?;
    let modifications = parsed(
        modifications::MODIFICATIONS_PATH,
        Modifications::parse(
            needed(data, modifications::MODIFICATIONS_PATH)?,
            needed(data, modifications::CRAFT_PATH)?,
            needed(data, modifications::REROLL_PATH)?,
        ),
    )?;
    let params = item_params::derive(data, &dictionary)?;
    for row in &params {
        for &id in row.default_modifications {
            if modifications.modification(id).is_none() {
                let item = dictionary
                    .item(row.dict_id)
                    .expect("a row of the dictionary");
                return Err(GameDataError(format!(
                    "{}: default modification {id} is not in {}; the client would crash on it",
                    item.cfg_name,
                    modifications::MODIFICATIONS_PATH
                )));
            }
        }
    }
    modifications::derive_item_mods(data, &dictionary)?;
    maps::derive(data, &dictionary)?;
    let player = needed(data, item_params::PLAYER_PATH)?;
    parsed(
        item_params::PLAYER_PATH,
        ConfigValue::parse(player).and_then(|config| PlayerLimits::from_config(&config)),
    )?;
    parsed(drops::PATH, Drops::parse(needed(data, drops::PATH)?))?;
    parsed(quests::PATH, Quests::parse(needed(data, quests::PATH)?))?;
    parsed(skills::PATH, SkillsTree::parse(needed(data, skills::PATH)?))?;
    parsed(
        super::awards::PATH,
        Awards::parse(needed(data, super::awards::PATH)?),
    )?;
    parsed(
        loadouts::PATH,
        Loadouts::parse(needed(data, loadouts::PATH)?, &dictionary),
    )?;
    if let Some(pair) = dictionary
        .items
        .windows(2)
        .find(|w| w[0].dict_id == w[1].dict_id)
    {
        return Err(GameDataError(format!(
            "{}: dict_id {} is in items_dict twice",
            dictionary::PATH,
            pair[0].dict_id
        )));
    }
    Ok(())
}

/// An item the code names rather than the game's files: where (the
/// constant), and the item's name in the dictionary (`items_dict`'s
/// `name`, as [`Dictionary::item_named`] finds it).
pub type Named = (&'static str, &'static str);

/// Every item this crate's code names: the starter kit and stash, the
/// shop's exceptions and ammo prices, the bundles' contents. Each is
/// looked up by name when used, and one the dictionary lacks would be
/// skipped there, or panic: [`check`] refuses data that lacks any.
pub fn named_items() -> Vec<Named> {
    let mut named: Vec<Named> = Vec::new();
    named.extend(
        loadouts::STARTER
            .iter()
            .map(|&(_, name)| ("loadouts::STARTER", name)),
    );
    named.extend(
        loadouts::STARTER_STASH
            .iter()
            .map(|&name| ("loadouts::STARTER_STASH", name)),
    );
    named.extend(
        super::prices::EXCEPTIONS
            .iter()
            .map(|&(name, _)| ("prices::EXCEPTIONS", name)),
    );
    named.extend(
        super::prices::AMMO_PRICES
            .iter()
            .map(|&(name, _, _)| ("prices::AMMO_PRICES", name)),
    );
    for bundle in &super::shop_offers::BUNDLES {
        named.extend(
            bundle
                .items
                .iter()
                .map(|&(name, _)| ("shop_offers::BUNDLES", name)),
        );
    }
    named
}

/// Of `named`, those `dictionary` lacks, each as `<where> <name>`, once.
pub fn lacking(dictionary: &Dictionary, named: &[Named]) -> Vec<String> {
    let mut lacking: Vec<String> = named
        .iter()
        .filter(|(_, name)| dictionary.item_named(name).is_none())
        .map(|(place, name)| format!("{place} {name}"))
        .collect();
    lacking.dedup();
    lacking
}

/// The error for code that names what the game's data of `origin` lacks.
pub fn lacking_error(origin: &str, lacking: &[String]) -> GameDataError {
    GameDataError(format!(
        "{origin} lacks what srp's code names: {}. The data was made without a patch these \
         come from, or this srp names items of a patch not released yet: build the data with \
         every patch (srp-game-data build), or run the srp of the release",
        lacking.join(", ")
    ))
}

/// What [`check`] says of the code's items: [`named_items`] all in
/// `dictionary`, and the starter kit's items fit their slots.
fn check_named(data: &GameData, dictionary: &Dictionary) -> Result<(), GameDataError> {
    let mut missing = lacking(dictionary, &named_items());
    for (slot, name) in loadouts::STARTER {
        let item = dictionary.item_named(name);
        if item.is_some_and(|item| !dictionary.fits(item.dict_id, slot)) {
            missing.push(format!("loadouts::STARTER {name} (not of slot {slot})"));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(lacking_error(&data.origin, &missing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_items_the_code_names_are_in_the_data() {
        let named = named_items();
        assert!(named.contains(&("loadouts::STARTER", "tt_33")));
        assert!(named.contains(&("shop_offers::BUNDLES", "vss_premium")));
        assert!(named.contains(&("prices::EXCEPTIONS", "medkit")));
        assert_eq!(lacking(Dictionary::shared(), &named), Vec::<String>::new());
        assert_eq!(check(super::super::game_data::shared()), Ok(()));
    }

    /// Data without an item the code names (a patch left out of it): the
    /// start is refused, every such item named with where.
    #[test]
    fn data_without_an_item_the_code_names_is_refused_naming_it() {
        let mut dictionary = Dictionary::shared().clone();
        dictionary
            .items
            .retain(|item| !matches!(item.name.as_str(), "tt_33" | "vss_premium"));
        let missing = lacking(&dictionary, &named_items());
        assert_eq!(
            missing,
            [
                "loadouts::STARTER tt_33",
                "shop_offers::BUNDLES vss_premium"
            ]
        );
        let error = lacking_error("server-game-data.db", &missing).to_string();
        assert!(
            error.contains(
                "server-game-data.db lacks what srp's code names: loadouts::STARTER tt_33, \
                 shop_offers::BUNDLES vss_premium."
            ),
            "{error}"
        );
        assert!(error.contains("srp-game-data build"), "{error}");
    }
}
