// A player's damage model, ported from the game's shared code:
// `damage_model.cpp`, `body_part_parameters.cpp` and `hit_type_parameters.cpp`
// in `vostok/game_core/sources`. The client runs the same code on every hit
// the server reports, so the numbers here must match it exactly.

use std::collections::HashMap;

use super::data::{BodyTemplate, CombatData, Protection};

/// `hit_affects_type_enum` (all of it, as the client numbers it).
#[allow(dead_code)]
pub mod affect {
    pub const DEATH: u8 = 0;
    pub const BLEEDING: u8 = 1;
    pub const CONCUSSION: u8 = 2;
    pub const HAND_DAMAGE: u8 = 3;
    pub const LEG_DAMAGE: u8 = 4;
    pub const BLINDNESS: u8 = 8;
    pub const COUNT: usize = 9;

    /// `affects_durations`, seconds.
    pub const DURATIONS: [u32; COUNT] = [10, 10, 10, 100, 10, 120, 10, 10, 10];

    /// How long `affect` lasts once applied (`apply_affect_by_force`:
    /// `affects_durations[ affect ] * 1000`), in ms.
    pub fn duration_ms(affect: u8) -> u32 {
        1000 * DURATIONS.get(usize::from(affect)).copied().unwrap_or(10)
    }
}

/// One body part as `body_part_parameters::serialize` writes it, its times
/// on the server clock: health, the last hit (0: never) and each affect with
/// when it was applied.
#[derive(Clone, Debug, PartialEq)]
pub struct PartState {
    pub health: f32,
    pub last_hit_ms: u32,
    pub affects: Vec<(u8, u32)>,
}

/// `affect_event_type_enum`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AffectEvent {
    Applying = 0,
    Recalling = 1,
    Canceling = 2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AffectChange {
    pub body_part: String,
    pub affect: u8,
    pub event: AffectEvent,
}

/// Reduces damage on one body part while active (medkits, oxygen tanks):
/// `amount < threshold ? 0 : (amount - threshold) * hit_coeff`.
#[derive(Clone, Debug, PartialEq)]
pub struct Protector {
    pub source: u32,
    pub hit_type: String,
    pub hit_coeff: f32,
    pub threshold: f32,
}

#[derive(Clone, Debug)]
struct HitType {
    name: String,
    protection: Protection,
    bdb: Vec<(usize, f32)>,
}

#[derive(Clone, Debug)]
struct BodyPart {
    name: String,
    max_health: f32,
    health: f32,
    regeneration_speed: f32,
    regeneration_timeout_ms: u32,
    last_hit_time_ms: u32,
    damage_group: Option<u8>,
    hit_types: Vec<HitType>,
    thresholds: Vec<(f32, usize, Vec<u8>)>,
    /// (affect, expiry); the engine's `fixed_vector< ..., 8 >`.
    affects: Vec<(u8, u32)>,
    protectors: Vec<Protector>,
}

/// The armour sums of everything a player wears
/// (`player_parameters_modifyer_cook`): per body part, extra health and
/// regeneration, and per hit type the summed protection that replaces the
/// base values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    pub parts: HashMap<String, (f32, f32, HashMap<String, Protection>)>,
}

impl Equipment {
    /// Sums the `hit_params` of every item dict id worn in a profile slot.
    pub fn from_items(data: &CombatData, dict_ids: impl IntoIterator<Item = u16>) -> Self {
        let mut equipment = Self::default();
        for dict_id in dict_ids {
            let Some(armour) = data.armour(dict_id) else {
                continue;
            };
            for part in &armour.parts {
                let entry = equipment
                    .parts
                    .entry(part.body_part.clone())
                    .or_insert_with(|| (0.0, 0.0, HashMap::new()));
                entry.0 += part.health;
                entry.1 += part.regeneration_speed;
                for (hit_type, protection) in &part.hit_types {
                    let sum = entry.2.entry(hit_type.clone()).or_default();
                    sum.armor += protection.armor;
                    sum.reduce += protection.reduce;
                    sum.absorption += protection.absorption;
                }
            }
        }
        equipment
    }
}

/// Which hit a bullet last registered, so one bullet passing through two
/// boxes of the same part (or of a damage group) counts once
/// (`damage_model::hit_body_part`).
pub type LastHit = Option<(u8, usize)>;

#[derive(Clone, Debug)]
pub struct DamageModel {
    parts: Vec<BodyPart>,
    changes: Vec<AffectChange>,
}

impl DamageModel {
    pub fn new(template: &BodyTemplate, equipment: &Equipment) -> Self {
        let mut parts: Vec<BodyPart> = template
            .parts
            .iter()
            .map(|part| BodyPart {
                name: part.name.clone(),
                max_health: part.health,
                health: part.health,
                regeneration_speed: part.regeneration_speed,
                regeneration_timeout_ms: part.regeneration_timeout_ms,
                last_hit_time_ms: 0,
                damage_group: part.damage_group,
                hit_types: part
                    .hit_types
                    .iter()
                    .map(|hit_type| HitType {
                        name: hit_type.name.clone(),
                        protection: hit_type.protection,
                        bdb: hit_type.bdb.clone(),
                    })
                    .collect(),
                thresholds: part
                    .thresholds
                    .iter()
                    .map(|t| (t.value, t.target, t.affects.clone()))
                    .collect(),
                affects: Vec::new(),
                protectors: Vec::new(),
            })
            .collect();

        // `player_parameters_modifyer::apply`.
        for (name, (health, regeneration, hit_types)) in &equipment.parts {
            let Some(part) = parts.iter_mut().find(|part| &part.name == name) else {
                continue;
            };
            part.max_health += health;
            part.health = part.max_health;
            part.regeneration_speed += regeneration;
            for (type_name, protection) in hit_types {
                if let Some(hit_type) = part.hit_types.iter_mut().find(|t| &t.name == type_name) {
                    hit_type.protection = *protection;
                }
            }
        }
        Self {
            parts,
            changes: Vec::new(),
        }
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.parts.iter().position(|part| part.name == name)
    }

    #[cfg(test)]
    pub fn health(&self, index: usize) -> f32 {
        self.parts[index].health
    }

    /// `damage_model::hit_body_part`. Returns false when the bullet already
    /// hit this part (or its damage group) of this player.
    #[allow(clippy::too_many_arguments)]
    pub fn hit_body_part(
        &mut self,
        victim: u8,
        part: usize,
        hit_type: &str,
        amount: f32,
        armor_piercing: f32,
        now_ms: u32,
        last_hit: &mut LastHit,
    ) -> bool {
        if let Some((last_victim, last_part)) = *last_hit {
            if last_victim == victim {
                let same_group = self.parts[last_part].damage_group.is_some()
                    && self.parts[last_part].damage_group == self.parts[part].damage_group;
                if last_part == part || same_group {
                    return false;
                }
            }
        }
        self.hit_by_type(part, hit_type, now_ms, amount, armor_piercing, 0);
        *last_hit = Some((victim, part));
        true
    }

    /// `body_part_parameters::hit_by_type`.
    fn hit_by_type(
        &mut self,
        index: usize,
        hit_type: &str,
        now_ms: u32,
        amount: f32,
        armor_piercing: f32,
        depth: u32,
    ) {
        // Damage spread only goes to parts without spreads of their own in
        // the shipped data; the guard keeps a bad config from looping.
        if depth > 8 {
            return;
        }
        let part = &self.parts[index];
        let Some(params) = part.hit_types.iter().find(|t| t.name == hit_type) else {
            log::warn!("{}: no `{hit_type}` hit type", part.name);
            return;
        };
        let protection = params.protection;
        let bdb = params.bdb.clone();

        let arp_arm_coeff = if protection.armor == 0.0 {
            1.0
        } else {
            (armor_piercing / protection.armor - 1.0).min(1.0)
        };
        let e = arp_arm_coeff.max(0.0);
        let mut delta = amount * e
            + ((1.0 - protection.reduce) * amount * (1.0 - e) - protection.absorption).max(0.0);

        for protector in &part.protectors {
            if delta > 0.0 && protector.hit_type == hit_type {
                delta = if delta < protector.threshold {
                    0.0
                } else {
                    (delta - protector.threshold) * protector.hit_coeff
                };
            }
        }
        let delta = delta.max(0.0);

        let part = &mut self.parts[index];
        part.health = (part.health - delta).clamp(0.0, part.max_health);
        part.last_hit_time_ms = now_ms;
        self.check_affects(index, now_ms);

        // `hit_type_parameters::apply_damage`.
        for (target, coefficient) in bdb {
            if coefficient > 0.0 {
                self.hit_by_type(
                    target,
                    hit_type,
                    now_ms,
                    coefficient * delta,
                    0.0,
                    depth + 1,
                );
            }
        }
    }

    /// `body_part_parameters::check_affects`: the hit part's thresholds apply
    /// their affects to their target parts.
    fn check_affects(&mut self, index: usize, now_ms: u32) {
        let part = &self.parts[index];
        let reached: Vec<(usize, Vec<u8>)> = part
            .thresholds
            .iter()
            .filter(|(value, _, _)| part.health <= part.max_health * value)
            .map(|(_, target, affects)| (*target, affects.clone()))
            .collect();
        for (target, affects) in reached {
            for affect in affects {
                self.apply_affect(target, affect, now_ms);
            }
        }
    }

    fn apply_affect(&mut self, index: usize, affect: u8, now_ms: u32) {
        let part = &mut self.parts[index];
        if part.affects.iter().any(|(applied, _)| *applied == affect) || part.affects.len() >= 8 {
            return;
        }
        part.affects
            .push((affect, now_ms + affect::duration_ms(affect)));
        self.changes.push(AffectChange {
            body_part: part.name.clone(),
            affect,
            event: AffectEvent::Applying,
        });
    }

    /// `damage_model::tick` → `body_part_parameters::regenerate`.
    pub fn regenerate(&mut self, delta_ms: u32, now_ms: u32) {
        for part in &mut self.parts {
            let mut regenerate_ms = delta_ms;
            if part.regeneration_timeout_ms != 0 {
                let next = part.last_hit_time_ms + part.regeneration_timeout_ms;
                if now_ms <= next {
                    continue;
                }
                regenerate_ms = (now_ms - next).min(delta_ms);
            }
            let amount = regenerate_ms as f32 * part.regeneration_speed / 1000.0;
            part.health = (part.health + amount).clamp(0.0, part.max_health);

            // `update_affects`: expired affects are recalled.
            let mut i = part.affects.len();
            while i > 0 {
                i -= 1;
                if part.affects[i].1 <= now_ms {
                    let (affect, _) = part.affects.remove(i);
                    self.changes.push(AffectChange {
                        body_part: part.name.clone(),
                        affect,
                        event: AffectEvent::Recalling,
                    });
                }
            }
        }
    }

    /// `damage_model::apply_med_kit`.
    pub fn increase_health(&mut self, index: usize, amount: f32) {
        let part = &mut self.parts[index];
        part.health = (part.health + amount).clamp(0.0, part.max_health);
    }

    /// `body_part_parameters::cancel_affect_by_force`.
    pub fn cancel_affect(&mut self, index: usize, affect: u8) {
        let part = &mut self.parts[index];
        let before = part.affects.len();
        part.affects.retain(|(applied, _)| *applied != affect);
        if part.affects.len() != before {
            self.changes.push(AffectChange {
                body_part: part.name.clone(),
                affect,
                event: AffectEvent::Canceling,
            });
        }
    }

    pub fn add_protector(&mut self, index: usize, protector: Protector) {
        self.parts[index].protectors.push(protector);
    }

    pub fn remove_protectors(&mut self, source: u32) {
        for part in &mut self.parts {
            part.protectors
                .retain(|protector| protector.source != source);
        }
    }

    pub fn part_has_affect(&self, index: usize, affect: u8) -> bool {
        self.parts[index]
            .affects
            .iter()
            .any(|(applied, _)| *applied == affect)
    }

    pub fn has_affect(&self, affect: u8) -> bool {
        self.parts
            .iter()
            .any(|part| part.affects.iter().any(|(applied, _)| *applied == affect))
    }

    /// Affect changes since the last call, in order.
    pub fn take_changes(&mut self) -> Vec<AffectChange> {
        std::mem::take(&mut self.changes)
    }

    /// Every body part in the model's order, which is the client's
    /// (`fill_damage_model` adds the `hit_params` entries in their order,
    /// and `damage_model::deserialize` reads them in it, with no count).
    pub fn state(&self) -> Vec<PartState> {
        self.parts
            .iter()
            .map(|part| PartState {
                health: part.health,
                last_hit_ms: part.last_hit_time_ms,
                affects: part
                    .affects
                    .iter()
                    .map(|(affect, expiry)| {
                        (*affect, expiry.wrapping_sub(affect::duration_ms(*affect)))
                    })
                    .collect(),
            })
            .collect()
    }

    /// Lowest health of the parts that can kill, as a fraction (logging).
    pub fn vital_fraction(&self) -> f32 {
        self.parts
            .iter()
            .filter(|part| {
                part.thresholds
                    .iter()
                    .any(|(_, _, affects)| affects.contains(&affect::DEATH))
            })
            .map(|part| part.health / part.max_health)
            .fold(1.0, f32::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::data::tests::game_data;

    fn model(data: &CombatData, items: &[u16]) -> DamageModel {
        DamageModel::new(
            &data.body,
            &Equipment::from_items(data, items.iter().copied()),
        )
    }

    fn shoot(model: &mut DamageModel, part: &str, amount: f32, ap: f32, now: u32) -> f32 {
        let index = model.index_of(part).unwrap();
        let before = model.health(index);
        let mut last = None;
        assert!(model.hit_body_part(1, index, "injury", amount, ap, now, &mut last));
        before - model.health(index)
    }

    #[test]
    fn unarmoured_parts_take_full_damage_and_spread_pain() {
        let Some(data) = game_data() else {
            return;
        };
        let mut body = model(&data, &[]);
        let taken = shoot(&mut body, "body", 0.4, 0.5, 1000);
        assert!((taken - 0.4).abs() < 1e-6);
        let pain = body.index_of("pain").unwrap();
        assert!((body.health(pain) - 2.6).abs() < 1e-6);
        // Arms spread half their damage into pain.
        shoot(&mut body, "left_arm", 0.4, 0.5, 1000);
        assert!((body.health(pain) - 2.4).abs() < 1e-6);
    }

    #[test]
    fn armour_matches_the_clients_formula() {
        let Some(data) = game_data() else {
            return;
        };
        // scavenger_torso_3 (armor 0.3, reduce 0.4) against an AK round.
        let mut armoured = model(&data, &[48]);
        let taken = shoot(&mut armoured, "body", 0.4, 0.5, 1000);
        let e: f32 = 0.5 / 0.3 - 1.0;
        let expected = 0.4 * e + (0.6 * 0.4 * (1.0 - e));
        assert!((taken - expected).abs() < 1e-6, "{taken} vs {expected}");
        // Three hits kill through it, two do not.
        let mut armoured = model(&data, &[48]);
        let mut last = None;
        let body = armoured.index_of("body").unwrap();
        for _ in 0..2 {
            last = None;
            armoured.hit_body_part(1, body, "injury", 0.4, 0.5, 1000, &mut last);
        }
        assert!(!armoured.has_affect(affect::DEATH));
        let _ = last;
        let mut last = None;
        armoured.hit_body_part(1, body, "injury", 0.4, 0.5, 1000, &mut last);
        assert!(armoured.has_affect(affect::DEATH));
    }

    #[test]
    fn one_bullet_counts_once_per_part_and_damage_group() {
        let Some(data) = game_data() else {
            return;
        };
        let mut body = model(&data, &[]);
        let head = body.index_of("head").unwrap();
        let face = body.index_of("face").unwrap();
        let leg = body.index_of("left_leg").unwrap();
        let mut last = None;
        assert!(body.hit_body_part(1, head, "injury", 0.1, 0.5, 1000, &mut last));
        assert!(!body.hit_body_part(1, head, "injury", 0.1, 0.5, 1000, &mut last));
        assert!(!body.hit_body_part(1, face, "injury", 0.1, 0.5, 1000, &mut last));
        assert!(body.hit_body_part(1, leg, "injury", 0.1, 0.5, 1000, &mut last));
        // Another player's parts are not deduplicated against this one.
        assert!(body.hit_body_part(2, leg, "injury", 0.1, 0.5, 1000, &mut last));
    }

    #[test]
    fn a_headshot_kills_and_a_broken_leg_applies_and_expires() {
        let Some(data) = game_data() else {
            return;
        };
        let mut body = model(&data, &[]);
        shoot(&mut body, "head", 0.4, 0.5, 1000);
        assert!(body.has_affect(affect::DEATH));

        let mut body = model(&data, &[]);
        let leg = body.index_of("left_leg").unwrap();
        for _ in 0..3 {
            let mut last = None;
            body.hit_body_part(1, leg, "injury", 0.4, 0.5, 1000, &mut last);
        }
        let changes = body.take_changes();
        assert!(changes.iter().any(|c| c.affect == affect::LEG_DAMAGE
            && c.body_part == "left_leg"
            && c.event == AffectEvent::Applying));
        assert!(!body.has_affect(affect::DEATH));
        // Leg damage lasts 10 s.
        body.regenerate(100, 11_001);
        assert!(
            body.take_changes()
                .iter()
                .any(|c| c.affect == affect::LEG_DAMAGE && c.event == AffectEvent::Recalling)
        );
    }

    /// The parts in the order of `human_hit_params` (the only `hit_params`
    /// in the game data), which the client's model has too.
    #[test]
    fn the_state_lists_the_parts_as_the_client_reads_them() {
        let Some(data) = game_data() else {
            return;
        };
        let mut body = model(&data, &[]);
        let names: Vec<&str> = body.parts.iter().map(|part| part.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "body",
                "back",
                "head",
                "face",
                "right_leg",
                "left_leg",
                "right_foot",
                "left_foot",
                "right_arm",
                "left_arm",
                "right_hand",
                "left_hand",
                "pain",
                "infection",
                "radiation",
            ]
        );
        let leg = body.index_of("left_leg").unwrap();
        for _ in 0..3 {
            let mut last = None;
            body.hit_body_part(1, leg, "injury", 0.4, 0.5, 1000, &mut last);
        }
        let state = body.state();
        assert_eq!(state.len(), 15);
        let left_leg = &state[leg];
        assert_eq!(left_leg.health, body.health(leg));
        assert_eq!(left_leg.last_hit_ms, 1000);
        // Applied at 1000 ms, whatever its duration.
        assert!(left_leg.affects.contains(&(affect::LEG_DAMAGE, 1000)));
        assert_eq!(state[0].last_hit_ms, 0, "the body was never hit");
    }

    #[test]
    fn health_regenerates_after_the_timeout() {
        let Some(data) = game_data() else {
            return;
        };
        let mut body = model(&data, &[]);
        let index = body.index_of("body").unwrap();
        shoot(&mut body, "body", 0.4, 0.5, 1000);
        body.regenerate(1000, 6000);
        assert!(
            (body.health(index) - 0.6).abs() < 1e-6,
            "no regen inside the timeout"
        );
        body.regenerate(1000, 7000);
        assert!(
            (body.health(index) - 0.7).abs() < 1e-5,
            "{}",
            body.health(index)
        );
    }

    #[test]
    fn protectors_subtract_a_threshold_then_scale() {
        let Some(data) = game_data() else {
            return;
        };
        let mut body = model(&data, &[]);
        let index = body.index_of("body").unwrap();
        body.add_protector(
            index,
            Protector {
                source: 7,
                hit_type: "injury".to_owned(),
                hit_coeff: 0.5,
                threshold: 0.1,
            },
        );
        let taken = shoot(&mut body, "body", 0.4, 0.5, 1000);
        assert!((taken - 0.15).abs() < 1e-6, "{taken}");
        body.remove_protectors(7);
        let taken = shoot(&mut body, "body", 0.4, 0.5, 1000);
        assert!((taken - 0.4).abs() < 1e-6);
    }
}
