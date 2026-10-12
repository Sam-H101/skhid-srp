// Quick-slot drugs: bandages, medkits and painkillers (`medkit` in the
// client). A press of the slot's key uses one: after the activation delay it
// cancels its affects, then heals its body parts over its activity time;
// its damage protection holds from the press to the end. The client runs the
// same item on every copy of the player, so the server only has to agree.

use std::sync::Arc;

use super::damage::{DamageModel, Protector};
use super::data::DrugData;

/// Quick slots 1-6.
pub const QUICK_SLOTS: usize = 6;

/// The period `medkit::set_active` registers its tick with. Registered from
/// time 0, the client's `scheduler::on_frame` catches up in one call, so the
/// first tick comes on the next frame; later ones fall on the client's own
/// 300 ms game-time grid, whose phase the server cannot know (so they may
/// land up to 300 ms apart from the client's).
const TICK_MS: u64 = 300;

/// `process_quick_slots_for_*_player`: quick slot `slot` (0-5) has bit
/// `14 + 2 * slot` in the frame its key goes down and the next bit in the
/// frame it comes up; a held key sets neither.
pub fn quick_slot_pressed(mask: u32, slot: usize) -> bool {
    mask >> (14 + 2 * slot) & 1 != 0
}

#[derive(Clone, Debug)]
struct Activity {
    activity_ms: u32,
    delay_ms: u32,
    next_tick_ms: u64,
}

/// One quick slot's stack of a drug.
#[derive(Clone, Debug)]
pub struct Drug {
    pub dict_id: u16,
    data: Arc<DrugData>,
    pub amount: u32,
    active: Option<Activity>,
    /// Tags this drug's protectors in the damage model.
    source: u32,
}

impl Drug {
    pub fn new(dict_id: u16, data: Arc<DrugData>, amount: u32, source: u32) -> Self {
        Self {
            dict_id,
            data,
            amount,
            active: None,
            source,
        }
    }

    #[cfg(test)]
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// `medkit::action( true )`: uses one unless one is already working.
    pub fn press(&mut self, now_ms: u64, damage: &mut DamageModel) -> bool {
        if self.active.is_some() || self.amount == 0 {
            return false;
        }
        self.amount -= 1;
        self.active = Some(Activity {
            activity_ms: self.data.activity_ms,
            delay_ms: self.data.delay_ms,
            next_tick_ms: now_ms,
        });
        for protection in &self.data.protection {
            if let Some(index) = damage.index_of(&protection.body_part) {
                damage.add_protector(
                    index,
                    Protector {
                        source: self.source,
                        hit_type: protection.hit_type.clone(),
                        hit_coeff: protection.hit_coeff,
                        threshold: protection.threshold,
                    },
                );
            }
        }
        true
    }

    /// Runs the ticks due by `now_ms`.
    pub fn tick(&mut self, now_ms: u64, damage: &mut DamageModel) {
        loop {
            match self.active.as_mut() {
                Some(activity) if activity.next_tick_ms <= now_ms => {
                    activity.next_tick_ms += TICK_MS;
                }
                _ => return,
            }
            self.active_tick(TICK_MS as u32, damage);
        }
    }

    /// `medkit::active_tick`.
    fn active_tick(&mut self, frame_time_ms: u32, damage: &mut DamageModel) {
        let Some(activity) = self.active.as_mut() else {
            return;
        };
        let mut time_left_ms = frame_time_ms;
        if activity.delay_ms != 0 {
            let delay = activity.delay_ms.min(time_left_ms);
            activity.delay_ms -= delay;
            if activity.delay_ms != 0 {
                return;
            }
            time_left_ms -= delay;
            if time_left_ms == 0 {
                return;
            }
        }
        if activity.activity_ms == self.data.activity_ms {
            // `remove_affects`.
            for (body_part, affect) in &self.data.remove_affects {
                if let Some(index) = damage.index_of(body_part) {
                    damage.cancel_affect(index, *affect);
                }
            }
        }
        let medkit_time = activity.activity_ms.min(time_left_ms);
        activity.activity_ms -= medkit_time;
        for (body_part, per_second) in &self.data.influences {
            if let Some(index) = damage.index_of(body_part) {
                damage.increase_health(index, medkit_time as f32 / 1000.0 * per_second);
            }
        }
        if activity.activity_ms == 0 {
            self.active = None;
            damage.remove_protectors(self.source);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::damage::{Equipment, affect};
    use crate::combat::data::CombatData;
    use crate::combat::data::tests::game_data;

    const PAINKILLER: u16 = 65;
    const BANDAGES: u16 = 66;
    const MEDKIT: u16 = 67;

    fn drug(data: &CombatData, dict_id: u16, amount: u32) -> Drug {
        Drug::new(dict_id, data.drug(dict_id).unwrap().clone(), amount, 1)
    }

    fn body(data: &CombatData) -> DamageModel {
        DamageModel::new(&data.body, &Equipment::from_items(data, []))
    }

    fn hit(body: &mut DamageModel, part: &str, amount: f32, now_ms: u32) {
        let index = body.index_of(part).unwrap();
        body.hit_body_part(1, index, "injury", amount, 0.0, now_ms, &mut None);
    }

    #[test]
    fn bandages_mend_a_broken_leg_after_the_delay_and_heal_it() {
        let Some(data) = game_data() else {
            return;
        };
        let mut body = body(&data);
        // A leg breaks when its health runs out.
        hit(&mut body, "left_leg", 2.0, 0);
        let leg = body.index_of("left_leg").unwrap();
        assert!(body.part_has_affect(leg, affect::LEG_DAMAGE));
        let wounded = body.health(leg);

        let mut bandages = drug(&data, BANDAGES, 3);
        assert!(bandages.press(0, &mut body));
        assert_eq!(bandages.amount, 2);
        // Ticks at 0, 300 and 600 ms spend 900 ms of the 1 s delay.
        bandages.tick(899, &mut body);
        assert!(body.part_has_affect(leg, affect::LEG_DAMAGE));
        assert_eq!(body.health(leg), wounded);
        // The 900 ms tick ends the delay and works for the other 200 ms.
        bandages.tick(900, &mut body);
        assert!(!body.part_has_affect(leg, affect::LEG_DAMAGE));
        assert!((body.health(leg) - (wounded + 0.1)).abs() < 1e-5);
        // 2 s of 0.5/s in all: the leg is whole again.
        bandages.tick(10_000, &mut body);
        assert!(!bandages.is_active());
        assert!((body.health(leg) - (wounded + 1.0).min(1.0)).abs() < 1e-5);
    }

    #[test]
    fn a_working_or_empty_stack_ignores_the_key() {
        let Some(data) = game_data() else {
            return;
        };
        let mut body = body(&data);
        let mut medkit = drug(&data, MEDKIT, 1);
        assert!(medkit.press(0, &mut body));
        assert!(!medkit.press(100, &mut body), "already working");
        medkit.tick(10_000, &mut body);
        assert!(!medkit.press(10_000, &mut body), "none left");
        assert_eq!(medkit.amount, 0);
    }

    #[test]
    fn painkillers_shield_pain_only_while_working() {
        let Some(data) = game_data() else {
            return;
        };
        let pain_taken = |shielded: bool| {
            let mut body = body(&data);
            let mut painkiller = drug(&data, PAINKILLER, 1);
            if shielded {
                painkiller.press(0, &mut body);
            }
            let pain = body.index_of("pain").unwrap();
            let before = body.health(pain);
            hit(&mut body, "pain", 0.5, 0);
            before - body.health(pain)
        };
        assert!((pain_taken(true) - pain_taken(false) * 0.2).abs() < 1e-5);

        let mut body = body(&data);
        let mut painkiller = drug(&data, PAINKILLER, 1);
        painkiller.press(0, &mut body);
        painkiller.tick(60_000, &mut body);
        let pain = body.index_of("pain").unwrap();
        let before = body.health(pain);
        hit(&mut body, "pain", 0.5, 60_000);
        assert!((before - body.health(pain) - pain_taken(false)).abs() < 1e-5);
    }

    #[test]
    fn quick_slot_keys_are_bit_pairs_from_bit_14() {
        assert!(quick_slot_pressed(0x4000, 0));
        assert!(!quick_slot_pressed(0x8000, 0), "the key coming up");
        assert!(quick_slot_pressed(0x40000, 2));
        assert!(quick_slot_pressed(0x1000000, 5));
    }
}
