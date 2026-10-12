// The combat runtime: one fighter per joined player, the bullets in flight,
// and lag compensation.
//
// Each client update is one client frame. Its time is mapped onto the server
// clock per client (the smallest arrival gap seen recently), so no two
// clocks are ever compared. The fighter's weapons replay the frame on the
// client's own clock and fire; as on the client, shots leave the first-person
// camera of the frame before, and their client times map onto the server's
// with the frame's. Bullets then pose every
// other player as the shooter saw them: the history (stamped when updates
// arrived) at the bullet's time minus the shooter's round trip. Other players
// are shown without an interpolation buffer, by their latest state carried
// forward with their last input, so in steady motion that is where they were
// one full trip ago.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use survarium::player_input::{player_input, player_stamina, player_state};
use survarium::player_profile::raw::{player_profile, profile_slot_enum};

use crate::game::WeaponLoad;

use super::ballistics::{Bullet, BulletHit, BulletSpec, BulletWorld, RayHit, Target};
use super::collision::{BULLET_RAY, Placed, SERVER_PROJECT, StaticWorld, TRAP_PLACEMENT_RAY};
use super::damage::{AffectEvent, DamageModel, Equipment, LastHit, PartState, affect};
use super::data::{ACTOR_MATERIAL, CombatData, Material};
use super::drugs::{Drug, QUICK_SLOTS, quick_slot_pressed};
use super::math::{Mat4, Vec3};
use super::pose::{PlayerModel, PoseRequest, View, direction};
use super::resources::{ResourceError, Resources};
use super::traps::{
    TrapChange, TrapId, TrapSet, USE_KEY, USE_REACH, UseKey, can_defuse, quick_slot_released,
};
use super::weapon::{Arsenal, Injuries, RESTING_RECOIL, Shot, WeaponState, bits, spawn_load};

pub const MAX_REWIND_VARIABLE: &str = "SRP_LAG_COMP_MAX_MS";
const DEFAULT_MAX_REWIND_MS: f64 = 300.0;
/// Position history kept per player: the largest rewind plus a bullet's
/// flight across the map.
const HISTORY_MS: u64 = 2000;
/// How long a client's arrival gaps are remembered for its clock mapping.
const CLOCK_WINDOW_MS: u64 = 5000;
/// Updates kept per player, whatever their rate (a client sends one per
/// frame; this bounds a flood).
const HISTORY_UPDATES: usize = 1024;
/// A ray this far from a player's middle cannot touch a hit box.
const PLAYER_REACH: f32 = 1.6;
const BODY_MIDDLE: f32 = 0.9;
/// A trap's sensor shape reaches no further than this from its middle.
const SENSOR_REACH: f32 = 0.5;
/// Nor does any other of its shapes from where it stands.
const TRAP_REACH: f32 = 0.5;
/// Bullets fly in steps this long at most. Other players are posed at the
/// end of each step, so one long step (a stalled loop, or a shot that left
/// in the past) would pose them too late; the client steps per frame.
const BULLET_STEP_MS: u64 = 4;

/// Weapon seeds sent in a spawn: (`random32`, `normal_random`).
pub type Seeds = [(u32, i32); 2];

#[derive(Clone, Debug, PartialEq)]
pub enum CombatEvent {
    Hit {
        match_id: u32,
        shooter: u8,
        victim: u8,
        body_part: String,
        hit_type: String,
        amount: f32,
        armor_piercing: f32,
    },
    /// Hand or leg damage applied or recalled (the HUD shows only these).
    LimbAffect {
        match_id: u32,
        player_id: u8,
        body_part: String,
        affect: u8,
        applying: bool,
    },
    Kill {
        match_id: u32,
        victim: SocketAddr,
        victim_id: u8,
        killer_id: u8,
        is_headshot: bool,
        weapon_dict_id: u16,
    },
    /// Trap `index` of the set in `player_id`'s quick slot `slot` (a
    /// `profile_slot_enum`) is armed at `rotation_xyz( angles )` then
    /// `position`.
    TrapPlaced {
        match_id: u32,
        player_id: u8,
        slot: u8,
        index: u8,
        position: Vec3,
        angles: Vec3,
    },
    /// The same trap fired, was disarmed or went back in its set.
    TrapChanged {
        match_id: u32,
        player_id: u8,
        slot: u8,
        index: u8,
        change: TrapChange,
    },
}

/// A trap out of its set, as `booby_trap_core::serialize` describes it to a
/// client that joins later.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrapReport {
    pub player_id: u8,
    /// A `profile_slot_enum`.
    pub slot: u8,
    pub index: u8,
    /// A `booby_trap_state`.
    pub state: u8,
    pub position: Vec3,
    pub angles: Vec3,
}

/// A living player's weapons and stacks as they are now, for a spawn message
/// that shows the player to a client which doesn't have it in its world.
#[derive(Clone, Debug, PartialEq)]
pub struct Kit {
    /// The weapon slot (0 or 1) in hand or being switched to.
    pub weapon_slot: usize,
    /// By weapon slot; `None` for a weapon combat doesn't run.
    pub weapons: [Option<WeaponLoad>; 2],
    /// What each stack slot holds in hand: the ammo carried outside the
    /// weapons, the drugs and the traps left to place.
    pub amounts: Vec<(profile_slot_enum, u32)>,
    /// On the player's own clock.
    pub stamina: player_stamina,
}

/// Maps one client's clock onto the server's.
#[derive(Debug, Default)]
struct ClientClock {
    /// The window's arrival gaps that can still be its smallest: increasing
    /// from the front, so the front is the minimum.
    gaps: VecDeque<(u64, i64)>,
}

impl ClientClock {
    fn map(&mut self, client_ms: u32, arrival_ms: u64, max_rewind_ms: u64) -> u64 {
        let gap = arrival_ms as i64 - i64::from(client_ms);
        while self.gaps.back().is_some_and(|(_, newer)| *newer >= gap) {
            self.gaps.pop_back();
        }
        self.gaps.push_back((arrival_ms, gap));
        while self
            .gaps
            .front()
            .is_some_and(|(at, _)| *at + CLOCK_WINDOW_MS < arrival_ms)
        {
            self.gaps.pop_front();
        }
        let offset = self.gaps.front().map_or(gap, |(_, gap)| *gap);
        let mapped = i64::from(client_ms) + offset;
        mapped.clamp(
            arrival_ms.saturating_sub(max_rewind_ms) as i64,
            arrival_ms as i64,
        ) as u64
    }

    /// Server ms minus client ms, by the smallest recent gap; `None` before
    /// the first update.
    fn offset(&self) -> Option<i64> {
        self.gaps.front().map(|(_, gap)| *gap)
    }
}

#[derive(Clone, Copy, Debug)]
struct Snapshot {
    at_ms: u64,
    position: Vec3,
    yaw: f32,
    pitch: f32,
    mask: u32,
    alive: bool,
    weapon_dict_id: u16,
    recoil: (f32, f32, f32),
    movement_since_ms: u64,
}

impl Snapshot {
    fn crouch(&self) -> bool {
        self.mask & bits::CROUCH != 0
    }

    fn aimed(&self) -> bool {
        self.mask & bits::AIM != 0
    }

    fn character(&self) -> Mat4 {
        Mat4::rotation_y(self.yaw) * Mat4::translation(self.position)
    }

    fn pose_request(&self, view: View, at_ms: f64) -> PoseRequest {
        PoseRequest {
            view,
            weapon_dict_id: self.weapon_dict_id,
            crouch: self.crouch(),
            aimed: self.aimed(),
            direction: direction(self.mask),
            movement_time: ((at_ms - self.movement_since_ms as f64) / 1000.0).max(0.0) as f32,
            look_pitch: self.pitch,
            recoil: self.recoil,
        }
    }
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let delta = (b - a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
    a + delta * t
}

pub struct Fighter {
    pub match_id: u32,
    pub player_id: u8,
    pub team: u8,
    pub alive: bool,
    damage: DamageModel,
    arsenal: Arsenal,
    /// The drug in each quick slot.
    drugs: [Option<Drug>; QUICK_SLOTS],
    /// The trap set in each quick slot.
    traps: [Option<TrapSet>; QUICK_SLOTS],
    /// The profile slot of each weapon's ammo, in its `ammo` order.
    ammo_slots: [Vec<profile_slot_enum>; 2],
    use_key: UseKey,
    /// The trap the player is defusing and since when (server ms): the
    /// `usable_object_user_data` of its use.
    defusing: Option<(TrapId, u64)>,
    history: VecDeque<Snapshot>,
    clock: ClientClock,
    movement_key: (usize, bool, bool),
    movement_since_ms: u64,
    last_update_ms: u64,
    last_regen_ms: u64,
}

impl Fighter {
    fn injuries(&self) -> Injuries {
        let count = |parts: [&str; 2], kind: u8| {
            parts
                .iter()
                .filter(|part| {
                    self.damage
                        .index_of(part)
                        .is_some_and(|i| self.damage.part_has_affect(i, kind))
                })
                .count() as u8
        };
        Injuries {
            broken_hands: count(["left_hand", "right_hand"], affect::HAND_DAMAGE),
            broken_legs: count(["left_leg", "right_leg"], affect::LEG_DAMAGE),
        }
    }

    /// The player as it was at `at_ms` (server time), between the two
    /// surrounding updates.
    fn snapshot_at(&self, at_ms: f64) -> Option<Snapshot> {
        // The history is in time order.
        let after = self.history.partition_point(|s| s.at_ms as f64 <= at_ms);
        match after {
            i if i == self.history.len() => self.history.back().copied(),
            0 => self.history.front().copied(),
            i => {
                let a = self.history[i - 1];
                let b = self.history[i];
                let span = (b.at_ms - a.at_ms) as f64;
                let t = if span > 0.0 {
                    ((at_ms - a.at_ms as f64) / span) as f32
                } else {
                    0.0
                };
                Some(Snapshot {
                    position: a.position.lerp(b.position, t),
                    yaw: lerp_angle(a.yaw, b.yaw, t),
                    pitch: a.pitch + (b.pitch - a.pitch) * t,
                    ..a
                })
            }
        }
    }
}

/// What combat reads from the game's `resources.db`, loaded once per process
/// (the supervisor restarts the match loop without reloading it).
pub struct CombatAssets {
    data: Arc<CombatData>,
    /// What stops bullets.
    world: StaticWorld,
    /// What players walk on, where traps go.
    walkable: StaticWorld,
    model: PlayerModel,
}

static ASSETS: OnceLock<Option<Arc<CombatAssets>>> = OnceLock::new();

impl CombatAssets {
    fn load(path: &Path) -> Result<Self, ResourceError> {
        let started = Instant::now();
        let mut resources = Resources::open(path)?;
        let data = CombatData::load(&mut resources)?;
        let mut worlds = StaticWorld::load_for(
            &mut resources,
            SERVER_PROJECT,
            &[BULLET_RAY, TRAP_PLACEMENT_RAY],
        )?;
        let walkable = worlds.pop().expect("one world per filter");
        let world = worlds.pop().expect("one world per filter");
        let weapons: Vec<(u16, String)> = data
            .items
            .iter()
            .filter_map(|(id, item)| match item {
                super::data::Item::Weapon(weapon) => Some((*id, weapon.config.clone())),
                _ => None,
            })
            .collect();
        let model = PlayerModel::load(&mut resources, &weapons)?;
        log::info!(
            "combat: loaded {} from {} in {:?}: {} items, {} map triangles and {} primitives \
             against bullets, {} and {} to walk on",
            SERVER_PROJECT,
            path.display(),
            started.elapsed(),
            data.items.len(),
            world.triangle_count(),
            world.primitive_count(),
            walkable.triangle_count(),
            walkable.primitive_count(),
        );
        Ok(Self {
            data: Arc::new(data),
            world,
            walkable,
            model,
        })
    }

    /// The process's assets, loaded on first use; `None` (logged once) when
    /// `resources.db` cannot be read.
    pub fn shared() -> Option<Arc<Self>> {
        ASSETS
            .get_or_init(|| Self::load_or_log(&Resources::configured_path()))
            .clone()
    }

    /// Tests run from the crate directory, two levels below srp's.
    #[cfg(test)]
    pub fn for_tests() -> Option<Arc<Self>> {
        static TEST_ASSETS: OnceLock<Option<Arc<CombatAssets>>> = OnceLock::new();
        TEST_ASSETS
            .get_or_init(|| {
                let path = Resources::configured_path();
                let path = if path.is_relative() {
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../..")
                        .join(path)
                } else {
                    path
                };
                Self::load_or_log(&path)
            })
            .clone()
    }

    fn load_or_log(path: &Path) -> Option<Arc<Self>> {
        // A damaged file must not take the match loop down with it (the
        // supervisor would restart it into the same failure).
        let loaded = std::panic::catch_unwind(|| Self::load(path)).unwrap_or_else(|_| {
            Err(ResourceError(format!(
                "{}: unreadable game data",
                path.display()
            )))
        });
        match loaded {
            Ok(assets) => Some(Arc::new(assets)),
            Err(error) => {
                log::error!(
                    "combat: cannot load the game data ({error}); set {} to the game's                      resources.db. Until then players cannot hurt each other.",
                    super::resources::PATH_VARIABLE,
                );
                None
            }
        }
    }
}

const QUICK_SLOT_ENUMS: [profile_slot_enum; QUICK_SLOTS] = [
    profile_slot_enum::quick_slot1,
    profile_slot_enum::quick_slot2,
    profile_slot_enum::quick_slot3,
    profile_slot_enum::quick_slot4,
    profile_slot_enum::quick_slot5,
    profile_slot_enum::quick_slot6,
];

pub struct Combat {
    assets: Arc<CombatAssets>,
    fighters: HashMap<SocketAddr, Fighter>,
    bullets: Vec<(u32, Bullet)>,
    /// What client updates did, sent with the next tick's events.
    pending: Vec<CombatEvent>,
    epoch: Instant,
    max_rewind_ms: f64,
    /// What each player used up (rounds, drugs, traps), by dict id, since
    /// the match started.
    used: HashMap<SocketAddr, BTreeMap<u16, u32>>,
    /// Per client, server ms minus client ms by its time sync requests (the
    /// smallest gap seen), for a client that has sent no update yet.
    sync_offsets: HashMap<SocketAddr, i64>,
}

/// What a bullet of one match can hit at a given moment.
struct MatchView<'a> {
    match_id: u32,
    world: &'a StaticWorld,
    model: &'a PlayerModel,
    data: &'a CombatData,
    fighters: &'a HashMap<SocketAddr, Fighter>,
}

impl BulletWorld for MatchView<'_> {
    fn ray(
        &self,
        origin: Vec3,
        dir: Vec3,
        length: f32,
        time_ms: f64,
        shooter: u8,
    ) -> Option<RayHit> {
        let mut best = self.world.ray_cast(origin, dir, length).map(|hit| RayHit {
            t: hit.t,
            point: hit.point,
            normal: hit.normal,
            material: hit.material,
            target: None,
        });
        for fighter in self.fighters.values() {
            if fighter.match_id != self.match_id || fighter.player_id == shooter || !fighter.alive {
                continue;
            }
            let Some(snapshot) = fighter.snapshot_at(time_ms) else {
                continue;
            };
            if !snapshot.alive {
                continue;
            }
            // Cheap rejection before posing.
            let middle = snapshot.position + Vec3::new(0.0, BODY_MIDDLE, 0.0);
            let along = (middle - origin).dot(dir).clamp(0.0, length);
            if (origin + dir * along - middle).length() > PLAYER_REACH {
                continue;
            }
            let Some(pose) = self
                .model
                .pose(&snapshot.pose_request(View::ThirdPerson, time_ms))
            else {
                continue;
            };
            let limit = best.as_ref().map_or(length, |b| b.t);
            for (placed, body_part) in self.model.place_hit_boxes(&pose, &snapshot.character()) {
                if let Some((t, normal)) = placed.ray(origin, dir) {
                    if t <= limit && best.as_ref().is_none_or(|b| t < b.t) {
                        best = Some(RayHit {
                            t,
                            point: origin + dir * t,
                            normal,
                            material: ACTOR_MATERIAL,
                            target: Some(Target::Player(fighter.player_id, body_part.to_owned())),
                        });
                    }
                }
            }
        }
        // The armed traps of the match, as they are now.
        for fighter in self.fighters.values() {
            if fighter.match_id != self.match_id {
                continue;
            }
            for (slot, set) in fighter.traps.iter().enumerate() {
                for (index, trap) in set.iter().flat_map(TrapSet::armed) {
                    let middle = trap.transform.c;
                    let along = (middle - origin).dot(dir).clamp(0.0, length);
                    if (origin + dir * along - middle).length() > TRAP_REACH {
                        continue;
                    }
                    for (placed, material) in trap.hittable() {
                        if let Some((t, normal)) = placed.ray(origin, dir)
                            && t <= length
                            && best.as_ref().is_none_or(|b| t < b.t)
                        {
                            best = Some(RayHit {
                                t,
                                point: origin + dir * t,
                                normal,
                                material: *material,
                                target: Some(Target::Trap(TrapId {
                                    owner: fighter.player_id,
                                    slot,
                                    index,
                                })),
                            });
                        }
                    }
                }
            }
        }
        best
    }

    fn contains(&self, point: Vec3) -> bool {
        self.world.contains(point)
    }

    fn material(&self, id: u16) -> Material {
        self.data.materials.get(id)
    }
}

impl Combat {
    pub fn new(assets: Arc<CombatAssets>) -> Self {
        let max_rewind_ms = std::env::var(MAX_REWIND_VARIABLE)
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(DEFAULT_MAX_REWIND_MS)
            .max(0.0);
        log::info!("combat: lag compensation up to {max_rewind_ms} ms ({MAX_REWIND_VARIABLE})");
        Self {
            assets,
            fighters: HashMap::new(),
            bullets: Vec::new(),
            pending: Vec::new(),
            epoch: Instant::now(),
            max_rewind_ms,
            used: HashMap::new(),
            sync_offsets: HashMap::new(),
        }
    }

    fn use_up(&mut self, addr: SocketAddr, dict_id: u16) {
        *self
            .used
            .entry(addr)
            .or_default()
            .entry(dict_id)
            .or_default() += 1;
    }

    /// What a player used up since the match started (or since the last
    /// take), by dict id.
    pub fn take_used(&mut self, addr: SocketAddr) -> Vec<(u16, u32)> {
        self.used
            .remove(&addr)
            .map(|used| used.into_iter().collect())
            .unwrap_or_default()
    }

    fn now_ms(&self, at: Instant) -> u64 {
        at.saturating_duration_since(self.epoch).as_millis() as u64
    }

    /// A living player's damage model (`None` for a dead or unknown one),
    /// its times on the server clock combat runs on.
    pub fn damage_state(&self, addr: SocketAddr) -> Option<Vec<PartState>> {
        let fighter = self.fighters.get(&addr).filter(|f| f.alive)?;
        Some(fighter.damage.state())
    }

    /// A time sync request (`network_client::send_sync_request`) stamped
    /// `client_ms` on the client's clock arrived at `arrival`. The request
    /// reads the game's permanent timer and the updates its frame timer, and
    /// the two only part while the game is paused, so these samples stand in
    /// for the updates' clock only until updates come.
    pub fn note_sync_request(&mut self, addr: SocketAddr, client_ms: u32, arrival: Instant) {
        let gap = self.now_ms(arrival) as i64 - i64::from(client_ms);
        let offset = self.sync_offsets.entry(addr).or_insert(gap);
        *offset = (*offset).min(gap);
    }

    /// Server ms minus the client's clock (ms), from its updates, else from
    /// its time sync requests; `None` while neither has come.
    pub fn client_offset(&self, addr: SocketAddr) -> Option<i64> {
        self.fighters
            .get(&addr)
            .and_then(|fighter| fighter.clock.offset())
            .or_else(|| self.sync_offsets.get(&addr).copied())
    }

    /// The magazine and chamber a weapon spawns with (`None` if unknown).
    pub fn spawn_load(&self, weapon_dict_id: u16) -> Option<(u16, Option<bool>)> {
        let weapon = self.assets.data.weapon(weapon_dict_id)?;
        let (magazine, chambered) = spawn_load(weapon);
        Some((magazine, weapon.has_chamber_state.then_some(chambered)))
    }

    /// A (re)spawned player with weapons fully loaded (see `spawn_loaded`).
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &mut self,
        addr: SocketAddr,
        match_id: u32,
        player_id: u8,
        team: u8,
        profile: &player_profile,
        active_slot: profile_slot_enum,
        seeds: Seeds,
        position: Vec3,
        yaw: f32,
        now: Instant,
    ) {
        self.spawn_loaded(
            addr,
            match_id,
            player_id,
            team,
            profile,
            active_slot,
            seeds,
            position,
            yaw,
            now,
            [None, None],
        );
    }

    /// What a player's life left in each stack slot, for the next one
    /// (`inventory::unload_to_profile`): the ammo carried, and a weapon's
    /// magazine and chamber with its current ammo; the drugs and the traps
    /// not placed (a respawn takes placed traps away).
    pub fn leftovers(&self, addr: SocketAddr) -> Vec<(profile_slot_enum, u32)> {
        let Some(fighter) = self.fighters.get(&addr) else {
            return Vec::new();
        };
        let mut left = Vec::new();
        for (weapon, slots) in fighter.arsenal.weapons.iter().zip(&fighter.ammo_slots) {
            let Some(weapon) = weapon else {
                continue;
            };
            for (i, ((_, _, carried), slot)) in weapon.ammo.iter().zip(slots).enumerate() {
                let in_weapon = if i == weapon.current_ammo {
                    u32::from(weapon.ammo_in_magazine) + u32::from(weapon.round_chambered)
                } else {
                    0
                };
                left.push((*slot, carried + in_weapon));
            }
        }
        for (i, slot) in QUICK_SLOT_ENUMS.into_iter().enumerate() {
            if let Some(drug) = &fighter.drugs[i] {
                left.push((slot, drug.amount));
            }
            if let Some(set) = &fighter.traps[i] {
                left.push((slot, set.amount));
            }
        }
        left
    }

    /// A (re)spawned player: full health for its worn armour, weapons loaded
    /// as `loads` says (by weapon slot; `None` is a full load), the seeds its
    /// spawn message carries. Weapon recoil carries over, as it does on the
    /// client, and so does the breath sway's clock (activating the weapon
    /// resets the rest of it).
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_loaded(
        &mut self,
        addr: SocketAddr,
        match_id: u32,
        player_id: u8,
        team: u8,
        profile: &player_profile,
        active_slot: profile_slot_enum,
        seeds: Seeds,
        position: Vec3,
        yaw: f32,
        now: Instant,
        loads: [Option<(u16, Option<bool>)>; 2],
    ) {
        let now_ms = self.now_ms(now);
        let worn = profile
            .slots
            .iter()
            .filter(|slot| slot.id != 0)
            .map(|slot| slot.dict_id);
        let damage = DamageModel::new(
            &self.assets.data.body,
            &Equipment::from_items(&self.assets.data, worn),
        );

        let slots = [
            (
                profile_slot_enum::weapon1_slot,
                [
                    profile_slot_enum::ammo1_weapon1_slot,
                    profile_slot_enum::ammo2_weapon1_slot,
                ],
            ),
            (
                profile_slot_enum::weapon2_slot,
                [
                    profile_slot_enum::ammo1_weapon2_slot,
                    profile_slot_enum::ammo2_weapon2_slot,
                ],
            ),
        ];
        let previous = self.fighters.remove(&addr);
        let mut weapons: [Option<WeaponState>; 2] = [None, None];
        let mut weapon_ammo_slots: [Vec<profile_slot_enum>; 2] = [Vec::new(), Vec::new()];
        for (i, (weapon_slot, ammo_slots)) in slots.into_iter().enumerate() {
            let item = profile.slots[weapon_slot];
            if item.id == 0 {
                continue;
            }
            let Some(data) = self.assets.data.weapon(item.dict_id) else {
                continue;
            };
            let (ammo, ammo_slots): (Vec<_>, Vec<_>) = ammo_slots
                .iter()
                .filter(|slot| profile.slots[**slot].id != 0)
                .filter_map(|slot| {
                    let ammo = profile.slots[*slot];
                    let amount = ammo.condition_or_stack.min(ammo.amount_in_inventory);
                    Some((
                        (
                            ammo.dict_id,
                            self.assets.data.ammo(ammo.dict_id)?.clone(),
                            amount,
                        ),
                        *slot,
                    ))
                })
                .unzip();
            let load = match loads[i] {
                Some((magazine, chamber)) => (magazine, chamber.unwrap_or(false)),
                None => spawn_load(data),
            };
            let Some(mut state) = WeaponState::loaded(
                item.dict_id,
                data.clone(),
                ammo,
                seeds[i],
                &self.assets.data.character_dispersion,
                load,
            ) else {
                log::warn!(
                    "weapon {}: its clips are missing from the game data",
                    item.dict_id
                );
                continue;
            };
            if let Some(old) = previous
                .as_ref()
                .and_then(|f| f.arsenal.weapons[i].as_ref())
                .filter(|old| old.dict_id == item.dict_id)
            {
                state.recoil = old.recoil.clone();
                state.breath = old.breath.clone();
            }
            weapons[i] = Some(state);
            weapon_ammo_slots[i] = ammo_slots;
        }
        let active = usize::from(active_slot == profile_slot_enum::weapon2_slot);
        let arsenal = Arsenal::new(
            weapons,
            active,
            self.assets.data.character_dispersion,
            self.assets.data.breath,
            self.assets.data.stamina,
        );
        let clock = previous.map(|f| f.clock).unwrap_or_default();
        let quick_slots = QUICK_SLOT_ENUMS;
        // The stacks the spawn message gives the clients.
        let drugs = std::array::from_fn(|i| {
            let item = profile.slots[quick_slots[i]];
            let data = self
                .assets
                .data
                .drug(item.dict_id)
                .filter(|_| item.id != 0)?;
            let amount = item
                .condition_or_stack
                .min(item.amount_in_inventory)
                .min(u32::from(u16::MAX));
            Some(Drug::new(item.dict_id, data.clone(), amount, i as u32 + 1))
        });
        // The client makes one trap object per unit of the profile's stack
        // (`inventory_cook`: a u8), and a respawn takes every placed trap
        // away (`player::remove`), so a set starts full with none out.
        let traps = std::array::from_fn(|i| {
            let item = profile.slots[quick_slots[i]];
            let data = self
                .assets
                .data
                .trap(item.dict_id)
                .filter(|_| item.id != 0)?;
            let amount = item
                .condition_or_stack
                .min(item.amount_in_inventory)
                .min(u32::from(u16::MAX));
            Some(TrapSet::new(
                item.dict_id,
                data.clone(),
                item.condition_or_stack as u8,
                amount,
            ))
        });
        let mut fighter = Fighter {
            match_id,
            player_id,
            team,
            alive: true,
            damage,
            arsenal,
            drugs,
            traps,
            ammo_slots: weapon_ammo_slots,
            use_key: UseKey::default(),
            defusing: None,
            history: VecDeque::new(),
            clock,
            movement_key: (0, false, false),
            movement_since_ms: now_ms,
            last_update_ms: now_ms,
            last_regen_ms: now_ms,
        };
        let weapon_dict_id = fighter
            .arsenal
            .active_weapon()
            .map_or(0, |weapon| weapon.dict_id);
        fighter.history.push_back(Snapshot {
            at_ms: now_ms,
            position,
            yaw,
            pitch: 0.0,
            mask: 0,
            alive: true,
            weapon_dict_id,
            recoil: RESTING_RECOIL,
            movement_since_ms: now_ms,
        });
        self.fighters.insert(addr, fighter);
    }

    /// A player who left, with its bullets in flight: a hit names its
    /// shooter, and the victim's client looks the shooter up unchecked
    /// (`player::apply_hit_directly`).
    pub fn remove(&mut self, addr: SocketAddr) {
        self.used.remove(&addr);
        self.sync_offsets.remove(&addr);
        if let Some(fighter) = self.fighters.remove(&addr) {
            self.bullets.retain(|(match_id, bullet)| {
                !(*match_id == fighter.match_id && bullet.shooter == fighter.player_id)
            });
        }
    }

    /// One client frame: replay its weapons, fire, and record where it was.
    pub fn update(
        &mut self,
        addr: SocketAddr,
        input: &player_input,
        state: &player_state,
        client_ms: u32,
        round_trip: Option<Duration>,
        arrival: Instant,
    ) {
        let arrival_ms = self.now_ms(arrival);
        let max_rewind_ms = self.max_rewind_ms as u64;
        let Some(fighter) = self.fighters.get_mut(&addr) else {
            return;
        };
        let at_ms = fighter
            .clock
            .map(client_ms, arrival_ms, max_rewind_ms)
            .max(fighter.last_update_ms);
        fighter.last_update_ms = at_ms;
        let mask = input.action_mask;
        let position = Vec3::from(state.translation_w);
        let yaw = state.euler_angle_y;
        let pitch = state.look_pitch.clamp(-1.0, 1.0);
        if !(position.is_finite() && yaw.is_finite() && pitch.is_finite()) {
            return;
        }

        // The shots of this frame leave the camera the last frame drew.
        let drawn = fighter.history.back().copied();
        let mut shots = Vec::new();
        let mut released_traps = Vec::new();
        let mut drugs_used = Vec::new();
        if fighter.alive {
            released_traps = (0..QUICK_SLOTS)
                .filter(|&slot| fighter.traps[slot].is_some() && quick_slot_released(mask, slot))
                .collect();
            let injuries = fighter.injuries();
            shots = fighter.arsenal.frame(mask, client_ms, injuries);
            for (slot, drug) in fighter.drugs.iter_mut().enumerate() {
                if let Some(drug) = drug
                    && quick_slot_pressed(mask, slot)
                    && drug.press(at_ms, &mut fighter.damage)
                {
                    drugs_used.push(drug.dict_id);
                    log::info!(
                        "player {} uses item {} from quick slot {}; {} left",
                        fighter.player_id,
                        drug.dict_id,
                        slot + 1,
                        drug.amount
                    );
                }
            }
        }
        let key = (
            direction(mask),
            mask & bits::CROUCH != 0,
            mask & bits::AIM != 0,
        );
        if key != fighter.movement_key {
            fighter.movement_key = key;
            fighter.movement_since_ms = at_ms;
        }
        let (weapon_dict_id, recoil) = fighter
            .arsenal
            .active_weapon()
            .map_or((0, RESTING_RECOIL), |w| (w.dict_id, w.recoil.clip_times()));
        let snapshot = Snapshot {
            at_ms,
            position,
            yaw,
            pitch,
            mask,
            alive: fighter.alive,
            weapon_dict_id,
            recoil,
            movement_since_ms: fighter.movement_since_ms,
        };
        fighter.history.push_back(snapshot);
        while fighter.history.len() > HISTORY_UPDATES
            || fighter
                .history
                .front()
                .is_some_and(|s| s.at_ms + HISTORY_MS < at_ms)
        {
            fighter.history.pop_front();
        }

        let using = fighter.use_key.held(mask & USE_KEY != 0, at_ms) && fighter.alive;

        let shooter = fighter.player_id;
        let match_id = fighter.match_id;
        let rewind_ms = round_trip
            .map_or(0.0, |rtt| rtt.as_secs_f64() * 1000.0)
            .min(self.max_rewind_ms);
        let camera = drawn.filter(|drawn| drawn.alive).unwrap_or(snapshot);
        for slot in released_traps {
            self.place_trap(addr, slot, &camera, at_ms);
        }
        self.use_traps(addr, &snapshot, using, at_ms);
        for dict_id in drugs_used {
            self.use_up(addr, dict_id);
        }
        for shot in shots {
            self.use_up(addr, shot.ammo_dict_id);
            // A shot comes between the last frame and this one, on the
            // client's clock.
            let before = u64::from(client_ms.wrapping_sub(shot.at_ms));
            let shot_ms = at_ms.saturating_sub(before).max(camera.at_ms);
            self.fire(match_id, shooter, &camera, &shot, shot_ms, rewind_ms);
        }
    }

    /// `player::detect_usable_objects` for traps, each update: holding use
    /// on a trap it may defuse starts its use (`use_initialize`), keeping at
    /// it for the trap's defuse time disarms it (`use_execute`), and letting
    /// go or looking away ends it (`use_finalize`).
    fn use_traps(&mut self, addr: SocketAddr, at: &Snapshot, using: bool, now_ms: u64) {
        let Some(user) = self.fighters.get(&addr) else {
            return;
        };
        let (match_id, user_id, user_team) = (user.match_id, user.player_id, user.team);
        let current = user.defusing;
        let target = if user.alive {
            self.looked_at_trap(match_id, at)
        } else {
            None
        };
        let Some(target) = target.filter(|_| using) else {
            if let Some((trap, _)) = current {
                log::info!(
                    "match {match_id:#x}: player {user_id} stops defusing trap {} of player {}",
                    trap.index,
                    trap.owner
                );
                self.fighters.get_mut(&addr).unwrap().defusing = None;
            }
            return;
        };
        if let Some((trap, since)) = current
            && trap == target
        {
            let Some(set) = self.trap_set(match_id, trap.owner, trap.slot) else {
                return;
            };
            if now_ms.saturating_sub(since) >= set.defuse_ms() {
                log::info!(
                    "match {match_id:#x}: player {user_id} defuses trap {} of player {}",
                    trap.index,
                    trap.owner
                );
                let mut events = std::mem::take(&mut self.pending);
                self.disarm_trap(match_id, trap, now_ms, &mut events);
                self.pending = events;
            }
            return;
        }
        // Someone else at it, or a team mate's trap: no use.
        let owner_team = self
            .fighters
            .values()
            .find(|f| f.match_id == match_id && f.player_id == target.owner)
            .map(|f| f.team);
        let taken = self.fighters.values().any(|f| {
            f.match_id == match_id
                && f.player_id != user_id
                && f.alive
                && f.defusing.is_some_and(|(trap, _)| trap == target)
        });
        let starts = !taken
            && owner_team
                .is_some_and(|team| can_defuse((user_id, user_team), (target.owner, team)));
        if starts {
            log::info!(
                "match {match_id:#x}: player {user_id} starts defusing trap {} of player {}",
                target.index,
                target.owner
            );
        }
        self.fighters.get_mut(&addr).unwrap().defusing = starts.then_some((target, now_ms));
    }

    /// The armed trap of the match the player at `at` looks at within use
    /// reach, as the use ray finds it.
    fn looked_at_trap(&self, match_id: u32, at: &Snapshot) -> Option<TrapId> {
        let traps: Vec<(TrapId, &super::traps::Trap)> = self
            .fighters
            .values()
            .filter(|f| f.match_id == match_id)
            .flat_map(|f| {
                f.traps.iter().enumerate().flat_map(move |(slot, set)| {
                    set.iter()
                        .flat_map(TrapSet::armed)
                        .map(move |(index, trap)| {
                            let id = TrapId {
                                owner: f.player_id,
                                slot,
                                index,
                            };
                            (id, trap)
                        })
                })
            })
            .filter(|(_, trap)| {
                (trap.transform.c - at.position).length() < USE_REACH + PLAYER_REACH + TRAP_REACH
            })
            .collect();
        if traps.is_empty() {
            return None;
        }
        let pose = self
            .assets
            .model
            .pose(&at.pose_request(View::FirstPerson, at.at_ms as f64))?;
        let head = pose.head_matrix() * at.character();
        let dir = head.k.normalized_or(Vec3::new(0.0, 0.0, 1.0));
        let mut best: Option<(f32, TrapId)> = None;
        for (id, trap) in traps {
            for volume in trap.usable() {
                if let Some((t, _)) = volume.ray(head.c, dir)
                    && t <= USE_REACH
                    && best.is_none_or(|(b, _)| t < b)
                {
                    best = Some((t, id));
                }
            }
        }
        best.map(|(_, id)| id)
    }

    fn trap_set(&self, match_id: u32, owner: u8, slot: usize) -> Option<&TrapSet> {
        self.fighters
            .values()
            .find(|f| f.match_id == match_id && f.player_id == owner)?
            .traps
            .get(slot)?
            .as_ref()
    }

    /// `booby_trap_core::defuse_completed`: an armed trap is disarmed.
    fn disarm_trap(
        &mut self,
        match_id: u32,
        trap: TrapId,
        now_ms: u64,
        events: &mut Vec<CombatEvent>,
    ) {
        let change = self
            .fighters
            .values_mut()
            .find(|f| f.match_id == match_id && f.player_id == trap.owner)
            .and_then(|f| f.traps.get_mut(trap.slot)?.as_mut())
            .and_then(|set| set.disarm(trap.index, now_ms));
        if let Some(change) = change {
            events.push(CombatEvent::TrapChanged {
                match_id,
                player_id: trap.owner,
                slot: profile_slot_enum::quick_slot1 as u8 + trap.slot as u8,
                index: trap.index,
                change,
            });
        }
    }

    /// The trap set in quick slot `slot` (0-5) places a trap from the
    /// first-person camera of the frame `at` drew, if it can, at server time
    /// `now_ms`.
    fn place_trap(&mut self, addr: SocketAddr, slot: usize, at: &Snapshot, now_ms: u64) {
        let Some(pose) = self
            .assets
            .model
            .pose(&at.pose_request(View::FirstPerson, at.at_ms as f64))
        else {
            return;
        };
        let head = pose.head_matrix() * at.character();
        let Some(match_id) = self.fighters.get(&addr).map(|f| f.match_id) else {
            return;
        };
        // Every armed trap of the match refuses another on top of it.
        let refusing: Vec<_> = self
            .fighters
            .values()
            .filter(|f| f.match_id == match_id)
            .flat_map(|f| f.traps.iter().flatten())
            .flat_map(TrapSet::refusing_volumes)
            .collect();
        let Some(fighter) = self.fighters.get_mut(&addr) else {
            return;
        };
        let Some(set) = fighter.traps[slot].as_mut() else {
            return;
        };
        match set.try_place(
            &head,
            &self.assets.walkable,
            &self.assets.data.materials,
            &refusing,
            now_ms,
        ) {
            Ok((index, transform)) => {
                let dict_id = set.dict_id;
                *self
                    .used
                    .entry(addr)
                    .or_default()
                    .entry(dict_id)
                    .or_default() += 1;
                log::info!(
                    "player {} places trap {index} of item {} from quick slot {} at \
                     ({:.2}, {:.2}, {:.2}); {} left",
                    fighter.player_id,
                    set.dict_id,
                    slot + 1,
                    transform.c.x,
                    transform.c.y,
                    transform.c.z,
                    set.amount
                );
                self.pending.push(CombatEvent::TrapPlaced {
                    match_id,
                    player_id: fighter.player_id,
                    slot: profile_slot_enum::quick_slot1 as u8 + slot as u8,
                    index,
                    position: transform.c,
                    angles: transform.rotation_angles(),
                });
            }
            Err(refusal) => log::info!(
                "player {} places no trap from quick slot {}: {refusal}",
                fighter.player_id,
                slot + 1
            ),
        }
    }

    /// Bullets for one shot, leaving the shooter's first-person camera as the
    /// frame `at` drew it, with the shot's random spread
    /// (`weapon_core::get_dispersed_bullet_dir`).
    fn fire(
        &mut self,
        match_id: u32,
        shooter: u8,
        at: &Snapshot,
        shot: &Shot,
        shot_ms: u64,
        rewind_ms: f64,
    ) {
        let request = PoseRequest {
            recoil: shot.recoil,
            weapon_dict_id: shot.weapon_dict_id,
            ..at.pose_request(View::FirstPerson, at.at_ms as f64)
        };
        let Some(pose) = self.assets.model.pose(&request) else {
            return;
        };
        let camera = pose.head_matrix() * at.character();
        let forward = camera.k.normalized_or(Vec3::new(0.0, 0.0, 1.0));
        let right = camera.i.normalized_or(Vec3::new(1.0, 0.0, 0.0));
        // For the shot log: the same camera without recoil, to see what the kick adds.
        let resting = log::log_enabled!(log::Level::Info)
            .then(|| {
                self.assets.model.pose(&PoseRequest {
                    recoil: RESTING_RECOIL,
                    ..request
                })
            })
            .flatten()
            .map(|pose| {
                (pose.head_matrix() * at.character())
                    .k
                    .normalized_or(Vec3::new(0.0, 0.0, 1.0))
            });
        for (angle, k) in &shot.pellets {
            let axis = Mat4::rotation_axis(forward, *k).transform_direction(right);
            let degrees = shot.dispersion_degrees * angle;
            let dir = Mat4::rotation_axis(axis, degrees.to_radians()).transform_direction(forward);
            if log::log_enabled!(log::Level::Info) {
                self.log_shot(
                    match_id, shooter, at, shot, shot_ms, rewind_ms, camera.c, dir, forward,
                    resting,
                );
            }
            let spec = BulletSpec {
                shooter,
                weapon_dict_id: shot.weapon_dict_id,
                damage: shot.weapon.bullet_damage * shot.ammo.k_damage,
                pierce: shot.weapon.bullet_pierce * shot.ammo.k_arp,
                ricochet_angle_degrees: shot.ammo.ricochet_angle,
                air_resistance: shot.ammo.air_resistance,
                distance_coef: shot.ammo.distance_coef,
                rewind_ms,
            };
            let bullet = Bullet::new(spec, camera.c, dir * shot.ammo.muzzle_speed, shot_ms);
            self.bullets.push((match_id, bullet));
        }
    }

    /// Debug: where a shot starts and aims, what map it meets first, and how
    /// far it passes from every other player's head at the time its bullet
    /// would reach them.
    #[allow(clippy::too_many_arguments)]
    fn log_shot(
        &self,
        match_id: u32,
        shooter: u8,
        at: &Snapshot,
        shot: &Shot,
        shot_ms: u64,
        rewind_ms: f64,
        origin: Vec3,
        dir: Vec3,
        forward: Vec3,
        resting: Option<Vec3>,
    ) {
        const RANGE: f32 = 300.0;
        // What the recoil and the breath sway add to the camera: degrees
        // up, degrees aside.
        let kick = resting.map_or("unknown".to_owned(), |rest| {
            let up = forward.y.clamp(-1.0, 1.0).asin() - rest.y.clamp(-1.0, 1.0).asin();
            let side = forward.x.atan2(forward.z) - rest.x.atan2(rest.z);
            format!(
                "{:+.2} up, {:+.2} aside",
                up.to_degrees(),
                side.to_degrees()
            )
        });
        let sway = &shot.sway;
        let sway = if sway.aimed {
            format!(
                "breath sway {:+.4} h, {:+.4} v (x{:.2})",
                sway.horizontal, sway.vertical, sway.multiplier
            )
        } else {
            format!("no breath sway from the hip (x{:.2})", sway.multiplier)
        };
        let map = self
            .assets
            .world
            .ray_cast(origin, dir, RANGE)
            .map_or("none".to_owned(), |hit| format!("{:.2} m", hit.t));
        let mut heads = Vec::new();
        for fighter in self.fighters.values() {
            if fighter.match_id != match_id || fighter.player_id == shooter || !fighter.alive {
                continue;
            }
            let time_ms = shot_ms as f64 - rewind_ms;
            let Some(snapshot) = fighter.snapshot_at(time_ms) else {
                continue;
            };
            let Some(pose) = self
                .assets
                .model
                .pose(&snapshot.pose_request(View::ThirdPerson, time_ms))
            else {
                continue;
            };
            let boxes = self
                .assets
                .model
                .place_hit_boxes(&pose, &snapshot.character());
            let Some((head, _)) = boxes.iter().find(|(_, part)| *part == "head") else {
                continue;
            };
            let center = head.to_world.c;
            let along = (center - origin).dot(dir);
            let miss = origin + dir * along - center;
            heads.push(format!(
                "player {} head {:.2} m away at ({:.3}, {:.3}, {:.3}): ray passes {:+.3} up, ({:+.3}, {:+.3}) across, {:.3} off",
                fighter.player_id,
                along,
                center.x,
                center.y,
                center.z,
                miss.y,
                miss.x,
                miss.z,
                miss.length(),
            ));
        }
        log::info!(
            "shot: player {shooter} weapon {} at {shot_ms} ms (rewind {rewind_ms:.0}) from              ({:.3}, {:.3}, {:.3}) feet ({:.3}, {:.3}, {:.3}) pitch {:.4} dir ({:.4}, {:.4}, {:.4})              spread {:.3} deg; recoil clips {:?} with {sway} add {kick} deg; map at {map}; {}",
            shot.weapon_dict_id,
            origin.x,
            origin.y,
            origin.z,
            at.position.x,
            at.position.y,
            at.position.z,
            at.pitch,
            dir.x,
            dir.y,
            dir.z,
            shot.dispersion_degrees,
            shot.recoil,
            heads.join("; "),
        );
    }

    /// Flies every bullet to `now`, applies the hits, regenerates health.
    /// Bullets of a match that `can_hurt` refuses (time is up) are dropped.
    pub fn tick(&mut self, now: Instant, can_hurt: impl Fn(u32) -> bool) -> Vec<CombatEvent> {
        let now_ms = self.now_ms(now);
        let mut events = std::mem::take(&mut self.pending);
        let mut bullets = std::mem::take(&mut self.bullets);
        bullets.retain(|(match_id, _)| can_hurt(*match_id));
        for (match_id, bullet) in &mut bullets {
            let mut until = bullet.flown_until_ms();
            while !bullet.is_finished() && until < now_ms {
                until = (until + BULLET_STEP_MS).min(now_ms);
                let view = MatchView {
                    match_id: *match_id,
                    world: &self.assets.world,
                    model: &self.assets.model,
                    data: &self.assets.data,
                    fighters: &self.fighters,
                };
                let hits = bullet.tick(until, &view);
                for hit in hits {
                    match hit {
                        BulletHit::Player { victim, body_part } => {
                            log::info!(
                                "match {match_id:#x}: player {}'s bullet (weapon {}) hits                                  player {victim}'s {body_part}, damage {:.3}",
                                bullet.shooter,
                                bullet.weapon_dict_id,
                                bullet.damage
                            );
                            self.apply_hit(
                                *match_id,
                                bullet.shooter,
                                bullet.weapon_dict_id,
                                "injury",
                                bullet.damage,
                                bullet.pierce,
                                &mut bullet.last_hit,
                                victim,
                                &body_part,
                                now_ms,
                                &mut events,
                            )
                        }
                        BulletHit::Trap(trap) => {
                            log::info!(
                                "match {match_id:#x}: player {}'s bullet hits trap {} of \
                                 player {}'s quick slot {}",
                                bullet.shooter,
                                trap.index,
                                trap.owner,
                                trap.slot + 1
                            );
                            self.disarm_trap(*match_id, trap, now_ms, &mut events);
                        }
                    }
                }
            }
        }
        bullets.retain(|(_, bullet)| !bullet.is_finished());
        bullets.append(&mut self.bullets);
        self.bullets = bullets;

        self.spring_traps(now_ms, &can_hurt, &mut events);
        self.time_traps(now_ms, &mut events);

        for fighter in self.fighters.values_mut() {
            if !fighter.alive {
                fighter.last_regen_ms = now_ms;
                continue;
            }
            for drug in fighter.drugs.iter_mut().flatten() {
                drug.tick(now_ms, &mut fighter.damage);
            }
            let delta = now_ms.saturating_sub(fighter.last_regen_ms);
            fighter.last_regen_ms = now_ms;
            fighter.damage.regenerate(delta as u32, now_ms as u32);
            push_limb_changes(fighter, &mut events);
        }
        events
    }

    /// Every armed trap that someone stands in goes off: the players its
    /// sensor touches take its damage, from its owner, and it fires
    /// (`booby_trap_core::on_enter`). The sensor meets the players' hit
    /// models, posed where they are now.
    fn spring_traps(
        &mut self,
        now_ms: u64,
        can_hurt: &impl Fn(u32) -> bool,
        events: &mut Vec<CombatEvent>,
    ) {
        let mut sprung = Vec::new();
        for (owner_addr, owner) in &self.fighters {
            if !can_hurt(owner.match_id) {
                continue;
            }
            for (slot, set) in owner.traps.iter().enumerate() {
                let Some(set) = set else {
                    continue;
                };
                for (index, trap) in set.armed() {
                    let caught: Vec<u8> = self
                        .fighters
                        .values()
                        .filter(|f| f.match_id == owner.match_id && f.alive)
                        .filter(|f| self.touches(f, trap.sensor()))
                        .map(|f| f.player_id)
                        .collect();
                    if !caught.is_empty() {
                        sprung.push((*owner_addr, slot, index, trap.transform.c, caught));
                    }
                }
            }
        }
        for (owner_addr, slot, index, at, caught) in sprung {
            let Some(owner) = self.fighters.get(&owner_addr) else {
                continue;
            };
            let (match_id, owner_id) = (owner.match_id, owner.player_id);
            let Some(set) = owner.traps[slot].as_ref() else {
                continue;
            };
            let dict_id = set.dict_id;
            let damage = set.damage().to_vec();
            for victim in caught {
                log::info!(
                    "match {match_id:#x}: player {victim} sets off trap {index} of player \
                     {owner_id}'s quick slot {} at ({:.2}, {:.2}, {:.2})",
                    slot + 1,
                    at.x,
                    at.y,
                    at.z
                );
                for entry in &damage {
                    // Not a bullet: nothing stops one part hit twice.
                    self.apply_hit(
                        match_id,
                        owner_id,
                        dict_id,
                        &entry.hit_type,
                        entry.amount,
                        entry.armor_piercing,
                        &mut None,
                        victim,
                        &entry.body_part,
                        now_ms,
                        events,
                    );
                }
            }
            let change = self
                .fighters
                .get_mut(&owner_addr)
                .and_then(|owner| owner.traps[slot].as_mut()?.fire(index, now_ms));
            if let Some(change) = change {
                events.push(CombatEvent::TrapChanged {
                    match_id,
                    player_id: owner_id,
                    slot: profile_slot_enum::quick_slot1 as u8 + slot as u8,
                    index,
                    change,
                });
            }
        }
    }

    /// Whether `sensor` touches the fighter's hit boxes, posed as its latest
    /// update shows it.
    fn touches(&self, fighter: &Fighter, sensor: &[Placed]) -> bool {
        let Some(snapshot) = fighter.history.back().filter(|s| s.alive) else {
            return false;
        };
        let middle = snapshot.position + Vec3::new(0.0, BODY_MIDDLE, 0.0);
        if sensor
            .iter()
            .all(|shape| (shape.to_world.c - middle).length() > PLAYER_REACH + SENSOR_REACH)
        {
            return false;
        }
        let Some(pose) = self
            .assets
            .model
            .pose(&snapshot.pose_request(View::ThirdPerson, snapshot.at_ms as f64))
        else {
            return false;
        };
        let boxes = self
            .assets
            .model
            .place_hit_boxes(&pose, &snapshot.character());
        sensor
            .iter()
            .any(|shape| boxes.iter().any(|(hit_box, _)| shape.overlaps(hit_box)))
    }

    /// Runs out the traps' states (`booby_trap_core::tick`).
    fn time_traps(&mut self, now_ms: u64, events: &mut Vec<CombatEvent>) {
        for fighter in self.fighters.values_mut() {
            for (slot, set) in fighter.traps.iter_mut().enumerate() {
                let Some(set) = set else {
                    continue;
                };
                for (index, change) in set.tick(now_ms) {
                    events.push(CombatEvent::TrapChanged {
                        match_id: fighter.match_id,
                        player_id: fighter.player_id,
                        slot: profile_slot_enum::quick_slot1 as u8 + slot as u8,
                        index,
                        change,
                    });
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_hit(
        &mut self,
        match_id: u32,
        shooter: u8,
        weapon_dict_id: u16,
        hit_type: &str,
        amount: f32,
        armor_piercing: f32,
        last_hit: &mut LastHit,
        victim: u8,
        body_part: &str,
        now_ms: u64,
        events: &mut Vec<CombatEvent>,
    ) {
        let Some((addr, fighter)) = self
            .fighters
            .iter_mut()
            .find(|(_, f)| f.match_id == match_id && f.player_id == victim)
        else {
            return;
        };
        if !fighter.alive {
            return;
        }
        let Some(part) = fighter.damage.index_of(body_part) else {
            log::warn!("hit box part `{body_part}` is not in the damage model");
            return;
        };
        if !fighter.damage.hit_body_part(
            victim,
            part,
            hit_type,
            amount,
            armor_piercing,
            now_ms as u32,
            last_hit,
        ) {
            return;
        }
        events.push(CombatEvent::Hit {
            match_id,
            shooter,
            victim,
            body_part: body_part.to_owned(),
            hit_type: hit_type.to_owned(),
            amount,
            armor_piercing,
        });
        push_limb_changes(fighter, events);
        if fighter.damage.has_affect(affect::DEATH) {
            fighter.alive = false;
            events.push(CombatEvent::Kill {
                match_id,
                victim: *addr,
                victim_id: victim,
                killer_id: shooter,
                is_headshot: matches!(body_part, "head" | "face"),
                weapon_dict_id,
            });
        }
    }

    /// The player at `addr` killed itself: it can no longer shoot or be hit.
    /// The weapon it held, for the kill feed; `None` if it was already dead.
    pub fn commit_suicide(&mut self, addr: SocketAddr) -> Option<Option<u16>> {
        let fighter = self.fighters.get_mut(&addr)?;
        if !fighter.alive {
            return None;
        }
        fighter.alive = false;
        Some(fighter.arsenal.active_weapon().map(|weapon| weapon.dict_id))
    }

    /// Every trap out in the match, of every player.
    pub fn traps_out(&self, match_id: u32) -> Vec<TrapReport> {
        let mut reports = Vec::new();
        for fighter in self.fighters.values().filter(|f| f.match_id == match_id) {
            for (slot, set) in fighter.traps.iter().enumerate() {
                for (index, trap) in set.iter().flat_map(TrapSet::out) {
                    reports.push(TrapReport {
                        player_id: fighter.player_id,
                        slot: profile_slot_enum::quick_slot1 as u8 + slot as u8,
                        index,
                        state: trap.state as u8,
                        position: trap.transform.c,
                        angles: trap.transform.rotation_angles(),
                    });
                }
            }
        }
        reports
    }

    /// Every weapon in the game data, by dictionary id.
    #[cfg(test)]
    pub fn weapon_dict_ids(&self) -> Vec<u16> {
        let mut ids: Vec<u16> = self
            .assets
            .data
            .items
            .iter()
            .filter(|(_, item)| matches!(item, super::data::Item::Weapon(_)))
            .map(|(id, _)| *id)
            .collect();
        ids.sort();
        ids
    }

    /// What a living player holds now (`None` for a dead or unknown one): each
    /// weapon's random generators where they draw next, magazine, chamber,
    /// ammo slot and fire mode, the stacks in hand, and the stamina.
    pub fn kit(&self, addr: SocketAddr) -> Option<Kit> {
        let fighter = self.fighters.get(&addr).filter(|f| f.alive)?;
        let mut amounts = Vec::new();
        let weapons = std::array::from_fn(|i| {
            let weapon = fighter.arsenal.weapons[i].as_ref()?;
            let slots = &fighter.ammo_slots[i];
            for ((_, _, carried), slot) in weapon.ammo.iter().zip(slots) {
                amounts.push((*slot, *carried));
            }
            Some(WeaponLoad {
                seeds: (weapon.random.0, weapon.normal_random.0),
                load: (
                    weapon.ammo_in_magazine,
                    weapon
                        .data
                        .has_chamber_state
                        .then_some(weapon.round_chambered),
                ),
                ammo_slot: slots.get(weapon.current_ammo).copied(),
                fire_queue_type: weapon.fire_queue_type as u8,
            })
        });
        for (i, slot) in QUICK_SLOT_ENUMS.into_iter().enumerate() {
            if let Some(drug) = &fighter.drugs[i] {
                amounts.push((slot, drug.amount));
            }
            if let Some(set) = &fighter.traps[i] {
                amounts.push((slot, set.amount));
            }
        }
        Some(Kit {
            weapon_slot: fighter.arsenal.requested_slot(),
            weapons,
            amounts,
            stamina: fighter.arsenal.stamina().snapshot(),
        })
    }

    /// The living player at `addr` was sent its own spawn as it is now (a
    /// world resync): its client puts it back in the world and restarts its
    /// weapons idle from the spawn, so the replay restarts them the same way
    /// (`Arsenal::respawned`), and a defuse in progress starts over, as the
    /// client's use does. Health, drugs, traps and the clock carry on.
    pub fn respawn_weapons(&mut self, addr: SocketAddr) {
        if let Some(fighter) = self.fighters.get_mut(&addr).filter(|f| f.alive) {
            fighter.arsenal = fighter.arsenal.respawned();
            fighter.defusing = None;
        }
    }

    /// The weapon slot (0 or 1) a fighter holds or is switching to, as the
    /// client's `inventory::get_active_slot` reports it.
    pub fn weapon_slot(&self, addr: SocketAddr) -> Option<usize> {
        self.fighters.get(&addr).map(|f| f.arsenal.requested_slot())
    }

    /// For tests and the console: a fighter's lowest vital health fraction.
    pub fn vital_fraction(&self, addr: SocketAddr) -> Option<f32> {
        self.fighters.get(&addr).map(|f| f.damage.vital_fraction())
    }

    pub fn bullets_in_flight(&self) -> usize {
        self.bullets.len()
    }
}

/// Whether the other clients hear of `affect` coming and going
/// (`LimbAffect`). Only those reach their copies of a player: a remote
/// player's damage model is read-only there (`player_cook` gives only the
/// local player `type_apply_directly`), so its hits apply no affects and its
/// affects never run out (`body_part_parameters::regenerate`).
pub fn is_announced(affect: u8) -> bool {
    matches!(affect, affect::HAND_DAMAGE | affect::LEG_DAMAGE)
}

fn push_limb_changes(fighter: &mut Fighter, events: &mut Vec<CombatEvent>) {
    for change in fighter.damage.take_changes() {
        if is_announced(change.affect) {
            events.push(CombatEvent::LimbAffect {
                match_id: fighter.match_id,
                player_id: fighter.player_id,
                body_part: change.body_part,
                affect: change.affect,
                applying: change.event == AffectEvent::Applying,
            });
        }
    }
}

#[cfg(test)]
impl Combat {
    /// A hit on the living player at `victim`'s `body_part`, armour
    /// notwithstanding, from nobody (a hit with no bullet).
    pub fn hurt(
        &mut self,
        victim: SocketAddr,
        body_part: &str,
        amount: f32,
        at: Instant,
    ) -> Vec<CombatEvent> {
        let now_ms = self.now_ms(at);
        let fighter = &self.fighters[&victim];
        let (match_id, player_id) = (fighter.match_id, fighter.player_id);
        let mut events = Vec::new();
        self.apply_hit(
            match_id,
            u8::MAX,
            0,
            "injury",
            amount,
            1000.0,
            &mut None,
            player_id,
            body_part,
            now_ms,
            &mut events,
        );
        events
    }

    /// An open stretch of the map for shooting tests: feet at a respawn point,
    /// a heading, and the feet `range` metres down it, with nothing in the
    /// way at body heights.
    pub fn firing_range(&self, range: f32) -> (Vec3, f32, Vec3) {
        let up = Vec3::new(0.0, 1.0, 0.0);
        for point in crate::map::RESPAWN_POINTS {
            let from = Vec3::from(point.position);
            for step in 0..16 {
                let yaw = step as f32 * std::f32::consts::TAU / 16.0;
                let forward = Mat4::rotation_y(yaw).transform_direction(Vec3::new(0.0, 0.0, 1.0));
                let to = from + forward * range;
                let clear = [0.3, 1.0, 1.6, 2.0].iter().all(|height| {
                    self.assets
                        .world
                        .ray_cast(from + up * *height, forward, range + 1.0)
                        .is_none()
                });
                if clear {
                    return (from, yaw, to);
                }
            }
        }
        panic!("no open ground on the map");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use survarium::player_profile::raw::player_profile;
    use vostok::math::float2;

    fn update(feet: Vec3, yaw: f32, mask: u32) -> (player_input, player_state) {
        (
            player_input {
                angular_velocity: float2 { x: 0.0, y: 0.0 },
                angualr_acceleration: float2 { x: 0.0, y: 0.0 },
                action_mask: mask,
            },
            player_state {
                translation_w: feet.into(),
                euler_angle_y: yaw,
                look_pitch: 0.0,
            },
        )
    }

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    /// The shooter, crouched (its eye line crosses a standing torso, which
    /// takes the whole aimed spread), aims where the victim stood a round
    /// trip ago; the victim has since stepped aside: only the rewound pose is
    /// hit.
    fn shoot_at_a_moving_target(round_trip: Option<Duration>) -> Vec<CombatEvent> {
        let mut combat = Combat::new(CombatAssets::for_tests().unwrap());
        combat.max_rewind_ms = 300.0;
        let (from, yaw, to) = combat.firing_range(8.0);
        let side = Mat4::rotation_y(yaw).transform_direction(Vec3::new(1.0, 0.0, 0.0));
        let (shooter, victim) = (addr(1), addr(2));
        let profile = player_profile::new_dummy(0, 0, "fighter");
        let start = Instant::now();
        for (addr, id, feet) in [(shooter, 0, from), (victim, 1, to)] {
            combat.spawn(
                addr,
                7,
                id,
                1,
                &profile,
                profile_slot_enum::weapon1_slot,
                [(11, 12), (13, 14)],
                feet,
                yaw,
                start,
            );
        }
        // The victim stands at `to` until 800 ms, then 3 m aside. The shooter
        // draws its weapon (600 ms) aiming, then fires with the frame at
        // 1000: the shot leaves 9 ms after the frame at 975.
        let aiming = bits::AIM | bits::CROUCH;
        for ms in (0..=1000).step_by(25) {
            let at = start + Duration::from_millis(ms);
            let feet = if ms <= 800 { to } else { to + side * 3.0 };
            let (input, state) = update(feet, yaw, 0);
            combat.update(victim, &input, &state, ms as u32, None, at);
            let mask = if ms == 1000 {
                aiming | bits::FIRE
            } else {
                aiming
            };
            let (input, state) = update(from, yaw, mask);
            combat.update(shooter, &input, &state, ms as u32, round_trip, at);
        }
        assert_eq!(combat.bullets_in_flight(), 1);
        combat.tick(start + Duration::from_millis(1200), |_| true)
    }

    #[test]
    fn the_clock_offset_is_the_smallest_gap_of_the_last_five_seconds() {
        let mut clock = ClientClock::default();
        let mut seen: Vec<(u64, i64)> = Vec::new();
        // Jittery arrivals of a client clock 1 s behind, then a clock jump.
        let mut state = 7u64;
        for frame in 0..2000u64 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let jitter = (state >> 33) % 40;
            let client_ms = (frame * 16 + if frame > 1500 { 3000 } else { 0 }) as u32;
            let arrival_ms = 1000 + frame * 16 + jitter;
            let mapped = clock.map(client_ms, arrival_ms, 300);
            seen.push((arrival_ms, arrival_ms as i64 - i64::from(client_ms)));
            let offset = seen
                .iter()
                .filter(|(at, _)| at + CLOCK_WINDOW_MS >= arrival_ms)
                .map(|(_, gap)| *gap)
                .min()
                .unwrap();
            let expected = (i64::from(client_ms) + offset)
                .clamp(arrival_ms as i64 - 300, arrival_ms as i64)
                as u64;
            assert_eq!(mapped, expected, "frame {frame}");
        }
    }

    #[test]
    fn a_life_that_uses_nothing_leaves_what_it_was_given() {
        let Some(assets) = CombatAssets::for_tests() else {
            return;
        };
        let mut combat = Combat::new(assets);
        let profile = player_profile::new_dummy(0, 0, "fighter");
        let player = addr(1);
        let mut supplies = crate::supplies::Supplies::default();
        let life = supplies.next_life(&profile, |dict_id| combat.spawn_load(dict_id).unwrap());
        combat.spawn_loaded(
            player,
            7,
            0,
            1,
            &life.profile,
            profile_slot_enum::weapon1_slot,
            [(11, 12), (13, 14)],
            Vec3::from(crate::map::RESPAWN_POINTS[0].position),
            0.0,
            Instant::now(),
            life.loads,
        );

        // The magazine came out of the first ammo slot and goes back to it.
        let ak = combat.fighters[&player].arsenal.weapons[0]
            .as_ref()
            .unwrap();
        assert_eq!(ak.ammo[0].2 + u32::from(ak.ammo_in_magazine), 60);
        use profile_slot_enum::*;
        let mut left = combat.leftovers(player);
        left.sort_by_key(|(slot, _)| *slot as u8);
        assert_eq!(
            left,
            [
                (ammo1_weapon1_slot, 60),
                (ammo2_weapon1_slot, 30),
                (ammo1_weapon2_slot, 10),
                (ammo2_weapon2_slot, 10),
                (quick_slot1, 2),
                (quick_slot2, 2),
            ]
        );
    }

    #[test]
    fn a_quick_slot_press_mends_a_broken_leg() {
        let Some(assets) = CombatAssets::for_tests() else {
            return;
        };
        let mut combat = Combat::new(assets);
        let profile = player_profile::new_dummy(0, 0, "fighter");
        let player = addr(1);
        let start = Instant::now();
        let feet = Vec3::from(crate::map::RESPAWN_POINTS[0].position);
        combat.spawn(
            player,
            7,
            0,
            1,
            &profile,
            profile_slot_enum::weapon1_slot,
            [(1, 2), (3, 4)],
            feet,
            0.0,
            start,
        );
        let now_ms = combat.now_ms(start) as u32;
        let fighter = combat.fighters.get_mut(&player).unwrap();
        let leg = fighter.damage.index_of("left_leg").unwrap();
        fighter
            .damage
            .hit_body_part(1, leg, "injury", 2.0, 0.0, now_ms, &mut None);
        let leg_damage = |applying| CombatEvent::LimbAffect {
            match_id: 7,
            player_id: 0,
            body_part: "left_leg".to_owned(),
            affect: affect::LEG_DAMAGE,
            applying,
        };
        assert_eq!(combat.tick(start, |_| true), [leg_damage(true)]);

        // Quick slot 1 holds the bandages in the dummy loadout.
        let (input, state) = update(feet, 0.0, 0x4000);
        combat.update(player, &input, &state, 0, None, start);
        assert_eq!(
            combat.tick(start + Duration::from_millis(1000), |_| true),
            [leg_damage(false)]
        );
        assert_eq!(
            combat.fighters[&player].drugs[0].as_ref().unwrap().amount,
            1
        );
        // Used up, for the account.
        let bandages = combat.fighters[&player].drugs[0].as_ref().unwrap().dict_id;
        assert_eq!(combat.take_used(player), [(bandages, 1)]);
        assert_eq!(combat.take_used(player), []);
    }

    #[test]
    fn releasing_the_trap_key_places_a_trap_where_the_player_looks() {
        let Some(assets) = CombatAssets::for_tests() else {
            return;
        };
        let mut combat = Combat::new(assets);
        let profile = player_profile::new_dummy(0, 0, "trapper");
        let player = addr(1);
        let start = Instant::now();
        // Team 1's base floor, open for 8 m toward -x (yaw a quarter turn).
        let feet = Vec3::new(-31.2, 0.504, -66.7);
        let yaw = std::f32::consts::FRAC_PI_2;
        combat.spawn(
            player,
            7,
            3,
            1,
            &profile,
            profile_slot_enum::weapon1_slot,
            [(1, 2), (3, 4)],
            feet,
            yaw,
            start,
        );
        // Quick slot 2 holds the traps: down, then up, looking at the floor
        // in reach.
        let looking_down = |mask| {
            let (input, mut state) = update(feet, yaw, mask);
            state.look_pitch = -0.9;
            (input, state)
        };
        let (input, state) = looking_down(0x10000);
        combat.update(player, &input, &state, 0, None, start);
        assert_eq!(combat.tick(start, |_| true), []);
        let (input, state) = looking_down(0x20000);
        let at = start + Duration::from_millis(16);
        combat.update(player, &input, &state, 16, None, at);
        let events = combat.tick(at, |_| true);
        let [
            CombatEvent::TrapPlaced {
                match_id: 7,
                player_id: 3,
                slot: 0x0E,
                index: 0,
                position,
                angles,
            },
        ] = events[..]
        else {
            panic!("{events:?}");
        };
        assert!((position.y - feet.y).abs() < 0.05, "{position:?}");
        let ahead = feet.x - position.x;
        assert!((0.3..1.2).contains(&ahead), "{position:?}");
        // Upright, facing away from the player.
        let rotation = Mat4::rotation_xyz(angles);
        assert!(rotation.j.y > 0.99 && rotation.k.x < -0.99, "{rotation:?}");
        assert_eq!(combat.take_used(player), [(68, 1)], "one trap used up");
        assert_eq!(
            combat.fighters[&player].traps[1].as_ref().unwrap().amount,
            1
        );

        // Looking at the same spot again: the trap there refuses another.
        let (input, state) = looking_down(0x10000);
        combat.update(player, &input, &state, 32, None, at);
        let (input, state) = looking_down(0x20000);
        combat.update(player, &input, &state, 48, None, at);
        assert_eq!(combat.tick(at, |_| true), []);

        // A respawn takes the placed traps away and refills the set.
        combat.spawn(
            player,
            7,
            3,
            1,
            &profile,
            profile_slot_enum::weapon1_slot,
            [(1, 2), (3, 4)],
            feet,
            yaw,
            at,
        );
        let set = combat.fighters[&player].traps[1].as_ref().unwrap();
        assert_eq!((set.amount, set.refusing_volumes().count()), (2, 0));
    }

    /// Player 3 sets a trap on team 1's base floor, a step ahead of it; the
    /// trap's events and where it went.
    fn set_a_trap(combat: &mut Combat, start: Instant) -> (Vec3, Vec<CombatEvent>) {
        let feet = Vec3::new(-31.2, 0.504, -66.7);
        let yaw = std::f32::consts::FRAC_PI_2;
        let profile = player_profile::new_dummy(0, 0, "trapper");
        combat.spawn(
            addr(1),
            7,
            3,
            1,
            &profile,
            profile_slot_enum::weapon1_slot,
            [(1, 2), (3, 4)],
            feet,
            yaw,
            start,
        );
        for (ms, mask) in [(0, 0x10000), (16, 0x20000)] {
            let (input, mut state) = update(feet, yaw, mask);
            state.look_pitch = -0.9;
            let at = start + Duration::from_millis(ms);
            combat.update(addr(1), &input, &state, ms as u32, None, at);
        }
        let events = combat.tick(start + Duration::from_millis(16), |_| true);
        let Some(&CombatEvent::TrapPlaced { position, .. }) = events.first() else {
            panic!("{events:?}");
        };
        (position, events)
    }

    /// Player `id` (team 1, or 2 for player 4) crouches behind the trap and
    /// looks down at it from `from_ms` to `to_ms`, holding use but for the
    /// frames `drops` names; the trap's changes.
    fn crouch_over_the_trap(
        combat: &mut Combat,
        start: Instant,
        trap: Vec3,
        (port, id): (u16, u8),
        (from_ms, to_ms): (u64, u64),
        drops: impl Fn(u64) -> bool,
    ) -> Vec<(u64, TrapChange)> {
        let yaw = std::f32::consts::FRAC_PI_2;
        let feet = Vec3::new(trap.x + 0.65, 0.504, trap.z);
        if !combat.fighters.contains_key(&addr(port)) {
            combat.spawn(
                addr(port),
                7,
                id,
                if id == 4 { 2 } else { 1 },
                &player_profile::new_dummy(0, 0, "defuser"),
                profile_slot_enum::weapon1_slot,
                [(5, 6), (7, 8)],
                feet,
                yaw,
                start + Duration::from_millis(from_ms),
            );
        }
        let mut changes = Vec::new();
        for ms in (from_ms..=to_ms).step_by(50) {
            // A second crouched still before use.
            let using = ms >= from_ms + 1000 && !drops(ms);
            let mask = bits::CROUCH | if using { USE_KEY } else { 0 };
            let (input, mut state) = update(feet, yaw, mask);
            state.look_pitch = -1.0;
            let at = start + Duration::from_millis(ms);
            combat.update(addr(port), &input, &state, ms as u32, None, at);
            for event in combat.tick(at, |_| true) {
                if let CombatEvent::TrapChanged { change, .. } = event {
                    changes.push((ms, change));
                }
            }
        }
        changes
    }

    #[test]
    fn an_enemy_defuses_a_trap_with_five_seconds_of_use() {
        let Some(assets) = CombatAssets::for_tests() else {
            return;
        };
        let mut combat = Combat::new(assets);
        let start = Instant::now();
        let (trap, _) = set_a_trap(&mut combat, start);
        // A team mate of its owner can hold use as long as it likes.
        let changes =
            crouch_over_the_trap(&mut combat, start, trap, (3, 5), (100, 7100), |_| false);
        assert_eq!(changes, []);
        assert!(combat.fighters[&addr(3)].defusing.is_none());
        combat.remove(addr(3));

        // An enemy needs 5 s from its first frame of use (8100 ms), through
        // the client's drops of the bit.
        let changes = crouch_over_the_trap(&mut combat, start, trap, (2, 4), (7100, 13500), |ms| {
            (10_000..10_150).contains(&ms)
        });
        assert_eq!(
            changes,
            [(13_100, TrapChange::Disarmed)],
            "{:?}",
            combat.fighters[&addr(2)].defusing
        );
        // It is gone 3 s later; the defuser's use ended with it.
        assert!(combat.fighters[&addr(2)].defusing.is_none());
        let events = combat.tick(start + Duration::from_millis(16_100), |_| true);
        assert!(
            events.iter().any(|event| matches!(
                event,
                CombatEvent::TrapChanged {
                    change: TrapChange::Removed,
                    ..
                }
            )),
            "{events:?}"
        );
    }

    #[test]
    fn letting_go_of_use_starts_the_defusing_over() {
        let Some(assets) = CombatAssets::for_tests() else {
            return;
        };
        let mut combat = Combat::new(assets);
        let start = Instant::now();
        let (trap, _) = set_a_trap(&mut combat, start);
        // Use from 1100 ms, let go for 300 ms at 4000: 5 s from 4300.
        let changes = crouch_over_the_trap(&mut combat, start, trap, (2, 4), (100, 9500), |ms| {
            (4000..4300).contains(&ms)
        });
        assert_eq!(changes, [(9300, TrapChange::Disarmed)]);
    }

    #[test]
    fn a_bullet_disarms_the_trap_it_hits() {
        let Some(assets) = CombatAssets::for_tests() else {
            return;
        };
        let mut combat = Combat::new(assets);
        let start = Instant::now();
        let (trap, _) = set_a_trap(&mut combat, start);
        let spec = BulletSpec {
            shooter: 3,
            weapon_dict_id: 13,
            damage: 0.4,
            pierce: 0.5,
            ricochet_angle_degrees: 30.0,
            air_resistance: 1.0,
            distance_coef: 1.0,
            rewind_ms: 0.0,
        };
        // Straight down onto it from a metre up.
        let born = combat.now_ms(start + Duration::from_millis(100));
        let bullet = Bullet::new(
            spec,
            trap + Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, -700.0, 0.0),
            born,
        );
        combat.bullets.push((7, bullet));
        let events = combat.tick(start + Duration::from_millis(200), |_| true);
        assert_eq!(
            events,
            [CombatEvent::TrapChanged {
                match_id: 7,
                player_id: 3,
                slot: 0x0E,
                index: 0,
                change: TrapChange::Disarmed,
            }]
        );
    }

    #[test]
    fn stepping_on_a_trap_hurts_the_feet_and_fires_it() {
        let Some(assets) = CombatAssets::for_tests() else {
            return;
        };
        let mut combat = Combat::new(assets);
        let start = Instant::now();
        let (trap, _) = set_a_trap(&mut combat, start);
        // Player 4 waits 3 m off, then walks onto it.
        let victim = addr(2);
        let off = trap + Vec3::new(-3.0, 0.0, 0.0);
        combat.spawn(
            victim,
            7,
            4,
            2,
            &player_profile::new_dummy(0, 0, "victim"),
            profile_slot_enum::weapon1_slot,
            [(5, 6), (7, 8)],
            off,
            0.0,
            start,
        );
        let at = start + Duration::from_millis(100);
        let (input, state) = update(off, 0.0, 0);
        combat.update(victim, &input, &state, 100, None, at);
        assert_eq!(combat.tick(at, |_| true), [], "nobody near the trap");

        let at = start + Duration::from_millis(200);
        let on = Vec3::new(trap.x, 0.504, trap.z);
        let (input, state) = update(on, 0.0, 0);
        combat.update(victim, &input, &state, 200, None, at);
        let events = combat.tick(at, |_| true);
        let hits: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                CombatEvent::Hit {
                    shooter: 3,
                    victim: 4,
                    body_part,
                    hit_type,
                    amount,
                    armor_piercing,
                    ..
                } => Some((
                    body_part.as_str(),
                    hit_type.as_str(),
                    *amount,
                    *armor_piercing,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            hits,
            [
                ("right_foot", "injury", 1.0, 1.0),
                ("left_foot", "injury", 1.0, 1.0),
                ("pain", "injury", 1.5, 1.0),
            ],
            "{events:?}"
        );
        let changed = |change| CombatEvent::TrapChanged {
            match_id: 7,
            player_id: 3,
            slot: 0x0E,
            index: 0,
            change,
        };
        assert_eq!(
            events.last(),
            Some(&changed(TrapChange::Fired)),
            "{events:?}"
        );
        // A client joining now sees it fired where it was set.
        let [report] = combat.traps_out(7)[..] else {
            panic!("{:?}", combat.traps_out(7));
        };
        assert_eq!(
            (report.player_id, report.slot, report.index, report.state),
            (3, 0x0E, 0, 2)
        );
        assert_eq!(report.position, trap);
        assert!(combat.traps_out(8).is_empty(), "another match's");

        // A fired trap catches nobody else; it is gone 3 s later.
        let mut trap_events = |ms| {
            let events = combat.tick(start + Duration::from_millis(ms), |_| true);
            events
                .into_iter()
                .filter(|event| !matches!(event, CombatEvent::LimbAffect { .. }))
                .collect::<Vec<_>>()
        };
        assert_eq!(trap_events(3100), []);
        assert_eq!(trap_events(3200), [changed(TrapChange::Removed)]);
    }

    #[test]
    fn lag_compensation_hits_the_victim_where_the_shooter_saw_it() {
        if CombatAssets::for_tests().is_none() {
            return;
        }
        let rewound = shoot_at_a_moving_target(Some(Duration::from_millis(250)));
        assert!(
            rewound.iter().any(|event| matches!(
                event,
                CombatEvent::Hit {
                    shooter: 0,
                    victim: 1,
                    ..
                }
            )),
            "{rewound:?}"
        );
        let present = shoot_at_a_moving_target(None);
        assert!(
            !present
                .iter()
                .any(|event| matches!(event, CombatEvent::Hit { .. })),
            "{present:?}"
        );
    }
}
