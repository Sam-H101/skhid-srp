// Quick-slot trap sets (`booby_trap_set` in the client). Online the client
// never places a trap itself: holding the slot's key shows a ghost model and
// releasing it hides the model again, and that is all. The server places the
// trap on the release (`booby_trap_set_core::try_place_trap`) and tells
// everyone (`trap_placed`); every client then inserts the trap and takes one
// off the set's count. Online the clients neither sense nor time their
// traps: the server fires them, runs out each state's life and tells the
// clients (`trap_fired`, `trap_removed`), which only swap the model.
// Defusing is the server's too (`trap_disarmed`): its owner or an enemy
// holds use while looking at the trap for its defuse time, or a bullet hits
// it. The client runs its own copy of the use only for the progress bar.
//
// The placement test follows `survarium.exe` (0x6FE420), where the
// reconstructed source has its branches inverted: a ray from the head, as
// long as the deploy distance, has to meet something to stand on (what
// players walk on, or another trap's sensor, which refuses it), at most the
// slope limit off level, of a material that takes a trap. The trap then sits
// 5 mm back along the ray, upright on the surface and facing away from the
// head. The engine finally lets Bullet push the trap's sensor out of
// anything it overlaps (`recover_from_penetrations`, which never refuses);
// srp leaves that out, so near walls a trap can sit a few centimetres off
// where the client's own ghost model showed it.

use std::fmt;
use std::sync::Arc;

use super::collision::{BULLET_RAY, Placed, RayFilter, Shape, StaticWorld, TRAP_PLACEMENT_RAY};
use super::data::{Materials, TrapDamage, TrapData};
use super::math::{Mat4, Vec3};

/// The ray meeting an object in collision group 0x200 (a trap's sensor)
/// refuses the place.
const REFUSING_GROUP: u16 = 0x200;
/// `get_visible_place_transform`: the trap goes this far back toward the
/// head from the point the ray met.
const SURFACE_OFFSET: f32 = 0.005;
/// `create_place_matrix_for_looking_point`: below this the look direction
/// is along the surface normal.
const PARALLEL_EPSILON: f32 = 1e-3;

/// The use key's bit in `action_mask`.
pub const USE_KEY: u32 = 0x1000_0000;
/// `player::detect_usable_objects`: the ray from the head that finds what
/// the player can use, and its length (`usable_objects_detection_distance`).
const USE_RAY: RayFilter = RayFilter {
    group: 0x100,
    mask: 0x80,
};
pub const USE_REACH: f32 = 1.0;
/// The stock client drops the use bit for an update or a few while the key
/// is held (see `match_server::USE_RELEASE_DELAY`).
const USE_RELEASE_DELAY_MS: u64 = 200;

/// `quick_slot_pressed`'s pair: quick slot `slot` (0-5) has bit
/// `15 + 2 * slot` in the frame its key comes up.
pub fn quick_slot_released(mask: u32, slot: usize) -> bool {
    mask >> (15 + 2 * slot) & 1 != 0
}

/// Why a released key placed no trap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refusal {
    NoneLeft,
    NothingInReach,
    OnAnotherTrap,
    TooSteep {
        slope_degrees: f32,
    },
    CannotPlaceOn {
        material: u16,
    },
    CannotStickTo {
        material: u16,
    },
    /// Every trap of the set is still out (placed, fired or disarmed).
    AllOut,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoneLeft => write!(f, "none left"),
            Self::NothingInReach => write!(f, "nothing to stand on in reach"),
            Self::OnAnotherTrap => write!(f, "another trap is there"),
            Self::TooSteep { slope_degrees } => write!(f, "{slope_degrees:.0} degree slope"),
            Self::CannotPlaceOn { material } => write!(f, "material {material} takes no traps"),
            Self::CannotStickTo { material } => {
                write!(f, "material {material} takes no stuck traps")
            }
            Self::AllOut => write!(f, "all of the set's traps are out"),
        }
    }
}

/// The use key as the player holds it: released only after the bit has been
/// missing for `USE_RELEASE_DELAY_MS`.
#[derive(Clone, Copy, Debug, Default)]
pub struct UseKey {
    down: bool,
    missing_since_ms: Option<u64>,
}

impl UseKey {
    pub fn held(&mut self, bit: bool, now_ms: u64) -> bool {
        if bit {
            self.down = true;
            self.missing_since_ms = None;
        } else if self.down {
            let since = *self.missing_since_ms.get_or_insert(now_ms);
            if now_ms.saturating_sub(since) >= USE_RELEASE_DELAY_MS {
                *self = Self::default();
            }
        }
        self.down
    }
}

/// A trap of a match: its owner's player id, the owner's quick slot (0-5)
/// and the trap's index in that set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrapId {
    pub owner: u8,
    pub slot: usize,
    pub index: u8,
}

/// `booby_trap_core::can_defuse`: the owner, or anyone of another team.
pub fn can_defuse(user: (u8, u8), owner: (u8, u8)) -> bool {
    let ((user_id, user_team), (owner_id, owner_team)) = (user, owner);
    user_id == owner_id || user_team != owner_team
}

/// `booby_trap_state`, but for `removed`: a free trap is `None` in its set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrapState {
    Armed = 1,
    Fired = 2,
    Disarmed = 3,
}

/// What became of a trap, for the clients (`trap_fired`, `trap_disarmed`,
/// `trap_removed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrapChange {
    Fired,
    Disarmed,
    Removed,
}

/// A trap out of its set.
#[derive(Clone, Debug)]
pub struct Trap {
    pub transform: Mat4,
    pub state: TrapState,
    /// When the state's life runs out (server ms); `None` never.
    until_ms: Option<u64>,
    /// The sensor's shapes where the trap stands.
    sensor: Vec<Placed>,
    /// Where a player looks to defuse it, if the use ray meets it.
    usable: Vec<Placed>,
    /// What a bullet hits to disarm it, with the game material, if the trap
    /// is disarmed by hits and bullets meet it.
    hittable: Vec<(Placed, u16)>,
}

impl Trap {
    pub fn sensor(&self) -> &[Placed] {
        &self.sensor
    }

    pub fn usable(&self) -> &[Placed] {
        &self.usable
    }

    pub fn hittable(&self) -> &[(Placed, u16)] {
        &self.hittable
    }
}

/// One quick slot's trap set.
#[derive(Clone, Debug)]
pub struct TrapSet {
    pub dict_id: u16,
    data: Arc<TrapData>,
    /// Traps left to place: the client's `m_amount`.
    pub amount: u32,
    /// The trap objects the client made for the set, one per unit of the
    /// profile's stack (as a u8): a trap's index here is its index on the
    /// wire. `None` is free (`find_free_trap_predicate`).
    traps: Vec<Option<Trap>>,
}

impl TrapSet {
    pub fn new(dict_id: u16, data: Arc<TrapData>, stack_size: u8, amount: u32) -> Self {
        Self {
            dict_id,
            data,
            amount,
            traps: vec![None; usize::from(stack_size)],
        }
    }

    /// What of this set's armed traps refuses a trap placed over it: their
    /// sensors, which the placement ray meets and which are in group 0x200.
    /// A trap takes its sensor out of the world when it fires or is disarmed.
    pub fn refusing_volumes(&self) -> impl Iterator<Item = Placed> + '_ {
        let sensor = &self.data.sensor;
        let refuses = TRAP_PLACEMENT_RAY.meets(sensor.group, sensor.mask)
            && sensor.group & REFUSING_GROUP != 0;
        self.armed()
            .filter(move |_| refuses)
            .flat_map(|(_, trap)| trap.sensor.iter().cloned())
    }

    /// The armed traps, by index.
    pub fn armed(&self) -> impl Iterator<Item = (u8, &Trap)> + '_ {
        self.traps.iter().enumerate().filter_map(|(index, trap)| {
            let trap = trap.as_ref().filter(|t| t.state == TrapState::Armed)?;
            Some((index as u8, trap))
        })
    }

    /// What a firing trap does to each player it caught.
    pub fn damage(&self) -> &[TrapDamage] {
        &self.data.damage
    }

    /// `booby_trap_core::use_execute`'s defuse time, with no engineer
    /// booster (srp has none: its factor is 1).
    pub fn defuse_ms(&self) -> u64 {
        u64::from(self.data.defuse_ms)
    }

    /// `booby_trap_set_core::try_place_trap` from the head transform of the
    /// frame the key came up, at server time `now_ms`: the new trap's index
    /// and transform.
    pub fn try_place(
        &mut self,
        head: &Mat4,
        world: &StaticWorld,
        materials: &Materials,
        refusing: &[Placed],
        now_ms: u64,
    ) -> Result<(u8, Mat4), Refusal> {
        if self.amount == 0 {
            return Err(Refusal::NoneLeft);
        }
        let transform = place_transform(&self.data, head, world, materials, refusing)?;
        let index = self
            .traps
            .iter()
            .position(Option::is_none)
            .ok_or(Refusal::AllOut)?;
        let place = |shapes: &[(Shape, Mat4, u16)]| -> Vec<(Placed, u16)> {
            shapes
                .iter()
                .filter_map(|(shape, local, material)| {
                    Some((Placed::new(*shape, *local * transform)?, *material))
                })
                .collect()
        };
        let shapes_only = |placed: Vec<(Placed, u16)>| -> Vec<Placed> {
            placed.into_iter().map(|(p, _)| p).collect()
        };
        let usable = &self.data.usable;
        let hittable = self
            .data
            .hittable
            .as_ref()
            .filter(|h| self.data.defuse_by_hit && BULLET_RAY.meets(h.group, h.mask));
        self.traps[index] = Some(Trap {
            transform,
            state: TrapState::Armed,
            until_ms: life_end(self.data.armed_life_ms, now_ms),
            sensor: shapes_only(place(&self.data.sensor.shapes)),
            usable: if USE_RAY.meets(usable.group, usable.mask) {
                shapes_only(place(&usable.shapes))
            } else {
                Vec::new()
            },
            hittable: hittable.map_or_else(Vec::new, |h| place(&h.shapes)),
        });
        self.amount -= 1;
        Ok((index as u8, transform))
    }

    /// `booby_trap_core::on_enter`'s end: an armed trap goes off.
    pub fn fire(&mut self, index: u8, now_ms: u64) -> Option<TrapChange> {
        self.leave_armed(index, TrapState::Fired, now_ms)
    }

    /// `booby_trap_core::defuse_completed`, by use or by a hit.
    pub fn disarm(&mut self, index: u8, now_ms: u64) -> Option<TrapChange> {
        self.leave_armed(index, TrapState::Disarmed, now_ms)
    }

    /// Every trap out of the set, by index.
    pub fn out(&self) -> impl Iterator<Item = (u8, &Trap)> + '_ {
        self.traps
            .iter()
            .enumerate()
            .filter_map(|(index, trap)| Some((index as u8, trap.as_ref()?)))
    }

    /// The armed trap at `index`, if it is.
    #[cfg(test)]
    pub fn armed_trap(&self, index: u8) -> Option<&Trap> {
        self.traps
            .get(usize::from(index))?
            .as_ref()
            .filter(|t| t.state == TrapState::Armed)
    }

    /// `switch_to_state` from armed to fired or disarmed: the new state for
    /// its life, or the trap straight back in the set when it has none.
    fn leave_armed(&mut self, index: u8, state: TrapState, now_ms: u64) -> Option<TrapChange> {
        let slot = self.traps.get_mut(usize::from(index))?;
        slot.as_ref().filter(|t| t.state == TrapState::Armed)?;
        let (life_ms, change) = match state {
            TrapState::Fired => (self.data.fired_life_ms, TrapChange::Fired),
            _ => (self.data.disarmed_life_ms, TrapChange::Disarmed),
        };
        if life_ms == 0 {
            *slot = None;
            return Some(TrapChange::Removed);
        }
        let trap = slot.as_mut()?;
        trap.state = state;
        trap.until_ms = life_end(life_ms, now_ms);
        Some(change)
    }

    /// `booby_trap_core::tick`'s timer: the traps whose state ran out by
    /// `now_ms` (`on_state_timer_finished`). An armed trap that runs out is
    /// disarmed; a fired or disarmed one goes back in the set.
    pub fn tick(&mut self, now_ms: u64) -> Vec<(u8, TrapChange)> {
        let mut changes = Vec::new();
        for index in 0..self.traps.len() {
            let Some(trap) = &self.traps[index] else {
                continue;
            };
            if trap.until_ms.is_none_or(|until| until > now_ms) {
                continue;
            }
            let index = index as u8;
            let change = if trap.state == TrapState::Armed {
                self.leave_armed(index, TrapState::Disarmed, now_ms)
            } else {
                self.traps[usize::from(index)] = None;
                Some(TrapChange::Removed)
            };
            changes.extend(change.map(|change| (index, change)));
        }
        changes
    }

    #[cfg(test)]
    pub fn trap(&self, index: u8) -> Option<&Trap> {
        self.traps.get(usize::from(index))?.as_ref()
    }
}

/// When a state of `life_ms` entered at `now_ms` runs out; 0 never does.
fn life_end(life_ms: u32, now_ms: u64) -> Option<u64> {
    (life_ms != 0).then(|| now_ms + u64::from(life_ms))
}

/// `booby_trap_set_core::get_visible_place_transform` as `survarium.exe`
/// runs it (see the top of this file).
fn place_transform(
    data: &TrapData,
    head: &Mat4,
    world: &StaticWorld,
    materials: &Materials,
    refusing: &[Placed],
) -> Result<Mat4, Refusal> {
    let from = head.c;
    let dir = head.k.normalized_or(Vec3::new(0.0, 0.0, 1.0));
    let length = data.max_deploy_distance;
    // (t, normal, the map's material; `None` for a refusing volume).
    let mut hit = world
        .ray_cast(from, dir, length)
        .map(|hit| (hit.t, hit.normal, Some(hit.material)));
    for volume in refusing {
        if let Some((t, normal)) = volume.ray(from, dir)
            && t <= length
            && hit.is_none_or(|(best, _, _)| t < best)
        {
            hit = Some((t, normal, None));
        }
    }
    let Some((t, normal, material)) = hit else {
        return Err(Refusal::NothingInReach);
    };
    let Some(material) = material else {
        return Err(Refusal::OnAnotherTrap);
    };
    if normal.y < data.max_slope_cos {
        return Err(Refusal::TooSteep {
            slope_degrees: normal.y.clamp(-1.0, 1.0).acos().to_degrees(),
        });
    }
    let surface = materials.get(material);
    if data.material_can_place_test && !surface.can_place_mine {
        return Err(Refusal::CannotPlaceOn { material });
    }
    if data.material_can_stick_test && !surface.can_stick_mine {
        return Err(Refusal::CannotStickTo { material });
    }
    let looking_point = place_matrix(from + dir * t, normal, head);
    Ok(looking_point * Mat4::translation(-dir * SURFACE_OFFSET))
}

/// `create_place_matrix_for_looking_point`: at the hit point, up along the
/// surface normal and forward away from the head.
fn place_matrix(point: Vec3, normal: Vec3, head: &Mat4) -> Mat4 {
    let right = normal.cross(head.k);
    let (right, forward) = if right.length() > PARALLEL_EPSILON {
        let right = right.normalized_or(head.i);
        (right, right.cross(normal).normalized_or(head.k))
    } else {
        let forward = head.i.cross(normal).normalized_or(head.k);
        (normal.cross(forward).normalized_or(head.i), forward)
    };
    Mat4 {
        i: right,
        j: normal,
        k: forward,
        c: point,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::collision::{SERVER_PROJECT, TRAP_PLACEMENT_RAY};
    use crate::combat::data::tests::game_data;
    use crate::combat::resources::tests::game_resources;
    use std::sync::OnceLock;

    fn walking_world() -> Option<&'static StaticWorld> {
        static WORLD: OnceLock<Option<StaticWorld>> = OnceLock::new();
        WORLD
            .get_or_init(|| {
                let mut resources = game_resources()?;
                let mut worlds =
                    StaticWorld::load_for(&mut resources, SERVER_PROJECT, &[TRAP_PLACEMENT_RAY])
                        .ok()?;
                Some(worlds.remove(0))
            })
            .as_ref()
    }

    /// Team 1's base floor, with nothing in the way for 8 m toward -x.
    const FLOOR: Vec3 = Vec3::new(-31.2, 0.504, -66.7);

    /// A head 1.6 m over `FLOOR`, looking toward -x and `down` radians below
    /// level.
    fn head_looking_down(down: f32) -> Mat4 {
        let look = Mat4::rotation_xyz(Vec3::new(-down, 0.0, 0.0))
            * Mat4::rotation_y(std::f32::consts::FRAC_PI_2);
        Mat4 {
            c: FLOOR + Vec3::new(0.0, 1.6, 0.0),
            ..look
        }
    }

    #[test]
    fn the_release_bit_follows_the_press_bit() {
        assert!(quick_slot_released(0x8000, 0));
        assert!(!quick_slot_released(0x4000, 0));
        assert!(quick_slot_released(0x200000, 3));
        assert!(!quick_slot_released(0x100000, 3));
    }

    #[test]
    fn the_place_matrix_stands_on_the_surface_facing_away() {
        let head = head_looking_down(1.0);
        let m = place_matrix(Vec3::new(1.0, 0.0, 2.0), Vec3::Y, &head);
        assert_eq!(m.j, Vec3::Y);
        assert!((m.k - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5, "{m:?}");
        assert!(
            (m.i.cross(m.j) - m.k).length() < 1e-5,
            "right-handed like the engine: {m:?}"
        );
        assert_eq!(m.c, Vec3::new(1.0, 0.0, 2.0));
        // Looking straight down the normal falls back on the head's right.
        let straight_down = Mat4 {
            k: -Vec3::Y,
            j: Vec3::new(0.0, 0.0, 1.0),
            ..Mat4::IDENTITY
        };
        let m = place_matrix(Vec3::ZERO, Vec3::Y, &straight_down);
        assert!(m.i.length() > 0.99 && m.k.length() > 0.99, "{m:?}");
        assert!(m.k.y.abs() < 1e-5, "{m:?}");
    }

    #[test]
    fn places_a_trap_on_the_floor_in_reach() {
        let (Some(data), Some(world)) = (game_data(), walking_world()) else {
            return;
        };
        let trap = data.trap(68).unwrap().clone();
        let mut set = TrapSet::new(68, trap, 100, 100);

        // Looking at the floor 0.6 m ahead (the eye is 1.6 m up, so the
        // 2 m reach ends 1.2 m out).
        let head = head_looking_down((1.6f32 / 0.6).atan());
        let (index, transform) = set
            .try_place(&head, world, &data.materials, &[], 0)
            .expect("the floor takes a trap");
        assert_eq!((index, set.amount), (0, 99));
        assert!((transform.c.y - FLOOR.y).abs() < 0.05, "{transform:?}");
        assert!(
            (transform.c.x - (FLOOR.x - 0.6)).abs() < 0.05,
            "{transform:?}"
        );
        assert!(transform.j.y > 0.99, "upright: {transform:?}");
        assert!(
            transform.k.x < -0.99,
            "facing away from the head: {transform:?}"
        );
        assert!(set.trap(0).is_some());

        // The floor 1.6 m ahead is past the 2 m deploy distance.
        let far = head_looking_down((1.6f32 / 1.6).atan());
        assert_eq!(
            set.try_place(&far, world, &data.materials, &[], 0)
                .unwrap_err(),
            Refusal::NothingInReach
        );
        // The first trap's sensor stops a second one on the same spot.
        let sensors: Vec<_> = set.refusing_volumes().collect();
        assert_eq!(sensors.len(), 1);
        assert_eq!(
            set.try_place(&head, world, &data.materials, &sensors, 0)
                .unwrap_err(),
            Refusal::OnAnotherTrap
        );
        // Half a metre further on, the ray clears it.
        let next = head_looking_down((1.6f32 / 1.1).atan());
        let (index, _) = set
            .try_place(&next, world, &data.materials, &sensors, 0)
            .unwrap();
        assert_eq!((index, set.amount), (1, 98));
    }

    #[test]
    fn a_set_runs_out() {
        let (Some(data), Some(world)) = (game_data(), walking_world()) else {
            return;
        };
        let trap = data.trap(68).unwrap().clone();
        let head = head_looking_down((1.6f32 / 0.6).atan());
        let mut empty = TrapSet::new(68, trap.clone(), 100, 0);
        assert_eq!(
            empty
                .try_place(&head, world, &data.materials, &[], 0)
                .unwrap_err(),
            Refusal::NoneLeft
        );
        // More to place than the client made trap objects for.
        let mut small = TrapSet::new(68, trap, 1, 5);
        small
            .try_place(&head, world, &data.materials, &[], 0)
            .unwrap();
        let next = head_looking_down((1.6f32 / 1.1).atan());
        assert_eq!(
            small
                .try_place(&next, world, &data.materials, &[], 0)
                .unwrap_err(),
            Refusal::AllOut
        );
        assert_eq!(small.amount, 4);
    }

    #[test]
    fn a_fired_trap_goes_back_in_the_set_after_its_life() {
        let (Some(data), Some(world)) = (game_data(), walking_world()) else {
            return;
        };
        let trap = data.trap(68).unwrap().clone();
        let mut set = TrapSet::new(68, trap, 100, 100);
        let head = head_looking_down((1.6f32 / 0.6).atan());
        let (index, _) = set
            .try_place(&head, world, &data.materials, &[], 1000)
            .unwrap();
        // An armed trap waits for ever (armed_life_time 0).
        assert_eq!(set.tick(1_000_000), []);
        assert_eq!(set.armed().count(), 1);

        assert_eq!(set.fire(index, 2000), Some(TrapChange::Fired));
        assert_eq!(set.fire(index, 2000), None, "only an armed trap fires");
        assert_eq!(set.trap(index).unwrap().state, TrapState::Fired);
        // A fired trap no longer senses or refuses, but keeps its index.
        assert_eq!(set.armed().count(), 0);
        assert_eq!(set.refusing_volumes().count(), 0);
        let (next, _) = set
            .try_place(&head, world, &data.materials, &[], 2100)
            .unwrap();
        assert_eq!(next, 1);

        // fired_life_time 3 s.
        assert_eq!(set.tick(4999), []);
        assert_eq!(set.tick(5000), [(index, TrapChange::Removed)]);
        assert!(set.trap(index).is_none());
        assert!(set.trap(next).is_some());
    }

    #[test]
    fn a_disarmed_trap_shows_for_its_life_and_the_use_key_outlasts_flicker() {
        let (Some(data), Some(world)) = (game_data(), walking_world()) else {
            return;
        };
        let trap = data.trap(68).unwrap().clone();
        let mut set = TrapSet::new(68, trap, 100, 100);
        let head = head_looking_down((1.6f32 / 0.6).atan());
        let (index, _) = set
            .try_place(&head, world, &data.materials, &[], 0)
            .unwrap();
        let armed = set.armed_trap(index).unwrap();
        assert_eq!((armed.usable().len(), armed.hittable().len()), (1, 1));
        assert_eq!(armed.hittable()[0].1, 18, "the hittable mesh's material");
        assert_eq!(set.defuse_ms(), 5000);

        assert_eq!(set.disarm(index, 100), Some(TrapChange::Disarmed));
        assert_eq!(set.fire(index, 100), None, "a disarmed trap is safe");
        assert!(set.armed_trap(index).is_none());
        assert_eq!(set.tick(3099), []);
        assert_eq!(set.tick(3100), [(index, TrapChange::Removed)]);

        // Use, held through a 150 ms gap in the bit; released 200 ms after.
        let mut key = UseKey::default();
        assert!(!key.held(false, 0));
        assert!(key.held(true, 10));
        assert!(key.held(false, 20));
        assert!(key.held(false, 160));
        assert!(key.held(true, 170));
        assert!(key.held(false, 180));
        assert!(!key.held(false, 380));
        assert!(!key.held(false, 390));

        assert!(can_defuse((3, 1), (3, 1)), "its owner");
        assert!(can_defuse((4, 2), (3, 1)), "an enemy");
        assert!(!can_defuse((5, 1), (3, 1)), "not a team mate");
    }

    #[test]
    fn refuses_steep_ground_and_soft_materials() {
        let Some(data) = game_data() else {
            return;
        };
        let trap = data.trap(68).unwrap();
        let head = head_looking_down(1.0);
        let plane = |normal: Vec3, material: u16| {
            // A huge wall-like triangle 1 m below the head, tilted by `normal`.
            let tangent = normal
                .cross(Vec3::new(0.0, 0.0, 1.0))
                .normalized_or(Vec3::new(1.0, 0.0, 0.0));
            let bitangent = normal.cross(tangent);
            let center = head.c + head.k * 1.0;
            StaticWorld::from_triangles_for_tests(
                &[
                    center + tangent * 50.0,
                    center - tangent * 50.0 + bitangent * 50.0,
                    center - tangent * 50.0 - bitangent * 50.0,
                ],
                material,
            )
        };
        let level = plane(Vec3::Y, 19);
        assert!(place_transform(trap, &head, &level, &data.materials, &[]).is_ok());
        let tilt = |degrees: f32| {
            let r = degrees.to_radians();
            Vec3::new(r.sin(), r.cos(), 0.0)
        };
        assert!(place_transform(trap, &head, &plane(tilt(29.0), 19), &data.materials, &[]).is_ok());
        assert!(matches!(
            place_transform(trap, &head, &plane(tilt(31.0), 19), &data.materials, &[]),
            Err(Refusal::TooSteep { .. })
        ));
        assert_eq!(
            place_transform(trap, &head, &plane(Vec3::Y, 23), &data.materials, &[]).unwrap_err(),
            Refusal::CannotPlaceOn { material: 23 }
        );
    }
}
