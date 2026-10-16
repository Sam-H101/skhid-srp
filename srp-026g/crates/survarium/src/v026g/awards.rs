//! The awards of the tutorial (`gameplay/awards/unique_awards.options` of
//! the 0.26g `resources.db`; dump
//! `tools/re-kit/v026g/data/other/gameplay_awards_unique_awards.options.txt`).
//!
//! The client reads the file only to show what a step gives
//! (`root.set_tutorial_rewards`, 0x22A930 from the tutorial's states
//! 0x283190 / 0x283270); the server pays. PROVEN: the five names and their
//! rows. An award's `type` is the lobby's `survarium.constants.Reward`
//! (0 experience, 1 reputation, 2 money: the client hands the rows to its
//! reward icons as they are); for money, `param` 0 is silver and 1 gold
//! (`MessagePlayerRank`: `reward_id == 0 ? regular_money : premium_money`,
//! the same pair). LIKELY for `param`.

use binary_config_parser::tree::{ConfigError, ConfigValue};

/// Its path in the archive.
pub const PATH: &str = "gameplay/awards/unique_awards.options";

/// `type` of an award.
pub mod kind {
    pub const EXPERIENCE: u8 = 0;
    pub const REPUTATION: u8 = 1;
    pub const MONEY: u8 = 2;
}

/// The awards' names.
pub mod name {
    /// The tutorial is done: the client's step 1 (`4B 01 01`).
    pub const TUTORIAL_COMPLETED: &str = "tutorial_completed";
    /// The survival notes about items were read: step 2.
    pub const ITEM_NOTES: &str = "survival_item_notes_completed";
    /// The survival notes about preparing for battle were read: step 3.
    pub const BATTLE_NOTES: &str = "survival_prepare_to_battle_notes_completed";
    /// The tutorial inside a match was done: step 4.
    pub const GAME_WORLD_TUTORIAL: &str = "game_world_tutorial_completed";
    /// The nickname was chosen: no rows.
    pub const CHANGE_NAME: &str = "tutorial_change_name";
}

/// One row of an award.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Award {
    /// [`kind`].
    pub kind: u8,
    pub amount: u32,
    /// For money: 0 silver, 1 gold.
    pub param: u32,
}

/// The file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Awards {
    /// `(name, rows)`, in the file's order.
    pub awards: Vec<(String, Vec<Award>)>,
}

impl Awards {
    /// The players' awards ([`super::game_data`]), parsed once.
    pub fn shared() -> &'static Self {
        static SHARED: std::sync::OnceLock<Awards> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| Self::parse(super::game_data::file(PATH)).expect("the awards parse"))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ConfigError> {
        let root = ConfigValue::parse(bytes)?;
        let number = |row: &ConfigValue, key: &str| -> Result<u32, ConfigError> {
            let value = row.i32_at(key)?;
            u32::try_from(value)
                .map_err(|_| ConfigError(format!("`{key}` = {value} is out of range")))
        };
        let awards = root
            .req("awards")?
            .entries()
            .iter()
            .map(|(name, rows)| {
                let rows = rows
                    .children()
                    .into_iter()
                    .map(|row| {
                        Ok(Award {
                            kind: number(row, "type")? as u8,
                            amount: number(row, "amount")?,
                            param: number(row, "param")?,
                        })
                    })
                    .collect::<Result<Vec<_>, ConfigError>>()?;
                Ok((name.clone(), rows))
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        Ok(Self { awards })
    }

    /// The rows of the award `name`; none for a name the file lacks.
    pub fn get(&self, name: &str) -> &[Award] {
        self.awards
            .iter()
            .find(|(award, _)| award == name)
            .map_or(&[], |(_, rows)| rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_026g_awards() {
        let awards = Awards::shared();
        assert_eq!(awards.awards.len(), 5);
        assert_eq!(awards.get(name::CHANGE_NAME), []);
        assert_eq!(
            awards.get(name::TUTORIAL_COMPLETED),
            [
                Award {
                    kind: kind::MONEY,
                    amount: 1000,
                    param: 0
                },
                Award {
                    kind: kind::EXPERIENCE,
                    amount: 250,
                    param: 0
                },
            ]
        );
        let gold = |amount| {
            [Award {
                kind: kind::MONEY,
                amount,
                param: 1,
            }]
        };
        assert_eq!(awards.get(name::ITEM_NOTES), gold(25));
        assert_eq!(awards.get(name::BATTLE_NOTES), gold(25));
        assert_eq!(awards.get(name::GAME_WORLD_TUTORIAL), gold(50));
        assert_eq!(awards.get("no_such_award"), []);
    }
}
