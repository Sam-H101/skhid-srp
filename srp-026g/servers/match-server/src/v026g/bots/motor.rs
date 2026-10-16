//! How a bot moves its body: the keys and the view's turn for each tick,
//! made to look like a player's hands.
//!
//! - **The view turns smoothly.** A player's mouse does not jump to a new
//!   heading: the view speeds up, turns and slows into it. The turn of a
//!   tick comes from a turning speed that eases toward what the remaining
//!   angle asks ([`Feel::turn_time_s`]), within a top speed and a top
//!   change of speed a tick, and never past the heading aimed at.
//! - **It walks while it turns**, as players do, and lets go of forward
//!   only for a turn of more than [`Feel::stop_turn`] (picking it up again
//!   once facing near the way).
//! - **It steers at a point ahead** on the route ([`super::path`]), so it
//!   rounds a corner in a curve; never a point it could not walk to in a
//!   line, or it would cut the corner into the wall (the look ahead is
//!   shortened till the line is clear).
//! - **It sprints on long straight stretches** only: after facing the way
//!   for a moment, and not into a sharp corner.
//! - **Its heading wanders a little** (a slow sway of a degree or two) and
//!   its view settles near level, so no two bots walk the same line.
//! - **Standing**, it looks about: a new look every second or two, eased
//!   into as any turn.
//!
//! The numbers are [`Feel`]'s: tuning, not game data.

use std::f32::consts::PI;

use mapdata::Vec3;

use super::encode::Decision;
use super::kin::{TICK_S, wrap, yaw_of};
use super::path::Route;
use super::rng::Rng;
use crate::v026g::drive::key;

/// The look ahead is not shortened below this, metres.
const MIN_AHEAD_M: f32 = 0.5;

/// The motor's tuning.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Feel {
    /// The time to take most of a turn, seconds: the turning speed asked is
    /// the remaining angle over this.
    pub turn_time_s: f32,
    /// Top turning speed, radians a second.
    pub max_turn_speed: f32,
    /// Top change of the turning speed, radians a second per second.
    pub max_turn_accel: f32,
    /// Forward is let go for a turn larger than this, radians …
    pub stop_turn: f32,
    /// … and held again once the turn left is under this.
    pub go_turn: f32,
    /// The point steered at is this far ahead, metres, plus
    /// `look_ahead_per_speed` × speed.
    pub look_ahead_m: f32,
    pub look_ahead_per_speed: f32,
    /// Sprint only with this much straight route ahead, metres …
    pub sprint_from_m: f32,
    /// … and stop sprinting with this little left before a corner.
    pub sprint_until_m: f32,
    /// Facing the way within this for `sprint_after_s` before a sprint.
    pub sprint_facing: f32,
    pub sprint_after_s: f32,
    /// A corner sharper than this ends a straight stretch, radians.
    pub corner: f32,
    /// The heading's sway: amplitude (radians) and period (seconds).
    pub sway: f32,
    pub sway_period_s: f32,
    /// The route's end is reached this near, metres.
    pub arrive_m: f32,
    /// Standing, a new look every this many seconds (low, high) …
    pub look_every_s: (f32, f32),
    /// … turning by up to this, radians.
    pub look_turn: f32,
}

impl Default for Feel {
    fn default() -> Self {
        Self {
            turn_time_s: 0.15,
            max_turn_speed: 5.0,
            max_turn_accel: 45.0,
            stop_turn: 1.4,
            go_turn: 0.7,
            look_ahead_m: 1.6,
            look_ahead_per_speed: 0.35,
            sprint_from_m: 10.0,
            sprint_until_m: 3.5,
            sprint_facing: 0.17,
            sprint_after_s: 0.3,
            corner: 0.6,
            sway: 0.025,
            sway_period_s: 3.7,
            arrive_m: 0.6,
            look_every_s: (0.8, 2.2),
            look_turn: 1.1,
        }
    }
}

/// Where the body is, as the motor needs it.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub feet: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// Ground speed, m/s.
    pub speed: f32,
}

/// What the motor did with a route this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Walked {
    OnTheWay,
    Arrived,
    /// Farther off the route than the caller allows.
    Off(f32),
}

/// A jump not yet up its ledge is tried again after this long, seconds.
pub const JUMP_AGAIN_S: f32 = 1.0;
/// A low leg this near ahead, metres, is walked crouched already.
pub const LOW_AHEAD_M: f32 = 1.0;

/// One bot's hands.
#[derive(Clone, Debug)]
pub struct Motor {
    pub feel: Feel,
    /// The view's turning speed now (yaw, pitch), radians a second.
    rate: (f32, f32),
    walking: bool,
    sprinting: bool,
    /// Ticks faced the way well enough to sprint.
    facing_ticks: u32,
    /// The waypoint last jumped to and the ticks since
    /// ([`Motor::jump_key`]).
    jumped_at: Option<(usize, u32)>,
    /// The sway's phase and the time, seconds, for it.
    phase: f32,
    t: f32,
    /// Standing: the heading looked at, and ticks till the next look.
    look: Option<(f32, u32)>,
    /// The turn's time constant for this tick when not the feel's (the
    /// reflex aims faster or slower by skill).
    pub turn_time: Option<f32>,
}

impl Motor {
    pub fn new(feel: Feel, rng: &mut Rng) -> Self {
        Self {
            feel,
            rate: (0.0, 0.0),
            walking: false,
            sprinting: false,
            facing_ticks: 0,
            jumped_at: None,
            phase: rng.range(0.0, 2.0 * PI),
            t: 0.0,
            look: None,
            turn_time: None,
        }
    }

    /// The tick's turn toward `yaw` and `pitch` from the pose: eased, within
    /// the top speed and change of speed, never past the aim.
    fn turn_toward(&mut self, pose: &Pose, yaw: f32, pitch: f32) -> (f32, f32) {
        let feel = &self.feel;
        let turn_time = self.turn_time.unwrap_or(feel.turn_time_s).max(0.02);
        let ease = |rate: &mut f32, left: f32| {
            let want = (left / turn_time).clamp(-feel.max_turn_speed, feel.max_turn_speed);
            let step = feel.max_turn_accel * TICK_S;
            *rate += (want - *rate).clamp(-step, step);
            let mut turn = *rate * TICK_S;
            // Not past the aim: a player slows into it.
            if turn * left <= 0.0 && left.abs() > 1e-4 {
                turn = 0.0;
                *rate = 0.0;
            } else if turn.abs() > left.abs() {
                turn = left;
                *rate = left / TICK_S;
            }
            turn
        };
        let yaw_left = wrap(yaw - pose.yaw);
        let pitch_left = pitch - pose.pitch;
        let (mut yaw_rate, mut pitch_rate) = self.rate;
        let turn = (
            ease(&mut yaw_rate, yaw_left),
            ease(&mut pitch_rate, pitch_left),
        );
        self.rate = (yaw_rate, pitch_rate);
        turn
    }

    fn sway(&mut self) -> f32 {
        self.t += TICK_S;
        self.feel.sway * (self.phase + self.t * 2.0 * PI / self.feel.sway_period_s).sin()
    }

    /// The decision of one tick that walks the route from `pose`. `off_m`:
    /// how far off the route the caller lets the body be before it plans
    /// again; `clear(point)`: whether the body walks to `point` in a line.
    pub fn walk(
        &mut self,
        route: &mut Route,
        pose: &Pose,
        off_m: f32,
        clear: impl Fn(Vec3) -> bool,
    ) -> (Decision, Walked) {
        self.look = None;
        let (s, off) = route.progress(pose.feet);
        let left = route.length() - s;
        if left <= self.feel.arrive_m || pose.feet.distance_xz(route.end()) <= self.feel.arrive_m {
            self.walking = false;
            self.sprinting = false;
            let turn = self.turn_toward(pose, pose.yaw, 0.0);
            return (Decision { keys: 0, turn }, Walked::Arrived);
        }
        if off > off_m {
            return (Decision::default(), Walked::Off(off));
        }
        let mut ahead = self.feel.look_ahead_m + self.feel.look_ahead_per_speed * pose.speed;
        let mut target = route.point_at(s + ahead);
        while ahead > MIN_AHEAD_M && !clear(target) {
            ahead *= 0.6;
            target = route.point_at(s + ahead);
        }
        let (dx, dz) = (target.x - pose.feet.x, target.z - pose.feet.z);
        let sway = self.sway();
        let heading = if dx.abs() + dz.abs() < 1e-3 {
            pose.yaw
        } else {
            yaw_of([dx, dz]) + sway
        };
        let turn = self.turn_toward(pose, heading, 0.0);
        let still = wrap(heading - (pose.yaw + turn.0)).abs();

        // Walk while turning; let go only for a big turn.
        if self.walking && still > self.feel.stop_turn {
            self.walking = false;
        } else if !self.walking && still < self.feel.go_turn {
            self.walking = true;
        }
        // Sprint on a long straight stretch, facing the way.
        let (straight, _) = route.straight_ahead(s, self.feel.corner);
        if self.walking && still < self.feel.sprint_facing {
            self.facing_ticks += 1;
        } else {
            self.facing_ticks = 0;
        }
        let settled = self.facing_ticks as f32 * TICK_S >= self.feel.sprint_after_s;
        if !self.sprinting && settled && straight >= self.feel.sprint_from_m {
            self.sprinting = true;
        } else if self.sprinting
            && (!self.walking
                || straight < self.feel.sprint_until_m
                || still > 2.0 * self.feel.sprint_facing)
        {
            self.sprinting = false;
        }
        // Crouched where a standing body does not fit (a low passage):
        // no sprint there.
        let low = route.low_ahead(s, LOW_AHEAD_M);
        if low {
            self.sprinting = false;
        }
        let mut keys = 0;
        if self.walking {
            keys |= key::FORWARD;
            if self.sprinting {
                keys |= key::SPRINT;
            }
            if low {
                keys |= key::CROUCH;
            }
        }
        keys |= self.jump_key(route, s);
        (Decision { keys, turn }, Walked::OnTheWay)
    }

    /// The decision of one tick that walks the route while the view looks
    /// at `aim` (yaw, pitch): the legs pick the key of the eight ways
    /// nearest the way to go, as a player strafes while aiming; no sprint.
    pub fn walk_aiming(
        &mut self,
        route: &mut Route,
        pose: &Pose,
        off_m: f32,
        clear: impl Fn(Vec3) -> bool,
        aim: (f32, f32),
    ) -> (Decision, Walked) {
        self.look = None;
        self.sprinting = false;
        self.facing_ticks = 0;
        let (s, off) = route.progress(pose.feet);
        let turn = self.turn_toward(pose, aim.0, aim.1);
        if route.length() - s <= self.feel.arrive_m
            || pose.feet.distance_xz(route.end()) <= self.feel.arrive_m
        {
            self.walking = false;
            return (Decision { keys: 0, turn }, Walked::Arrived);
        }
        if off > off_m {
            return (Decision { keys: 0, turn }, Walked::Off(off));
        }
        let mut ahead = self.feel.look_ahead_m;
        let mut target = route.point_at(s + ahead);
        while ahead > MIN_AHEAD_M && !clear(target) {
            ahead *= 0.6;
            target = route.point_at(s + ahead);
        }
        let (dx, dz) = (target.x - pose.feet.x, target.z - pose.feet.z);
        self.walking = true;
        let mut keys = keys_toward(wrap(yaw_of([dx, dz]) - (pose.yaw + turn.0)));
        if route.low_ahead(s, LOW_AHEAD_M) {
            keys |= key::CROUCH;
        }
        // Up a ledge while aiming too (the walk aiming had no jump: a body
        // walked into the ledge's wall for as long as it aimed).
        keys |= self.jump_key(route, s);
        (Decision { keys, turn }, Walked::OnTheWay)
    }

    /// The jump of this tick up the route's next ledge: just before it,
    /// walking into it; again every [`JUMP_AGAIN_S`] while the body is still
    /// short of it (a jump that did not get it up: it hit the edge, or it
    /// came too early).
    fn jump_key(&mut self, route: &Route, s: f32) -> u32 {
        if let Some((_, ticks)) = self.jumped_at.as_mut() {
            *ticks = ticks.saturating_add(1);
        }
        let Some((k, before)) = route.next_jump(s) else {
            return 0;
        };
        let again = match self.jumped_at {
            Some((at, ticks)) => at != k || ticks as f32 * TICK_S >= JUMP_AGAIN_S,
            None => true,
        };
        if self.walking && before < 0.8 && again {
            self.jumped_at = Some((k, 0));
            key::JUMP
        } else {
            0
        }
    }

    /// The decision of one tick standing and looking at `aim`.
    pub fn aim_still(&mut self, pose: &Pose, aim: (f32, f32)) -> Decision {
        self.walking = false;
        self.sprinting = false;
        self.facing_ticks = 0;
        self.look = None;
        let turn = self.turn_toward(pose, aim.0, aim.1);
        Decision { keys: 0, turn }
    }

    /// The decision of one tick standing still and looking about.
    pub fn stand(&mut self, pose: &Pose, rng: &mut Rng) -> Decision {
        self.walking = false;
        self.sprinting = false;
        self.facing_ticks = 0;
        let (low, high) = self.feel.look_every_s;
        let (aim, ticks) = match self.look {
            Some((aim, ticks)) if ticks > 0 => (aim, ticks - 1),
            _ => (
                wrap(pose.yaw + rng.range(-self.feel.look_turn, self.feel.look_turn)),
                (rng.range(low, high) / TICK_S) as u32,
            ),
        };
        self.look = Some((aim, ticks));
        let turn = self.turn_toward(pose, aim, 0.0);
        Decision { keys: 0, turn }
    }

    /// Hands off: no keys, no turn (dead, or waiting for a world).
    pub fn rest(&mut self) -> Decision {
        self.rate = (0.0, 0.0);
        self.walking = false;
        self.sprinting = false;
        self.facing_ticks = 0;
        self.look = None;
        Decision::default()
    }
}

/// The keys of the eight ways nearest `relative` (the way to go less the
/// view's yaw; a positive angle is to the left, `kin`).
pub fn keys_toward(relative: f32) -> u32 {
    use std::f32::consts::FRAC_PI_4;
    let sector = (wrap(relative) / FRAC_PI_4).round() as i32;
    match sector.rem_euclid(8) {
        0 => key::FORWARD,
        1 => key::FORWARD | key::LEFT,
        2 => key::LEFT,
        3 => key::BACK | key::LEFT,
        4 => key::BACK,
        5 => key::BACK | key::RIGHT,
        6 => key::RIGHT,
        _ => key::FORWARD | key::RIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v026g::bots::kin::Kinematics;

    #[test]
    fn the_eight_ways_match_the_movement_model() {
        let kin = Kinematics::default();
        for k in -8..8 {
            let relative = k as f32 * 0.4;
            let keys = keys_toward(relative);
            let [x, z] = kin.target_velocity(keys, 0.0, 1.0);
            // The way the keys go is within 22.5 degrees of the asked one.
            let went = yaw_of([x, z]);
            assert!(
                wrap(went - relative).abs() <= std::f32::consts::FRAC_PI_8 + 1e-3,
                "{relative} -> {keys:#x}"
            );
        }
    }
    use crate::v026g::bots::path::Waypoint;

    fn route(points: &[(f32, f32)]) -> Route {
        Route::new(
            points
                .iter()
                .map(|&(x, z)| Waypoint {
                    at: Vec3::new(x, 0.0, z),
                    node: 0,
                    jump: false,
                    low: false,
                })
                .collect(),
        )
    }

    /// A route along x with waypoint 1 (at x = 5) up a ledge, and the leg
    /// to waypoint 2 low (from x = 5 to 10).
    fn ledge_then_low() -> Route {
        let p = |x: f32, jump, low| Waypoint {
            at: Vec3::new(x, 0.0, 0.0),
            node: 0,
            jump,
            low,
        };
        Route::new(vec![
            p(0.0, false, false),
            p(5.0, true, false),
            p(10.0, false, true),
            p(20.0, false, false),
        ])
    }

    fn facing_x(feet: Vec3) -> Pose {
        Pose {
            feet,
            yaw: yaw_of([1.0, 0.0]),
            pitch: 0.0,
            speed: 0.0,
        }
    }

    /// A body kept short of a ledge (it hits the edge) jumps again every
    /// [`JUMP_AGAIN_S`], walking or aiming; once up, it does not.
    #[test]
    fn a_jump_that_did_not_get_up_is_tried_again_also_aiming() {
        let mut rng = Rng::new(3);
        for aiming in [false, true] {
            let mut motor = Motor::new(Feel::default(), &mut rng);
            let mut route = ledge_then_low();
            let pose = facing_x(Vec3::new(4.5, 0.0, 0.0));
            let mut jumps = Vec::new();
            for tick in 0..100 {
                let (decision, _) = if aiming {
                    motor.walk_aiming(&mut route, &pose, 5.0, |_| true, (pose.yaw, 0.0))
                } else {
                    motor.walk(&mut route, &pose, 5.0, |_| true)
                };
                if decision.keys & key::JUMP != 0 {
                    jumps.push(tick);
                }
            }
            // 5 s held short of it: a jump at once and every second.
            let again = (JUMP_AGAIN_S / TICK_S) as usize;
            assert_eq!(jumps.len(), 100 / again, "{aiming}: {jumps:?}");
            assert!(jumps.windows(2).all(|w| w[1] - w[0] == again), "{jumps:?}");
            // Up on the ledge: no more jumps.
            let up = facing_x(Vec3::new(5.5, 0.0, 0.0));
            for _ in 0..40 {
                let (decision, _) = motor.walk(&mut route, &up, 5.0, |_| true);
                assert_eq!(decision.keys & key::JUMP, 0);
            }
        }
    }

    /// A low leg is walked crouched, from a metre before it, and not
    /// sprinted; past it the body stands again.
    #[test]
    fn a_low_leg_is_walked_crouched() {
        let mut rng = Rng::new(5);
        let mut motor = Motor::new(Feel::default(), &mut rng);
        let mut route = ledge_then_low();
        let crouched = |motor: &mut Motor, route: &mut Route, x: f32| {
            let (decision, _) = motor.walk(route, &facing_x(Vec3::new(x, 0.0, 0.0)), 5.0, |_| true);
            decision.keys & key::CROUCH != 0
        };
        assert!(!crouched(&mut motor, &mut route, 2.0));
        assert!(crouched(&mut motor, &mut route, 4.5));
        assert!(crouched(&mut motor, &mut route, 7.0));
        let (decision, _) =
            motor.walk(&mut route, &facing_x(Vec3::new(7.0, 0.0, 0.0)), 5.0, |_| {
                true
            });
        assert_eq!(decision.keys & key::SPRINT, 0);
        assert!(!crouched(&mut motor, &mut route, 12.0));
        // Aiming too (a route of its own: a route keeps where the body was).
        let mut route = ledge_then_low();
        let (decision, _) = motor.walk_aiming(
            &mut route,
            &facing_x(Vec3::new(7.0, 0.0, 0.0)),
            5.0,
            |_| true,
            (0.0, 0.0),
        );
        assert_ne!(decision.keys & key::CROUCH, 0, "aiming too");
    }

    /// Walks a body that does what the keys and turns say, with the
    /// movement model, until it arrives; returns the decisions.
    fn walk(route: &mut Route, start: Vec3, yaw: f32, ticks: usize) -> (Vec<Decision>, Pose) {
        let kin = Kinematics::default();
        let mut rng = Rng::new(7);
        let mut motor = Motor::new(Feel::default(), &mut rng);
        let mut pose = Pose {
            feet: start,
            yaw,
            pitch: 0.0,
            speed: 0.0,
        };
        let mut blend = crate::v026g::bots::kin::Blend::default();
        let mut decisions = Vec::new();
        for _ in 0..ticks {
            let (decision, walked) = motor.walk(route, &pose, 5.0, |_| true);
            assert!(!matches!(walked, Walked::Off(_)), "{pose:?}");
            decisions.push(decision);
            if walked == Walked::Arrived {
                break;
            }
            pose.yaw = wrap(pose.yaw + decision.turn.0);
            pose.pitch += decision.turn.1;
            let local = blend.tick(kin.target_local(decision.keys, 1.0), kin.blend_ticks);
            let velocity = crate::v026g::bots::kin::to_world(local, pose.yaw);
            pose.feet = Vec3::new(
                pose.feet.x + velocity[0] * TICK_S,
                0.0,
                pose.feet.z + velocity[1] * TICK_S,
            );
            pose.speed = velocity[0].hypot(velocity[1]);
        }
        (decisions, pose)
    }

    #[test]
    fn a_body_walks_a_route_round_a_corner_smoothly_and_sprints_on_the_straight() {
        let feel = Feel::default();
        // 30 m along +z, a right angle, 20 m along +x; starting faced away.
        let mut r = route(&[(0.0, 0.0), (0.0, 30.0), (20.0, 30.0)]);
        let (decisions, pose) = walk(&mut r, Vec3::ZERO, PI * 0.9, 600);
        assert!(pose.feet.distance_xz(Vec3::new(20.0, 0.0, 30.0)) <= feel.arrive_m + 0.3);
        // Smooth: the turning speed changes by at most the top change a
        // tick, and never passes the top speed.
        let rates: Vec<f32> = decisions.iter().map(|d| d.turn.0 / TICK_S).collect();
        for pair in rates.windows(2) {
            assert!(
                (pair[1] - pair[0]).abs() <= feel.max_turn_accel * TICK_S + 1e-3
                    || pair[1] == 0.0
                    || pair[1].abs() < pair[0].abs(),
                "{pair:?}"
            );
        }
        assert!(rates.iter().all(|r| r.abs() <= feel.max_turn_speed + 1e-3));
        // It sprinted on the 30 m stretch, and not into the corner.
        let sprint = |d: &Decision| d.keys & key::SPRINT != 0;
        assert!(decisions.iter().any(sprint));
        // The about-turn at the start: forward let go, then taken up.
        assert_eq!(decisions[0].keys & key::FORWARD, 0);
        assert!(decisions.iter().any(|d| d.keys & key::FORWARD != 0));
        // 50 m at 3.3 to 5.25 m/s plus the turn: well under 20 s.
        assert!(decisions.len() < 400, "{}", decisions.len());
        // Keys change seldom: no flicker of forward or sprint.
        let changes = decisions
            .windows(2)
            .filter(|w| w[0].keys != w[1].keys)
            .count();
        assert!(changes <= 8, "{changes}");
    }

    #[test]
    fn standing_it_looks_about_in_eased_turns() {
        let mut rng = Rng::new(3);
        let mut motor = Motor::new(Feel::default(), &mut rng);
        let mut pose = Pose {
            feet: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            speed: 0.0,
        };
        let mut turned = 0.0_f32;
        for _ in 0..200 {
            let decision = motor.stand(&pose, &mut rng);
            assert_eq!(decision.keys, 0);
            assert!(decision.turn.0.abs() <= Feel::default().max_turn_speed * TICK_S + 1e-4);
            pose.yaw = wrap(pose.yaw + decision.turn.0);
            turned += decision.turn.0.abs();
        }
        assert!(turned > 1.0, "it looked about: {turned}");
    }
}
