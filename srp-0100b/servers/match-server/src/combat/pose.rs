// Player poses from the game's own animation clips, so the server aims and
// places hit boxes the way clients draw them:
//
// * `.b-spline` clips: per bone, nine clamped cubic B-spline channels
//   (translation, Euler rotation, scale) in frames at 30 fps
//   (`animation/sources/bi_spline_*`, `poly_curve_inline.h`);
// * layers mixed as `bone_matrices_computer.cpp` does: the movement clip, the
//   recoil clips and the additive look-pitch clip;
// * the skeleton from `animations/skeletons/<name>.skeleton`, chained from
//   `Root` (the root-motion bone is skipped);
// * the camera as `animation::calculated_head_matrix` (aim origin and
//   direction), hit boxes as `animated_rigid_body` places them.
//
// Not modelled: hand and leg IK (hands may sit a few cm off), clip blend
// transitions, and the exact phase of a looping movement clip.

use std::collections::HashMap;

use binary_config_parser::ConfigValue;

use super::collision::{Placed, Shape};
use super::math::{Mat4, Vec3};
use super::resources::{ResourceError, Resources};

const FPS: f32 = 30.0;
const KNOT_EPSILON: f32 = 1e-7;
const EPSILON_5: f32 = 1e-5;
const EPSILON_7: f32 = 1e-7;

const MODEL: &str = "models/character/human/scavengers_01/scavengers_01.skinned_model";

/// Movement direction → index base in a 27-entry clip list
/// (`player_logic_base_state`): idle, forward, forward-right, right,
/// back-right, back, back-left, left, forward-left; +2 is its look clip.
const MOVES: [usize; 9] = [0, 3, 6, 9, 12, 15, 18, 21, 24];
const RECOIL_VERTICAL: usize = 0;
const RECOIL_HORIZONTAL: usize = 1;
const RECOIL_BACK: usize = 2;

// ------------------------------------------------------------- quaternions

#[derive(Clone, Copy, Debug, PartialEq)]
struct Quat {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

impl Quat {
    const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    /// `quaternion( float3 angles )`; its matrix equals `create_rotation`.
    fn from_euler(a: Vec3) -> Self {
        let (sx, cx) = (a.x * 0.5).sin_cos();
        let (sy, cy) = (a.y * 0.5).sin_cos();
        let (sz, cz) = (a.z * 0.5).sin_cos();
        let (xs_yc, xc_ys, xc_yc, xs_ys) = (sx * cy, cx * sy, cx * cy, sx * sy);
        Self {
            x: -xs_yc * cz - xc_ys * sz,
            y: -xc_ys * cz + xs_yc * sz,
            z: -xc_yc * sz - xs_ys * cz,
            w: -xc_yc * cz + xs_ys * sz,
        }
    }

    /// `R(l * r) == R(l) * R(r)`.
    fn mul(self, r: Self) -> Self {
        let l = self;
        Self {
            x: l.w * r.x + l.x * r.w + l.y * r.z - l.z * r.y,
            y: l.w * r.y - l.x * r.z + l.y * r.w + l.z * r.x,
            z: l.w * r.z + l.x * r.y - l.y * r.x + l.z * r.w,
            w: l.w * r.w - l.x * r.x - l.y * r.y - l.z * r.z,
        }
    }

    fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    fn axis_angle(self) -> (Vec3, f32) {
        let s = (self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if s > 1e-7 {
            (
                Vec3::new(self.x / s, self.y / s, self.z / s),
                2.0 * s.atan2(self.w),
            )
        } else {
            (Vec3::new(0.0, 0.0, 1.0), 0.0)
        }
    }

    fn from_axis_angle(axis: Vec3, angle: f32) -> Self {
        let (s, c) = (angle * 0.5).sin_cos();
        Self {
            x: axis.x * s,
            y: axis.y * s,
            z: axis.z * s,
            w: c,
        }
    }

    fn scaled(self, weight: f32) -> Self {
        let (axis, angle) = self.axis_angle();
        Self::from_axis_angle(axis, angle * weight)
    }

    /// `slerp_optimized` (`core/sources/math_quaternion.cpp`).
    fn slerp(self, other: Self, t: f32) -> Self {
        let mut cosom = self.dot(other);
        let mut sign = 1.0;
        if cosom < 0.0 {
            cosom = -cosom;
            sign = -1.0;
        }
        let (s0, mut s1) = if cosom < 1.0 - EPSILON_5 {
            let omega = cosom.acos();
            let inv = 1.0 / omega.sin();
            ((omega - t * omega).sin() * inv, (t * omega).sin() * inv)
        } else {
            (1.0 - t, t)
        };
        s1 *= sign;
        Self {
            x: self.x * s0 + other.x * s1,
            y: self.y * s0 + other.y * s1,
            z: self.z * s0 + other.z * s1,
            w: self.w * s0 + other.w * s1,
        }
    }

    fn matrix(self) -> Mat4 {
        let Self { x, y, z, w } = self;
        let (xx, yy, zz) = (x * x, y * y, z * z);
        let (xy, xz, yz) = (x * y, x * z, y * z);
        let (wx, wy, wz) = (w * x, w * y, w * z);
        Mat4 {
            i: Vec3::new(1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy)),
            j: Vec3::new(2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx)),
            k: Vec3::new(2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy)),
            c: Vec3::ZERO,
        }
    }
}

// ------------------------------------------------------------------ curves

/// One channel as per-span cubic polynomials (`poly_curve`).
#[derive(Clone, Debug)]
struct Curve {
    knots: Vec<f32>,
    coefficients: Vec<[f32; 4]>,
    constant: Option<f32>,
}

impl Curve {
    fn new(points: &[(f32, f32)]) -> Self {
        let n = points.len();
        let knot = |i: usize| points[i.min(n.saturating_sub(1))].0;
        let point = |i: usize| points[i.min(n.saturating_sub(1))].1;
        let mut knots = Vec::new();
        let mut coefficients = Vec::new();
        if n >= 5 {
            for i in 3..n - 1 {
                if knot(i + 1) - knot(i) < KNOT_EPSILON {
                    continue;
                }
                coefficients.push(spline_params(&knot, &point, i));
                knots.push(knot(i));
            }
            knots.push(knot(n - 1));
        }
        let constant = if coefficients.is_empty() {
            Some(if n > 0 { point(n - 1) } else { 0.0 })
        } else {
            None
        };
        Self {
            knots,
            coefficients,
            constant,
        }
    }

    fn range(&self) -> (f32, f32) {
        match (self.knots.first(), self.knots.last()) {
            (Some(first), Some(last)) => (*first, *last),
            _ => (0.0, 0.0),
        }
    }

    fn evaluate(&self, t: f32) -> f32 {
        if let Some(value) = self.constant {
            return value;
        }
        let (first, last) = self.range();
        let clamped = t.clamp(first, last);
        // The span containing t: the last knot not after it.
        let span = match self.knots.partition_point(|&k| k <= clamped) {
            0 => 0,
            p => (p - 1).min(self.coefficients.len() - 1),
        };
        let c = self.coefficients[span];
        // The engine evaluates with the unclamped time.
        let x = t - self.knots[span];
        ((c[3] * x + c[2]) * x + c[1]) * x + c[0]
    }
}

/// `get_spline_params` (`poly_curve_inline.h`): the cubic for span `index`.
fn spline_params(
    knot: &dyn Fn(usize) -> f32,
    point: &dyn Fn(usize) -> f32,
    index: usize,
) -> [f32; 4] {
    let dt = [
        knot(index - 1) - knot(index - 2),
        knot(index) - knot(index - 1),
        knot(index + 1) - knot(index),
        knot(index + 2) - knot(index + 1),
        knot(index + 3) - knot(index + 2),
    ];
    let j = 1;
    let k0 = 1.0 / (dt[j + 1] * (dt[j + 1] + dt[j]) * (dt[j + 1] + dt[j] + dt[j - 1]));
    let k10 = k0;
    let k11 = 1.0 / (dt[j + 1] * (dt[j + 1] + dt[j]) * (dt[j + 1] + dt[j] + dt[j + 2]));
    let k12 = 1.0 / (dt[j + 1] * (dt[j + 1] + dt[j + 2]) * (dt[j + 1] + dt[j] + dt[j + 2]));
    let k20 = k11;
    let k21 = k12;
    let k22 = 1.0 / (dt[j + 1] * (dt[j + 1] + dt[j + 2]) * (dt[j + 1] + dt[j + 2] + dt[j + 3]));
    let k3 = k22;
    let sq = |v: f32| v * v;
    let f0 = [k3, -k20 - k21 - k22, k10 + k11 + k12, -k0];
    let f1 = [
        0.0,
        k20 * (dt[j + 1] - 2.0 * dt[j])
            + k21 * (dt[j + 1] + dt[j + 2] - dt[j])
            + k22 * (dt[j + 1] + dt[j + 2] + dt[j + 3]),
        k10 * (dt[j - 1] + dt[j] - 2.0 * dt[j + 1])
            + k11 * (-2.0 * dt[j + 1] - dt[j + 2] + dt[j])
            + k12 * (-2.0 * (dt[j + 1] + dt[j + 2])),
        k0 * 3.0 * dt[j + 1],
    ];
    let f2 = [
        0.0,
        k20 * (2.0 * dt[j] * dt[j + 1] - sq(dt[j])) + k21 * (dt[j] * (dt[j + 1] + dt[j + 2])),
        k10 * (sq(dt[j + 1]) - 2.0 * dt[j + 1] * (dt[j - 1] + dt[j]))
            + k11 * (-dt[j] * dt[j + 1] + (dt[j + 1] - dt[j]) * (dt[j + 1] + dt[j + 2]))
            + k12 * sq(dt[j + 1] + dt[j + 2]),
        -k0 * 3.0 * sq(dt[j + 1]),
    ];
    let f3 = [
        0.0,
        k20 * (dt[j + 1] * sq(dt[j])),
        k10 * (sq(dt[j + 1]) * (dt[j - 1] + dt[j]))
            + k11 * (dt[j + 1] * dt[j] * (dt[j + 1] + dt[j + 2])),
        k0 * dt[j + 1] * sq(dt[j + 1]),
    ];
    let p = [
        point(index),
        point(index - 1),
        point(index - 2),
        point(index - 3),
    ];
    let dot = |f: [f32; 4]| f[0] * p[0] + f[1] * p[1] + f[2] * p[2] + f[3] * p[3];
    [dot(f3), dot(f2), dot(f1), dot(f0)]
}

// -------------------------------------------------------------------- clips

struct BoneFrame {
    translation: Vec3,
    rotation: Vec3,
    scale: Vec3,
}

const IDENTITY_FRAME: BoneFrame = BoneFrame {
    translation: Vec3::ZERO,
    rotation: Vec3::ZERO,
    scale: Vec3::new(1.0, 1.0, 1.0),
};

pub struct Clip {
    bones: Vec<[Curve; 9]>,
    index: HashMap<String, usize>,
    length_frames: f32,
}

impl Clip {
    /// A `.b-spline` file and its `.bones_names` config.
    fn parse(bytes: &[u8], names: &ConfigValue) -> Result<Self, ResourceError> {
        let error = |what: &str| ResourceError(format!("animation clip: {what}"));
        let u16_at = |at: usize| -> Result<u16, ResourceError> {
            Ok(u16::from_le_bytes(
                bytes
                    .get(at..at + 2)
                    .ok_or_else(|| error("truncated"))?
                    .try_into()
                    .unwrap(),
            ))
        };
        let u32_at = |at: usize| -> Result<u32, ResourceError> {
            Ok(u32::from_le_bytes(
                bytes
                    .get(at..at + 4)
                    .ok_or_else(|| error("truncated"))?
                    .try_into()
                    .unwrap(),
            ))
        };
        let f32_at = |at: usize| -> Result<f32, ResourceError> { Ok(f32::from_bits(u32_at(at)?)) };

        let bone_count = usize::from(u16_at(0)?);
        let event_count = usize::from(*bytes.get(2).ok_or_else(|| error("truncated"))?);
        let mut at = 4 + bone_count * 0x48 + event_count * 0x10;
        let mut bones = Vec::with_capacity(bone_count);
        for _ in 0..bone_count {
            let mut channels = Vec::with_capacity(9);
            for _ in 0..9 {
                let n = u32_at(at)? as usize;
                at += 4;
                if n > 100_000 {
                    return Err(error("implausible key count"));
                }
                let mut points = Vec::with_capacity(n);
                for i in 0..n {
                    points.push((f32_at(at + 8 * i)?, f32_at(at + 8 * i + 4)?));
                }
                at += 8 * n;
                channels.push(Curve::new(&points));
            }
            bones.push(channels.try_into().map_err(|_| error("channels"))?);
        }
        let names: Vec<String> = names
            .req("bones_names")?
            .children()
            .iter()
            .map(|name| name.as_str().unwrap_or_default().to_owned())
            .collect();
        if names.len() != bone_count {
            return Err(error("bone name count mismatch"));
        }
        let index = names.into_iter().enumerate().map(|(i, n)| (n, i)).collect();
        // `cubic_spline_skeleton_animation::length_in_frames`: bone 0's tx.
        let length_frames = bones
            .first()
            .map(|channels: &[Curve; 9]| {
                let (first, last) = channels[0].range();
                last - first
            })
            .unwrap_or(0.0);
        Ok(Self {
            bones,
            index,
            length_frames,
        })
    }

    fn frame(&self, bone: &str, frame: f32) -> BoneFrame {
        let Some(&i) = self.index.get(bone) else {
            return IDENTITY_FRAME;
        };
        let c = &self.bones[i];
        let v = |k: usize| c[k].evaluate(frame);
        BoneFrame {
            translation: Vec3::new(v(0), v(1), v(2)),
            rotation: Vec3::new(v(3), v(4), v(5)),
            scale: Vec3::new(v(6), v(7), v(8)),
        }
    }
}

/// One event channel of a clip (`animation_event_channels`): marks at
/// `knots` (frames), each span carrying a domain value.
#[derive(Clone, Debug, PartialEq)]
pub struct EventChannel {
    pub name: String,
    pub knots: Vec<f32>,
    pub domains: Vec<u8>,
}

/// What weapon logic needs of a clip: its length and its event marks.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipTiming {
    pub length_frames: f32,
    pub events: Vec<EventChannel>,
}

impl ClipTiming {
    pub fn load(resources: &mut Resources, path: &str) -> Result<Self, ResourceError> {
        let path = path.strip_prefix("resources/").unwrap_or(path);
        let bytes = resources.read(&format!("{path}.b-spline"))?;
        Self::parse(&bytes).map_err(|e| ResourceError(format!("{path}: {e}")))
    }

    /// The `.b-spline` layout: header, bone and event slots, then per bone
    /// nine channels of (knot, value) pairs, then per event channel its
    /// knots, its domains and its name, padded to 4 bytes.
    fn parse(bytes: &[u8]) -> Result<Self, ResourceError> {
        let error = |what: &str| ResourceError(format!("animation clip: {what}"));
        let slice =
            |at: usize, len: usize| bytes.get(at..at + len).ok_or_else(|| error("truncated"));
        let u32_at = |at: usize| -> Result<u32, ResourceError> {
            Ok(u32::from_le_bytes(slice(at, 4)?.try_into().unwrap()))
        };
        let bone_count = usize::from(u16::from_le_bytes(slice(0, 2)?.try_into().unwrap()));
        let event_count = usize::from(slice(2, 1)?[0]);
        let headers = 4 + bone_count * 0x48;
        let mut at = headers + event_count * 0x10;
        let mut length_frames = 0.0;
        for bone in 0..bone_count {
            for channel in 0..9 {
                let n = u32_at(at)? as usize;
                at += 4;
                if n > 100_000 {
                    return Err(error("implausible key count"));
                }
                if bone == 0 && channel == 0 {
                    let points = (0..n)
                        .map(|i| {
                            Ok((
                                f32::from_bits(u32_at(at + 8 * i)?),
                                f32::from_bits(u32_at(at + 8 * i + 4)?),
                            ))
                        })
                        .collect::<Result<Vec<_>, ResourceError>>()?;
                    // `length_in_frames`: bone 0's tx.
                    let (first, last) = Curve::new(&points).range();
                    length_frames = last - first;
                }
                at += 8 * n;
            }
        }
        let mut events = Vec::with_capacity(event_count);
        for event in 0..event_count {
            let header = slice(headers + event * 0x10, 0x10)?;
            let knot_count = usize::from(u16::from_le_bytes([header[8], header[9]]));
            let domain_count = usize::from(header[10]);
            let knots = (0..knot_count)
                .map(|i| Ok(f32::from_bits(u32_at(at + 4 * i)?)))
                .collect::<Result<Vec<_>, ResourceError>>()?;
            at += 4 * knot_count;
            let domains = slice(at, domain_count)?.to_vec();
            let name_at = at + domain_count;
            let name_length = bytes
                .get(name_at..)
                .and_then(|rest| rest.iter().position(|&b| b == 0))
                .ok_or_else(|| error("unterminated event name"))?;
            let name = String::from_utf8_lossy(&bytes[name_at..name_at + name_length]).into_owned();
            at += (domain_count + name_length + 1 + 3) & !3;
            events.push(EventChannel {
                name,
                knots,
                domains,
            });
        }
        Ok(Self {
            length_frames,
            events,
        })
    }

    /// The frames of a channel's marks with the given domain value.
    pub fn marks(&self, channel: &str, domain: u8) -> Vec<f32> {
        self.events
            .iter()
            .filter(|event| event.name == channel)
            .flat_map(|event| {
                event
                    .knots
                    .iter()
                    .zip(&event.domains)
                    .filter(move |&(_, &d)| d == domain)
                    .map(|(&knot, _)| knot)
            })
            .collect()
    }
}

// ------------------------------------------------------------------ the model

struct Bone {
    name: String,
    /// Index into the posed bone list; `None` for children of the skipped
    /// root.
    parent: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct HitTarget {
    pub body_part: String,
    bone: usize,
    shape: Shape,
    local: Mat4,
}

/// Clip lists of one weapon's `.user_animations`.
struct ClipLists {
    sections: HashMap<String, Vec<String>>,
}

pub struct PlayerModel {
    bones: Vec<Bone>,
    head: usize,
    hit_targets: Vec<HitTarget>,
    clips: HashMap<String, Clip>,
    /// Per weapon dict id.
    clip_lists: HashMap<u16, ClipLists>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// The player's own camera: where shots come from.
    FirstPerson,
    /// How everyone else draws the player: where hit boxes are.
    ThirdPerson,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoseRequest {
    pub view: View,
    pub weapon_dict_id: u16,
    pub crouch: bool,
    pub aimed: bool,
    /// 0 idle, 1..=8 as `MOVES`.
    pub direction: usize,
    /// Seconds into the looping movement clip.
    pub movement_time: f32,
    pub look_pitch: f32,
    /// Recoil clip time fractions: horizontal, vertical, back (rest 0.5, 0.5,
    /// ~0).
    pub recoil: (f32, f32, f32),
}

impl PoseRequest {
    #[cfg(test)]
    pub fn at_rest(view: View, weapon_dict_id: u16) -> Self {
        Self {
            view,
            weapon_dict_id,
            crouch: false,
            aimed: false,
            direction: 0,
            movement_time: 0.0,
            look_pitch: 0.0,
            recoil: (0.5, 0.5, EPSILON_7),
        }
    }
}

/// Movement direction index from the input bits (forward 1, back 2, left 4,
/// right 8).
pub fn direction(mask: u32) -> usize {
    let forward = (mask & 1 != 0) as i32 - (mask & 2 != 0) as i32;
    let right = (mask & 8 != 0) as i32 - (mask & 4 != 0) as i32;
    match (forward, right) {
        (0, 0) => 0,
        (1, 0) => 1,
        (1, 1) => 2,
        (0, 1) => 3,
        (-1, 1) => 4,
        (-1, 0) => 5,
        (-1, -1) => 6,
        (0, -1) => 7,
        _ => 8,
    }
}

/// A computed pose in model space (feet at the origin, facing +Z).
pub struct Pose {
    bones: Vec<Mat4>,
    head: usize,
}

impl Pose {
    /// `animation::calculated_head_matrix` in model space.
    pub fn head_matrix(&self) -> Mat4 {
        let pre = Mat4::rotation_xyz(Vec3::new(0.0, std::f32::consts::FRAC_PI_2, 0.0))
            * Mat4::rotation_xyz(Vec3::new(0.0, 0.0, std::f32::consts::FRAC_PI_2));
        let mut m = pre * self.bones[self.head];
        m.c += m.j * 0.1;
        m
    }
}

impl PlayerModel {
    /// The scavenger model every player uses, with the clips of the given
    /// weapons.
    pub fn load(
        resources: &mut Resources,
        weapon_configs: &[(u16, String)],
    ) -> Result<Self, ResourceError> {
        let settings = resources.config(&format!("{MODEL}/skeleton"))?;
        let skeleton_path = format!("{}.skeleton", settings.str_at("skeleton")?);
        let bones = load_skeleton(&resources.config(&skeleton_path)?)?;
        let head = bones
            .iter()
            .position(|b| b.name == "Head")
            .ok_or_else(|| ResourceError("skeleton has no Head".to_owned()))?;

        let mut hit_targets = Vec::new();
        for target in resources
            .config(&format!("{MODEL}/hit_targets"))?
            .req("hit_targets")?
            .children()
        {
            let bone_name = target.str_at("animation_bone")?;
            let Some(bone) = bones.iter().position(|b| b.name == bone_name) else {
                continue;
            };
            let Some(shape) = Shape::from_engine(
                target.i32_at("type")?,
                Vec3::from_array(target.float3_at("scale")?),
            ) else {
                continue;
            };
            hit_targets.push(HitTarget {
                body_part: target.str_at("hit_param")?.to_owned(),
                bone,
                shape,
                local: Mat4::rotation_xyz(Vec3::from_array(target.float3_at("rotation")?))
                    * Mat4::translation(Vec3::from_array(target.float3_at("position")?)),
            });
        }

        let mut model = Self {
            bones,
            head,
            hit_targets,
            clips: HashMap::new(),
            clip_lists: HashMap::new(),
        };
        for (dict_id, weapon_config) in weapon_configs {
            let config = resources.config(weapon_config)?;
            let lists_path = config.str_at("user_animations")?.to_owned();
            let lists = resources.config(&lists_path)?;
            let mut sections = HashMap::new();
            for (name, clips) in lists.entries() {
                sections.insert(
                    name.clone(),
                    clips
                        .children()
                        .iter()
                        .map(|c| c.as_str().unwrap_or_default().to_owned())
                        .collect::<Vec<_>>(),
                );
            }
            let lists = ClipLists { sections };
            for section in [
                "stand",
                "crouch",
                "aimed_stand",
                "aimed_crouch",
                "stand_hud",
                "crouch_hud",
                "aimed_stand_hud",
                "aimed_crouch_hud",
            ] {
                if let Some(clips) = lists.sections.get(section) {
                    for path in clips.iter().take(27) {
                        model.load_clip(resources, path)?;
                    }
                }
            }
            for section in [
                "stand_hands_only",
                "crouch_hands_only",
                "aimed_stand_hands_only",
                "aimed_crouch_hands_only",
                "stand_hands_only_hud",
                "crouch_hands_only_hud",
                "aimed_stand_hands_only_hud",
                "aimed_crouch_hands_only_hud",
            ] {
                if let Some(clips) = lists.sections.get(section) {
                    for path in clips.iter().take(3) {
                        model.load_clip(resources, path)?;
                    }
                }
            }
            model.clip_lists.insert(*dict_id, lists);
        }
        Ok(model)
    }

    fn load_clip(&mut self, resources: &mut Resources, path: &str) -> Result<(), ResourceError> {
        if path.is_empty() || self.clips.contains_key(path) {
            return Ok(());
        }
        let bytes = resources.read(&format!("{path}.b-spline"))?;
        let names = resources.config(&format!("{path}.bones_names"))?;
        let clip =
            Clip::parse(&bytes, &names).map_err(|e| ResourceError(format!("{path}: {e}")))?;
        self.clips.insert(path.to_owned(), clip);
        Ok(())
    }

    fn clip_set(&self, request: &PoseRequest) -> Option<(&Clip, &Clip, [&Clip; 3])> {
        let lists = self
            .clip_lists
            .get(&request.weapon_dict_id)
            .or_else(|| self.clip_lists.values().next())?;
        let stance = if request.crouch { "crouch" } else { "stand" };
        let prefix = if request.aimed { "aimed_" } else { "" };
        let suffix = if request.view == View::FirstPerson {
            "_hud"
        } else {
            ""
        };
        let moves = lists.sections.get(&format!("{prefix}{stance}{suffix}"))?;
        let hands = lists
            .sections
            .get(&format!("{prefix}{stance}_hands_only{suffix}"))?;
        let i = MOVES[request.direction.min(8)];
        let clip = |path: &String| self.clips.get(path);
        Some((
            clip(moves.get(i)?)?,
            clip(moves.get(i + 2)?)?,
            [
                clip(hands.get(RECOIL_HORIZONTAL)?)?,
                clip(hands.get(RECOIL_VERTICAL)?)?,
                clip(hands.get(RECOIL_BACK)?)?,
            ],
        ))
    }

    /// The pose for a steady state (`bone_matrices_computer`): layer 0 the
    /// movement clip, layer 2 recoil back, layer 3 recoil horizontal and
    /// vertical, layer 4 the additive look clip.
    pub fn pose(&self, request: &PoseRequest) -> Option<Pose> {
        let (base, look, [horizontal, vertical, back]) = self.clip_set(request)?;
        let base_frame = if base.length_frames > 0.0 {
            (request.movement_time * FPS).rem_euclid(base.length_frames)
        } else {
            0.0
        };
        let look_factor = (request.look_pitch.clamp(-1.0, 1.0) / 2.0 + 0.5).min(1.0 - EPSILON_5);
        let (h, v, b) = request.recoil;
        let layers: [(&[(&Clip, f32)], usize); 4] = [
            (&[(base, base_frame)], 0),
            (
                &[(
                    back,
                    b.clamp(EPSILON_7, 1.0 - EPSILON_7) * back.length_frames,
                )],
                2,
            ),
            (
                &[
                    (horizontal, h * horizontal.length_frames),
                    (vertical, v * vertical.length_frames),
                ],
                3,
            ),
            (&[(look, look_factor * look.length_frames)], 4),
        ];
        let mut posed: Vec<Mat4> = Vec::with_capacity(self.bones.len());
        for bone in &self.bones {
            let mut translation = Vec3::ZERO;
            let mut rotation = Quat::IDENTITY;
            let mut scale = Vec3::new(1.0, 1.0, 1.0);
            for (clips, layer) in &layers {
                let (t, q, s) = layer_transform(&bone.name, clips, *layer);
                translation += t;
                rotation = rotation.mul(q);
                scale = Vec3::new(scale.x * s.x, scale.y * s.y, scale.z * s.z);
            }
            let local = Mat4::scale(scale) * rotation.matrix() * Mat4::translation(translation);
            let object = match bone.parent {
                Some(parent) => local * posed[parent],
                None => local,
            };
            posed.push(object);
        }
        Some(Pose {
            bones: posed,
            head: self.head,
        })
    }

    /// The hit boxes of `pose` placed by `character` (yaw, then position).
    pub fn place_hit_boxes(&self, pose: &Pose, character: &Mat4) -> Vec<(Placed, &str)> {
        self.hit_targets
            .iter()
            .filter_map(|target| {
                let world = target.local * pose.bones[target.bone] * *character;
                Placed::new(target.shape, world).map(|p| (p, target.body_part.as_str()))
            })
            .collect()
    }
}

/// `computed_local_bone_transform` for one layer with weight-1 clips.
fn layer_transform(bone: &str, clips: &[(&Clip, f32)], layer: usize) -> (Vec3, Quat, Vec3) {
    let frames: Vec<BoneFrame> = clips
        .iter()
        .map(|(clip, frame)| clip.frame(bone, *frame))
        .collect();
    let translation = frames.iter().fold(Vec3::ZERO, |sum, f| sum + f.translation);
    let rotations: Vec<Quat> = frames
        .iter()
        .map(|f| Quat::from_euler(f.rotation))
        .collect();
    let normalize = layer < 2;
    let rotation = match rotations[..] {
        [] => Quat::IDENTITY,
        [q] => {
            if normalize {
                q
            } else {
                q.scaled(1.0)
            }
        }
        [q0, q1] => {
            let q = q0.slerp(q1, 0.5);
            if normalize { q } else { q.scaled(2.0) }
        }
        _ => {
            let mut result = rotations[0];
            let mut weight = 1.0;
            for q in &rotations[1..] {
                result = result.slerp(*q, 1.0 / (1.0 + weight));
                weight += 1.0;
            }
            result
        }
    };
    let scale = frames.iter().fold(Vec3::new(1.0, 1.0, 1.0), |s, f| {
        Vec3::new(s.x * f.scale.x, s.y * f.scale.y, s.z * f.scale.z)
    });
    (translation, rotation, scale)
}

/// `skeleton_cook`: bones in parent-before-child order, the root-motion bone
/// skipped.
fn load_skeleton(config: &ConfigValue) -> Result<Vec<Bone>, ResourceError> {
    let (root_name, root) = config
        .entries()
        .first()
        .ok_or_else(|| ResourceError("empty skeleton".to_owned()))?;
    // Breadth-first by sibling blocks, as the cook allocates indices.
    let mut order: Vec<(String, Option<usize>, &ConfigValue)> =
        vec![(root_name.clone(), None, root)];
    let mut next = 0;
    while next < order.len() {
        let (_, _, value) = order[next];
        let children: Vec<(String, &ConfigValue)> = value
            .entries()
            .iter()
            .filter(|(_, v)| matches!(v, ConfigValue::Named(_) | ConfigValue::Indexed(_)))
            .map(|(k, v)| (k.clone(), v))
            .collect();
        for (name, child) in children {
            order.push((name, Some(next), child));
        }
        next += 1;
    }
    // Drop the root; reindex parents (children of the root have no parent).
    Ok(order
        .iter()
        .skip(1)
        .map(|(name, parent, _)| Bone {
            name: name.clone(),
            parent: parent.and_then(|p| p.checked_sub(1)),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::resources::tests::game_resources;

    fn model() -> Option<PlayerModel> {
        let mut resources = game_resources()?;
        Some(
            PlayerModel::load(
                &mut resources,
                &[(13, "gameplay/weapons/ak_74u.options".to_owned())],
            )
            .unwrap(),
        )
    }

    fn pitch_degrees(forward: Vec3) -> f32 {
        forward
            .y
            .atan2((forward.x * forward.x + forward.z * forward.z).sqrt())
            .to_degrees()
    }

    #[test]
    fn first_person_pitch_matches_the_decoded_clips() {
        let Some(model) = model() else {
            return;
        };
        for (crouch, pitch, expected) in [
            (false, -1.0, -75.43),
            (false, 0.0, 0.0),
            (false, 1.0, 70.74),
            (true, -1.0, -70.54),
            (true, 1.0, 72.98),
        ] {
            let request = PoseRequest {
                crouch,
                look_pitch: pitch,
                ..PoseRequest::at_rest(View::FirstPerson, 13)
            };
            let head = model.pose(&request).unwrap().head_matrix();
            let got = pitch_degrees(head.k);
            assert!(
                (got - expected).abs() < 0.05,
                "crouch {crouch} pitch {pitch}: {got}"
            );
        }
        let head = model
            .pose(&PoseRequest::at_rest(View::FirstPerson, 13))
            .unwrap()
            .head_matrix();
        assert!((head.c.y - 1.585).abs() < 0.005, "{:?}", head.c);
    }

    #[test]
    fn third_person_hit_boxes_stand_and_kneel() {
        let Some(model) = model() else {
            return;
        };
        let head_sphere = |crouch: bool| {
            let request = PoseRequest {
                crouch,
                ..PoseRequest::at_rest(View::ThirdPerson, 13)
            };
            let pose = model.pose(&request).unwrap();
            let boxes = model.place_hit_boxes(&pose, &Mat4::IDENTITY);
            let (sphere, _) = boxes
                .iter()
                .find(|(p, part)| *part == "head" && matches!(p.shape, Shape::Sphere { .. }))
                .unwrap();
            sphere.to_world.c
        };
        assert!(
            (head_sphere(false).y - 1.602).abs() < 0.01,
            "{:?}",
            head_sphere(false)
        );
        assert!(
            (head_sphere(true).y - 1.026).abs() < 0.01,
            "{:?}",
            head_sphere(true)
        );
    }
}
