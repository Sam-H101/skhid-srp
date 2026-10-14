//! How a player's body moves for the keys it holds: the bot's model of
//! the game's movement, used to keep track of where its body is between
//! two worlds ([`super::body`]) and to plan its steps ([`super::motor`]).
//!
//! Measured on the saved worlds of real matches (163,369 player rows of
//! 165 matches; the numbers and how sure each is are in the private
//! `KINEMATICS.md` of the bot research, summed up in
//! `docs/notes/player_bots.md` §4):
//! - forward at `yaw` is `(-sin yaw, cos yaw)` in the map frame; a
//!   positive yaw turns left; LEFT goes to yaw + 90°, RIGHT to yaw − 90°;
//!   diagonals at ±45° at the forward speed (PROVEN);
//! - the yaw is never wrapped by the game (it runs up past ±50 rad); the
//!   pitch is up-positive, clamped at ±1 (PROVEN; the clamp LIKELY);
//! - speeds in m/s: walk 3.5 (each life at ×0.95, ×1 or ×1.05 of it),
//!   sprint about 5.2, strafe and crouch 0.75 of walk, aimed 0.9, aimed
//!   strafe 0.6 (PROVEN), back 0.85 (LIKELY);
//! - a body speeds up by about 0.6 m/s a tick and slows by about 0.5
//!   (LIKELY), and a world's position does not show that world's keys yet
//!   (LIKELY): the keys of a tick move the body from the next one.
//!
//! The model needs only to be near: every world the server is given puts
//! the body back where the game has it.

use std::f32::consts::{PI, TAU};

use crate::v026g::drive::key;

/// The game's ticks, seconds.
pub const TICK_S: f32 = 0.05;

/// Speeds in m/s and how fast they are reached.
#[derive(Clone, Debug, PartialEq)]
pub struct Kinematics {
    pub walk: f32,
    pub sprint: f32,
    /// Of `walk`.
    pub strafe: f32,
    pub back: f32,
    pub crouch: f32,
    pub crouch_strafe: f32,
    pub aim: f32,
    pub aim_strafe: f32,
    /// m/s a tick.
    pub speed_up: f32,
    pub slow_down: f32,
}

impl Default for Kinematics {
    fn default() -> Self {
        Self {
            walk: 3.5,
            sprint: 5.2,
            strafe: 0.75,
            back: 0.85,
            crouch: 0.75,
            crouch_strafe: 0.6,
            aim: 0.9,
            aim_strafe: 0.6,
            speed_up: 0.6,
            slow_down: 0.5,
        }
    }
}

/// The ground direction a body faces at `yaw` (map frame, x and z).
pub fn forward(yaw: f32) -> [f32; 2] {
    let (s, c) = yaw.sin_cos();
    [-s, c]
}

/// The yaw that faces the ground direction `(x, z)`; the inverse of
/// [`forward`], in `(-π, π]`.
pub fn yaw_of(direction: [f32; 2]) -> f32 {
    (-direction[0]).atan2(direction[1])
}

/// `angle` brought into `(-π, π]`.
pub fn wrap(angle: f32) -> f32 {
    let a = (angle + PI).rem_euclid(TAU) - PI;
    if a <= -PI { a + TAU } else { a }
}

impl Kinematics {
    /// The ground velocity the keys aim for at `yaw`, for a body whose
    /// speeds are `level` times the nominal ones.
    pub fn target_velocity(&self, keys: u32, yaw: f32, level: f32) -> [f32; 2] {
        let held = |bit| f32::from(u8::from(keys & bit != 0));
        let ahead = held(key::FORWARD) - held(key::BACK);
        let side = held(key::RIGHT) - held(key::LEFT);
        if ahead == 0.0 && side == 0.0 {
            return [0.0, 0.0];
        }
        let (crouched, aimed) = (keys & key::CROUCH != 0, keys & key::AIM != 0);
        let sideways = ahead == 0.0;
        let factor = match (crouched, aimed, sideways) {
            (false, false, false) if ahead < 0.0 => self.back,
            (false, false, false) => 1.0,
            (false, false, true) => self.strafe,
            (true, false, false) => self.crouch,
            (true, false, true) => self.crouch_strafe,
            (false, true, false) => self.aim,
            (false, true, true) => self.aim_strafe,
            (true, true, false) => self.crouch * self.aim,
            (true, true, true) => self.crouch_strafe * self.aim,
        };
        let sprinting = ahead > 0.0 && keys & key::SPRINT != 0 && !crouched && !aimed;
        let speed = level
            * if sprinting {
                self.sprint
            } else {
                self.walk * factor
            };
        let [fx, fz] = forward(yaw);
        // RIGHT is a quarter turn clockwise of forward: (cos, sin).
        let [rx, rz] = [fz, -fx];
        let (x, z) = (ahead * fx + side * rx, ahead * fz + side * rz);
        let n = (x * x + z * z).sqrt();
        [x / n * speed, z / n * speed]
    }

    /// The velocity one tick later, from `now` toward `target`: by at most
    /// [`Self::speed_up`] a tick in speed while gaining, [`Self::slow_down`]
    /// while losing; a turn of the velocity is at once.
    pub fn approach(&self, now: [f32; 2], target: [f32; 2]) -> [f32; 2] {
        let (now_speed, target_speed) = (now[0].hypot(now[1]), target[0].hypot(target[1]));
        let speed = if target_speed >= now_speed {
            (now_speed + self.speed_up).min(target_speed)
        } else {
            (now_speed - self.slow_down).max(target_speed)
        };
        let direction = if target_speed > 1e-4 {
            [target[0] / target_speed, target[1] / target_speed]
        } else if now_speed > 1e-4 {
            [now[0] / now_speed, now[1] / now_speed]
        } else {
            return [0.0, 0.0];
        };
        [direction[0] * speed, direction[1] * speed]
    }

    /// The body's speed level from a world: its speed over the nominal one
    /// of its keys, if it moved steadily under keys that have a level
    /// (walking, not sprinting), else `None`.
    pub fn level_of(&self, keys: u32, speed: f32) -> Option<f32> {
        if keys & key::SPRINT != 0
            || keys & (key::FORWARD | key::BACK | key::LEFT | key::RIGHT) == 0
        {
            return None;
        }
        let nominal = self.target_velocity(keys, 0.0, 1.0);
        let nominal = nominal[0].hypot(nominal[1]);
        let level = speed / nominal;
        (0.93..=1.07).contains(&level).then_some(level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_and_direction_are_inverse_and_angles_wrap() {
        for yaw in [-3.0_f32, -1.0, 0.0, 0.5, 2.0, 3.1] {
            assert!((yaw_of(forward(yaw)) - yaw).abs() < 1e-5, "{yaw}");
        }
        assert!((wrap(3.0 * PI) - PI).abs() < 1e-5);
        assert!((wrap(-PI) - PI).abs() < 1e-5);
        assert!((wrap(0.25 - TAU) - 0.25).abs() < 1e-5);
        // The game's yaw runs past ±π; the direction is the same.
        let (a, b) = (forward(0.4), forward(0.4 + 8.0 * PI));
        assert!((a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4);
    }

    #[test]
    fn keys_give_the_measured_speeds_and_directions() {
        let k = Kinematics::default();
        let speed = |keys| {
            let [x, z] = k.target_velocity(keys, 0.7, 1.0);
            x.hypot(z)
        };
        let close = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(close(speed(key::FORWARD), 3.5));
        assert!(close(speed(key::FORWARD | key::SPRINT), 5.2));
        assert!(
            close(speed(key::FORWARD | key::LEFT), 3.5),
            "diagonals at full speed"
        );
        assert!(close(speed(key::LEFT), 2.625));
        assert!(close(speed(key::FORWARD | key::CROUCH), 2.625));
        assert!(close(speed(key::FORWARD | key::AIM), 3.15));
        assert!(close(speed(key::RIGHT | key::AIM), 2.1));
        assert!(
            close(speed(key::FORWARD | key::CROUCH | key::SPRINT), 2.625),
            "no crouched sprint"
        );
        assert_eq!(speed(key::FORWARD | key::BACK), 0.0);
        assert_eq!(speed(key::FIRE), 0.0);
        // At yaw 0 forward is +z, LEFT is -x, RIGHT +x (left-handed, a
        // positive yaw turns left).
        let at = |keys| k.target_velocity(keys, 0.0, 1.0);
        assert!(close(at(key::FORWARD)[1], 3.5));
        assert!(close(at(key::LEFT)[0], -2.625));
        assert!(close(at(key::RIGHT)[0], 2.625));
        assert!(
            forward(0.3)[0] < 0.0,
            "a positive yaw turns toward -x: left"
        );
        // A level scales every speed.
        assert!(close(k.target_velocity(key::FORWARD, 0.0, 0.95)[1], 3.325));
    }

    #[test]
    fn a_body_speeds_up_and_slows_down_by_steps() {
        let k = Kinematics::default();
        let target = k.target_velocity(key::FORWARD, 0.0, 1.0);
        let mut v = [0.0, 0.0];
        let mut ticks = 0;
        while v[1] < 3.5 - 1e-4 {
            v = k.approach(v, target);
            ticks += 1;
        }
        assert_eq!(ticks, 6, "0 to 3.5 m/s in about 0.3 s");
        let v = k.approach(v, [0.0, 0.0]);
        assert!((v[1] - 3.0).abs() < 1e-4);
        assert!((k.level_of(key::FORWARD, 3.675).unwrap() - 1.05).abs() < 1e-4);
        assert_eq!(k.level_of(key::FORWARD | key::SPRINT, 5.2), None);
        assert_eq!(k.level_of(key::FORWARD, 1.0), None, "not steady");
    }
}
