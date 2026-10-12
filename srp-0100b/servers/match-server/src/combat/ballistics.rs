// A bullet in flight, ported from `game_core/sources/bullet.cpp`: a parabolic
// path with linear air drag, cut into segments that stay within 5 cm of the
// curve, each ray-tested; surfaces either stop the bullet, let it through
// slower, or ricochet it. Damage does not depend on speed or distance.

use super::damage::LastHit;
use super::data::Material;
use super::math::Vec3;
use super::traps::TrapId;

pub const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);
/// `s_bm_current_air_resistance`: the drag a bullet starts with. The ammo's
/// own `air_resistance` only applies after a pierce or ricochet.
const INITIAL_AIR_RESISTANCE: f32 = 1.0;
const MAX_TRAJECTORY_CHANGES: u32 = 32;
const MAX_RICOCHETS: u8 = 2;
const EPSILON_3: f32 = 1e-3;
const EPSILON_5: f32 = 1e-5;

fn is_similar(a: f32, b: f32, epsilon: f32) -> bool {
    (a - b).abs() < epsilon
}

#[derive(Clone, Debug, PartialEq)]
pub struct RayHit {
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub material: u16,
    pub target: Option<Target>,
}

/// What a bullet can hurt.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A player's hit box: (player id, body part).
    Player(u8, String),
    /// A trap's hittable volume (`booby_trap_core::hit`).
    Trap(TrapId),
}

/// What a bullet can hit.
pub trait BulletWorld {
    /// The closest hit along unit `dir` within `length`, with players posed
    /// as they were at server time `time_ms`; never the shooter.
    fn ray(
        &self,
        origin: Vec3,
        dir: Vec3,
        length: f32,
        time_ms: f64,
        shooter: u8,
    ) -> Option<RayHit>;
    fn contains(&self, point: Vec3) -> bool;
    fn material(&self, id: u16) -> Material;
}

#[derive(Clone, Debug, PartialEq)]
pub enum BulletHit {
    Player { victim: u8, body_part: String },
    Trap(TrapId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Collision {
    None,
    Collide,
    Pierced,
    Reflected,
}

#[derive(Clone, Debug)]
pub struct Bullet {
    pub shooter: u8,
    pub weapon_dict_id: u16,
    /// `bullet_damage * k_damage`.
    pub damage: f32,
    /// `bullet_pierce * k_arp`.
    pub pierce: f32,
    /// How far back other players are posed for this bullet (lag
    /// compensation, fixed when fired).
    pub rewind_ms: f64,
    pub last_hit: LastHit,
    ricochet_angle: f32,
    air_resistance: f32,
    current_resistance: f32,
    max_distance: f32,
    flown_distance: f32,
    start_position: Vec3,
    start_velocity: Vec3,
    position: Vec3,
    velocity: Vec3,
    born_time_ms: u64,
    life_time: f32,
    change_trajectory_count: u32,
    ricochet_count: u8,
    finished: bool,
}

pub struct BulletSpec {
    pub shooter: u8,
    pub weapon_dict_id: u16,
    pub damage: f32,
    pub pierce: f32,
    pub ricochet_angle_degrees: f32,
    pub air_resistance: f32,
    pub distance_coef: f32,
    pub rewind_ms: f64,
}

impl Bullet {
    pub fn new(spec: BulletSpec, position: Vec3, velocity: Vec3, born_time_ms: u64) -> Self {
        Self {
            shooter: spec.shooter,
            weapon_dict_id: spec.weapon_dict_id,
            damage: spec.damage,
            pierce: spec.pierce,
            rewind_ms: spec.rewind_ms,
            last_hit: None,
            ricochet_angle: spec.ricochet_angle_degrees.to_radians(),
            air_resistance: spec.air_resistance * INITIAL_AIR_RESISTANCE,
            current_resistance: INITIAL_AIR_RESISTANCE,
            max_distance: spec.distance_coef * 1000.0,
            flown_distance: 0.0,
            start_position: position,
            start_velocity: velocity,
            position,
            velocity,
            born_time_ms,
            life_time: 0.0,
            change_trajectory_count: 0,
            ricochet_count: 0,
            finished: false,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    #[cfg(test)]
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// The server time (ms) the bullet has flown up to.
    pub fn flown_until_ms(&self) -> u64 {
        self.born_time_ms + (f64::from(self.life_time) * 1000.0) as u64
    }

    /// `bullet::tick`: flies up to `now_ms`, returning the player hits in
    /// order.
    pub fn tick(&mut self, now_ms: u64, world: &dyn BulletWorld) -> Vec<BulletHit> {
        let mut hits = Vec::new();
        if self.finished {
            return hits;
        }
        let mut low_time = self.life_time;
        let mut high_time = now_ms.saturating_sub(self.born_time_ms) as f32 / 1000.0;
        // A runaway loop would stall the match; real flights need a few dozen
        // segments per tick at most.
        for _ in 0..10_000 {
            if self.velocity.squared_length() < 1.0
                || self.change_trajectory_count >= MAX_TRAJECTORY_CHANGES
            {
                self.finish();
                return hits;
            }
            if low_time >= high_time {
                return hits;
            }
            let mut time = self.pick_next_permissible_time(low_time, high_time);
            if low_time == time {
                self.finish();
                return hits;
            }
            let result = self.check_collision(self.position, low_time, time, world, &mut hits);
            if result == Collision::Collide {
                self.finish();
                return hits;
            }
            if matches!(result, Collision::Pierced | Collision::Reflected) {
                high_time -= time;
                time = self.life_time;
            }
            if time != 0.0 && !self.update_bullet_position(time, world) {
                self.finish();
                return hits;
            }
            if is_similar(time, high_time, EPSILON_5) {
                return hits;
            }
            low_time = time;
        }
        log::warn!(
            "bullet from player {} did not settle; dropped",
            self.shooter
        );
        self.finish();
        hits
    }

    fn finish(&mut self) {
        self.finished = true;
    }

    fn compute_parabolic_velocity(&self, time: f32) -> Vec3 {
        let xz = Vec3::new(self.start_velocity.x, 0.0, self.start_velocity.z);
        if xz.squared_length().abs() < EPSILON_5 {
            return self.start_velocity + GRAVITY * time;
        }
        self.start_velocity * (1.0 - self.current_resistance * time).max(0.0) + GRAVITY * time
    }

    fn compute_parabolic_position(&self, time: f32) -> Vec3 {
        let xz = Vec3::new(self.start_velocity.x, 0.0, self.start_velocity.z);
        let half_t2 = time * time * 0.5;
        if xz.squared_length().abs() < EPSILON_5 {
            return self.start_position + self.start_velocity * time + GRAVITY * half_t2;
        }
        self.start_position
            + self.start_velocity * time
            + self.start_velocity * -self.current_resistance * half_t2
            + GRAVITY * half_t2
    }

    fn parabolic_time(&self) -> f32 {
        (1.0 / (self.current_resistance * 1.1)).max(0.0)
    }

    fn trajectory_velocity(&self, time: f32) -> Vec3 {
        let parabolic_time = self.parabolic_time();
        let fall = time - parabolic_time;
        if fall < 0.0 {
            return self.compute_parabolic_velocity(time);
        }
        self.compute_parabolic_velocity(parabolic_time) + GRAVITY * fall
    }

    fn trajectory_position(&self, time: f32) -> Vec3 {
        let parabolic_time = self.parabolic_time();
        let fall = time - parabolic_time;
        if fall < 0.0 {
            return self.compute_parabolic_position(time);
        }
        self.compute_parabolic_position(parabolic_time)
            + self.compute_parabolic_velocity(parabolic_time) * fall
            + GRAVITY * (fall * fall * 0.5)
    }

    fn pick_next_permissible_time(&self, low_time: f32, high_time: f32) -> f32 {
        let start_high_time = high_time;
        let (in_vacuum, mut high_time) = self.pick_permissible_range(low_time, high_time);
        if in_vacuum {
            return high_time.min(start_high_time);
        }
        let mut low = low_time;
        let mut check_time = high_time;
        for _ in 0..64 {
            if is_similar(low, high_time, EPSILON_5) {
                break;
            }
            if self.compute_max_error(low_time, check_time) < 0.05 {
                low = check_time;
            } else {
                high_time = check_time;
            }
            check_time = (low + high_time) * 0.5;
        }
        low
    }

    /// Returns (true, time) past the parabolic part, else (false, time).
    fn pick_permissible_range(&self, low_time: f32, high_time: f32) -> (bool, f32) {
        let parabolic_time = self.parabolic_time();
        if low_time > parabolic_time {
            return (true, self.check_time_in_vacuum(low_time, high_time));
        }
        if high_time < parabolic_time {
            return (false, self.check_time(low_time, high_time));
        }
        if !is_similar(parabolic_time, low_time, EPSILON_5) {
            return (false, self.check_time(low_time, parabolic_time));
        }
        (false, self.check_time_in_vacuum(parabolic_time, high_time))
    }

    fn check_time_in_vacuum(&self, start_low: f32, high: f32) -> f32 {
        let max_test_distance = self.max_distance - self.flown_distance;
        let dt = high - start_low;
        let time_to_fly = (self.start_velocity * dt + GRAVITY * (dt * dt * 0.5)).length();
        if time_to_fly <= max_test_distance {
            return high;
        }
        let speed = self.trajectory_velocity(start_low).length();
        let g = -GRAVITY.y;
        let time = ((speed * speed + 2.0 * max_test_distance * g).sqrt() - speed) / g;
        (start_low + time).clamp(start_low, high)
    }

    fn check_time(&self, start_low: f32, high: f32) -> f32 {
        let max_test_distance = self.max_distance - self.flown_distance;
        let start = self.trajectory_position(start_low);
        let mut low = start_low;
        let mut high = high;
        let mut check_time = high;
        for _ in 0..64 {
            if is_similar(low, high, EPSILON_5) {
                break;
            }
            let intermediate = self.trajectory_position((start_low + check_time) * 0.5);
            let target = self.trajectory_position(check_time);
            let distance = (intermediate - start).length() + (intermediate - target).length();
            if distance < max_test_distance {
                low = check_time;
            } else {
                high = check_time;
            }
            check_time = (low + high) * 0.5;
        }
        low
    }

    fn compute_max_error(&self, low: f32, high: f32) -> f32 {
        let start = self.trajectory_position(low);
        let target = self.trajectory_position(high);
        let max_error = self.trajectory_position((low + high) * 0.5);
        let to_max_error = max_error - start;
        let magnitude = to_max_error.length();
        if magnitude == 0.0 {
            return 0.0;
        }
        let to_max_error = to_max_error * (1.0 / magnitude);
        let to_target = (target - start).normalized_or(to_max_error);
        let cosine = to_max_error.dot(to_target).clamp(-1.0, 1.0);
        magnitude * (1.0 - cosine * cosine).sqrt()
    }

    fn check_collision(
        &mut self,
        start_position: Vec3,
        start_time: f32,
        current_time: f32,
        world: &dyn BulletWorld,
        hits: &mut Vec<BulletHit>,
    ) -> Collision {
        let target_position = self.trajectory_position(current_time);
        let direction = target_position - start_position;
        let distance = direction.length();
        if distance.abs() < EPSILON_5 {
            return Collision::None;
        }
        let direction = direction * (1.0 / distance);
        // Other players are posed at the end of this segment, shifted back by
        // the shooter's rewind.
        let segment_time_ms =
            self.born_time_ms as f64 + f64::from(current_time) * 1000.0 - self.rewind_ms;
        let Some(hit) = world.ray(
            start_position,
            direction,
            distance,
            segment_time_ms,
            self.shooter,
        ) else {
            return Collision::None;
        };
        self.process_ray_query(
            &hit,
            distance,
            start_position,
            direction,
            start_time,
            current_time,
            world,
            hits,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn process_ray_query(
        &mut self,
        hit: &RayHit,
        distance: f32,
        start_position: Vec3,
        direction: Vec3,
        start_time: f32,
        current_time: f32,
        world: &dyn BulletWorld,
        hits: &mut Vec<BulletHit>,
    ) -> Collision {
        let distance_to_hit = (start_position - hit.point).length();
        let mut collide_point = start_position + direction * distance_to_hit;
        let mut collision_time =
            start_time + distance_to_hit / distance * (current_time - start_time);
        let cos_alpha = hit.normal.dot(direction);
        if cos_alpha >= 0.0 || cos_alpha.abs() < EPSILON_5 {
            return Collision::None;
        }
        let material = world.material(hit.material);
        self.fix_collision_point_and_time(
            &mut collide_point,
            &mut collision_time,
            start_time,
            current_time,
            hit.normal,
        );
        let velocity = self.trajectory_velocity(collision_time);
        let speed = velocity.length();
        if speed == 0.0 {
            return Collision::Collide;
        }
        let fly_direction = velocity * (1.0 / speed);
        let angle_alpha = cos_alpha.clamp(-1.0, 1.0).acos();
        if self.ricochet_count < MAX_RICOCHETS
            && angle_alpha - std::f32::consts::FRAC_PI_2
                <= material.k_ricochet * self.ricochet_angle
        {
            return self.try_reflect(
                collide_point,
                fly_direction,
                hit.normal,
                speed,
                collision_time,
                cos_alpha,
                material,
            );
        }
        // `collide_front_face`.
        match &hit.target {
            Some(Target::Player(victim, body_part)) if *victim != self.shooter => {
                hits.push(BulletHit::Player {
                    victim: *victim,
                    body_part: body_part.clone(),
                });
            }
            Some(Target::Trap(trap)) => hits.push(BulletHit::Trap(*trap)),
            _ => {}
        }
        if material.resistance > self.pierce {
            return Collision::Collide;
        }
        let new_speed = (self.pierce / material.resistance - 1.0).clamp(0.0, 1.0) * speed;
        self.change_trajectory(
            collide_point + fly_direction * EPSILON_3,
            fly_direction * new_speed,
            collision_time,
        );
        Collision::Pierced
    }

    /// Moves the hit onto the curve where it crosses the surface plane.
    fn fix_collision_point_and_time(
        &self,
        collide_point: &mut Vec3,
        collision_time: &mut f32,
        start_time: f32,
        current_time: f32,
        normal: Vec3,
    ) {
        let mut new_point = self.trajectory_position(*collision_time);
        let mut delta = (new_point - *collide_point).dot(normal);
        if delta.abs() < EPSILON_3 {
            return;
        }
        let mut low_time = start_time;
        let mut high_time = current_time;
        for _ in 0..64 {
            if delta.abs() < EPSILON_3 {
                break;
            }
            if delta < 0.0 {
                high_time = *collision_time;
            } else {
                low_time = *collision_time;
            }
            *collision_time = (low_time + high_time) * 0.5;
            new_point = self.trajectory_position(*collision_time);
            delta = (new_point - *collide_point).dot(normal);
        }
        *collide_point = new_point;
    }

    #[allow(clippy::too_many_arguments)]
    fn try_reflect(
        &mut self,
        collide_point: Vec3,
        direction: Vec3,
        normal: Vec3,
        speed: f32,
        collision_time: f32,
        cos_alpha: f32,
        material: Material,
    ) -> Collision {
        let ricochet_angle = material.k_ricochet * self.ricochet_angle;
        let angle_alpha = cos_alpha.clamp(-1.0, 1.0).acos() - std::f32::consts::FRAC_PI_2;
        let koeff = if ricochet_angle == 0.0 {
            0.0
        } else {
            1.0 - angle_alpha / ricochet_angle
        };
        let down = material.reflection_speed_down;
        let speed = speed * (koeff * (1.0 - down) + down);
        if speed < 0.0 {
            return Collision::Collide;
        }
        let direction = normal * (2.0 * -cos_alpha) + direction;
        self.change_trajectory(collide_point, direction * speed, collision_time);
        self.ricochet_count += 1;
        Collision::Reflected
    }

    fn change_trajectory(&mut self, position: Vec3, velocity: Vec3, collision_time: f32) {
        self.change_trajectory_count += 1;
        self.start_position = position;
        self.start_velocity = velocity;
        self.position = position;
        self.velocity = velocity;
        self.current_resistance = self.air_resistance;
        self.born_time_ms += (1000.0 * collision_time).floor().max(0.0) as u64;
        self.life_time = 0.0;
    }

    fn update_bullet_position(&mut self, time: f32, world: &dyn BulletWorld) -> bool {
        let new_position = self.trajectory_position(time);
        if !world.contains(new_position) {
            return false;
        }
        self.flown_distance += (new_position - self.position).length();
        if self.flown_distance >= self.max_distance {
            self.flown_distance = self.max_distance;
            return false;
        }
        // As shipped: the velocity of the previous point, not the new one.
        self.velocity = self.trajectory_velocity(self.life_time);
        if self.velocity.squared_length().abs() < EPSILON_5 {
            return false;
        }
        self.position = new_position;
        self.life_time = time;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wall plane at z = `wall_z` of one material, and one player box.
    struct TestWorld {
        wall_z: f32,
        wall_material: u16,
        player_z: f32,
        materials: Vec<Material>,
    }

    impl BulletWorld for TestWorld {
        fn ray(
            &self,
            origin: Vec3,
            dir: Vec3,
            length: f32,
            _time_ms: f64,
            shooter: u8,
        ) -> Option<RayHit> {
            let mut best: Option<RayHit> = None;
            let mut plane = |z: f32, material: u16, target: Option<Target>| {
                if dir.z.abs() < 1e-9 {
                    return;
                }
                let t = (z - origin.z) / dir.z;
                if t < 0.0 || t > length || best.as_ref().is_some_and(|b| b.t <= t) {
                    return;
                }
                if matches!(&target, Some(Target::Player(id, _)) if *id == shooter) {
                    return;
                }
                best = Some(RayHit {
                    t,
                    point: origin + dir * t,
                    normal: Vec3::new(0.0, 0.0, -dir.z.signum()),
                    material,
                    target,
                });
            };
            plane(self.wall_z, self.wall_material, None);
            plane(self.player_z, 1, Some(Target::Player(2, "body".to_owned())));
            best
        }

        fn contains(&self, point: Vec3) -> bool {
            point.length() < 2000.0
        }

        fn material(&self, id: u16) -> Material {
            self.materials[id as usize]
        }
    }

    fn materials() -> Vec<Material> {
        let m = |resistance| Material {
            resistance,
            k_ricochet: 0.017,
            reflection_speed_down: 0.5,
            can_place_mine: true,
            can_stick_mine: true,
        };
        vec![m(1.0), m(0.3), m(0.4)]
    }

    fn ak_bullet(z_speed: f32) -> Bullet {
        Bullet::new(
            BulletSpec {
                shooter: 1,
                weapon_dict_id: 13,
                damage: 0.4,
                pierce: 0.5,
                ricochet_angle_degrees: 300.0,
                air_resistance: 1.0,
                distance_coef: 1.0,
                rewind_ms: 0.0,
            },
            Vec3::new(0.0, 1.5, 0.0),
            Vec3::new(0.0, 0.0, z_speed),
            0,
        )
    }

    #[test]
    fn hits_a_player_in_the_open_and_flies_on_through() {
        let world = TestWorld {
            wall_z: 500.0,
            wall_material: 0,
            player_z: 20.0,
            materials: materials(),
        };
        let mut bullet = ak_bullet(800.0);
        let hits = bullet.tick(100, &world);
        assert_eq!(
            hits,
            [BulletHit::Player {
                victim: 2,
                body_part: "body".to_owned()
            }]
        );
        // The actor material (0.3) lets an AP 0.5 round through.
        assert!(!bullet.is_finished());
        assert!(bullet.position().z > 20.0);
    }

    #[test]
    fn concrete_stops_it_before_the_player() {
        let world = TestWorld {
            wall_z: 10.0,
            wall_material: 0,
            player_z: 20.0,
            materials: materials(),
        };
        let mut bullet = ak_bullet(800.0);
        assert!(bullet.tick(1000, &world).is_empty());
        assert!(bullet.is_finished());
    }

    #[test]
    fn thin_wood_lets_it_through_slower() {
        let world = TestWorld {
            wall_z: 10.0,
            wall_material: 2,
            player_z: 20.0,
            materials: materials(),
        };
        let mut bullet = ak_bullet(800.0);
        assert_eq!(bullet.tick(1000, &world).len(), 1);
    }

    #[test]
    fn the_shooter_is_never_hit() {
        let world = TestWorld {
            wall_z: 500.0,
            wall_material: 0,
            player_z: 20.0,
            materials: materials(),
        };
        let mut bullet = ak_bullet(800.0);
        bullet.shooter = 2;
        assert!(bullet.tick(100, &world).is_empty());
    }

    #[test]
    fn drops_under_gravity() {
        let world = TestWorld {
            wall_z: 5000.0,
            wall_material: 0,
            player_z: 5000.0,
            materials: materials(),
        };
        let mut bullet = ak_bullet(800.0);
        bullet.tick(500, &world);
        let p = bullet.position();
        assert!(p.y < 1.5, "{p:?}");
        assert!(p.z > 200.0, "{p:?}");
    }
}
