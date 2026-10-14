//! What a bot senses at a think (`player_bots_behaviour.md` §2): the only
//! way enemy facts come to it besides its team's callouts and the cheats.
//!
//! - **Sight:** an enemy within `sight_m`, inside the view's width, with a
//!   clear line from the bot's eyes to his eyes or chest on the bullet
//!   mesh. The rays come from a budget shared by all bots of the match,
//!   and the enemies longest unchecked go first (research B5).
//! - **Hearing:** the sounds of the last think (shots, sprinting, a reload
//!   or switch, a use) within their ranges (× the hearing cheat), placed
//!   with an error that grows with the distance.
//!
//! The noticing delay is not here: it is the memory's (`know::Memory`), so
//! that no layer can skip it.

use mapdata::{RayTree, Vec3};

use super::kin::{forward, wrap};
use super::know::{Percept, Source};
use super::rng::Rng;
use super::tuning::{CheatTuning, SenseTuning};
use super::view::{MatchView, Place, PlaceView, SoundKind};

/// The rays a poll may still cast.
#[derive(Clone, Copy, Debug)]
pub struct RayBudget(pub u32);

impl RayBudget {
    fn take(&mut self) -> bool {
        if self.0 == 0 {
            return false;
        }
        self.0 -= 1;
        true
    }
}

/// Whether `to` lies within the view of a body facing `yaw` (`view_deg`
/// wide).
pub fn in_view(from: Vec3, yaw: f32, to: Vec3, view_deg: f32) -> bool {
    let (dx, dz) = (to.x - from.x, to.z - from.z);
    if dx.abs() + dz.abs() < 1e-3 {
        return true;
    }
    let facing = forward(yaw);
    let angle = wrap(super::kin::yaw_of([dx, dz]) - super::kin::yaw_of(facing)).abs();
    angle <= view_deg.to_radians() * 0.5
}

/// The percepts of the bot `me` at `now`, the sounds since `since`.
/// `order` gives the enemies to look at first (stalest first).
#[allow(clippy::too_many_arguments)]
pub fn sense(
    me: &PlaceView,
    view: &MatchView,
    sight: &RayTree,
    tuning: &SenseTuning,
    cheats: &CheatTuning,
    since: u32,
    order: &[Place],
    rays: &mut RayBudget,
    rng: &mut Rng,
) -> Vec<Percept> {
    let mut out = Vec::new();
    if !me.alive {
        return out;
    }
    let eye = me.eye();
    for &who in order {
        let Some(enemy) = view.place(who) else {
            continue;
        };
        if !enemy.alive || !enemy.present || enemy.team == me.team {
            continue;
        }
        let distance = enemy.feet.distance_xz(me.feet);
        if distance > tuning.sight_m || !in_view(me.feet, me.yaw, enemy.feet, tuning.view_deg) {
            continue;
        }
        let seen = [enemy.eye(), enemy.chest()]
            .into_iter()
            .any(|point| rays.take() && sight.visible(eye, point));
        if seen {
            out.push(Percept {
                who,
                at: enemy.feet,
                error_m: 0.0,
                time: view.clock,
                source: Source::Seen,
                busy: enemy.speed() > 1.0 || enemy.keys & crate::v026g::drive::key::FIRE != 0,
            });
        }
    }
    let hearing = cheats.hearing.max(1.0);
    for sound in view.sounds.iter().filter(|sound| sound.time > since) {
        let Some(enemy) = view.place(sound.from) else {
            continue;
        };
        if enemy.team == me.team {
            continue;
        }
        let range = hearing
            * match sound.kind {
                SoundKind::Shot => tuning.shot_m,
                SoundKind::Sprint | SoundKind::Jump => tuning.sprint_m,
                SoundKind::Reload | SoundKind::Switch => tuning.reload_m,
                SoundKind::Use => tuning.use_m,
            };
        let distance = sound.at.distance_xz(me.feet);
        if distance > range
            || out
                .iter()
                .any(|p| p.who == sound.from && p.source == Source::Seen)
        {
            continue;
        }
        let error = distance * tuning.heard_error;
        let (dx, dz) = (rng.range(-error, error), rng.range(-error, error));
        out.push(Percept {
            who: sound.from,
            at: Vec3::new(sound.at.x + dx, sound.at.y, sound.at.z + dz),
            error_m: error,
            time: sound.time,
            source: Source::Heard,
            busy: sound.kind == SoundKind::Shot,
        });
    }
    out
}

/// The noticing delay for a sight at `distance`: near to far, shorter for
/// a busy enemy, times the skill's factor.
pub fn notice_ms(tuning: &SenseTuning, distance: f32, busy: bool, skill_factor: f32) -> u32 {
    let t = (distance / tuning.notice_far_m).clamp(0.0, 1.0);
    let base = tuning.notice_near_ms as f32
        + (tuning.notice_far_ms as f32 - tuning.notice_near_ms as f32) * t;
    let base = if busy {
        base * tuning.notice_moving
    } else {
        base
    };
    (base * skill_factor) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v026g::bots::view::{Mode, Sound};

    fn place(place: Place, team: u8, feet: Vec3, yaw: f32) -> PlaceView {
        PlaceView {
            place,
            team,
            bot: place == 0,
            present: true,
            alive: true,
            feet,
            yaw,
            pitch: 0.0,
            velocity: [0.0, 0.0],
            keys: 0,
            health: 1.0,
            spawned: 0,
            kills: 0,
            deaths: 0,
            streak: 0,
            carrying: None,
            seen_at: 0,
        }
    }

    fn view(places: Vec<PlaceView>, sounds: Vec<Sound>) -> MatchView {
        MatchView {
            clock: 1_000,
            mode: Mode::TeamDeathmatch,
            time_left_s: 600.0,
            total_s: 900.0,
            points: [0, 0],
            places,
            batteries: vec![],
            containers: vec![],
            sounds,
            deaths: vec![],
            world_time: 0,
            can_respawn: [true, true],
        }
    }

    #[test]
    fn an_enemy_in_view_with_a_clear_line_is_seen_one_behind_is_only_heard() {
        // Facing +z (yaw 0): one enemy ahead, one behind firing.
        let me = place(0, 0, Vec3::ZERO, 0.0);
        let ahead = place(1, 1, Vec3::new(0.0, 0.0, 30.0), 0.0);
        let behind = place(2, 1, Vec3::new(0.0, 0.0, -20.0), 0.0);
        let shot = Sound {
            from: 2,
            kind: SoundKind::Shot,
            at: behind.feet,
            time: 900,
        };
        let view = view(vec![me.clone(), ahead, behind], vec![shot]);
        let sight = RayTree::build(&[]);
        let mut rays = RayBudget(10);
        let percepts = sense(
            &me,
            &view,
            &sight,
            &SenseTuning::default(),
            &CheatTuning::default(),
            500,
            &[1, 2],
            &mut rays,
            &mut Rng::new(1),
        );
        assert_eq!(percepts.len(), 2, "{percepts:?}");
        assert_eq!((percepts[0].who, percepts[0].source), (1, Source::Seen));
        assert_eq!((percepts[1].who, percepts[1].source), (2, Source::Heard));
        assert!(percepts[1].error_m > 0.0 && percepts[1].busy);
        // No rays left: nothing is seen.
        let mut none = RayBudget(0);
        let percepts = sense(
            &me,
            &view,
            &sight,
            &SenseTuning::default(),
            &CheatTuning::default(),
            500,
            &[1, 2],
            &mut none,
            &mut Rng::new(1),
        );
        assert!(percepts.iter().all(|p| p.source != Source::Seen));
    }

    #[test]
    fn noticing_takes_longer_far_away_and_less_for_a_busy_enemy() {
        let tuning = SenseTuning::default();
        assert_eq!(notice_ms(&tuning, 0.0, false, 1.0), 200);
        assert_eq!(notice_ms(&tuning, 100.0, false, 1.0), 500);
        assert!(notice_ms(&tuning, 30.0, true, 1.0) < notice_ms(&tuning, 30.0, false, 1.0));
        assert!(in_view(Vec3::ZERO, 0.0, Vec3::new(1.0, 0.0, 5.0), 90.0));
        assert!(!in_view(Vec3::ZERO, 0.0, Vec3::new(0.0, 0.0, -5.0), 90.0));
    }
}
