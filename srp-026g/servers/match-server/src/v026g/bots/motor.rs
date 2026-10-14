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
#[derive(Clone, Debug, PartialEq)]
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
    jumped_at: Option<usize>,
    /// The sway's phase and the time, seconds, for it.
    phase: f32,
    t: f32,
    /// Standing: the heading looked at, and ticks till the next look.
    look: Option<(f32, u32)>,
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
        }
    }

    /// The tick's turn toward `yaw` and `pitch` from the pose: eased, within
    /// the top speed and change of speed, never past the aim.
    fn turn_toward(&mut self, pose: &Pose, yaw: f32, pitch: f32) -> (f32, f32) {
        let feel = &self.feel;
        let ease = |rate: &mut f32, left: f32| {
            let want = (left / feel.turn_time_s).clamp(-feel.max_turn_speed, feel.max_turn_speed);
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
        let mut keys = 0;
        if self.walking {
            keys |= key::FORWARD;
            if self.sprinting {
                keys |= key::SPRINT;
            }
        }
        // A jump up a ledge: once, just before it, walking into it.
        if let Some((k, before)) = route.next_jump(s)
            && self.walking
            && before < 0.8
            && self.jumped_at != Some(k)
        {
            keys |= key::JUMP;
            self.jumped_at = Some(k);
        }
        (Decision { keys, turn }, Walked::OnTheWay)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v026g::bots::kin::Kinematics;
    use crate::v026g::bots::path::Waypoint;

    fn route(points: &[(f32, f32)]) -> Route {
        Route::new(
            points
                .iter()
                .map(|&(x, z)| Waypoint {
                    at: Vec3::new(x, 0.0, z),
                    node: 0,
                    jump: false,
                })
                .collect(),
        )
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
        let mut velocity = [0.0, 0.0];
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
            velocity = kin.approach(velocity, kin.target_velocity(decision.keys, pose.yaw, 1.0));
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
