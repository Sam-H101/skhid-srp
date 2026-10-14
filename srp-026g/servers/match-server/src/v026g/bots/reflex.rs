//! The combat reflex: a bot's second track (`player_bots_research.md` B1
//! point 2, Left 4 Dead's "Main" and "Legs"). The play decides where to be
//! and why; the reflex, at every think, picks a target, aims and fires, and
//! may turn the bot to a sound. It overrides only the look and the trigger
//! (and the stance); the play keeps the legs.
//!
//! The fairness rules hold by type: the reflex aims only at a
//! `know::SeenTrack`, so never at what no sense gave, and never before the
//! noticing delay. Its aim is a human's: it starts off by
//! `AimTuning::error_deg` × 3 and settles toward its error over `settle_s`,
//! both mixed by the bot's skill (play 23 changes only these), and it fires
//! in bursts only when its view is within `fire_within_deg` of the aim.
//!
//! Plays 5, 6, 7, 11, 19, 20 and 21's trade live here; this first form has
//! the target, the aim, the bursts and the turn to a sound (play 6).

use mapdata::Vec3;

use super::kin::{wrap, yaw_of};
use super::know::{Memory, SeenTrack, Source};
use super::rng::Rng;
use super::signals::mix;
use super::tuning::AimTuning;
use super::view::{MatchView, Place, PlaceView};

/// What the reflex wants of the motor this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Override {
    /// Look at this point (the aim, with its error), instead of the play's
    /// look.
    pub look_at: Option<Vec3>,
    /// The yaw and pitch error added to the aim, radians.
    pub error: (f32, f32),
    /// Hold fire this tick.
    pub fire: bool,
    /// The turn's time constant while aiming, seconds.
    pub turn_time_s: Option<f32>,
}

#[derive(Clone, Debug, Default)]
pub struct Reflex {
    target: Option<Place>,
    /// When it took the target.
    since: u32,
    /// The error drawn for this target, radians (yaw, pitch).
    error: (f32, f32),
    burst_until: u32,
    pause_until: u32,
    /// A heard shot to turn to, and until when.
    sound: Option<(Vec3, u32)>,
}

impl Reflex {
    pub fn target(&self) -> Option<Place> {
        self.target
    }

    /// At a think: the target among the seen enemies (the present one if
    /// still seen, else the nearest in range), or a sound to turn to.
    pub fn think(
        &mut self,
        me: &PlaceView,
        memory: &Memory,
        now: u32,
        aim: &AimTuning,
        skill: f32,
        rng: &mut Rng,
    ) {
        let seen = memory.all_seen(now);
        let in_range = |t: &&SeenTrack| t.at().distance_xz(me.feet) <= aim.range_m;
        let keep = self
            .target
            .and_then(|target| seen.iter().filter(in_range).find(|t| t.who() == target));
        let choice = keep.or_else(|| {
            seen.iter().filter(in_range).min_by(|a, b| {
                a.at()
                    .distance_xz(me.feet)
                    .total_cmp(&b.at().distance_xz(me.feet))
            })
        });
        match choice {
            Some(track) if Some(track.who()) != self.target => {
                self.target = Some(track.who());
                self.since = now;
                // A fresh target: the aim starts well off and settles.
                let error = mix(aim.error_deg, skill).to_radians() * 3.0;
                self.error = (rng.range(-error, error), rng.range(-error, error) * 0.5);
                self.sound = None;
            }
            Some(_) => {}
            None => {
                self.target = None;
                // Play 6: a shot heard lately, nothing seen: turn to it.
                let heard = memory
                    .tracks()
                    .iter()
                    .filter(|t| t.source == Source::Heard && t.time + 1_500 >= now)
                    .min_by(|a, b| {
                        a.at.distance_xz(me.feet)
                            .total_cmp(&b.at.distance_xz(me.feet))
                    });
                if let Some(track) = heard {
                    self.sound = Some((track.at, now + 2_000));
                }
            }
        }
    }

    /// One tick: where to look and whether to fire. `fire_allowed`: the
    /// play's word (a carrier cannot shoot).
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        me: &PlaceView,
        view: &MatchView,
        memory: &Memory,
        now: u32,
        fire_allowed: bool,
        aim: &AimTuning,
        skill: f32,
        rng: &mut Rng,
    ) -> Override {
        let seen = self.target.and_then(|target| memory.seen(target, now));
        let Some(track) = seen else {
            self.target = None;
            return match self.sound {
                Some((at, until)) if now < until => Override {
                    look_at: Some(at + Vec3::new(0.0, 1.4, 0.0)),
                    ..Override::default()
                },
                _ => Override::default(),
            };
        };
        // The seen body where it is now (a seen enemy is tracked by eye).
        let body = view.place(track.who()).filter(|p| p.alive);
        let point = body.map_or(track.at() + Vec3::new(0.0, 1.3, 0.0), PlaceView::chest);
        // The error settles from its start toward the skill's.
        let settle_s = mix(aim.settle_s, skill).max(0.05);
        let t = now.saturating_sub(self.since) as f32 / 1000.0;
        let fade = (-t / settle_s).exp();
        let floor = mix(aim.error_deg, skill).to_radians();
        let error = (
            self.error.0 * fade + floor * (rng.unit() - 0.5),
            self.error.1 * fade + floor * 0.5 * (rng.unit() - 0.5),
        );
        // Fire in bursts when the view is near the aim.
        let eye = me.eye();
        let (dx, dz) = (point.x - eye.x, point.z - eye.z);
        let want_yaw = yaw_of([dx, dz]) + error.0;
        let want_pitch = (point.y - eye.y).atan2(dx.hypot(dz)) + error.1;
        let off = wrap(want_yaw - me.yaw)
            .abs()
            .max((want_pitch - me.pitch).abs());
        let within = off <= aim.fire_within_deg.to_radians();
        let mut fire = false;
        if fire_allowed && within && point.distance_xz(me.feet) <= aim.range_m {
            if now < self.burst_until {
                fire = true;
            } else if now >= self.pause_until {
                self.burst_until =
                    now + rng.range(aim.burst_ms.0 as f32, aim.burst_ms.1 as f32) as u32;
                self.pause_until = self.burst_until
                    + rng.range(aim.pause_ms.0 as f32, aim.pause_ms.1 as f32) as u32;
                fire = true;
            }
        }
        Override {
            look_at: Some(point),
            error,
            fire,
            turn_time_s: Some(settle_s * 0.5),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v026g::bots::know::Percept;
    use crate::v026g::bots::view::Mode;

    fn body(place: Place, team: u8, feet: Vec3, yaw: f32) -> PlaceView {
        PlaceView {
            place,
            team,
            bot: true,
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

    #[test]
    fn it_aims_only_at_a_noticed_enemy_and_fires_once_on_target() {
        let me = body(0, 0, Vec3::ZERO, 0.0);
        let enemy = body(1, 1, Vec3::new(0.0, 0.0, 20.0), 0.0);
        let view = MatchView {
            clock: 0,
            mode: Mode::TeamDeathmatch,
            time_left_s: 600.0,
            total_s: 900.0,
            points: [0, 0],
            places: vec![me.clone(), enemy],
            batteries: vec![],
            containers: vec![],
            sounds: vec![],
            deaths: vec![],
            world_time: 0,
            can_respawn: [true, true],
        };
        let aim = AimTuning::default();
        let mut memory = Memory::default();
        let mut reflex = Reflex::default();
        let mut rng = Rng::new(9);
        let sight = |time| Percept {
            who: 1,
            at: Vec3::new(0.0, 0.0, 20.0),
            error_m: 0.0,
            time,
            source: Source::Seen,
            busy: false,
        };
        memory.update(&[sight(1_000)], 1_000, |_| 300, 3_000, 20_000);
        reflex.think(&me, &memory, 1_000, &aim, 1.0, &mut rng);
        assert_eq!(reflex.target(), None, "not noticed yet");
        // Noticed: it aims; facing the enemy squarely, it fires once the
        // error has settled.
        memory.update(&[sight(1_300)], 1_300, |_| 300, 3_000, 20_000);
        reflex.think(&me, &memory, 1_300, &aim, 1.0, &mut rng);
        assert_eq!(reflex.target(), Some(1));
        let mut fired = false;
        for k in 0..20 {
            let now = 1_300 + k * 50;
            memory.update(&[sight(now)], now, |_| 300, 3_000, 20_000);
            let o = reflex.tick(&me, &view, &memory, now, true, &aim, 1.0, &mut rng);
            assert!(o.look_at.is_some());
            fired |= o.fire;
        }
        assert!(fired);
        // The play holds fire: no shot.
        let o = reflex.tick(&me, &view, &memory, 2_400, false, &aim, 1.0, &mut rng);
        assert!(!o.fire);
    }
}
