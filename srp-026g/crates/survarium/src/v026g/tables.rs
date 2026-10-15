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
/// `modifications.options`, ids are once in the dictionary.
pub fn check(data: &GameData) -> Result<(), GameDataError> {
    let dictionary = parsed(
        dictionary::PATH,
        Dictionary::parse(needed(data, dictionary::PATH)?),
    )?;
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
