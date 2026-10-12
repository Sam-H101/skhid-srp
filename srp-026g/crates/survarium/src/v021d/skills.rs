//! What a profile's skills add to its player: the game's
//! `gameplay/db_skills_tree` (0.21d's `resources.db`).
//!
//! Each skill has 20 levels, one per point, and each level has boosters
//! `{id, value}`. A profile with `points` in a skill gets the boosters of
//! levels 1..=points, each into player modifier `id - 1`
//! (`player_params_modifiers_enum`): `lobby_menu::fill_character_data`
//! 0x1759DF sums them so for the lobby's stats panel. In a match the player
//! reads its modifiers only from the 20 floats of its profile
//! (`player_params_modifiers_container::apply_modifier` 0x1DEBA7), which
//! the server sends with the profile (`deserialize_static`), so the server
//! must sum them in. Perks have no data and no code in a match: they are
//! only shown in the lobby.
//!
//! The player applies a modifier as `sum * scale + base` if it is one of
//! [`ADDITIVE`], else `(sum * scale + 1) * base`, and every call in a match
//! passes scale 1 (`base_player`, `player_stamina`, the weapons). So a
//! multiplicative modifier is a fraction: artefacts give 0.15 movement speed
//! for 15 %, modifications -0.1 dispersion. The tree's boosters are percents
//! (0.5 movement speed a point; as a fraction 20 points would make the
//! player 11 times as fast), so the server sends them divided by 100. Of the
//! additive ones the tree only has carried weight, 1 kg a point.

use binary_config_parser::tree::{ConfigError, ConfigValue};

const SKILLS_TREE: &[u8] =
    include_bytes!("../../../../resources/binary-config-examples/db_skills_tree_021d");

/// The player modifiers a profile carries (`player_params_modifiers_enum`).
pub const MODIFIERS: usize = 20;

/// The modifiers applied by adding to the base, not scaling it (table
/// 0x73ECC4): carried weight, engineer success chance, artefact and device
/// slots, ammo bags, quick slot size.
pub const ADDITIVE: [usize; 6] = [5, 10, 14, 15, 16, 17];

/// One skill level: its boosters as `(modifier, value)`.
type Level = Vec<(usize, f32)>;

#[derive(Clone, Debug, PartialEq)]
pub struct SkillsTree {
    /// `(skill id, levels by points - 1)`.
    skills: Vec<(u8, Vec<Level>)>,
}

impl SkillsTree {
    /// The tree the 0.21d client ships, parsed once.
    pub fn shared() -> &'static Self {
        static SHARED: std::sync::OnceLock<SkillsTree> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| Self::parse(SKILLS_TREE).expect("the embedded skills tree parses"))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ConfigError> {
        let skills = ConfigValue::parse(bytes)?
            .children()
            .into_iter()
            .map(|skill| {
                let id = u8::try_from(skill.i32_at("id")?)
                    .map_err(|_| ConfigError("a skill id is out of range".into()))?;
                let mut levels = skill
                    .req("levels")?
                    .children()
                    .into_iter()
                    .map(|level| {
                        let boosters = level
                            .req("boosters")?
                            .children()
                            .into_iter()
                            .map(|booster| {
                                let id = booster.i32_at("id")?;
                                let modifier = usize::try_from(id - 1)
                                    .ok()
                                    .filter(|&modifier| modifier < MODIFIERS)
                                    .ok_or_else(|| {
                                        ConfigError(format!("booster id {id} is no modifier"))
                                    })?;
                                Ok((modifier, booster.f32_at("value")?))
                            })
                            .collect::<Result<Level, ConfigError>>()?;
                        Ok((level.i32_at("points")?, boosters))
                    })
                    .collect::<Result<Vec<_>, ConfigError>>()?;
                levels.sort_by_key(|&(points, _)| points);
                if levels
                    .iter()
                    .enumerate()
                    .any(|(i, &(points, _))| points != i as i32 + 1)
                {
                    return Err(ConfigError(format!("skill {id} skips a level")));
                }
                Ok((
                    id,
                    levels.into_iter().map(|(_, boosters)| boosters).collect(),
                ))
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        Ok(Self { skills })
    }

    /// The modifiers `skills` (`(skill id, points)`) give, as the profile
    /// carries them: summed skill by skill, level by level as the lobby's
    /// stats panel sums them, then the multiplicative ones as fractions.
    pub fn modifiers(&self, skills: &[(u8, u8)]) -> [f32; MODIFIERS] {
        let mut modifiers = [0.0; MODIFIERS];
        for &(id, points) in skills {
            let Some((_, levels)) = self.skills.iter().find(|(skill, _)| *skill == id) else {
                continue;
            };
            for level in levels.iter().take(points.into()) {
                for &(modifier, value) in level {
                    modifiers[modifier] += value;
                }
            }
        }
        for (modifier, value) in modifiers.iter_mut().enumerate() {
            if !ADDITIVE.contains(&modifier) {
                *value /= 100.0;
            }
        }
        modifiers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `player_params_modifiers_enum`.
    const STAMINA_REGENERATION: usize = 3;
    const MOVEMENT_SPEED: usize = 4;
    const MAX_CARRIED_WEIGHT: usize = 5;

    #[test]
    fn physical_training_adds_a_kilogram_of_carried_weight_a_point() {
        let tree = SkillsTree::shared();
        let mut expected = [0.0; MODIFIERS];
        // +50 % stamina regeneration, +5 % speed, +10 kg.
        expected[STAMINA_REGENERATION] = 0.5;
        expected[MOVEMENT_SPEED] = 0.05;
        expected[MAX_CARRIED_WEIGHT] = 10.0;
        assert_eq!(tree.modifiers(&[(2, 10)]), expected);
    }

    #[test]
    fn each_skill_has_twenty_levels_and_adds_to_its_own_modifiers() {
        let tree = SkillsTree::shared();
        assert_eq!(tree.skills.len(), 5);
        assert!(tree.skills.iter().all(|(_, levels)| levels.len() == 20));
        // Sniper: dispersion and aiming speed; medicine: health
        // regeneration and pain; the others theirs.
        let at = |skill| {
            let modifiers = tree.modifiers(&[(skill, 1)]);
            (0..MODIFIERS)
                .filter(|&i| modifiers[i] != 0.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(at(1), [0, 1]);
        assert_eq!(at(2), [3, 4, 5]);
        assert_eq!(at(3), [11, 12]);
        assert_eq!(at(4), [2, 6]);
        assert_eq!(at(5), [7, 8]);
        // Sniper 4 points: 10 % less dispersion.
        assert_eq!(tree.modifiers(&[(1, 4)])[0], -0.1);
    }

    #[test]
    fn no_skills_or_unknown_ones_add_nothing() {
        let tree = SkillsTree::shared();
        assert_eq!(tree.modifiers(&[]), [0.0; MODIFIERS]);
        assert_eq!(tree.modifiers(&[(9, 3), (2, 0)]), [0.0; MODIFIERS]);
    }
}
