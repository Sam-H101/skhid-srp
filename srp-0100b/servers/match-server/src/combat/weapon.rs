// A player's weapons, replayed from their input the way the client runs them,
// so the server fires the same shots at the same moments with the same random
// spread. Sources (`vostok/game_core/sources` unless noted): `weapon_core.cpp`
// and its state classes, `weapon_user_animations_selector.cpp`,
// `player_stamina.cpp`, and the frame loop in `game/sources/player_tick.cpp`.
//
// The client paces a weapon by its clips: a shot leaves at the fire clip's
// `shoot` mark, a reload or a cycled bolt completes at its clip's end, a weapon
// shows and hides in its clip's length. Its logic runs only at "selections":
// at the previous frame's time when a frame's input differs from the last one
// (or a selection is forced), and at the exact millisecond of each shot mark
// and of each clip end a state waits for. The replay keeps those moments on
// the client's own frame times, with the clips' lengths and marks from the
// game data, in single precision as the client computes them.
//
// The camera the bullets leave also sways while aimed (the breath vibration,
// `Breath`): two sines of the client's clock added to the recoil clips.
//
// Not modelled: physics the server does not run (`can_jump`, `can_stand` and
// `can_crouch` hold; a moving jump lasts its jump and landing clips without
// the movement lead-in) and the third-person camera (first-person clips).

use std::sync::Arc;

use survarium::player_input::player_stamina;

use super::data::{
    AmmoData, BreathParams, CharacterDispersion, RecoilParams, StaminaParams, StateClips,
    WeaponData,
};
use super::pose::ClipTiming;

pub mod bits {
    pub const FORWARD: u32 = 0x1;
    pub const BACK: u32 = 0x2;
    pub const LEFT: u32 = 0x4;
    pub const RIGHT: u32 = 0x8;
    pub const JUMP: u32 = 0x10;
    pub const FIRE: u32 = 0x20;
    pub const RELOAD: u32 = 0x40;
    pub const AIM: u32 = 0x80;
    pub const CROUCH: u32 = 0x100;
    pub const SPRINT: u32 = 0x200;
    pub const FIRE_MODE: u32 = 0x400;
    pub const AMMO_TYPE: u32 = 0x800;
    pub const WEAPON_1: u32 = 0x1000;
    pub const WEAPON_2: u32 = 0x2000;
    /// With `AIM`, holds the breath (left shift while aiming).
    pub const HOLD_BREATH: u32 = 0x0800_0000;
}

const AUTOMATIC: u8 = 0xFF;
/// Clip frames per second (`default_fps`).
const FPS: f32 = 30.0;
/// Every weight transition here lasts this long (`linear_interpolator( 0.3f )`):
/// the sprint clip's fade-out, and the time a replaced clip still lends its
/// phase to a new clip of its time-sync group.
const FADE_MS: u32 = 300;
/// Bounds the clip events one frame can raise, should a client's clock jump.
const MAX_EVENTS_PER_FRAME: usize = 4096;
/// `animation_player::skip_time_if_needed`: clips this far behind move on to
/// the new time without raising the events in between.
const SKIP_LAG_MS: u32 = 10_000;
/// `player_input::is_sprinting`: back, sides, fire, reload and crouch stop a
/// sprint.
const NOT_WHILE_SPRINTING: u32 = 0x16E;

/// `math::random32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Random32(pub u32);

impl Random32 {
    pub fn random(&mut self, range: u32) -> u32 {
        self.0 = self.0.wrapping_mul(0x0808_8405).wrapping_add(1);
        ((u64::from(self.0) * u64::from(range)) >> 32) as u32
    }

    pub fn random_f(&mut self, range: f32) -> f32 {
        range * (self.random(1024 * 1024) as f32 / (1024.0 * 1024.0))
    }
}

/// `survarium::normal_random`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NormalRandom(pub i32);

impl NormalRandom {
    fn rand_i(&mut self) -> i32 {
        self.0 = self.0.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        (self.0 >> 16) & 0x7fff
    }

    fn rand_f(&mut self) -> f32 {
        self.rand_i() as f32 / 32767.0
    }

    pub fn rand_n(&mut self, sigma: f32) -> f32 {
        const ONE_OVER_SIGMA_EXP: f32 = 1.253_918_5;
        if sigma == 0.0 {
            return 0.0;
        }
        let mut y;
        loop {
            y = -self.rand_f().ln();
            if self.rand_f() <= (-(y - 1.0) * (y - 1.0) * 0.5).exp() {
                break;
            }
        }
        if self.rand_i() & 1 != 0 {
            y * sigma * ONE_OVER_SIGMA_EXP
        } else {
            -y * sigma * ONE_OVER_SIGMA_EXP
        }
    }
}

/// The selector's states (`weapon_user_state_enum`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    Stand,
    Crouch,
    Sprint,
    Jump,
}

/// `player_input::is_sprinting`.
fn is_sprinting(mask: u32) -> bool {
    mask & bits::SPRINT != 0 && mask & bits::FORWARD != 0 && mask & NOT_WHILE_SPRINTING == 0
}

/// `weapon_core::update_bones_matrices` compares the masked bits themselves,
/// so any movement key counts, even two opposite ones.
pub fn is_moving(mask: u32) -> bool {
    mask & (bits::FORWARD | bits::BACK | bits::LEFT | bits::RIGHT) != 0
}

/// What the damage model says about the player's limbs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Injuries {
    pub broken_hands: u8,
    pub broken_legs: u8,
}

// ---------------------------------------------------------------- stamina

/// `player_stamina`, ticked once per frame.
#[derive(Clone, Debug)]
pub struct Stamina {
    params: StaminaParams,
    value: f32,
    last_spending_ms: u32,
    last_tick_ms: u32,
    lower_threshold_reached: bool,
}

/// Regeneration starts this long after the last spending.
const REGENERATION_DELAY_MS: u32 = 1000;

impl Stamina {
    pub fn new(params: StaminaParams) -> Self {
        Self {
            params,
            value: params.max_value,
            last_spending_ms: 0,
            last_tick_ms: 0,
            lower_threshold_reached: false,
        }
    }

    /// Returns whether it ran out.
    fn tick(&mut self, now_ms: u32, sprinting: bool) -> bool {
        let mut depleted = false;
        if sprinting {
            // The client ticks from the start of play; a first tick here has
            // no previous frame to measure from.
            let dt = if self.last_tick_ms == 0 {
                0.0
            } else {
                now_ms.wrapping_sub(self.last_tick_ms) as f32 / 1000.0
            };
            depleted = self.decrease(self.params.spending_speed * dt);
            self.last_spending_ms = now_ms;
        }
        if self.params.max_value > self.value
            && (self.value == 0.0
                || (self.last_spending_ms != 0
                    && self.last_spending_ms.wrapping_add(REGENERATION_DELAY_MS) < now_ms))
            && self.last_tick_ms != 0
        {
            let dt = now_ms.wrapping_sub(self.last_tick_ms) as f32 / 1000.0;
            self.increase(dt * self.params.regeneration_speed);
        }
        self.last_tick_ms = now_ms;
        depleted
    }

    fn increase(&mut self, amount: f32) {
        self.value = (self.value + amount).clamp(0.0, self.params.max_value);
        if self.value > self.params.spending_threshold {
            self.lower_threshold_reached = false;
        }
    }

    fn decrease(&mut self, amount: f32) -> bool {
        self.value = (self.value - amount).clamp(0.0, self.params.max_value);
        if self.params.regeneration_threshold >= self.value {
            self.lower_threshold_reached = true;
            return true;
        }
        false
    }

    fn spend(&mut self, amount: f32) {
        self.decrease(amount);
        self.last_spending_ms = self.last_tick_ms;
    }

    fn can_be_spent(&self) -> bool {
        self.params.spending_threshold < self.value || !self.lower_threshold_reached
    }

    fn amount_to_jump(&self) -> f32 {
        self.params.max_value / 5.0
    }

    #[cfg(test)]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// As `player_stamina::deserialize` reads it, its times on the player's
    /// own clock (0: never).
    pub fn snapshot(&self) -> player_stamina {
        player_stamina {
            value: self.value,
            last_spending_time_in_ms: self.last_spending_ms,
            last_tick_time_in_ms: self.last_tick_ms,
            lower_threshold_was_reached: self.lower_threshold_reached,
        }
    }
}

// ---------------------------------------------------------------- jumps

/// The jump clips every weapon uses (`weapons/ak/player/.../jump`, first
/// person): (start clip frames, frame of its `jump` mark, landing clip frames).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JumpKind {
    OnSite,
    /// Forward, back and the diagonals.
    Lengthwise,
    Sideways,
}

impl JumpKind {
    fn from_mask(mask: u32) -> Self {
        let lengthwise = (mask & bits::FORWARD != 0) != (mask & bits::BACK != 0);
        let sideways = (mask & bits::LEFT != 0) != (mask & bits::RIGHT != 0);
        if lengthwise {
            Self::Lengthwise
        } else if sideways {
            Self::Sideways
        } else {
            Self::OnSite
        }
    }

    fn frames(self) -> (f32, f32, f32) {
        match self {
            Self::OnSite => (24.0, 7.0, 12.0),
            Self::Lengthwise => (23.0, 6.0, 13.0),
            Self::Sideways => (20.0, 2.0, 6.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JumpStage {
    Inactive,
    Start,
    Landing,
}

/// `jump_logic`: the start clip (its `jump` mark spends stamina), then the
/// landing clip; the jump is over when the landing clip ends.
#[derive(Clone, Copy, Debug)]
struct Jump {
    kind: JumpKind,
    stage: JumpStage,
    anchor_ms: u32,
    mark_done: bool,
    start_ended: bool,
    finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JumpEvent {
    Mark,
    StartEnded,
    Landed,
}

impl Jump {
    fn next_event(&self) -> Option<(u32, JumpEvent)> {
        let (start, mark, landing) = self.kind.frames();
        let at = |frames: f32| clip_ms(self.anchor_ms, 0.0, frames / FPS, 1.0);
        match self.stage {
            JumpStage::Start if !self.mark_done => Some((at(mark), JumpEvent::Mark)),
            JumpStage::Start if !self.start_ended => Some((at(start), JumpEvent::StartEnded)),
            JumpStage::Landing if !self.finished => Some((at(landing), JumpEvent::Landed)),
            _ => None,
        }
    }
}

/// `weapon_user_animations_selector`: stand, crouch, sprint or jump.
#[derive(Clone, Debug)]
struct Selector {
    stance: Stance,
    forced_not_to_sprint: bool,
    jump: Option<Jump>,
}

impl Default for Selector {
    fn default() -> Self {
        Self {
            stance: Stance::Stand,
            forced_not_to_sprint: false,
            jump: None,
        }
    }
}

impl Selector {
    /// `activate` / `deactivate` restart the logic but keep the flag, which
    /// only releasing the sprint key clears.
    fn deactivate(&mut self) {
        self.stance = Stance::Stand;
        self.jump = None;
    }
}

// ---------------------------------------------------------------- dispersion

/// `weapon_dispersion_calculator` + `character_dispersion_calculator`.
#[derive(Clone, Debug)]
struct Dispersion {
    weapon_target: f32,
    weapon_current: f32,
    weapon_last_ms: Option<u32>,
    character_target: f32,
    character_current: f32,
    character_value: f32,
    character_last_ms: Option<u32>,
}

impl Dispersion {
    fn new(character: &CharacterDispersion) -> Self {
        Self {
            weapon_target: 0.0,
            weapon_current: 0.0,
            weapon_last_ms: None,
            character_target: character.idle,
            character_current: character.idle,
            character_value: character.idle,
            character_last_ms: None,
        }
    }

    /// `set_character_dispersion_params`, on every activation.
    fn reset_character(&mut self, character: &CharacterDispersion) {
        self.character_target = character.idle;
        self.character_current = character.idle;
        self.character_value = character.idle;
    }

    #[allow(clippy::too_many_arguments)]
    fn tick(
        &mut self,
        now_ms: u32,
        weapon: &WeaponData,
        character: &CharacterDispersion,
        stance: Stance,
        moving: bool,
        aiming: bool,
        broken_hands: u8,
    ) {
        let aiming_speed = weapon.dispersion.speed_of_aiming;

        // Weapon part: grows at 5/s toward its target, which decays at the
        // aiming speed (`m_max_value` 1).
        match self.weapon_last_ms {
            None => self.weapon_last_ms = Some(now_ms),
            Some(last) if last < now_ms => {
                self.weapon_last_ms = Some(now_ms);
                let dt = (now_ms - last) as f32 * 0.001;
                self.weapon_target = (self.weapon_target - aiming_speed * dt).max(0.0);
                if self.weapon_current > self.weapon_target {
                    self.weapon_current =
                        (self.weapon_current - aiming_speed * dt).max(self.weapon_target);
                } else if self.weapon_current < self.weapon_target {
                    self.weapon_current = (self.weapon_current + 5.0 * dt).min(self.weapon_target);
                }
            }
            Some(_) => {}
        }

        // Character part.
        let Some(last) = self.character_last_ms else {
            self.character_last_ms = Some(now_ms);
            return;
        };
        if last >= now_ms {
            return;
        }
        self.character_last_ms = Some(now_ms);
        let dt = (now_ms - last) as f32 / 1000.0;
        let target = match stance {
            Stance::Stand if moving => {
                if aiming {
                    character.walk_aim
                } else {
                    character.walk
                }
            }
            Stance::Stand => {
                if aiming {
                    character.idle_aim
                } else {
                    character.idle
                }
            }
            Stance::Crouch if moving => {
                if aiming {
                    character.crouch_walk_aim
                } else {
                    character.crouch_walk
                }
            }
            Stance::Crouch => {
                if aiming {
                    character.crouch_aim
                } else {
                    character.crouch
                }
            }
            Stance::Sprint => character.run,
            Stance::Jump => character.jump,
        };
        let penalty = match (broken_hands, weapon.double_handed) {
            (0, _) | (1, false) => 1.0,
            (_, true) => character.injury_penalty_double_handed,
            (_, false) => character.injury_penalty_one_handed,
        };
        self.character_target = target * penalty;
        self.character_current = self
            .character_target
            .max(self.character_current - aiming_speed * dt);
        if self.character_value > self.character_current {
            self.character_value = (self.character_value - 5.0 * dt).max(self.character_current);
        } else if self.character_current > self.character_value {
            self.character_value = (self.character_value + 5.0 * dt).min(self.character_current);
        }
    }

    fn reload(&mut self, weapon: &WeaponData) {
        self.weapon_target = (self.weapon_target + weapon.dispersion.reload_amount).min(1.0);
    }

    /// `dispersion_calculator::get_dispersion`, in degrees.
    fn degrees(&self, weapon: &WeaponData, ammo: &AmmoData, aimed: bool) -> f32 {
        let multiplier = if aimed {
            weapon.dispersion.aim_multiplier
        } else {
            weapon.dispersion.from_the_hip_multiplier
        };
        weapon.dispersion.base * ammo.dispersion * multiplier
            + (self.weapon_current + self.character_value)
    }
}

// ---------------------------------------------------------------- recoil

/// `weapon_recoil_calculator`: shots kick the camera through the recoil
/// clips. Its random generator starts at 0 with the weapon and is never
/// sent, so it is kept for the player's whole stay. The kick settles and
/// compensates back to rest on every tick; only the second, additive kick
/// waits `additive_recoil_time` after each shot.
#[derive(Clone, Debug)]
pub struct Recoil {
    random: Random32,
    time_since_change: f32,
    additive_timer: f32,
    vertical: f32,
    horizontal: f32,
    back: f32,
    target_vertical: f32,
    target_horizontal: f32,
    target_back: f32,
    last_ms: Option<u32>,
}

/// `m_interpolator( 0.1f )`, a linear interpolator.
const RECOIL_TRANSITION: f32 = 0.1;
const EPSILON_7: f32 = 1e-7;

impl Default for Recoil {
    fn default() -> Self {
        Self {
            random: Random32(0),
            time_since_change: 0.0,
            additive_timer: 0.0,
            vertical: 0.0,
            horizontal: 0.0,
            back: 0.0,
            target_vertical: 0.0,
            target_horizontal: 0.0,
            target_back: 0.0,
            last_ms: None,
        }
    }
}

impl Recoil {
    fn random_amount(&mut self) -> f32 {
        self.random.random_f(1.0).max(0.25)
    }

    fn normalize_target(&mut self) {
        let total = self.target_vertical * self.target_vertical
            + self.target_horizontal * self.target_horizontal;
        if total > 1.0 {
            let total = total.sqrt();
            self.target_vertical /= total;
            self.target_horizontal /= total;
        }
    }

    /// Run by the recoil clips' time calculators, so only while they play
    /// (standing or crouching).
    pub fn tick(&mut self, now_ms: u32, params: &RecoilParams) {
        let Some(last) = self.last_ms else {
            self.last_ms = Some(now_ms);
            return;
        };
        if last >= now_ms {
            return;
        }
        self.last_ms = Some(now_ms);
        let dt = (now_ms - last) as f32 * 0.001;
        self.time_since_change = (self.time_since_change + dt).min(RECOIL_TRANSITION);
        // Only the additive kick waits on its timer; the camera settles and
        // compensates on every tick (survarium.exe 0x58E8F5 skips just this).
        if self.additive_timer == 0.0 {
        } else if dt < self.additive_timer {
            self.additive_timer -= dt;
        } else {
            let angle = (params.additive_recoil_min_angle
                + self.random.random_f(params.additive_recoil_angle_range))
            .to_radians();
            let force = self.random_amount();
            let amount = force * params.additive_side_recoil;
            self.target_vertical = self.vertical + angle.cos() * amount;
            self.target_horizontal = self.horizontal + angle.sin() * amount;
            self.normalize_target();
            self.target_back = (self.target_back - params.additive_back_recoil * force).max(0.0);
            self.additive_timer = 0.0;
            self.back = (self.back - amount).max(0.0);
        }
        let t = self.time_since_change / RECOIL_TRANSITION;
        self.vertical = self.target_vertical * t + self.vertical * (1.0 - t);
        self.horizontal = self.target_horizontal * t + self.horizontal * (1.0 - t);
        self.back = self.target_back * t + self.back * (1.0 - t);

        // `process_compensation`.
        let side = (params.side_compensation_speed
            + (self.target_vertical * self.target_vertical
                + self.target_horizontal * self.target_horizontal)
                .sqrt())
            * dt;
        let compensate = |value: f32| {
            if value.abs() > side {
                value - value.signum() * side
            } else {
                0.0
            }
        };
        self.target_vertical = compensate(self.target_vertical);
        self.target_horizontal = compensate(self.target_horizontal);
        let back = (params.back_compensation_speed
            + (2.0 * self.target_back * self.target_back).sqrt())
            * dt;
        self.target_back = if back < self.target_back {
            self.target_back - back
        } else {
            0.0
        };
    }

    pub fn fire(&mut self, params: &RecoilParams) {
        let angle = (params.shoot_recoil_min_angle
            + self.random.random_f(params.shoot_recoil_angle_range))
        .to_radians();
        let force = self.random_amount();
        let first = self.target_vertical == 0.0 && self.target_horizontal == 0.0;
        let side = force
            * if first {
                params.first_shoot_side_recoil
            } else {
                params.shoot_side_recoil
            };
        self.target_vertical = self.vertical + angle.cos() * side;
        self.target_horizontal = self.horizontal + angle.sin() * side;
        self.normalize_target();
        let back = force
            * if first {
                params.first_shoot_back_recoil
            } else {
                params.shoot_back_recoil
            };
        self.target_back = (self.target_back + back).min(1.0);
        self.time_since_change = 0.0;
        self.additive_timer = params.additive_recoil_time;
    }

    /// After a reload or a chambered round.
    pub fn reset(&mut self) {
        self.time_since_change = 0.0;
        self.additive_timer = 0.0;
        self.target_vertical = 0.0;
        self.target_horizontal = 0.0;
    }

    /// The recoil clip time fractions (horizontal, vertical, back) without
    /// the breath sway.
    pub fn clip_times(&self) -> (f32, f32, f32) {
        self.swayed_clip_times((0.0, 0.0))
    }

    /// The recoil clip time fractions as `weapon_core::horizontal_recoil_value`
    /// (0x0BC660) and `vertical_recoil_value` (0x0BC710) compute them: the
    /// breath sway (horizontal, vertical; zero unless aimed) adds to the
    /// coefficients before the clamp. The back clip has none.
    pub fn swayed_clip_times(&self, (horizontal, vertical): (f32, f32)) -> (f32, f32, f32) {
        let half = 0.5 - EPSILON_7;
        (
            0.5 - (self.horizontal + horizontal).clamp(-half, half),
            (self.vertical + vertical).clamp(-half, half) + 0.5,
            self.back.clamp(EPSILON_7, 1.0 - EPSILON_7),
        )
    }
}

/// The recoil clip times of a camera at rest.
pub const RESTING_RECOIL: (f32, f32, f32) = (0.5, 0.5, EPSILON_7);

// ---------------------------------------------------------------- breath

/// `breath_vibration_calculator` (weapon_core+0x374): the aim sway. Two
/// sines of the client's clock, the vertical one twice as fast, scaled by a
/// multiplier that holding the breath lowers for a while and running out of
/// it raises. There is no randomness, and nothing of it is sent: the phase
/// is the frame time itself, the rest follows from each frame's input.
#[derive(Clone, Debug, PartialEq)]
pub struct Breath {
    state: BreathState,
    /// +0x1C: the last tick's client time.
    last_ms: u32,
    /// +0x20.
    multiplier: f32,
    /// +0x34: seconds of breath left.
    breath: f32,
    /// (horizontal +0x2C, vertical +0x28), in recoil coefficient units.
    sway: (f32, f32),
}

/// `breath_state_normal`, `_holding`, `_shortbreathing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BreathState {
    Normal,
    Holding,
    Short,
}

impl Default for Breath {
    fn default() -> Self {
        Self {
            state: BreathState::Normal,
            last_ms: 0,
            multiplier: 1.0,
            breath: 0.0,
            sway: (0.0, 0.0),
        }
    }
}

/// `ms * factor` as the x87 at 24-bit precision computes it from an
/// integer count of milliseconds (`fild`, `fmul dword`): one rounding of
/// the exact product.
fn ms_times(ms: u32, factor: f32) -> f32 {
    let bits = factor.to_bits();
    let mantissa = u64::from(bits & 0x7F_FFFF | 0x80_0000);
    let exponent = ((bits >> 23) & 0xFF) as i32 - 150;
    let scale = f32::from_bits(((exponent + 127) as u32) << 23);
    (u64::from(ms) * mantissa) as f32 * scale
}

impl Breath {
    /// `set_breath_holding_params` (0x5934D0), from `weapon_core::activate`:
    /// a full breath, normal breathing, multiplier 1. The last tick's time
    /// stays.
    fn reset(&mut self, params: &BreathParams) {
        self.state = BreathState::Normal;
        self.breath = params.max_breath_holding_time;
        self.multiplier = 1.0;
    }

    /// The recoil clips' time calculators run it (`computed_vertical_recoil_time`
    /// 0x5A3D47, `computed_horizontal_recoil_time` 0x5A3E07): the hold flag
    /// is AIM with HOLD_BREATH in the frame's input, not `is_aimed`. Each of
    /// the two ticks it, so the second, at the same time, only takes a
    /// transition the first one's breath now allows.
    pub fn tick(&mut self, now_ms: u32, mask: u32, params: &BreathParams) {
        let hold = mask & bits::AIM != 0 && mask & bits::HOLD_BREATH != 0;
        self.update(now_ms, hold, params);
        self.update(now_ms, hold, params);
    }

    /// `breath_vibration_calculator::tick(T, 1.0)` (0x5935F0).
    fn update(&mut self, now_ms: u32, hold: bool, params: &BreathParams) {
        if now_ms < self.last_ms {
            return;
        }
        let dt = ms_times(now_ms - self.last_ms, 0.001);
        self.last_ms = now_ms;
        // `fsm::tick` (0x5794F0): the state's first transition that holds.
        // Short breathing ends only on a full breath (`is_ready_for_transition`).
        let max = params.max_breath_holding_time;
        self.state = match self.state {
            BreathState::Normal if hold => BreathState::Holding,
            BreathState::Holding if !hold => BreathState::Normal,
            BreathState::Holding if self.breath <= 0.0 => BreathState::Short,
            BreathState::Short if self.breath == max => BreathState::Normal,
            state => state,
        };
        let target = match self.state {
            BreathState::Normal => {
                self.breath = max.min(self.breath + dt);
                1.0
            }
            BreathState::Holding => {
                self.breath = (self.breath - dt).max(0.0);
                params.breath_holding_multiplier
            }
            BreathState::Short => {
                let rate = max / params.shortbreathing_repair_time;
                self.breath = max.min(rate * dt + self.breath);
                params.shortbreathing_multiplier
            }
        };
        self.multiplier = if self.multiplier > target {
            (self.multiplier - params.multiplier_decrease_speed * dt).max(target)
        } else {
            target.min(params.multiplier_increase_speed * dt + self.multiplier)
        };
        // The phase is the absolute client time (`player::local_time` is
        // the identity), in single precision.
        let t = ms_times(now_ms, 0.001) * std::f32::consts::TAU;
        self.sway = (
            (t / params.horizontal_period).sin() * params.horizontal_amplitude * self.multiplier,
            (t / params.vertical_period).sin() * params.vertical_amplitude * self.multiplier,
        );
    }

    pub fn multiplier(&self) -> f32 {
        self.multiplier
    }

    /// (horizontal, vertical), in recoil coefficient units.
    pub fn sway(&self) -> (f32, f32) {
        self.sway
    }
}

/// The breath sway a frame drew: added to the recoil coefficients only when
/// the weapon was aimed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawnSway {
    pub aimed: bool,
    pub horizontal: f32,
    pub vertical: f32,
    pub multiplier: f32,
}

impl DrawnSway {
    pub const NONE: Self = Self {
        aimed: false,
        horizontal: 0.0,
        vertical: 0.0,
        multiplier: 1.0,
    };
}

// ---------------------------------------------------------------- clips

/// `n_ary_tree_time_in_ms_calculator`: the millisecond at which a clip
/// playing clip time `from` (seconds) at `anchor_ms` reaches clip time `to`.
fn clip_ms(anchor_ms: u32, from: f32, to: f32, time_scale: f32) -> u32 {
    let elapsed = ((to - from) * 1000.0 / time_scale).floor();
    anchor_ms.wrapping_add(if elapsed > 0.0 { elapsed as u32 } else { 0 })
}

/// A logic state's clips and the time scale its cook gives them.
#[derive(Clone, Debug)]
struct StateTiming {
    state_type: i32,
    clips: Vec<Arc<ClipTiming>>,
    time_scale: f32,
}

impl StateTiming {
    fn new(state: &StateClips, time_scale: impl Fn(&StateClips) -> f32) -> Option<Self> {
        if state.animations.is_empty() {
            return None;
        }
        let time_scale = time_scale(state);
        // Every pass of a clip takes at least a millisecond, so a looping one
        // cannot raise its end twice at the same time.
        let playable = time_scale.is_finite()
            && time_scale > 0.0
            && state
                .animations
                .iter()
                .all(|clip| event_seconds(clip) * 1000.0 / time_scale >= 1.0);
        playable.then(|| Self {
            state_type: state.state_type,
            clips: state.animations.clone(),
            time_scale,
        })
    }

    fn clip(&self, index: usize) -> &Arc<ClipTiming> {
        self.clips.get(index).unwrap_or(&self.clips[0])
    }
}

/// A clip's length as the time scales take it (`length_in_frames() * (1.f / 30.f)`).
fn seconds(clip: &ClipTiming) -> f32 {
    clip.length_frames * (1.0 / FPS)
}

/// A clip's length as its events take it (`length_in_frames() / default_fps`),
/// which can round 1 ms differently.
fn event_seconds(clip: &ClipTiming) -> f32 {
    clip.length_frames / FPS
}

/// `weapon_core_show_state` and the hide state: the weapon clip lasts as long
/// as the hands clip.
fn show_time_scale(state: &StateClips) -> f32 {
    match state.user_animations.first() {
        Some(hands) if hands.length_frames > 0.0 => {
            state.animations[0].length_frames / hands.length_frames
        }
        _ => 1.0,
    }
}

/// `computed_reload_animation_time_scale`.
fn reload_time_scale(reload_time: f32) -> impl Fn(&StateClips) -> f32 {
    move |state| seconds(&state.animations[0]) / reload_time
}

/// `computed_shooting_animation_time_scale`: one clip pass per `shoot` mark,
/// at `rounds_per_minute`.
fn shooting_time_scale(rounds_per_minute: f32) -> impl Fn(&StateClips) -> f32 {
    move |state| {
        let clip = &state.animations[0];
        let shots = clip.marks("shoot", 7).len() as f32;
        (rounds_per_minute / 60.0) / (shots / seconds(clip))
    }
}

#[derive(Clone, Debug)]
enum ReloadClips {
    /// The magazine (304), pistol (324) and double-barrel (317) reloads.
    Whole(StateTiming),
    /// The shotgun reload (310): start, one round per loop, finish.
    Rounds([StateTiming; 3]),
}

/// Every logic state's clips, from the weapon's config.
#[derive(Clone, Debug)]
pub struct WeaponClips {
    show: StateTiming,
    hide: StateTiming,
    fire: StateTiming,
    aimed_fire: StateTiming,
    reload: ReloadClips,
    chamber: Option<StateTiming>,
    chamber_aimed: Option<StateTiming>,
}

const SHOTGUN_RELOAD: i32 = 310;

impl WeaponClips {
    pub fn new(data: &WeaponData) -> Option<Self> {
        let state = |name: &str| data.states.get(name);
        let fixed = |_: &StateClips| 1.0;
        let show = StateTiming::new(state("show")?, show_time_scale)?;
        let hide = StateTiming::new(state("hide")?, show_time_scale)?;
        let fire = StateTiming::new(state("fire")?, shooting_time_scale(data.rounds_per_minute))?;
        let aimed_fire = state("aimed_fire")
            .and_then(|s| StateTiming::new(s, shooting_time_scale(data.rounds_per_minute)))
            .unwrap_or_else(|| fire.clone());
        let reload = state("reload")?;
        let reload = if reload.state_type == SHOTGUN_RELOAD {
            let step = |name: &str| reload.substates.get(name);
            let one_round = step("reload_one_substate")?;
            let capacity = f32::from(data.magazine_capacity);
            ReloadClips::Rounds([
                StateTiming::new(step("start_substate")?, fixed)?,
                // `computed_shotgun_reload_animation_time_scale`.
                StateTiming::new(one_round, |s| {
                    seconds(&s.animations[0]) * capacity / data.reload_time
                })?,
                StateTiming::new(step("finish_substate")?, fixed)?,
            ])
        } else {
            ReloadClips::Whole(StateTiming::new(
                reload,
                reload_time_scale(data.reload_time),
            )?)
        };
        Some(Self {
            show,
            hide,
            fire,
            aimed_fire,
            reload,
            chamber: state("chamber_a_round").and_then(|s| StateTiming::new(s, fixed)),
            chamber_aimed: state("chamber_a_round_aimed").and_then(|s| StateTiming::new(s, fixed)),
        })
    }
}

/// The first-person clip a state plays (`get_weapon_lexeme_pair` per class):
/// index `2·view + crouch`, with a weapon-state term for the pistol and
/// double-barrel classes. `variant` is the fire states' choice, fixed when
/// they start.
fn clip_index(state_type: i32, crouch: bool, magazine: u16, variant: usize) -> usize {
    let crouch = usize::from(crouch);
    match state_type {
        // Pistol show, hide, idle, aimed idle and reload: the empty variant.
        321..=324 | 326 => 2 * crouch + usize::from(magazine == 0),
        // Double-barrel show, hide, idle and aimed idle: by rounds loaded.
        314..=316 | 319 => 3 * crouch + usize::from(magazine.min(2)),
        // Double-barrel reload: one barrel or both.
        317 => 2 * crouch + usize::from(magazine != 1),
        318 | 320 | 325 | 327 => 2 * crouch + variant,
        _ => crouch,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Shoot,
    /// `aiming`: domain 9 starts aiming, anything else ends it.
    Aiming(u8),
}

/// A clip as a state asks for it; the same group, playback and clip mean the
/// same lexeme, which then plays on (the identifier and time scale do not
/// count: `mixing_n_ary_tree_comparer.cpp`).
#[derive(Clone, Debug)]
struct WantedClip {
    group: u8,
    cyclic: bool,
    clip: Arc<ClipTiming>,
    time_scale: f32,
}

impl WantedClip {
    fn plays_as(&self, lexeme: &Lexeme) -> bool {
        self.group == lexeme.group
            && self.cyclic == lexeme.cyclic
            && Arc::ptr_eq(&self.clip, &lexeme.clip)
    }
}

/// The weapon clip playing: the weapon lexeme pair's event lexeme.
#[derive(Clone, Debug)]
struct Lexeme {
    group: u8,
    cyclic: bool,
    clip: Arc<ClipTiming>,
    time_scale: f32,
    /// Clip time `phase` (seconds) plays at `anchor_ms`.
    anchor_ms: u32,
    phase: f32,
    marks: Vec<(f32, Mark)>,
    next_mark: usize,
    /// A once-through clip at its end raises nothing more.
    frozen: bool,
    /// The last mark raised, at its exact clip time.
    last_mark: Option<(u32, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipEvent {
    Mark(Mark),
    End,
}

impl Lexeme {
    fn new(wanted: WantedClip, at_ms: u32, phase: f32) -> Self {
        let mut marks: Vec<(f32, Mark)> = Vec::new();
        for event in &wanted.clip.events {
            let mark = match event.name.as_str() {
                "shoot" => |_| Mark::Shoot,
                "aiming" => Mark::Aiming as fn(u8) -> Mark,
                _ => continue,
            };
            for (i, knot) in event.knots.iter().enumerate() {
                let domain = event.domains.get(i).copied().unwrap_or(0);
                marks.push((knot / FPS, mark(domain)));
            }
        }
        marks.sort_by(|a, b| a.0.total_cmp(&b.0));
        let length = event_seconds(&wanted.clip);
        marks.retain(|(at, _)| *at <= length);
        // A new clip raises a mark at its start phase too (`aei.cpp`).
        let next_mark = marks.partition_point(|(at, _)| *at < phase);
        Self {
            group: wanted.group,
            cyclic: wanted.cyclic,
            clip: wanted.clip,
            time_scale: wanted.time_scale,
            anchor_ms: at_ms,
            phase,
            marks,
            next_mark,
            frozen: false,
            last_mark: None,
        }
    }

    fn length(&self) -> f32 {
        event_seconds(&self.clip)
    }

    /// The clip plays on `lag_ms` later, from where it was.
    fn delay(&mut self, lag_ms: u32) {
        self.anchor_ms = self.anchor_ms.wrapping_add(lag_ms);
        if let Some((mark_ms, _)) = self.last_mark.as_mut() {
            *mark_ms = mark_ms.wrapping_add(lag_ms);
        }
    }

    /// A looping clip skips to `at_ms` without raising what it passes.
    fn resume_at(&mut self, at_ms: u32) {
        if !self.cyclic {
            return;
        }
        if let Some(phase) = self.phase_at(at_ms) {
            self.anchor_ms = at_ms;
            self.phase = phase;
            self.next_mark = self.marks.partition_point(|(at, _)| *at <= phase);
            self.last_mark = None;
        }
    }

    fn next_event(&self) -> Option<(u32, ClipEvent)> {
        if self.frozen {
            return None;
        }
        let at = |time| clip_ms(self.anchor_ms, self.phase, time, self.time_scale);
        Some(match self.marks.get(self.next_mark) {
            Some(&(time, mark)) => (at(time), ClipEvent::Mark(mark)),
            None => (at(self.length()), ClipEvent::End),
        })
    }

    fn raise(&mut self, at_ms: u32) -> Option<ClipEvent> {
        let (_, event) = self.next_event()?;
        match event {
            ClipEvent::Mark(_) => {
                self.last_mark = Some((at_ms, self.marks[self.next_mark].0));
                self.next_mark += 1;
            }
            // A looping clip starts its next pass at the end's millisecond.
            ClipEvent::End if self.cyclic => {
                self.anchor_ms = at_ms;
                self.phase = 0.0;
                self.next_mark = 0;
                self.last_mark = None;
            }
            ClipEvent::End => self.frozen = true,
        }
        Some(event)
    }

    /// The clip time playing at `at_ms`, unless the clip is over.
    fn phase_at(&self, at_ms: u32) -> Option<f32> {
        if self.frozen {
            return None;
        }
        if let Some((mark_ms, time)) = self.last_mark
            && mark_ms == at_ms
        {
            return Some(time);
        }
        let length = self.length();
        let phase =
            self.phase + at_ms.wrapping_sub(self.anchor_ms) as f32 * self.time_scale / 1000.0;
        if phase < length {
            Some(phase)
        } else if self.cyclic && length > 0.0 {
            Some(phase.rem_euclid(length))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------- one weapon

/// `weapon_core`'s logic states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Inactive,
    Show,
    Hide,
    Idle,
    Reload,
    Fire,
    Aim,
    AimFire,
    Chamber,
    ChamberAimed,
}

/// `weapon_targets`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Inactive,
    Idle,
    Reload,
    Fire,
    Aim,
    AimFire,
}

/// The shotgun reload's own states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoundsStep {
    Start,
    OneRound,
    Finish,
}

/// One weapon slot's runtime state.
#[derive(Clone, Debug)]
pub struct WeaponState {
    pub dict_id: u16,
    pub data: Arc<WeaponData>,
    /// The weapon's ammo slots in use: (dict id, data, rounds carried).
    pub ammo: Vec<(u16, Arc<AmmoData>, u32)>,
    pub current_ammo: usize,
    pub ammo_in_magazine: u16,
    pub round_chambered: bool,
    pub fire_queue_type: usize,
    pub bullets_in_queue: u16,
    pub random: Random32,
    pub normal_random: NormalRandom,
    pub recoil: Recoil,
    pub breath: Breath,
    dispersion: Dispersion,
    clips: WeaponClips,
    selector: Selector,
    state: State,
    target: Target,
    old_mask: u32,
    /// `m_animation_has_been_ended`.
    clip_ended: bool,
    rounds_step: RoundsStep,
    rounds_start_ended: bool,
    is_idle: bool,
    aimed: bool,
    firing: bool,
    toggling: bool,
    shown: bool,
    in_sprint_transition: bool,
    aiming_state_transition: bool,
    /// The fire states' clip variant and playback, fixed when they start
    /// (pistols and double barrels).
    fire_variant: usize,
    fire_once: bool,
    activated: bool,
}

impl WeaponState {
    /// A freshly spawned weapon, loaded fully (`spawn_load`); `None` when the
    /// game data lacks its clips.
    #[cfg(test)]
    pub fn new(
        dict_id: u16,
        data: Arc<WeaponData>,
        ammo: Vec<(u16, Arc<AmmoData>, u32)>,
        seeds: (u32, i32),
        character: &CharacterDispersion,
    ) -> Option<Self> {
        let load = spawn_load(&data);
        Self::loaded(dict_id, data, ammo, seeds, character, load)
    }

    /// A freshly spawned weapon with this magazine and chamber, as the
    /// spawn message says.
    pub fn loaded(
        dict_id: u16,
        data: Arc<WeaponData>,
        ammo: Vec<(u16, Arc<AmmoData>, u32)>,
        seeds: (u32, i32),
        character: &CharacterDispersion,
        (ammo_in_magazine, round_chambered): (u16, bool),
    ) -> Option<Self> {
        let clips = WeaponClips::new(&data)?;
        Some(Self {
            dict_id,
            ammo,
            current_ammo: 0,
            ammo_in_magazine,
            round_chambered,
            fire_queue_type: 0,
            bullets_in_queue: 0,
            random: Random32(seeds.0),
            normal_random: NormalRandom(seeds.1),
            recoil: Recoil::default(),
            breath: Breath::default(),
            dispersion: Dispersion::new(character),
            clips,
            selector: Selector::default(),
            state: State::Inactive,
            target: Target::Idle,
            old_mask: 0,
            clip_ended: false,
            rounds_step: RoundsStep::Start,
            rounds_start_ended: false,
            is_idle: false,
            aimed: false,
            firing: false,
            toggling: false,
            shown: false,
            in_sprint_transition: false,
            aiming_state_transition: false,
            fire_variant: 0,
            fire_once: false,
            activated: false,
            data,
        })
    }

    /// This weapon as a spawn message of the player as it is restarts it on
    /// its own client (`weapon_core::deserialize`): idle, with this one's
    /// rounds, ammo, random generators and fire mode. The recoil and the
    /// breath aren't sent and carry on, as at a respawn.
    pub fn respawned(&self, character: &CharacterDispersion) -> Option<Self> {
        let mut weapon = Self::loaded(
            self.dict_id,
            self.data.clone(),
            self.ammo.clone(),
            (self.random.0, self.normal_random.0),
            character,
            (self.ammo_in_magazine, self.round_chambered),
        )?;
        weapon.current_ammo = self.current_ammo;
        weapon.fire_queue_type = self.fire_queue_type;
        weapon.recoil = self.recoil.clone();
        weapon.breath = self.breath.clone();
        Some(weapon)
    }

    fn ammo_data(&self) -> Option<&Arc<AmmoData>> {
        self.ammo.get(self.current_ammo).map(|(_, data, _)| data)
    }

    fn carried(&self) -> u32 {
        self.ammo.get(self.current_ammo).map_or(0, |(_, _, n)| *n)
    }

    fn take_carried(&mut self, count: u32) -> u32 {
        match self.ammo.get_mut(self.current_ammo) {
            Some((_, _, n)) => {
                let taken = count.min(*n);
                *n -= taken;
                taken
            }
            None => 0,
        }
    }

    fn in_weapon(&self) -> u16 {
        self.ammo_in_magazine + u16::from(self.round_chambered)
    }

    fn has_chamber_state(&self) -> bool {
        self.clips.chamber.is_some()
    }

    /// `maximum_ammo_in_weapon`.
    fn maximum(&self) -> u16 {
        let extra = self.has_chamber_state() && !self.data.chamber_a_round_on_reload;
        self.data.magazine_capacity + u16::from(extra)
    }

    fn fire_queue_length(&self) -> u8 {
        self.data
            .fire_queue_types
            .get(self.fire_queue_type)
            .copied()
            .unwrap_or(1)
    }

    /// `reset_fire_queue`.
    fn reset_fire_queue(&mut self) {
        self.bullets_in_queue = if self.fire_queue_length() == AUTOMATIC {
            self.in_weapon()
        } else {
            u16::from(self.fire_queue_length()).min(self.in_weapon())
        };
    }

    /// `set_next_fire_queue_type`.
    fn next_fire_queue_type(&mut self) {
        let count = self.data.fire_queue_types.len();
        self.fire_queue_type = if self.fire_queue_type + 1 >= count {
            0
        } else {
            self.fire_queue_type + 1
        };
    }

    /// `set_next_ammo_type`: unloads into the current stack and switches; the
    /// reload it asks for is overwritten in the same tick, so the weapon
    /// stays empty until reload or fire is pressed.
    fn next_ammo_type(&mut self) {
        if self.ammo.len() < 2 {
            return;
        }
        self.unload_ammo();
        self.current_ammo = 1 - self.current_ammo;
        self.target = Target::Reload;
    }

    /// `unload_ammo`.
    fn unload_ammo(&mut self) {
        let unloaded = u32::from(self.in_weapon());
        let Some((_, _, carried)) = self.ammo.get_mut(self.current_ammo) else {
            return;
        };
        *carried += unloaded;
        self.ammo_in_magazine = 0;
        self.round_chambered = false;
    }

    fn could_be_used(&self, injuries: Injuries) -> bool {
        !(injuries.broken_hands == 2 && self.data.double_handed)
    }

    /// `ready_to_reload`.
    fn ready_to_reload(&self) -> bool {
        self.in_weapon() != self.maximum()
            && self.ammo_data().is_some()
            && self.carried() != 0
            && !self.in_sprint_transition
            && self.selector.stance != Stance::Jump
    }

    /// `is_ready_to_shoot` (`m_ready_for_fire` is set again as soon as it is
    /// cleared, so it never holds a shot back).
    fn is_ready_to_shoot(&self) -> bool {
        let loaded = if self.has_chamber_state() {
            self.round_chambered
        } else {
            self.ammo_in_magazine > 0
        };
        loaded && self.bullets_in_queue != 0
    }

    /// `weapon_core::is_idle`.
    fn idle(&self) -> bool {
        self.is_idle || (self.aimed && !self.firing)
    }

    /// `weapon_core::is_toggling`: showing, hiding, or not shown yet.
    fn is_toggling(&self) -> bool {
        self.toggling || !self.shown
    }

    fn is_trying_to_aim(&self, mask: u32, injuries: Injuries) -> bool {
        let just_pressed = !self.old_mask & mask;
        injuries.broken_hands != 2
            && mask & bits::AIM != 0
            && !(is_sprinting(mask) && just_pressed & bits::SPRINT != 0)
            && self.selector.stance != Stance::Jump
    }

    /// `load_magazine`, chambering one if the weapon does that on reload.
    fn load_magazine(&mut self) {
        let room = u32::from(
            self.data
                .magazine_capacity
                .saturating_sub(self.ammo_in_magazine),
        );
        let load = self.take_carried(room) as u16;
        self.ammo_in_magazine += load;
        if self.data.chamber_a_round_on_reload {
            self.chamber_a_round();
        }
    }

    /// `chamber_a_round` (the client asserts a round is there).
    fn chamber_a_round(&mut self) {
        if self.ammo_in_magazine > 0 {
            self.ammo_in_magazine -= 1;
            self.round_chambered = true;
        }
    }

    /// `unload_chambered_round`.
    fn unload_chambered_round(&mut self) {
        self.ammo_in_magazine += 1;
        self.round_chambered = false;
    }

    /// `load_ammo`, on a weapon's first activation.
    fn load_ammo(&mut self) {
        if self.ammo_data().is_none() || self.carried() == 0 {
            return;
        }
        if self.ammo_in_magazine == 0 {
            self.load_magazine();
        }
        if self.has_chamber_state() && !self.data.chamber_a_round_on_reload {
            if self.take_carried(1) == 1 {
                self.round_chambered = true;
            } else if self.ammo_in_magazine != 0 {
                self.round_chambered = true;
                self.ammo_in_magazine -= 1;
            }
        }
    }

    /// `instant_reload`.
    fn instant_reload(&mut self) {
        self.aimed = false;
        self.load_magazine();
        self.recoil.reset();
        self.dispersion.reload(&self.data);
        self.reset_fire_queue();
    }

    /// `instant_chamber_a_round`.
    fn instant_chamber_a_round(&mut self) {
        self.aimed = false;
        self.chamber_a_round();
        self.recoil.reset();
    }

    /// `reload_one_round`.
    fn reload_one_round(&mut self) {
        if self.ammo_in_magazine != self.data.magazine_capacity && self.take_carried(1) == 1 {
            self.ammo_in_magazine += 1;
        }
    }

    fn instant_aim_start(&mut self, mask: u32) {
        if !self.firing && mask & bits::FIRE == 0 {
            self.reset_fire_queue();
        }
        self.aimed = true;
        self.aiming_state_transition = true;
    }

    fn instant_aim_end(&mut self) {
        self.aimed = false;
        self.aiming_state_transition = true;
    }

    /// `weapon_user_animations_selector::tick`.
    fn tick_selector(&mut self, now_ms: u32, mask: u32, injuries: Injuries, stamina: &Stamina) {
        if mask & bits::SPRINT == 0 {
            self.selector.forced_not_to_sprint = false;
        } else if !(is_sprinting(mask) && self.idle()) && self.selector.stance == Stance::Sprint {
            self.selector.forced_not_to_sprint = true;
        }
        let legs = injuries.broken_legs;
        let free = !self.firing && !self.is_toggling() && legs == 0;
        let sprint = free
            && !self.selector.forced_not_to_sprint
            && is_sprinting(mask)
            && stamina.can_be_spent();
        let crouch = legs == 2 || mask & bits::CROUCH != 0;
        let jump = free && mask & bits::JUMP != 0 && stamina.value >= stamina.amount_to_jump();
        let landed = self.selector.jump.is_some_and(|jump| jump.finished);
        let next = match self.selector.stance {
            Stance::Stand if crouch => Some(Stance::Crouch),
            Stance::Stand if sprint => Some(Stance::Sprint),
            Stance::Stand if jump => Some(Stance::Jump),
            Stance::Crouch if legs != 2 && mask & bits::CROUCH == 0 => Some(Stance::Stand),
            Stance::Crouch if sprint => Some(Stance::Sprint),
            Stance::Sprint if crouch => Some(Stance::Crouch),
            Stance::Sprint if !sprint => Some(Stance::Stand),
            Stance::Sprint if jump => Some(Stance::Jump),
            Stance::Jump if legs == 2 => Some(Stance::Crouch),
            Stance::Jump if landed => Some(Stance::Stand),
            _ => None,
        };
        if let Some(next) = next {
            self.selector.jump = (next == Stance::Jump).then(|| Jump {
                kind: JumpKind::from_mask(mask),
                stage: JumpStage::Inactive,
                anchor_ms: now_ms,
                mark_done: false,
                start_ended: false,
                finished: false,
            });
            self.selector.stance = next;
        }
        // The jump state's execute ticks `jump_logic`.
        if let Some(jump) = self.selector.jump.as_mut() {
            match jump.stage {
                JumpStage::Inactive => {
                    jump.stage = JumpStage::Start;
                    jump.anchor_ms = now_ms;
                }
                JumpStage::Start if jump.start_ended => {
                    jump.stage = JumpStage::Landing;
                    jump.anchor_ms = now_ms;
                }
                _ => {}
            }
        }
    }

    /// `selector.sprint_predicate()`.
    fn sprint_predicate(&self, mask: u32, injuries: Injuries, stamina: &Stamina) -> bool {
        !self.firing
            && !self.is_toggling()
            && injuries.broken_legs == 0
            && !self.selector.forced_not_to_sprint
            && is_sprinting(mask)
            && stamina.can_be_spent()
    }

    /// `weapon_core::set_target`.
    fn set_target(&mut self, target: Target) {
        let mut target = target;
        if matches!(target, Target::Fire | Target::AimFire) {
            if self.in_weapon() == 0 {
                target = Target::Reload;
            } else if !self.is_ready_to_shoot() {
                target = if target == Target::Fire {
                    Target::Idle
                } else {
                    Target::Aim
                };
            }
        }
        if !self.in_sprint_transition && self.selector.stance == Stance::Sprint {
            self.in_sprint_transition = true;
        }
        if self.in_sprint_transition || self.selector.stance == Stance::Jump {
            target = match target {
                Target::Fire => Target::Idle,
                Target::AimFire => Target::Aim,
                other => other,
            };
        }
        if target == Target::Reload && !self.ready_to_reload() {
            target = if matches!(self.target, Target::AimFire | Target::Aim) {
                Target::Aim
            } else {
                Target::Idle
            };
        }
        self.target = target;
    }

    /// `weapon_core::tick`.
    fn tick(
        &mut self,
        now_ms: u32,
        mask: u32,
        injuries: Injuries,
        stamina: &Stamina,
        is_target: bool,
    ) {
        self.tick_selector(now_ms, mask, injuries, stamina);
        if self.idle() && mask & bits::FIRE == 0 {
            if mask & bits::FIRE_MODE != 0 {
                self.next_fire_queue_type();
            } else if mask & bits::AMMO_TYPE != 0 {
                self.next_ammo_type();
            } else {
                self.reset_fire_queue();
            }
        }
        let aim = self.is_trying_to_aim(mask, injuries);
        let target = if !is_target {
            Target::Inactive
        } else if !self.could_be_used(injuries) {
            Target::Idle
        } else if mask & bits::FIRE != 0 {
            if aim { Target::AimFire } else { Target::Fire }
        } else if mask & bits::RELOAD != 0 {
            Target::Reload
        } else if aim {
            Target::Aim
        } else {
            Target::Idle
        };
        self.set_target(target);
        self.tick_logic(mask, injuries, stamina);
        if self.aiming_state_transition {
            self.tick_selector(now_ms, mask, injuries, stamina);
            self.aiming_state_transition = false;
        }
        self.old_mask = mask;
    }

    /// One `ai::fsm::tick`: at most one transition, then the state's
    /// `execute`.
    fn tick_logic(&mut self, mask: u32, injuries: Injuries, stamina: &Stamina) {
        let ready = match self.state {
            State::Show | State::Hide => self.clip_ended,
            _ => true,
        };
        if ready && let Some(next) = self.next_state(mask, injuries, stamina) {
            self.finalize(self.state, mask);
            self.state = next;
            self.initialize(next, mask);
        }
        match self.state {
            State::Fire | State::AimFire => self.clip_ended = false,
            State::Reload => self.tick_rounds(),
            _ => {}
        }
    }

    /// The transitions of `initialize_weapon_logic`, in their order.
    fn next_state(&self, mask: u32, injuries: Injuries, stamina: &Stamina) -> Option<State> {
        use State::*;
        let ended = self.clip_ended;
        let target = self.target;
        let aim = self.is_trying_to_aim(mask, injuries);
        // `must_chamber_a_round_predicate`, where the weapon has the states.
        let must_chamber = !self.round_chambered
            && self.ammo_in_magazine != 0
            && self.selector.stance != Stance::Jump;
        let chamber = self.clips.chamber.is_some() && must_chamber;
        let chamber_aimed = self.clips.chamber_aimed.is_some() && must_chamber && aim;
        let must_reload =
            self.ready_to_reload() && self.ammo_in_magazine == 0 && !self.round_chambered;
        let instant_idle =
            self.sprint_predicate(mask, injuries, stamina) || self.selector.stance == Stance::Jump;
        let to = |target: Target| match target {
            Target::Inactive => Hide,
            Target::Idle => Idle,
            Target::Reload => Reload,
            Target::Fire => Fire,
            Target::Aim => Aim,
            Target::AimFire => AimFire,
        };
        let hide = target == Target::Inactive;
        match self.state {
            Inactive => (!hide).then_some(Show),
            Show => {
                if hide {
                    Some(Hide)
                } else if chamber_aimed {
                    Some(ChamberAimed)
                } else if chamber {
                    Some(Chamber)
                } else if must_reload {
                    Some(Reload)
                } else {
                    Some(to(target))
                }
            }
            Hide => Some(if hide { Inactive } else { Show }),
            Idle => {
                if hide {
                    Some(Hide)
                } else if chamber_aimed {
                    Some(ChamberAimed)
                } else if chamber {
                    Some(Chamber)
                } else {
                    (target != Target::Idle).then(|| to(target))
                }
            }
            Reload => {
                let chamber_after = chamber && !self.data.chamber_a_round_on_reload && ended;
                if hide {
                    Some(Hide)
                } else if chamber_after {
                    Some(Chamber)
                } else if (target == Target::Idle && ended) || instant_idle {
                    Some(Idle)
                } else if matches!(target, Target::Fire | Target::Aim | Target::AimFire) && ended {
                    Some(to(target))
                } else {
                    None
                }
            }
            Fire | AimFire => {
                let firing_target = if self.state == Fire {
                    Target::Fire
                } else {
                    Target::AimFire
                };
                if hide && ended {
                    Some(Hide)
                } else if chamber_aimed && ended {
                    Some(ChamberAimed)
                } else if chamber && ended {
                    Some(Chamber)
                } else if ended && must_reload {
                    Some(Reload)
                } else if ended && target != firing_target && !hide {
                    Some(to(target))
                } else if self.state == Fire && aim {
                    Some(AimFire)
                } else if self.state == AimFire && !aim {
                    Some(Fire)
                } else {
                    None
                }
            }
            Aim => {
                if hide {
                    Some(Hide)
                } else if target == Target::Idle || instant_idle {
                    Some(Idle)
                } else {
                    (target != Target::Aim).then(|| to(target))
                }
            }
            Chamber | ChamberAimed => {
                if hide {
                    Some(Hide)
                } else if (target == Target::Idle && ended) || instant_idle {
                    Some(Idle)
                } else if matches!(target, Target::Fire | Target::Aim | Target::AimFire) && ended {
                    Some(to(target))
                } else if self.state == Chamber && self.clips.chamber_aimed.is_some() && aim {
                    Some(ChamberAimed)
                } else if self.state == ChamberAimed && !aim {
                    Some(Chamber)
                } else {
                    None
                }
            }
        }
    }

    fn initialize(&mut self, state: State, mask: u32) {
        match state {
            State::Show | State::Hide => {
                self.clip_ended = false;
                self.toggling = true;
            }
            State::Idle => {
                self.is_idle = true;
                if mask & bits::FIRE == 0 {
                    self.reset_fire_queue();
                }
            }
            State::Aim => self.instant_aim_start(mask),
            State::Reload => {
                self.clip_ended = false;
                // Both reload kinds first unload a chambered round of a
                // weapon that chambers on reload.
                if self.data.chamber_a_round_on_reload && self.round_chambered {
                    self.unload_chambered_round();
                }
                self.rounds_step = RoundsStep::Start;
                self.rounds_start_ended = false;
            }
            State::Fire | State::AimFire => {
                self.firing = true;
                self.clip_ended = false;
                if state == State::AimFire {
                    self.instant_aim_start(mask);
                }
                let fire = if state == State::Fire {
                    &self.clips.fire
                } else {
                    &self.clips.aimed_fire
                };
                self.fire_variant = match fire.state_type {
                    // Pistols: the last-shot clip.
                    325 | 327 => usize::from(if self.bullets_in_queue != 0 {
                        self.ammo_in_magazine == 1
                    } else {
                        self.ammo_in_magazine == 0
                    }),
                    // Double barrels: which barrel.
                    318 | 320 => usize::from(self.ammo_in_magazine != 2),
                    _ => 0,
                };
                self.fire_once = self.bullets_in_queue <= 1;
            }
            State::Chamber => self.clip_ended = false,
            State::ChamberAimed => {
                self.clip_ended = false;
                self.instant_aim_start(mask);
            }
            State::Inactive => {}
        }
    }

    fn finalize(&mut self, state: State, _mask: u32) {
        match state {
            State::Show | State::Hide => self.toggling = false,
            State::Idle => self.is_idle = false,
            State::Aim | State::ChamberAimed => self.instant_aim_end(),
            State::Fire => self.firing = false,
            State::AimFire => {
                self.instant_aim_end();
                self.firing = false;
            }
            // The shotgun reload chambers as its finish step is left.
            State::Reload
                if matches!(self.clips.reload, ReloadClips::Rounds(_))
                    && self.rounds_step == RoundsStep::Finish
                    && self.data.chamber_a_round_on_reload
                    && self.ammo_in_magazine != 0 =>
            {
                self.instant_chamber_a_round();
            }
            _ => {}
        }
    }

    /// The shotgun reload's own logic, ticked by its `execute`.
    fn tick_rounds(&mut self) {
        if !matches!(self.clips.reload, ReloadClips::Rounds(_)) {
            return;
        }
        match self.rounds_step {
            RoundsStep::Start if self.rounds_start_ended => self.rounds_step = RoundsStep::OneRound,
            // `finish_reload_predicate`.
            RoundsStep::OneRound
                if self.ammo_in_magazine == self.data.magazine_capacity
                    || self.carried() == 0
                    || (self.in_weapon() != 0
                        && matches!(self.target, Target::Fire | Target::AimFire)) =>
            {
                self.rounds_step = RoundsStep::Finish;
            }
            _ => {}
        }
    }

    /// The clip the current state plays.
    fn wanted_clip(&self) -> Option<WantedClip> {
        let crouch = self.selector.stance == Stance::Crouch;
        let wanted = |group, cyclic, timing: &StateTiming, variant| WantedClip {
            group,
            cyclic,
            clip: timing
                .clip(clip_index(
                    timing.state_type,
                    crouch,
                    self.ammo_in_magazine,
                    variant,
                ))
                .clone(),
            time_scale: timing.time_scale,
        };
        Some(match self.state {
            State::Inactive | State::Idle | State::Aim => return None,
            State::Show => wanted(6, false, &self.clips.show, 0),
            State::Hide => wanted(7, false, &self.clips.hide, 0),
            State::Reload => match &self.clips.reload {
                ReloadClips::Whole(timing) => wanted(2, false, timing, 0),
                ReloadClips::Rounds([start, one_round, finish]) => match self.rounds_step {
                    RoundsStep::Start => wanted(3, false, start, 0),
                    RoundsStep::OneRound => wanted(4, true, one_round, 0),
                    RoundsStep::Finish => wanted(5, false, finish, 0),
                },
            },
            State::Fire | State::AimFire => {
                let timing = if self.state == State::Fire {
                    &self.clips.fire
                } else {
                    &self.clips.aimed_fire
                };
                let cyclic = match timing.state_type {
                    318 | 320 => false,
                    325 | 327 => !self.fire_once,
                    // Chosen anew at every selection.
                    _ => self.bullets_in_queue > 1,
                };
                wanted(1, cyclic, timing, self.fire_variant)
            }
            State::Chamber => wanted(8, false, self.clips.chamber.as_ref()?, 0),
            State::ChamberAimed => wanted(8, false, self.clips.chamber_aimed.as_ref()?, 0),
        })
    }

    /// The breath sway the camera draws: `is_aimed` (weapon_core+0x488)
    /// gates it.
    pub fn drawn_sway(&self) -> DrawnSway {
        let (horizontal, vertical) = if self.aimed {
            self.breath.sway()
        } else {
            (0.0, 0.0)
        };
        DrawnSway {
            aimed: self.aimed,
            horizontal,
            vertical,
            multiplier: self.breath.multiplier(),
        }
    }

    /// `weapon_core::instant_fire`: one draw pair per pellet, then the round.
    /// `recoil` is the camera the last frame drew, which the bullets leave,
    /// and `sway` the breath sway in it.
    fn fire(&mut self, at_ms: u32, recoil: (f32, f32, f32), sway: DrawnSway) -> Option<Shot> {
        self.bullets_in_queue = self.bullets_in_queue.saturating_sub(1);
        let ammo_dict_id = self.ammo.get(self.current_ammo).map_or(0, |(id, _, _)| *id);
        let ammo = self.ammo_data().cloned();
        let shot = ammo.map(|ammo| {
            let dispersion_degrees = self.dispersion.degrees(&self.data, &ammo, self.aimed);
            let pellets = (0..ammo.buck_shot.max(1))
                .map(|_| {
                    let angle = self.normal_random.rand_n(1.0).clamp(-1.0, 1.0);
                    let k = self.random.random_f(std::f32::consts::TAU);
                    (angle, k)
                })
                .collect();
            Shot {
                at_ms,
                weapon_dict_id: self.dict_id,
                weapon: self.data.clone(),
                ammo_dict_id,
                ammo,
                dispersion_degrees,
                recoil,
                sway,
                pellets,
            }
        });
        if self.has_chamber_state() {
            self.round_chambered = false;
        } else {
            self.ammo_in_magazine = self.ammo_in_magazine.saturating_sub(1);
        }
        let params = self.data.recoil;
        let before = log::log_enabled!(log::Level::Debug).then(|| self.recoil.clone());
        self.recoil.fire(&params);
        if let Some(before) = before {
            log::debug!(
                "weapon {} fires at {at_ms} ms: drawn recoil {recoil:?}; recoil before {before:?}; after {:?}",
                self.dict_id,
                self.recoil
            );
        }
        shot
    }
}

/// Loaded as `load_magazine` leaves a weapon: chamber-on-reload weapons move
/// one round from the magazine into the chamber.
pub fn spawn_load(data: &WeaponData) -> (u16, bool) {
    if data.has_chamber_state {
        if data.chamber_a_round_on_reload {
            (data.magazine_capacity.saturating_sub(1), true)
        } else {
            (data.magazine_capacity, true)
        }
    } else {
        (data.magazine_capacity, false)
    }
}

/// One shot: the spread (degrees) and per pellet the two random draws of
/// `weapon_core::get_dispersed_bullet_dir`.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    /// The client's clock.
    pub at_ms: u32,
    pub weapon_dict_id: u16,
    /// The round it uses up.
    pub ammo_dict_id: u16,
    pub ammo: Arc<AmmoData>,
    pub weapon: Arc<WeaponData>,
    pub dispersion_degrees: f32,
    /// The camera recoil the last frame drew, breath sway included.
    pub recoil: (f32, f32, f32),
    /// The breath sway in `recoil`.
    pub sway: DrawnSway,
    /// (`dispersion_angle` in [-1, 1], `random_k` in [0, 2π)).
    pub pellets: Vec<(f32, f32)>,
}

// ---------------------------------------------------------------- the player

/// A player's two weapon slots and what the client runs around them: the
/// active object switch (`base_player::tick_active_object`), stamina, and
/// the clips whose events drive the weapon logic.
#[derive(Clone, Debug)]
pub struct Arsenal {
    pub weapons: [Option<WeaponState>; 2],
    current: usize,
    target: usize,
    character: CharacterDispersion,
    breath: BreathParams,
    stamina: Stamina,
    lexeme: Option<Lexeme>,
    /// Replaced clips still fading out, with when they were replaced.
    fading: Vec<(Lexeme, u32)>,
    sprint_clip: bool,
    sprint_fade_end: Option<u32>,
    /// The last frame: (client time, input).
    last_frame: Option<(u32, u32)>,
    force_selection: bool,
    activate_pending: bool,
    broken_legs: u8,
    /// The recoil the last frame drew, and the breath sway in it; the next
    /// frame's shots leave with it.
    drawn_recoil: (f32, f32, f32),
    drawn_sway: DrawnSway,
    /// The input of the frame being replayed.
    mask: u32,
}

impl Arsenal {
    pub fn new(
        weapons: [Option<WeaponState>; 2],
        active: usize,
        character: CharacterDispersion,
        breath: BreathParams,
        stamina: StaminaParams,
    ) -> Self {
        Self {
            weapons,
            current: active,
            target: active,
            character,
            breath,
            stamina: Stamina::new(stamina),
            lexeme: None,
            fading: Vec::new(),
            sprint_clip: false,
            sprint_fade_end: None,
            last_frame: None,
            force_selection: true,
            activate_pending: true,
            broken_legs: 0,
            drawn_recoil: RESTING_RECOIL,
            drawn_sway: DrawnSway::NONE,
            mask: 0,
        }
    }

    /// The arsenal as a spawn message of the player as it is restarts it on
    /// its own client: each weapon `respawned`, the one in hand or being
    /// switched to active, the stamina as it is (the spawn sends it).
    pub fn respawned(&self) -> Self {
        let weapons = std::array::from_fn(|i| {
            self.weapons[i]
                .as_ref()
                .and_then(|weapon| weapon.respawned(&self.character))
        });
        let mut arsenal = Self::new(
            weapons,
            self.target,
            self.character,
            self.breath,
            self.stamina.params,
        );
        arsenal.stamina = self.stamina.clone();
        arsenal
    }

    pub fn active_weapon(&self) -> Option<&WeaponState> {
        self.weapons[self.current].as_ref()
    }

    /// The slot being switched to, else the active one: the client's
    /// `inventory` takes the new slot as soon as the switch starts.
    pub fn requested_slot(&self) -> usize {
        self.target
    }

    pub fn stamina(&self) -> &Stamina {
        &self.stamina
    }

    fn stance(&self) -> Option<Stance> {
        self.active_weapon().map(|weapon| weapon.selector.stance)
    }

    /// One client frame (`player::tick`) with its input and time; returns
    /// the shots fired since the previous frame.
    pub fn frame(&mut self, mask: u32, time_ms: u32, injuries: Injuries) -> Vec<Shot> {
        let mut shots = Vec::new();
        let (previous_ms, previous_mask) = match self.last_frame {
            Some((at, mask)) if at <= time_ms => (at, mask),
            _ => (time_ms, mask),
        };
        self.mask = mask;
        if injuries.broken_legs != self.broken_legs {
            // `on_broken_limb_affect`.
            self.broken_legs = injuries.broken_legs;
            self.force_selection = true;
        }

        let sprinting = self.stance() == Some(Stance::Sprint);
        let can_sprint = self.stamina.can_be_spent();
        if self.stamina.tick(time_ms, can_sprint && sprinting) && sprinting {
            // The sprint state's depletion subscriber.
            self.force_selection = true;
        }

        // `process_quick_slots_for_current_player`: weapon keys.
        for (bit, slot) in [(bits::WEAPON_1, 0), (bits::WEAPON_2, 1)] {
            if mask & bit != 0
                && slot != self.current
                && self.target != slot
                && self.weapons[slot]
                    .as_ref()
                    .is_some_and(|weapon| weapon.could_be_used(injuries))
            {
                self.target = slot;
                self.force_selection = true;
            }
        }

        // The selection loop: at the previous frame's time if the input
        // changed, then at every interrupting clip event up to this frame.
        let mut at = previous_ms;
        if mask != previous_mask || self.force_selection {
            self.force_selection = false;
            self.select(at, injuries, &mut shots);
        }
        let lag_ms = time_ms.wrapping_sub(at);
        if lag_ms >= SKIP_LAG_MS {
            self.delay_clips(lag_ms);
            at = time_ms;
        }
        let mut budget = MAX_EVENTS_PER_FRAME;
        loop {
            match self.advance(at, time_ms, injuries, &mut shots, &mut budget) {
                Some(stop) if stop < time_ms => {
                    at = stop;
                    self.select(at, injuries, &mut shots);
                }
                Some(stop) => {
                    self.select(stop, injuries, &mut shots);
                    break;
                }
                None => break,
            }
        }

        // `render`: dispersion and the camera the next frame's shots use.
        let character = self.character;
        if let Some(weapon) = self.weapons[self.current].as_mut() {
            let stance = weapon.selector.stance;
            weapon.dispersion.tick(
                time_ms,
                &weapon.data,
                &character,
                stance,
                is_moving(mask),
                weapon.aimed,
                injuries.broken_hands,
            );
            self.drawn_sway = weapon.drawn_sway();
            self.drawn_recoil = weapon
                .recoil
                .swayed_clip_times((self.drawn_sway.horizontal, self.drawn_sway.vertical));
        }
        self.last_frame = Some((time_ms, mask));
        shots
    }

    /// `player::select_animations`: the active object's tick and switch,
    /// then the clips for the new state, whose events at `at` run at once.
    fn select(&mut self, at: u32, injuries: Injuries, shots: &mut Vec<Shot>) {
        let mask = self.mask;
        let stance = self.stance();
        if self.activate_pending {
            self.activate_pending = false;
            self.activate(self.current);
        }
        self.tick_weapon(self.current, at, injuries);
        let ready = self.weapons[self.current]
            .as_ref()
            .is_none_or(|weapon| weapon.state == State::Inactive);
        if self.current != self.target && ready {
            if let Some(weapon) = self.weapons[self.current].as_mut() {
                weapon.selector.deactivate();
            }
            self.current = self.target;
            self.activate(self.current);
            self.tick_weapon(self.current, at, injuries);
        }
        // Broken hands: switch to the other weapon if that one can be used.
        if self.current == self.target
            && injuries.broken_hands > 0
            && self.weapons[self.current]
                .as_ref()
                .is_some_and(|weapon| !weapon.could_be_used(injuries))
        {
            let other = 1 - self.target;
            if self.weapons[other]
                .as_ref()
                .is_some_and(|weapon| weapon.could_be_used(injuries))
            {
                self.target = other;
                self.force_selection = true;
            }
        }

        let wanted = self.weapons[self.current]
            .as_ref()
            .and_then(WeaponState::wanted_clip);
        let same = match (&wanted, &self.lexeme) {
            (Some(wanted), Some(lexeme)) => wanted.plays_as(lexeme),
            (None, None) => true,
            _ => false,
        };
        if !same {
            let old = self.lexeme.take();
            let phase = wanted
                .as_ref()
                .and_then(|wanted| self.inherited_phase(wanted, old.as_ref(), at))
                .unwrap_or(0.0);
            self.fading
                .retain(|(_, since)| at.wrapping_sub(*since) < FADE_MS);
            if let Some(old) = old
                && !old.frozen
            {
                // Only a group's newest fading clip lends its phase.
                self.fading.retain(|(lexeme, _)| lexeme.group != old.group);
                self.fading.push((old, at));
            }
            self.lexeme = wanted.map(|wanted| Lexeme::new(wanted, at, phase));
            // `set_target_and_tick`: the new clip's events at `at` run, but
            // cannot ask for another selection.
            while let Some((event_ms, _)) = self.lexeme.as_ref().and_then(Lexeme::next_event)
                && event_ms <= at
            {
                let event = self.lexeme.as_mut().and_then(|lexeme| lexeme.raise(at));
                if let Some(event) = event {
                    self.on_clip_event(at, event, mask, shots);
                }
            }
        }

        // `set_target_and_tick` plays the old clips and then the new ones to
        // `at`: the breath sees this frame's input there already.
        if [stance, self.stance()]
            .into_iter()
            .any(|stance| matches!(stance, Some(Stance::Stand | Stance::Crouch)))
            && let Some(weapon) = self.weapons[self.current].as_mut()
        {
            weapon.breath.tick(at, mask, &self.breath);
        }

        let sprinting = self.stance() == Some(Stance::Sprint);
        if sprinting {
            self.sprint_fade_end = None;
        } else if self.sprint_clip {
            self.sprint_fade_end = Some(at.wrapping_add(FADE_MS));
        }
        self.sprint_clip = sprinting;
    }

    /// A new clip of a time-sync group takes the playing phase of the
    /// group's clip it replaces (or of one still fading), unless that one is
    /// over (`mixing_n_ary_tree_transition_tree_constructor.cpp`).
    fn inherited_phase(&self, wanted: &WantedClip, old: Option<&Lexeme>, at: u32) -> Option<f32> {
        let candidate = old
            .filter(|old| old.group == wanted.group && !old.frozen)
            .or_else(|| {
                self.fading
                    .iter()
                    .rev()
                    .find(|(lexeme, since)| {
                        lexeme.group == wanted.group && at.wrapping_sub(*since) < FADE_MS
                    })
                    .map(|(lexeme, _)| lexeme)
            })?;
        let phase = candidate.phase_at(at)?;
        let old_length = candidate.length();
        (old_length > 0.0).then(|| phase / old_length * event_seconds(&wanted.clip))
    }

    fn activate(&mut self, slot: usize) {
        let character = self.character;
        let Some(weapon) = self.weapons[slot].as_mut() else {
            return;
        };
        // `weapon_core::activate`.
        weapon.dispersion.reset_character(&character);
        weapon.breath.reset(&self.breath);
        weapon.shown = false;
        weapon.in_sprint_transition = false;
        weapon.firing = false;
        weapon.selector.deactivate();
        weapon.state = State::Inactive;
        weapon.tick_logic(self.mask, Injuries::default(), &self.stamina);
        // The first ammunition slot that holds any, else the second.
        weapon.current_ammo = 0;
        if !weapon.activated {
            weapon.activated = true;
            weapon.load_ammo();
        }
        weapon.reset_fire_queue();
        // `instant_show`.
        weapon.aimed = false;
    }

    fn tick_weapon(&mut self, slot: usize, at: u32, injuries: Injuries) {
        let mask = self.mask;
        let is_target = slot == self.target;
        let stamina = &self.stamina;
        if let Some(weapon) = self.weapons[slot].as_mut() {
            weapon.tick(at, mask, injuries, stamina, is_target);
        }
    }

    /// Every clip, fade and jump plays on `lag_ms` later, from where it was.
    fn delay_clips(&mut self, lag_ms: u32) {
        if let Some(lexeme) = self.lexeme.as_mut() {
            lexeme.delay(lag_ms);
        }
        for (lexeme, since) in &mut self.fading {
            lexeme.delay(lag_ms);
            *since = since.wrapping_add(lag_ms);
        }
        if let Some(fade_end) = self.sprint_fade_end.as_mut() {
            *fade_end = fade_end.wrapping_add(lag_ms);
        }
        if let Some(jump) = self.weapons[self.current]
            .as_mut()
            .and_then(|weapon| weapon.selector.jump.as_mut())
        {
            jump.anchor_ms = jump.anchor_ms.wrapping_add(lag_ms);
        }
    }

    /// Plays the clips from `from` to `to`, raising their events in time
    /// order; returns the time of the first event that asks for a selection.
    fn advance(
        &mut self,
        from: u32,
        to: u32,
        _injuries: Injuries,
        shots: &mut Vec<Shot>,
        budget: &mut usize,
    ) -> Option<u32> {
        let mask = self.mask;
        let mut now = from;
        loop {
            let jump = self.weapons[self.current]
                .as_ref()
                .and_then(|weapon| weapon.selector.jump)
                .and_then(|jump| jump.next_event());
            let next = [
                self.lexeme
                    .as_ref()
                    .and_then(Lexeme::next_event)
                    .map(|(at, _)| at),
                self.sprint_fade_end,
                jump.map(|(at, _)| at),
            ]
            .into_iter()
            .flatten()
            .filter(|at| *at > now)
            .min();
            let Some(at) = next.filter(|at| *at <= to) else {
                self.tick_recoil(to);
                return None;
            };
            if *budget == 0 {
                log::warn!("weapon replay: too many clip events in one frame; skipping ahead");
                if let Some(lexeme) = self.lexeme.as_mut() {
                    lexeme.resume_at(to);
                }
                self.tick_recoil(to);
                return None;
            }
            *budget -= 1;
            self.tick_recoil(at);
            let mut interrupt = false;
            while let Some((event_ms, _)) = self.lexeme.as_ref().and_then(Lexeme::next_event)
                && event_ms == at
                && *budget > 0
            {
                *budget -= 1;
                let event = self.lexeme.as_mut().and_then(|lexeme| lexeme.raise(at));
                if let Some(event) = event {
                    interrupt |= self.on_clip_event(at, event, mask, shots);
                }
            }
            if self.sprint_fade_end == Some(at) {
                self.sprint_fade_end = None;
                // `on_sprint_animation_ended`, subscribed while latched.
                if let Some(weapon) = self.weapons[self.current].as_mut()
                    && weapon.in_sprint_transition
                {
                    weapon.in_sprint_transition = false;
                    interrupt = true;
                }
            }
            if let Some((jump_ms, event)) = jump
                && jump_ms == at
            {
                interrupt |= self.on_jump_event(event);
            }
            if interrupt {
                return Some(at);
            }
            now = at;
        }
    }

    /// The recoil clips play while standing or crouching
    /// (`player_logic_{stand,crouch}_state::get_recoil_animation_lexeme`,
    /// 0x780710); their time calculators tick the recoil and the breath.
    /// In a sprint or a jump both wait, and the breath then takes the whole
    /// elapsed time at once.
    fn tick_recoil(&mut self, at: u32) {
        if let Some(weapon) = self.weapons[self.current].as_mut()
            && matches!(weapon.selector.stance, Stance::Stand | Stance::Crouch)
        {
            let params = weapon.data.recoil;
            weapon.recoil.tick(at, &params);
            weapon.breath.tick(at, self.mask, &self.breath);
        }
    }

    fn on_jump_event(&mut self, event: JumpEvent) -> bool {
        let Some(weapon) = self.weapons[self.current].as_mut() else {
            return false;
        };
        let Some(jump) = weapon.selector.jump.as_mut() else {
            return false;
        };
        match event {
            JumpEvent::Mark => {
                jump.mark_done = true;
                let amount = self.stamina.amount_to_jump();
                self.stamina.spend(amount);
                false
            }
            JumpEvent::StartEnded => {
                jump.start_ended = true;
                true
            }
            JumpEvent::Landed => {
                jump.finished = true;
                true
            }
        }
    }

    /// The weapon's clip callbacks; returns whether one interrupts the
    /// animation tick (asking for a selection).
    fn on_clip_event(
        &mut self,
        at: u32,
        event: ClipEvent,
        mask: u32,
        shots: &mut Vec<Shot>,
    ) -> bool {
        let (recoil, sway) = (self.drawn_recoil, self.drawn_sway);
        let Some(weapon) = self.weapons[self.current].as_mut() else {
            return false;
        };
        let firing = matches!(weapon.state, State::Fire | State::AimFire);
        match event {
            ClipEvent::Mark(Mark::Shoot) if firing => {
                // `on_shot_event`.
                if weapon.bullets_in_queue == 0 {
                    log::debug!("weapon {}: shoot mark with an empty queue", weapon.dict_id);
                } else {
                    shots.extend(weapon.fire(at, recoil, sway));
                }
                true
            }
            ClipEvent::Mark(Mark::Aiming(domain)) if weapon.state == State::AimFire => {
                if domain == 9 {
                    weapon.instant_aim_start(mask);
                } else {
                    weapon.instant_aim_end();
                }
                false
            }
            ClipEvent::Mark(_) => false,
            ClipEvent::End => {
                match weapon.state {
                    State::Show => weapon.shown = true,
                    State::Hide => weapon.shown = false,
                    State::Reload => match weapon.clips.reload {
                        ReloadClips::Whole(_) => weapon.instant_reload(),
                        ReloadClips::Rounds(_) => match weapon.rounds_step {
                            RoundsStep::Start => {
                                weapon.rounds_start_ended = true;
                                return true;
                            }
                            RoundsStep::OneRound => {
                                weapon.reload_one_round();
                                return true;
                            }
                            RoundsStep::Finish => {}
                        },
                    },
                    State::Chamber | State::ChamberAimed => weapon.instant_chamber_a_round(),
                    State::Fire | State::AimFire => {}
                    State::Inactive | State::Idle | State::Aim => return false,
                }
                weapon.clip_ended = true;
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::data::CombatData;
    use crate::combat::data::tests::game_data;

    fn weapon(data: &CombatData, weapon: u16, ammo: u16, carried: u32) -> WeaponState {
        WeaponState::new(
            weapon,
            data.weapon(weapon).unwrap().clone(),
            vec![(ammo, data.ammo(ammo).unwrap().clone(), carried)],
            (0, 1),
            &data.character_dispersion,
        )
        .unwrap()
    }

    fn arsenal(data: &CombatData, weapons: [Option<WeaponState>; 2]) -> Arsenal {
        Arsenal::new(
            weapons,
            0,
            data.character_dispersion,
            data.breath,
            data.stamina,
        )
    }

    /// Frames 16 ms apart from `from` to `to` (inclusive) with one input.
    fn hold(arsenal: &mut Arsenal, mask: u32, from: u32, to: u32) -> Vec<Shot> {
        let mut shots = Vec::new();
        let mut t = from;
        while t <= to {
            shots.extend(arsenal.frame(mask, t, Injuries::default()));
            t += 16;
        }
        shots
    }

    fn times(shots: &[Shot]) -> Vec<u32> {
        shots.iter().map(|shot| shot.at_ms).collect()
    }

    /// An AK drawn at time 0 (its show clip ends at 600 ms).
    fn drawn_ak(data: &CombatData) -> Arsenal {
        let mut ak = arsenal(data, [Some(weapon(data, 13, 7, 90)), None]);
        hold(&mut ak, 0, 0, 992);
        ak
    }

    #[test]
    fn random_generators_match_the_engine() {
        let mut r = Random32(0);
        assert_eq!(r.random(1 << 20), 0); // seed 1 >> 12
        assert_eq!(r.0, 1);
        let mut n = NormalRandom(1);
        // MSVC rand(): the first value from seed 1 is 41.
        assert_eq!(n.rand_i(), 41);
        let mut n = NormalRandom(1);
        let draws: Vec<f32> = (0..1000).map(|_| n.rand_n(1.0)).collect();
        let mean = draws.iter().sum::<f32>() / draws.len() as f32;
        assert!(mean.abs() < 0.15, "{mean}");
    }

    #[test]
    fn clip_times_follow_the_engine_rounding() {
        // AK: 650 rpm over a 10-frame clip with its mark at frame 1.
        let scale = (650.0f32 / 60.0) / (1.0 / (10.0 * (1.0 / 30.0)));
        assert_eq!(clip_ms(1000, 0.0, 1.0 / 30.0, scale), 1009);
        assert_eq!(clip_ms(1000, 0.0, 10.0 / 30.0, scale), 1092);
    }

    #[test]
    fn a_drawn_weapon_fires_only_after_its_show_clip() {
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = arsenal(&data, [Some(weapon(&data, 13, 7, 90)), None]);
        // Fire held from the first frame: the show clip (600 ms) ends first.
        let shots = hold(&mut ak, bits::FIRE, 0, 700);
        assert_eq!(times(&shots)[0], 609, "{:?}", times(&shots));
    }

    #[test]
    fn an_automatic_fires_on_its_clip_until_the_magazine_is_empty() {
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = drawn_ak(&data);
        // The press arrives with the frame at 1008 and counts from the
        // previous frame (992); the mark is 9 ms in, a pass lasts 92 ms.
        let shots = hold(&mut ak, bits::FIRE, 1008, 3700);
        let at = times(&shots);
        assert_eq!(at.len(), 30, "{at:?}");
        assert_eq!(at[..3], [1001, 1093, 1185]);
        // The last round leaves with the one before it: the clip turns
        // play-once there and keeps its phase on the mark.
        assert_eq!((at[28], at[29]), (3577, 3577));
        assert_eq!(ak.active_weapon().unwrap().ammo_in_magazine, 0);
        // At that clip's end (3660) the empty weapon reloads by itself for
        // 2.5 s (2499 ms in single precision), then fires on.
        let shots = hold(&mut ak, bits::FIRE, 3716, 6400);
        assert_eq!(times(&shots), [6168, 6260, 6352]);
        let weapon = ak.active_weapon().unwrap();
        assert_eq!(weapon.ammo[0].2, 60);
        assert_eq!(weapon.ammo_in_magazine, 27);
    }

    #[test]
    fn a_long_client_stall_resumes_the_clip_where_it_was() {
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = drawn_ak(&data);
        assert_eq!(
            times(&hold(&mut ak, bits::FIRE, 1008, 1296)),
            [1001, 1093, 1185, 1277]
        );
        // 20 s without a frame: the clip waits instead of firing the gap.
        let shots = hold(&mut ak, bits::FIRE, 21_296, 21_400);
        assert_eq!(times(&shots), [21_369]);
        assert_eq!(ak.active_weapon().unwrap().ammo_in_magazine, 25);
    }

    #[test]
    fn a_weapon_drawn_again_takes_its_first_ammunition() {
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = weapon(&data, 13, 7, 90);
        ak.ammo.push((7, data.ammo(7).unwrap().clone(), 30));
        let rem = weapon(&data, 14, 51, 20);
        let mut arsenal = arsenal(&data, [Some(ak), Some(rem)]);
        hold(&mut arsenal, 0, 0, 992);
        hold(&mut arsenal, bits::AMMO_TYPE, 1008, 1008);
        hold(&mut arsenal, 0, 1024, 1200);
        assert_eq!(arsenal.active_weapon().unwrap().current_ammo, 1);
        hold(&mut arsenal, bits::WEAPON_2, 1216, 1216);
        hold(&mut arsenal, 0, 1232, 2600);
        assert_eq!(arsenal.active_weapon().unwrap().dict_id, 14);
        hold(&mut arsenal, bits::WEAPON_1, 2616, 2616);
        hold(&mut arsenal, 0, 2632, 4000);
        let ak = arsenal.active_weapon().unwrap();
        assert_eq!((ak.dict_id, ak.current_ammo), (13, 0));
    }

    #[test]
    fn single_fire_needs_a_new_trigger_pull() {
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = drawn_ak(&data);
        // Switch the AK to single shots.
        hold(&mut ak, bits::FIRE_MODE, 1008, 1008);
        hold(&mut ak, 0, 1024, 1200);
        assert_eq!(hold(&mut ak, bits::FIRE, 1216, 2000).len(), 1);
        hold(&mut ak, 0, 2016, 2100);
        assert_eq!(hold(&mut ak, bits::FIRE, 2116, 2500).len(), 1);
    }

    #[test]
    fn a_bolt_action_cycles_its_bolt_after_each_shot() {
        let Some(data) = game_data() else {
            return;
        };
        let mut rem = arsenal(&data, [Some(weapon(&data, 14, 51, 20)), None]);
        let weapon = rem.active_weapon().unwrap();
        assert_eq!((weapon.ammo_in_magazine, weapon.round_chambered), (4, true));
        hold(&mut rem, 0, 0, 992);
        // 120 rpm: the mark 50 ms in, the shot clip ends at 500 ms, then the
        // bolt clip takes 1333 ms. Pulls before then are lost.
        let shots = hold(&mut rem, bits::FIRE, 1008, 1100);
        assert_eq!(times(&shots), [1042]);
        hold(&mut rem, 0, 1116, 1200);
        assert!(hold(&mut rem, bits::FIRE, 1216, 1300).is_empty());
        // The bolt clip runs from 1492 to 2825.
        hold(&mut rem, 0, 1316, 2816);
        assert!(!rem.active_weapon().unwrap().round_chambered);
        hold(&mut rem, 0, 2832, 2900);
        let weapon = rem.active_weapon().unwrap();
        assert_eq!((weapon.ammo_in_magazine, weapon.round_chambered), (3, true));
        assert_eq!(weapon.state, State::Idle);
        // The pull counts from the last idle frame, 2896.
        let shots = hold(&mut rem, bits::FIRE, 2912, 3000);
        assert_eq!(times(&shots), [2946]);
    }

    #[test]
    fn shotguns_fire_their_pellets() {
        let Some(data) = game_data() else {
            return;
        };
        let mut toz = arsenal(&data, [Some(weapon(&data, 16, 22, 20)), None]);
        hold(&mut toz, 0, 0, 992);
        let shots = hold(&mut toz, bits::FIRE, 1008, 1100);
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].pellets.len(), 12);
    }

    #[test]
    fn a_pump_shotgun_reloads_round_by_round_and_chambers_at_the_end() {
        let Some(data) = game_data() else {
            return;
        };
        let mut rem = arsenal(&data, [Some(weapon(&data, 15, 22, 20)), None]);
        hold(&mut rem, 0, 0, 992);
        // Two shots, each followed by its pump.
        hold(&mut rem, bits::FIRE, 1008, 1100);
        hold(&mut rem, 0, 1116, 2600);
        hold(&mut rem, bits::FIRE, 2616, 2700);
        hold(&mut rem, 0, 2716, 4400);
        let weapon = rem.active_weapon().unwrap();
        assert_eq!((weapon.ammo_in_magazine, weapon.round_chambered), (3, true));
        // Reload: the chambered round goes back, then three rounds 500 ms
        // apart after the 1166 ms start, the 600 ms finish, and the chamber.
        hold(&mut rem, bits::RELOAD, 4416, 4416);
        hold(&mut rem, 0, 4432, 4432 + 1166 + 1000);
        assert!(!rem.active_weapon().unwrap().round_chambered);
        hold(&mut rem, 0, 6608, 8000);
        let weapon = rem.active_weapon().unwrap();
        assert_eq!((weapon.ammo_in_magazine, weapon.round_chambered), (5, true));
        assert_eq!(weapon.ammo[0].2, 18);
        assert_eq!(weapon.state, State::Idle);
    }

    #[test]
    fn a_burst_kicks_the_camera_up() {
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = drawn_ak(&data);
        let shots = hold(&mut ak, bits::FIRE, 1008, 1500);
        // The first shot leaves from a resting camera; later ones do not.
        assert_eq!(shots[0].recoil, RESTING_RECOIL);
        let (_, vertical, _) = ak.active_weapon().unwrap().recoil.clip_times();
        assert!(vertical > 0.5, "{vertical}");
        // Seen through the real clips: the first-person camera points higher.
        let mut resources = crate::combat::resources::tests::game_resources().unwrap();
        let model = crate::combat::pose::PlayerModel::load(
            &mut resources,
            &[(13, "gameplay/weapons/ak_74u.options".to_owned())],
        )
        .unwrap();
        let camera = |recoil| {
            let request = crate::combat::pose::PoseRequest {
                recoil,
                ..crate::combat::pose::PoseRequest::at_rest(
                    crate::combat::pose::View::FirstPerson,
                    13,
                )
            };
            model.pose(&request).unwrap().head_matrix().k.y
        };
        assert!(camera(ak.active_weapon().unwrap().recoil.clip_times()) > camera(RESTING_RECOIL));
    }

    #[test]
    fn the_camera_settles_between_single_shots() {
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = drawn_ak(&data);
        let mut t = 1008;
        let mut taps = 0;
        for _ in 0..8 {
            taps += hold(&mut ak, bits::FIRE, t, t + 16).len();
            // Each shot still kicks the camera up first.
            hold(&mut ak, 0, t + 32, t + 80);
            let (_, kicked, _) = ak.active_weapon().unwrap().recoil.clip_times();
            assert!(kicked > 0.51, "tap {taps}: kicked only to {kicked}");
            hold(&mut ak, 0, t + 96, t + 1500);
            t += 1504;
            // A second and a half after each shot the camera is back at rest,
            // so the kick does not build up over a series.
            let (horizontal, vertical, back) = ak.active_weapon().unwrap().recoil.clip_times();
            assert!(
                (vertical - 0.5).abs() < 1e-3,
                "tap {taps}: vertical {vertical}"
            );
            assert!(
                (horizontal - 0.5).abs() < 1e-3,
                "tap {taps}: horizontal {horizontal}"
            );
            assert!(back < 1e-3, "tap {taps}: back {back}");
        }
        assert_eq!(taps, 8);
    }

    #[test]
    fn switching_weapons_hides_then_shows() {
        let Some(data) = game_data() else {
            return;
        };
        let mut both = arsenal(
            &data,
            [
                Some(weapon(&data, 13, 7, 90)),
                Some(weapon(&data, 14, 51, 20)),
            ],
        );
        hold(&mut both, 0, 0, 992);
        hold(&mut both, bits::WEAPON_2, 1008, 1008);
        assert_eq!(both.requested_slot(), 1);
        // Hide from 992 to 1592, show to 2192, then the Rem 700's mark.
        assert!(hold(&mut both, bits::FIRE, 1024, 2176).is_empty());
        let shots = hold(&mut both, bits::FIRE, 2192, 2400);
        assert_eq!(times(&shots), [2242]);
        assert_eq!(shots[0].weapon_dict_id, 14);
    }

    #[test]
    fn sprinting_holds_fire_until_its_clip_has_faded() {
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = drawn_ak(&data);
        hold(&mut ak, bits::FORWARD | bits::SPRINT, 1008, 2000);
        assert_eq!(ak.stance(), Some(Stance::Sprint));
        // Fire stops the sprint, but the weapon waits for the sprint clip's
        // 300 ms fade, from the selection at 2000.
        let shots = hold(
            &mut ak,
            bits::FORWARD | bits::SPRINT | bits::FIRE,
            2016,
            2400,
        );
        assert_eq!(times(&shots)[0], 2309, "{:?}", times(&shots));
        assert!(ak.stamina().value() < 100.0);
    }

    #[test]
    fn a_standing_jump_holds_fire_until_landed() {
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = drawn_ak(&data);
        hold(&mut ak, bits::JUMP, 1008, 1008);
        assert_eq!(ak.stance(), Some(Stance::Jump));
        // The jump runs 800 + 400 ms from 992 and spends 20 stamina.
        let shots = hold(&mut ak, bits::FIRE, 1024, 2400);
        assert_eq!(times(&shots)[0], 2192 + 9, "{:?}", times(&shots));
        // Regeneration starts a second after the spending, at 10 a second.
        let stamina = ak.stamina().value();
        assert!((80.0..85.0).contains(&stamina), "{stamina}");
    }

    #[test]
    fn a_weapon_without_ammunition_fires_nothing() {
        let Some(data) = game_data() else {
            return;
        };
        let mut state = weapon(&data, 13, 7, 90);
        state.ammo.clear();
        let mut ak = arsenal(&data, [Some(state), None]);
        hold(&mut ak, 0, 0, 992);
        assert!(hold(&mut ak, bits::FIRE, 1008, 2000).is_empty());
        // The rounds still leave the magazine, as on the client.
        assert!(ak.active_weapon().unwrap().ammo_in_magazine < 30);
    }

    /// `character_breath_holding_params` as `default.player` ships them.
    const BREATH: BreathParams = BreathParams {
        max_breath_holding_time: 6.0,
        shortbreathing_repair_time: 3.0,
        breath_holding_multiplier: 0.1,
        shortbreathing_multiplier: 3.0,
        multiplier_increase_speed: 6.0,
        multiplier_decrease_speed: 2.0,
        horizontal_amplitude: 0.02,
        vertical_amplitude: 0.01,
        horizontal_period: 12.0,
        vertical_period: 6.0,
    };

    /// The sway at `ms` with this multiplier, in the client's single precision.
    fn sway(ms: u32, multiplier: f32) -> (f32, f32) {
        let t = ms_times(ms, 0.001) * std::f32::consts::TAU;
        (
            (t / 12.0).sin() * 0.02 * multiplier,
            (t / 6.0).sin() * 0.01 * multiplier,
        )
    }

    #[test]
    fn milliseconds_scale_with_one_rounding() {
        // Exact in double precision below 2^29 ms, so rounded once.
        for ms in [0, 1, 16, 1500, 2345, 123_456, 100_000_000] {
            assert_eq!(
                ms_times(ms, 0.001),
                (f64::from(ms) * f64::from(0.001f32)) as f32
            );
        }
    }

    #[test]
    fn the_breath_sways_as_two_sines_of_the_client_clock() {
        let sway_at = |ms: u32| {
            let mut breath = Breath::default();
            breath.reset(&BREATH);
            breath.tick(ms, 0, &BREATH);
            assert_eq!(breath.multiplier(), 1.0);
            breath.sway()
        };
        let close = |(h, v): (f32, f32), (eh, ev): (f32, f32)| {
            assert!(
                (h - eh).abs() < 1e-6 && (v - ev).abs() < 1e-6,
                "({h}, {v}) vs ({eh}, {ev})"
            );
        };
        // Horizontal over 12 s and vertical over 6 s, both from client time
        // 0: a figure eight.
        let diagonal = 0.02 * std::f32::consts::FRAC_1_SQRT_2;
        close(sway_at(0), (0.0, 0.0));
        close(sway_at(1500), (diagonal, 0.01));
        close(sway_at(3000), (0.02, 0.0));
        close(sway_at(4500), (diagonal, -0.01));
        close(sway_at(7500), (-diagonal, 0.01));
        close(sway_at(9000), (-0.02, 0.0));
        close(sway_at(12000), (0.0, 0.0));
        assert_eq!(sway_at(2345), sway(2345, 1.0));
    }

    #[test]
    fn holding_the_breath_calms_the_sway_until_it_runs_out() {
        let hold = bits::AIM | bits::HOLD_BREATH;
        let mut breath = Breath::default();
        breath.reset(&BREATH);
        breath.tick(1000, 0, &BREATH);
        // The key alone, without aiming, does not hold the breath.
        breath.tick(1000, bits::HOLD_BREATH, &BREATH);
        assert_eq!(breath.state, BreathState::Normal);

        // The multiplier falls to 0.1 at 2 a second; the breath runs down.
        breath.tick(1000, hold, &BREATH);
        assert_eq!(breath.state, BreathState::Holding);
        breath.tick(1100, hold, &BREATH);
        assert!((breath.multiplier - 0.8).abs() < 1e-6, "{breath:?}");
        assert!((breath.breath - 5.9).abs() < 1e-6, "{breath:?}");
        breath.tick(1600, hold, &BREATH);
        assert_eq!(breath.multiplier, 0.1);
        assert_eq!(breath.sway(), sway(1600, 0.1));

        // Let go: normal breathing, the multiplier back up at 6 a second,
        // the breath refilling at 1 a second.
        breath.tick(2000, 0, &BREATH);
        assert_eq!(breath.state, BreathState::Normal);
        assert_eq!(breath.multiplier, 1.0);
        assert!((breath.breath - 5.8).abs() < 1e-5, "{breath:?}");

        // Held until it runs out: short breathing, whatever the key says,
        // until the breath is full again 3 s later.
        breath.tick(2000, hold, &BREATH);
        breath.tick(8000, hold, &BREATH);
        assert_eq!(breath.state, BreathState::Short);
        assert_eq!((breath.breath, breath.multiplier), (0.0, 0.1));
        breath.tick(8100, hold, &BREATH);
        assert!((breath.multiplier - 0.7).abs() < 1e-6, "{breath:?}");
        assert!((breath.breath - 0.2).abs() < 1e-6, "{breath:?}");
        breath.tick(8500, 0, &BREATH);
        assert_eq!(breath.state, BreathState::Short);
        assert_eq!(breath.multiplier, 3.0);
        assert_eq!(breath.sway(), sway(8500, 3.0));
        breath.tick(10900, hold, &BREATH);
        assert_eq!(breath.state, BreathState::Short);
        breath.tick(11100, 0, &BREATH);
        assert_eq!(breath.state, BreathState::Normal);
        assert_eq!((breath.breath, breath.multiplier), (6.0, 3.0));
        // Back down to 1 at 2 a second.
        breath.tick(11600, 0, &BREATH);
        assert!((breath.multiplier - 2.0).abs() < 1e-6, "{breath:?}");
        breath.tick(12200, 0, &BREATH);
        assert_eq!(breath.multiplier, 1.0);

        // Drawing the weapon again restores the breath, not the clock.
        breath.tick(12300, hold, &BREATH);
        breath.reset(&BREATH);
        assert_eq!(breath.state, BreathState::Normal);
        assert_eq!((breath.breath, breath.multiplier), (6.0, 1.0));
        assert_eq!(breath.last_ms, 12300);
    }

    #[test]
    fn hip_fire_has_no_breath_sway() {
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = drawn_ak(&data);
        hold(&mut ak, 0, 1008, 2992);
        // The breath sways on, but only an aimed weapon draws it.
        assert!(ak.active_weapon().unwrap().breath.sway().0 > 0.019);
        assert_eq!(ak.drawn_recoil, RESTING_RECOIL);
        let shots = hold(&mut ak, bits::FIRE, 3008, 3008);
        assert_eq!(shots[0].recoil, RESTING_RECOIL);
        assert!(!shots[0].sway.aimed);
    }

    #[test]
    fn an_aimed_shot_leaves_the_swaying_camera() {
        use crate::combat::pose::{PlayerModel, PoseRequest, View};
        let Some(data) = game_data() else {
            return;
        };
        let mut ak = drawn_ak(&data);
        hold(&mut ak, bits::AIM, 1008, 7504);
        let weapon = ak.active_weapon().unwrap();
        assert!(weapon.aimed);
        // Near 7.5 s the vertical sine peaks and the horizontal one is at
        // -1/√2.
        let (h, v) = weapon.breath.sway();
        assert_eq!((h, v), sway(7504, 1.0));
        assert!((v - 0.01).abs() < 1e-4 && (h + 0.01414).abs() < 1e-4);
        // The shot leaves the camera of the frame before it.
        let shots = hold(&mut ak, bits::AIM | bits::FIRE, 7520, 7520);
        let shot = &shots[0];
        assert!(shot.sway.aimed);
        assert_eq!((shot.sway.horizontal, shot.sway.vertical), (h, v));
        assert_eq!(shot.recoil, (0.5 - h, v + 0.5, EPSILON_7));

        // Through the real clips: about 0.747 degrees of yaw per 0.02 of
        // horizontal coefficient and 0.43 degrees of pitch (up) per 0.01 of
        // vertical.
        let mut resources = crate::combat::resources::tests::game_resources().unwrap();
        let model = PlayerModel::load(
            &mut resources,
            &[(13, "gameplay/weapons/ak_74u.options".to_owned())],
        )
        .unwrap();
        let look = |recoil| {
            let request = PoseRequest {
                aimed: true,
                recoil,
                ..PoseRequest::at_rest(View::FirstPerson, 13)
            };
            let k = model.pose(&request).unwrap().head_matrix().k;
            (
                k.x.atan2(k.z).to_degrees(),
                k.y.clamp(-1.0, 1.0).asin().to_degrees(),
            )
        };
        let (yaw, pitch) = look(shot.recoil);
        let (rest_yaw, rest_pitch) = look(RESTING_RECOIL);
        let (yaw, pitch) = ((yaw - rest_yaw).abs(), pitch - rest_pitch);
        let expected = (0.747 * h.abs() / 0.02, 0.43 * v / 0.01);
        assert!(
            (yaw - expected.0).abs() < 0.02 && (pitch - expected.1).abs() < 0.02,
            "turned {yaw} deg aside, {pitch} up; expected {expected:?}"
        );
    }
}
