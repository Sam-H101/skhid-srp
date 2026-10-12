// The game's own combat data, loaded from `resources.db`: weapons, ammo,
// armour, drugs, traps, the human damage model and the surface materials.
// Each loader names the engine code that reads the same keys.

use std::collections::HashMap;
use std::sync::Arc;

use binary_config_parser::ConfigValue;

use super::collision::Shape;
use super::math::{Mat4, Vec3};
use super::pose::ClipTiming;
use super::resources::{ResourceError, Resources};

const ITEMS_DICTIONARY: &str = "gameplay/db_static_dictionaries";
const HIT_PARAMS: &str = "gameplay/hit_params/human_hit_params.options";
const PLAYER: &str = "gameplay/players/default.player";
const MATERIALS: &str = "game_materials/game.materials";

/// Engine state class ids (`resources_classes.h`) that change weapon logic.
const SHOTGUN_RELOAD_STATE: i32 = 310;
const DOUBLE_BARRELED_RELOAD_STATE: i32 = 317;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponDispersion {
    pub base: f32,
    pub from_the_hip_multiplier: f32,
    pub aim_multiplier: f32,
    pub speed_of_aiming: f32,
    pub reload_amount: f32,
}

/// `weapon_recoil_params`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RecoilParams {
    pub first_shoot_side_recoil: f32,
    pub shoot_side_recoil: f32,
    pub first_shoot_back_recoil: f32,
    pub shoot_back_recoil: f32,
    pub shoot_recoil_min_angle: f32,
    pub shoot_recoil_angle_range: f32,
    pub additive_recoil_time: f32,
    pub additive_side_recoil: f32,
    pub additive_back_recoil: f32,
    pub additive_recoil_min_angle: f32,
    pub additive_recoil_angle_range: f32,
    pub side_compensation_speed: f32,
    pub back_compensation_speed: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadKind {
    /// A whole magazine in `reload_time`.
    Magazine,
    /// One round at a time, `reload_time / magazine_capacity` each
    /// (`computed_shotgun_reload_animation_time_scale`).
    PerRound,
    /// Both barrels in `reload_time`.
    DoubleBarrel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponData {
    pub config: String,
    pub bullet_damage: f32,
    pub bullet_pierce: f32,
    pub rounds_per_minute: f32,
    pub magazine_capacity: u16,
    pub reload_time: f32,
    pub double_handed: bool,
    pub chamber_a_round_on_reload: bool,
    /// Bolt and pump actions chamber each round; the shot fires the chamber.
    pub has_chamber_state: bool,
    /// Rounds per trigger pull per mode; 0xFF is automatic.
    pub fire_queue_types: Vec<u8>,
    pub reload: ReloadKind,
    pub dispersion: WeaponDispersion,
    pub recoil: RecoilParams,
    /// Each logic state's clips by config name (`show`, `fire`, `reload`...).
    pub states: HashMap<String, StateClips>,
}

/// One weapon logic state as `weapon_core_state_cook` reads it: its class
/// and the timing of the weapon's own clips and the player's, in config
/// order (first person stand, first person crouch, third person stand,
/// third person crouch, repeated per weapon state where a class has more).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateClips {
    pub state_type: i32,
    pub animations: Vec<Arc<ClipTiming>>,
    pub user_animations: Vec<Arc<ClipTiming>>,
    /// The shotgun reload's `start_substate`, `reload_one_substate` and
    /// `finish_substate`.
    pub substates: HashMap<String, StateClips>,
}

/// Clip timings already read, by path.
type ClipCache = HashMap<String, Arc<ClipTiming>>;

fn load_clip_list(
    resources: &mut Resources,
    cache: &mut ClipCache,
    list: Option<&ConfigValue>,
) -> Result<Vec<Arc<ClipTiming>>, ResourceError> {
    let Some(list) = list else {
        return Ok(Vec::new());
    };
    list.children()
        .iter()
        .map(|path| {
            let path = path
                .as_str()
                .ok_or_else(|| ResourceError("animation path is not a string".to_owned()))?;
            if let Some(timing) = cache.get(path) {
                return Ok(timing.clone());
            }
            let timing = Arc::new(ClipTiming::load(resources, path)?);
            cache.insert(path.to_owned(), timing.clone());
            Ok(timing)
        })
        .collect()
}

/// `weapon_core_cook::query_weapon_states`: every state's clips.
fn load_states(
    resources: &mut Resources,
    cache: &mut ClipCache,
    states: &ConfigValue,
) -> Result<HashMap<String, StateClips>, ResourceError> {
    let mut result = HashMap::new();
    for (name, state) in states.entries() {
        let mut clips = StateClips {
            state_type: state
                .get("state_type")
                .and_then(ConfigValue::as_i32)
                .unwrap_or(0),
            animations: load_clip_list(resources, cache, state.get("animations"))?,
            user_animations: load_clip_list(resources, cache, state.get("user_animations"))?,
            substates: HashMap::new(),
        };
        for substate in ["start_substate", "reload_one_substate", "finish_substate"] {
            if let Some(config) = state.get(substate) {
                clips.substates.insert(
                    substate.to_owned(),
                    StateClips {
                        state_type: 0,
                        animations: load_clip_list(resources, cache, config.get("animations"))?,
                        user_animations: load_clip_list(
                            resources,
                            cache,
                            config.get("user_animations"),
                        )?,
                        substates: HashMap::new(),
                    },
                );
            }
        }
        result.insert(name.clone(), clips);
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AmmoData {
    pub config: String,
    pub k_damage: f32,
    pub k_arp: f32,
    pub muzzle_speed: f32,
    pub air_resistance: f32,
    pub buck_shot: u16,
    pub dispersion: f32,
    /// Degrees, as configured.
    pub ricochet_angle: f32,
    pub distance_coef: f32,
    pub game_material_id: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Protection {
    pub armor: f32,
    pub reduce: f32,
    pub absorption: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArmourPart {
    pub body_part: String,
    pub health: f32,
    pub regeneration_speed: f32,
    pub hit_types: Vec<(String, Protection)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArmourData {
    pub config: String,
    pub parts: Vec<ArmourPart>,
}

/// Damage protection while a drug works (`medkit::damage_protection`).
#[derive(Clone, Debug, PartialEq)]
pub struct DrugProtection {
    pub body_part: String,
    pub hit_type: String,
    pub hit_coeff: f32,
    pub threshold: f32,
}

/// A quick-slot drug, as `medkit::load` reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct DrugData {
    pub config: String,
    pub activity_ms: u32,
    pub delay_ms: u32,
    /// Health per second, per body part (`amount / activity_time_sec`).
    pub influences: Vec<(String, f32)>,
    /// Affects cancelled when the drug starts working.
    pub remove_affects: Vec<(String, u8)>,
    pub protection: Vec<DrugProtection>,
}

/// One trap volume (`collision_geometry` / `hittable_object`): its
/// primitives in the trap's own space, and its collision filter.
#[derive(Clone, Debug, PartialEq)]
pub struct TrapVolume {
    /// (shape, place in the trap, game material).
    pub shapes: Vec<(Shape, Mat4, u16)>,
    pub group: u16,
    pub mask: u16,
}

/// What a firing trap does to whoever set it off (`apply_damage`).
#[derive(Clone, Debug, PartialEq)]
pub struct TrapDamage {
    pub body_part: String,
    pub hit_type: String,
    pub amount: f32,
    pub armor_piercing: f32,
}

/// A quick-slot trap set, as `booby_trap_set_core::load` and the traps'
/// `booby_trap_core` read it. Times are in milliseconds; 0 means the state
/// never runs out.
#[derive(Clone, Debug, PartialEq)]
pub struct TrapData {
    pub config: String,
    pub max_deploy_distance: f32,
    pub max_slope_cos: f32,
    pub armed_life_ms: u32,
    pub fired_life_ms: u32,
    pub disarmed_life_ms: u32,
    pub defuse_ms: u32,
    pub defuse_by_hit: bool,
    pub material_can_place_test: bool,
    pub material_can_stick_test: bool,
    pub damage: Vec<TrapDamage>,
    /// What sets it off (`collision_sensor`).
    pub sensor: TrapVolume,
    /// Where a player looks to defuse it (`usable_object`).
    pub usable: TrapVolume,
    /// What a bullet hits to defuse it, if `defuse_by_hit`.
    pub hittable: Option<TrapVolume>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Weapon(Arc<WeaponData>),
    Ammo(Arc<AmmoData>),
    Armour(Arc<ArmourData>),
    Drug(Arc<DrugData>),
    Trap(Arc<TrapData>),
    /// A lifebone artefact (`item_type_artefact_lifebone`).
    Lifebone,
    /// Anything else combat does not read (scopes).
    Other,
}

/// `item_types_enum::item_type_base_trap`, the `data.type` that makes a
/// quick-slot item a trap set (`inventory_cook`).
const ITEM_TYPE_BASE_TRAP: i32 = 2;
/// `item_types_enum::item_type_artefact_lifebone`.
const ITEM_TYPE_ARTEFACT_LIFEBONE: i32 = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct HitTypeTemplate {
    pub name: String,
    pub protection: Protection,
    /// Damage spread to other body parts (by index), `bdb_coeff`.
    pub bdb: Vec<(usize, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThresholdTemplate {
    /// Fraction of max health at or below which the affects apply.
    pub value: f32,
    pub target: usize,
    pub affects: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyPartTemplate {
    pub name: String,
    pub health: f32,
    pub regeneration_speed: f32,
    pub regeneration_timeout_ms: u32,
    pub damage_group: Option<u8>,
    pub hit_types: Vec<HitTypeTemplate>,
    pub thresholds: Vec<ThresholdTemplate>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyTemplate {
    pub parts: Vec<BodyPartTemplate>,
}

impl BodyTemplate {
    #[cfg(test)]
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.parts.iter().position(|part| part.name == name)
    }
}

/// `character_dispersion_params` from the player config.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterDispersion {
    pub idle: f32,
    pub idle_aim: f32,
    pub walk: f32,
    pub walk_aim: f32,
    pub run: f32,
    pub jump: f32,
    pub crouch: f32,
    pub crouch_aim: f32,
    pub crouch_walk: f32,
    pub crouch_walk_aim: f32,
    pub injury_penalty_double_handed: f32,
    pub injury_penalty_one_handed: f32,
}

/// `stamina_params` from the player config (`player_stamina::load`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaminaParams {
    pub max_value: f32,
    pub spending_speed: f32,
    pub regeneration_speed: f32,
    pub spending_threshold: f32,
    pub regeneration_threshold: f32,
}

/// `character_breath_holding_params` from the player config
/// (`breath_holding_params::load`, 0x090C80): the aim sway and holding the
/// breath against it. Times in seconds; the amplitudes are in recoil
/// coefficient units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BreathParams {
    pub max_breath_holding_time: f32,
    pub shortbreathing_repair_time: f32,
    pub breath_holding_multiplier: f32,
    pub shortbreathing_multiplier: f32,
    pub multiplier_increase_speed: f32,
    pub multiplier_decrease_speed: f32,
    pub horizontal_amplitude: f32,
    pub vertical_amplitude: f32,
    pub horizontal_period: f32,
    pub vertical_period: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub resistance: f32,
    pub k_ricochet: f32,
    pub reflection_speed_down: f32,
    /// `mine.can_place` / `mine.can_stick`: whether a trap may be set on it.
    pub can_place_mine: bool,
    pub can_stick_mine: bool,
}

/// `game_material` defaults for keys a material leaves out.
const MATERIAL_DEFAULTS: Material = Material {
    resistance: 50.0,
    k_ricochet: 0.3,
    reflection_speed_down: 1.0,
    can_place_mine: false,
    can_stick_mine: false,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Materials {
    by_id: HashMap<u16, Material>,
    fallback: Material,
}

impl Materials {
    /// Unknown ids use the material named `default`
    /// (`game_material_manager::get_material`).
    pub fn get(&self, id: u16) -> Material {
        self.by_id.get(&id).copied().unwrap_or(self.fallback)
    }
}

/// The material every player hit box uses (`animated_object.cpp:33`).
pub const ACTOR_MATERIAL: u16 = 10;

#[derive(Clone, Debug)]
pub struct CombatData {
    pub items: HashMap<u16, Item>,
    pub body: BodyTemplate,
    pub character_dispersion: CharacterDispersion,
    pub stamina: StaminaParams,
    pub breath: BreathParams,
    pub materials: Materials,
}

impl CombatData {
    pub fn load(resources: &mut Resources) -> Result<Self, ResourceError> {
        let body = load_body(&resources.config(HIT_PARAMS)?)?;
        let player = resources.config(PLAYER)?;
        let character_dispersion =
            load_character_dispersion(player.req("player")?.req("character_dispersion_params")?);
        let stamina = player.req("player")?.req("stamina_params")?;
        let stamina = StaminaParams {
            max_value: stamina.f32_at("max_value")?,
            spending_speed: stamina.f32_at("spending_speed")?,
            regeneration_speed: stamina.f32_at("regeneration_speed")?,
            spending_threshold: stamina.f32_at("spending_threshold")?,
            regeneration_threshold: stamina.f32_at("regeneration_threshold")?,
        };
        let breath = load_breath(
            player
                .req("player")?
                .req("character_breath_holding_params")?,
        )?;
        let materials = load_materials(&resources.config(MATERIALS)?)?;
        let items = load_items(resources)?;
        Ok(Self {
            items,
            body,
            character_dispersion,
            stamina,
            breath,
            materials,
        })
    }

    pub fn weapon(&self, dict_id: u16) -> Option<&Arc<WeaponData>> {
        match self.items.get(&dict_id) {
            Some(Item::Weapon(weapon)) => Some(weapon),
            _ => None,
        }
    }

    pub fn ammo(&self, dict_id: u16) -> Option<&Arc<AmmoData>> {
        match self.items.get(&dict_id) {
            Some(Item::Ammo(ammo)) => Some(ammo),
            _ => None,
        }
    }

    pub fn armour(&self, dict_id: u16) -> Option<&Arc<ArmourData>> {
        match self.items.get(&dict_id) {
            Some(Item::Armour(armour)) => Some(armour),
            _ => None,
        }
    }

    pub fn drug(&self, dict_id: u16) -> Option<&Arc<DrugData>> {
        match self.items.get(&dict_id) {
            Some(Item::Drug(drug)) => Some(drug),
            _ => None,
        }
    }

    pub fn trap(&self, dict_id: u16) -> Option<&Arc<TrapData>> {
        match self.items.get(&dict_id) {
            Some(Item::Trap(trap)) => Some(trap),
            _ => None,
        }
    }
}

/// Some configs store flags as the strings "true" / "false".
fn flag(value: Option<&ConfigValue>, default: bool) -> bool {
    match value {
        Some(ConfigValue::String(text)) => text.eq_ignore_ascii_case("true"),
        Some(value) => value.as_bool().unwrap_or(default),
        None => default,
    }
}

fn number_or(value: &ConfigValue, key: &str, default: f32) -> f32 {
    value
        .get(key)
        .and_then(ConfigValue::as_f32)
        .unwrap_or(default)
}

fn load_items(resources: &mut Resources) -> Result<HashMap<u16, Item>, ResourceError> {
    let dictionaries = resources.config(ITEMS_DICTIONARY)?;
    let mut items = HashMap::new();
    let mut clips = ClipCache::new();
    for entry in dictionaries.req("items_dict")?.children() {
        let dict_id = u16::try_from(entry.i32_at("dict_id")?)
            .map_err(|_| ResourceError("item dict_id out of range".to_owned()))?;
        let config_path = entry.str_at("cfg_name")?.to_owned();
        if !resources.exists(&config_path) {
            log::warn!("item {dict_id}: missing config {config_path}");
            continue;
        }
        let config = resources.config(&config_path)?;
        let item_type = config
            .get("data")
            .and_then(|data| data.get("type"))
            .and_then(ConfigValue::as_i32);
        let item = if config_path.starts_with("gameplay/weapons/ammo/") {
            Item::Ammo(Arc::new(load_ammo(&config_path, &config)?))
        } else if config_path.starts_with("gameplay/weapons/") {
            let mut weapon = load_weapon(&config_path, &config)?;
            weapon.states = load_states(resources, &mut clips, config.req("states")?)?;
            Item::Weapon(Arc::new(weapon))
        } else if config_path.starts_with("gameplay/items/armour/") {
            Item::Armour(Arc::new(load_armour(&config_path, &config)?))
        } else if config_path.starts_with("gameplay/items/drugs/") {
            Item::Drug(Arc::new(load_drug(&config_path, &config)?))
        } else if item_type == Some(ITEM_TYPE_BASE_TRAP) {
            Item::Trap(Arc::new(load_trap(&config_path, &config)?))
        } else if item_type == Some(ITEM_TYPE_ARTEFACT_LIFEBONE) {
            Item::Lifebone
        } else {
            Item::Other
        };
        items.insert(dict_id, item);
    }
    Ok(items)
}

/// `medkit::load`.
fn load_drug(path: &str, config: &ConfigValue) -> Result<DrugData, ResourceError> {
    let data = config.req("data")?;
    let activity_time_sec = data.f32_at("activity_time_sec")?.max(1e-3);
    let delay_sec = data.f32_at("activation_delay_sec")?;
    let influences = data
        .req("influences")?
        .children()
        .into_iter()
        .map(|influence| {
            Ok((
                influence.str_at("body_part")?.to_owned(),
                influence.f32_at("amount")? / activity_time_sec,
            ))
        })
        .collect::<Result<_, ResourceError>>()?;
    let remove_affects = data
        .req("remove_affects")?
        .children()
        .into_iter()
        .map(|affect| {
            Ok((
                affect.str_at("body_part")?.to_owned(),
                affect.i32_at("affect")? as u8,
            ))
        })
        .collect::<Result<_, ResourceError>>()?;
    let protection = data
        .req("damage_protection")?
        .children()
        .into_iter()
        .map(|protection| {
            Ok(DrugProtection {
                body_part: protection.str_at("body_part")?.to_owned(),
                hit_type: protection.str_at("hit_type")?.to_owned(),
                hit_coeff: protection.f32_at("hit_coeff")?,
                threshold: protection.f32_at("threshold")?,
            })
        })
        .collect::<Result<_, ResourceError>>()?;
    Ok(DrugData {
        config: path.to_owned(),
        activity_ms: (1000.0 * activity_time_sec).floor() as u32,
        delay_ms: (1000.0 * delay_sec).floor().max(0.0) as u32,
        influences,
        remove_affects,
        protection,
    })
}

/// `booby_trap_set_core::load` (times floored to milliseconds) and the
/// volumes `booby_trap_core` builds from the same `data`.
fn load_trap(path: &str, config: &ConfigValue) -> Result<TrapData, ResourceError> {
    let data = config.req("data")?;
    let ms = |key: &str| -> Result<u32, ResourceError> {
        Ok((data.f32_at(key)? * 1000.0).floor().max(0.0) as u32)
    };
    let damage = data
        .req("damage_parameters")?
        .children()
        .into_iter()
        .map(|entry| {
            Ok(TrapDamage {
                body_part: entry.str_at("body_part")?.to_owned(),
                hit_type: entry.str_at("hit_type")?.to_owned(),
                amount: entry.f32_at("amount")?,
                armor_piercing: entry.f32_at("armor_piercing")?,
            })
        })
        .collect::<Result<_, ResourceError>>()?;
    let first_geometry = |key: &str| -> Result<TrapVolume, ResourceError> {
        let geometries = data.req(key)?.req("collision_geometries")?.children();
        let geometry = geometries
            .first()
            .ok_or_else(|| ResourceError(format!("{path}: {key} has no collision geometry")))?;
        load_trap_volume(geometry)
    };
    let defuse_by_hit = flag(data.get("defuse_by_hit"), false);
    Ok(TrapData {
        config: path.to_owned(),
        max_deploy_distance: data.f32_at("max_deploy_distance")?,
        max_slope_cos: data.f32_at("max_slope_angle")?.to_radians().cos(),
        armed_life_ms: ms("armed_life_time")?,
        fired_life_ms: ms("fired_life_time")?,
        disarmed_life_ms: ms("disarmed_life_time")?,
        defuse_ms: ms("defuse_time")?,
        defuse_by_hit,
        material_can_place_test: flag(data.get("material_can_place_test"), false),
        material_can_stick_test: flag(data.get("material_can_stick_test"), false),
        damage,
        sensor: first_geometry("collision_sensor")?,
        usable: first_geometry("usable_object")?,
        // `booby_trap_core` only builds it when traps defuse by hit.
        hittable: if defuse_by_hit {
            Some(load_trap_volume(data.req("hittable_object")?)?)
        } else {
            None
        },
    })
}

/// `physics::create_compound_shape( meshes )`: each mesh is a primitive of
/// `type` sized by `scale`, placed by `create_rotation( rotation ) *
/// create_translation( position )`, with an optional game material `mtl`.
/// The volume's own transform is replaced by the trap's when it is placed.
fn load_trap_volume(config: &ConfigValue) -> Result<TrapVolume, ResourceError> {
    let mut shapes = Vec::new();
    for mesh in config.req("meshes")?.children() {
        let kind = mesh.i32_at("type")?;
        let Some(shape) = Shape::from_engine(kind, Vec3::from_array(mesh.float3_at("scale")?))
        else {
            return Err(ResourceError(format!("trap volume: primitive type {kind}")));
        };
        let place = Mat4::rotation_xyz(Vec3::from_array(mesh.float3_at("rotation")?))
            * Mat4::translation(Vec3::from_array(mesh.float3_at("position")?));
        let material = mesh
            .get("mtl")
            .and_then(ConfigValue::as_i32)
            .and_then(|id| u16::try_from(id).ok())
            .unwrap_or(0);
        shapes.push((shape, place, material));
    }
    let filter = |key: &str| -> Result<u16, ResourceError> {
        u16::try_from(config.i32_at(key)?)
            .map_err(|_| ResourceError(format!("trap volume: {key} out of range")))
    };
    Ok(TrapVolume {
        shapes,
        group: filter("filter_group")?,
        mask: filter("filter_mask")?,
    })
}

/// `weapon_core_cook::load_weapon_parameters` and `query_weapon_states`.
fn load_weapon(path: &str, config: &ConfigValue) -> Result<WeaponData, ResourceError> {
    let parameters = config.req("parameters")?;
    let states = config.req("states")?;
    let reload_state = states.req("reload")?.i32_at("state_type")?;
    let dispersion = config.req("dispersion")?;
    let fire_queue_types = parameters
        .req("fire_queue_types")?
        .children()
        .iter()
        .map(|value| value.as_i32().unwrap_or(1) as i8 as u8)
        .collect();
    Ok(WeaponData {
        config: path.to_owned(),
        bullet_damage: parameters.f32_at("bullet_damage")?,
        bullet_pierce: parameters.f32_at("bullet_pierce")?,
        rounds_per_minute: parameters.f32_at("rounds_per_minute")?,
        magazine_capacity: u16::try_from(parameters.i32_at("magazine_capacity")?)
            .map_err(|_| ResourceError(format!("{path}: bad magazine_capacity")))?,
        reload_time: parameters.f32_at("reload_time")?,
        double_handed: flag(parameters.get("double_handed"), true),
        chamber_a_round_on_reload: flag(parameters.get("chamber_a_round_on_reload"), false),
        has_chamber_state: states.get("chamber_a_round").is_some(),
        fire_queue_types,
        reload: match reload_state {
            SHOTGUN_RELOAD_STATE => ReloadKind::PerRound,
            DOUBLE_BARRELED_RELOAD_STATE => ReloadKind::DoubleBarrel,
            _ => ReloadKind::Magazine,
        },
        // `weapon_dispersion_params`: the shipped configs carry
        // `one_shoot_dispersion_amount`, but the constructor zeroes it.
        dispersion: WeaponDispersion {
            base: number_or(dispersion, "base_dispersion", 0.0),
            from_the_hip_multiplier: number_or(dispersion, "from_the_hip_multiplier", 1.0),
            aim_multiplier: number_or(dispersion, "aim_multiplier", 1.0),
            speed_of_aiming: number_or(dispersion, "speed_of_aiming", 1.0),
            reload_amount: number_or(dispersion, "reload_dispersion_amount", 1.0),
        },
        recoil: config.get("recoil").map(load_recoil).unwrap_or_default(),
        states: HashMap::new(),
    })
}

/// `weapon_recoil_params( cfg )`: angle ranges are max minus min.
fn load_recoil(config: &ConfigValue) -> RecoilParams {
    let value = |key: &str| number_or(config, key, 0.0);
    let range = |min: &str, max: &str| match (config.get(min), config.get(max)) {
        (Some(_), Some(_)) => (value(min), value(max) - value(min)),
        _ => (0.0, 0.0),
    };
    let (shoot_recoil_min_angle, shoot_recoil_angle_range) =
        range("shoot_recoil_min_angle", "shoot_recoil_max_angle");
    let (additive_recoil_min_angle, additive_recoil_angle_range) =
        range("additive_recoil_min_angle", "additive_recoil_max_angle");
    RecoilParams {
        first_shoot_side_recoil: value("first_shoot_side_recoil"),
        shoot_side_recoil: value("shoot_side_recoil"),
        first_shoot_back_recoil: value("first_shoot_back_recoil"),
        shoot_back_recoil: value("shoot_back_recoil"),
        shoot_recoil_min_angle,
        shoot_recoil_angle_range,
        additive_recoil_time: number_or(config, "additive_recoil_time", 1e-3),
        additive_side_recoil: value("additive_side_recoil"),
        additive_back_recoil: value("additive_back_recoil"),
        additive_recoil_min_angle,
        additive_recoil_angle_range,
        side_compensation_speed: value("side_compensation_speed"),
        back_compensation_speed: value("back_compensation_speed"),
    }
}

/// `weapon_ammunition::load`.
fn load_ammo(path: &str, config: &ConfigValue) -> Result<AmmoData, ResourceError> {
    let data = config.req("data")?;
    Ok(AmmoData {
        config: path.to_owned(),
        k_damage: data.f32_at("k_damage")?,
        k_arp: data.f32_at("k_arp")?,
        muzzle_speed: data.f32_at("muzzle_speed")?,
        air_resistance: data.f32_at("air_resistance")?,
        buck_shot: u16::try_from(data.i32_at("buck_shot")?.max(0)).unwrap_or(1),
        dispersion: data.f32_at("dispersion")?,
        ricochet_angle: data.f32_at("ricochet_angle")?,
        distance_coef: data.f32_at("distance_coef")?,
        game_material_id: u16::try_from(data.i32_at("game_material_id")?).unwrap_or(0),
    })
}

/// The `hit_params` of an equipped item (`player_parameters_modifyer_cook`).
fn load_armour(path: &str, config: &ConfigValue) -> Result<ArmourData, ResourceError> {
    let mut parts = Vec::new();
    if let Some(hit_params) = config.get("hit_params") {
        for (body_part, part) in hit_params.entries() {
            let mut hit_types = Vec::new();
            if let Some(types) = part.get("hit_types") {
                for (name, values) in types.entries() {
                    hit_types.push((
                        name.clone(),
                        Protection {
                            armor: values.f32_at("armor")?,
                            reduce: values.f32_at("reduce")?,
                            absorption: values.f32_at("absorption")?,
                        },
                    ));
                }
            }
            parts.push(ArmourPart {
                body_part: body_part.clone(),
                health: number_or(part, "health", 0.0),
                regeneration_speed: number_or(part, "regeneration_speed", 0.0),
                hit_types,
            });
        }
    }
    Ok(ArmourData {
        config: path.to_owned(),
        parts,
    })
}

/// `damage_model_cook`: body parts, their hit types, damage spread and
/// thresholds.
fn load_body(config: &ConfigValue) -> Result<BodyTemplate, ResourceError> {
    let configs = config.req("hit_params")?.children();
    let names: Vec<String> = configs
        .iter()
        .map(|part| part.str_at("name").map(str::to_owned))
        .collect::<Result<_, _>>()?;
    let index = |name: &str| {
        names
            .iter()
            .position(|known| known == name)
            .ok_or_else(|| ResourceError(format!("hit params: unknown body part `{name}`")))
    };

    let mut groups = HashMap::new();
    if let Some(damage_groups) = config.get("damage_groups") {
        for (group, members) in damage_groups.children().iter().enumerate() {
            for member in members.children() {
                if let Some(name) = member.as_str() {
                    groups.insert(name.to_owned(), group as u8);
                }
            }
        }
    }

    let mut parts = Vec::with_capacity(configs.len());
    for (part, name) in configs.iter().zip(&names) {
        let mut hit_types = Vec::new();
        for (type_name, values) in part.req("hit_types")?.entries() {
            let mut bdb = Vec::new();
            if let Some(coefficients) = values.get("bdb_coeff") {
                for (target, coefficient) in coefficients.entries() {
                    let coefficient = coefficient.as_f32().unwrap_or(0.0);
                    bdb.push((index(target)?, coefficient));
                }
            }
            hit_types.push(HitTypeTemplate {
                name: type_name.clone(),
                protection: Protection {
                    armor: values.f32_at("armor")?,
                    reduce: values.f32_at("reduce")?,
                    absorption: values.f32_at("absorption")?,
                },
                bdb,
            });
        }
        let mut thresholds = Vec::new();
        if let Some(configured) = part.get("thresholds") {
            for threshold in configured.children() {
                thresholds.push(ThresholdTemplate {
                    value: threshold.f32_at("value")?,
                    target: index(threshold.str_at("target_bodypart")?)?,
                    affects: threshold
                        .req("affects")?
                        .children()
                        .iter()
                        .filter_map(|affect| affect.as_i32())
                        .map(|affect| affect as u8)
                        .collect(),
                });
            }
        }
        parts.push(BodyPartTemplate {
            name: name.clone(),
            health: part.f32_at("health")?,
            regeneration_speed: part.f32_at("regeneration_speed")?,
            // `body_part_parameters`: floor( 1000 * timeout ).
            regeneration_timeout_ms: (1000.0 * part.f32_at("regeneration_timeout")?).floor() as u32,
            damage_group: groups.get(name).copied(),
            hit_types,
            thresholds,
        });
    }
    Ok(BodyTemplate { parts })
}

/// `character_dispersion_params::load`; missing keys stay 1.
fn load_character_dispersion(config: &ConfigValue) -> CharacterDispersion {
    let value = |key| number_or(config, key, 1.0);
    CharacterDispersion {
        idle: value("idle_multiplier"),
        idle_aim: value("idle_aim_multiplier"),
        walk: value("walk_multiplier"),
        walk_aim: value("walk_aim_multiplier"),
        run: value("run_multiplier"),
        jump: value("jump_multiplier"),
        crouch: value("crouch_multiplier"),
        crouch_aim: value("crouch_aim_multiplier"),
        crouch_walk: value("crouch_walk_multiplier"),
        crouch_walk_aim: value("crouch_walk_aim_multiplier"),
        injury_penalty_double_handed: value("injury_penalty_for_double_handed"),
        injury_penalty_one_handed: value("injury_penalty_for_one_handed"),
    }
}

/// `breath_holding_params::load`. The client keeps a default for a missing
/// key; the shipped player config has them all.
fn load_breath(config: &ConfigValue) -> Result<BreathParams, ResourceError> {
    Ok(BreathParams {
        max_breath_holding_time: config.f32_at("max_breath_holding_time")?,
        shortbreathing_repair_time: config.f32_at("shortbreathing_repair_time")?,
        breath_holding_multiplier: config.f32_at("breath_holding_multiplier")?,
        shortbreathing_multiplier: config.f32_at("shortbreathing_multiplier")?,
        multiplier_increase_speed: config.f32_at("multiplier_increase_speed")?,
        multiplier_decrease_speed: config.f32_at("multiplier_decrease_speed")?,
        horizontal_amplitude: config.f32_at("horizontal_amplitude")?,
        vertical_amplitude: config.f32_at("vertical_amplitude")?,
        // Spelled so in the config.
        horizontal_period: config.f32_at("horizontal_peroid")?,
        vertical_period: config.f32_at("vertical_peroid")?,
    })
}

/// `game_material_manager_cook` and `game_material::load`; deleted entries
/// are skipped.
fn load_materials(config: &ConfigValue) -> Result<Materials, ResourceError> {
    let mut by_id = HashMap::new();
    let mut fallback = MATERIAL_DEFAULTS;
    for entry in config.req("materials")?.children() {
        if flag(entry.get("deleted"), false) {
            continue;
        }
        let physic = entry.req("physic")?;
        let mine = entry.get("mine");
        let mine_flag = |key: &str| flag(mine.and_then(|mine| mine.get(key)), false);
        let material = Material {
            resistance: number_or(physic, "resistance", MATERIAL_DEFAULTS.resistance),
            k_ricochet: number_or(physic, "k_ricochet", MATERIAL_DEFAULTS.k_ricochet),
            reflection_speed_down: number_or(
                physic,
                "reflection_speed_down",
                MATERIAL_DEFAULTS.reflection_speed_down,
            ),
            can_place_mine: mine_flag("can_place"),
            can_stick_mine: mine_flag("can_stick"),
        };
        let id = u16::try_from(entry.i32_at("id")?)
            .map_err(|_| ResourceError("material id out of range".to_owned()))?;
        if entry.get("name").and_then(ConfigValue::as_str) == Some("default") {
            fallback = material;
        }
        by_id.insert(id, material);
    }
    Ok(Materials { by_id, fallback })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::combat::resources::tests::game_resources;

    pub fn game_data() -> Option<CombatData> {
        let mut resources = game_resources()?;
        Some(CombatData::load(&mut resources).unwrap())
    }

    #[test]
    fn loads_the_shipped_weapons_ammo_and_armour() {
        let Some(data) = game_data() else {
            return;
        };
        let ak = data.weapon(13).unwrap();
        assert_eq!(ak.bullet_damage, 0.4);
        assert_eq!(ak.bullet_pierce, 0.5);
        assert_eq!(ak.rounds_per_minute, 650.0);
        assert_eq!(ak.magazine_capacity, 30);
        assert_eq!(ak.fire_queue_types, [0xFF, 1]);
        assert_eq!(ak.reload, ReloadKind::Magazine);
        assert!(!ak.has_chamber_state);

        let rem_700 = data.weapon(14).unwrap();
        assert_eq!(rem_700.reload, ReloadKind::PerRound);
        assert!(rem_700.has_chamber_state);
        let toz_66 = data.weapon(17).unwrap();
        assert_eq!(toz_66.reload, ReloadKind::DoubleBarrel);
        assert!(!toz_66.double_handed);

        let buck = data.ammo(22).unwrap();
        assert_eq!(buck.buck_shot, 12);
        assert_eq!(buck.k_damage, 0.1);

        let torso = data.armour(48).unwrap();
        let body = torso
            .parts
            .iter()
            .find(|part| part.body_part == "body")
            .unwrap();
        let injury = body
            .hit_types
            .iter()
            .find(|(name, _)| name == "injury")
            .unwrap()
            .1;
        assert_eq!(
            injury,
            Protection {
                armor: 0.3,
                reduce: 0.4,
                absorption: 0.0
            }
        );
    }

    #[test]
    fn loads_the_breath_holding_params() {
        let Some(data) = game_data() else {
            return;
        };
        assert_eq!(
            data.breath,
            BreathParams {
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
            }
        );
    }

    #[test]
    fn loads_the_weapon_state_clips() {
        let Some(data) = game_data() else {
            return;
        };
        let ak = data.weapon(13).unwrap();
        let fire = &ak.states["fire"];
        assert_eq!(fire.state_type, 307);
        assert_eq!(fire.animations.len(), 4);
        assert_eq!(fire.animations[0].length_frames, 10.0);
        assert_eq!(fire.animations[0].marks("shoot", 7), [1.0]);
        assert_eq!(ak.states["reload"].animations[0].length_frames, 70.0);

        let magnum = data.weapon(56).unwrap();
        assert_eq!(magnum.states["fire"].animations[0].marks("shoot", 7), [7.0]);
        assert_eq!(
            magnum.states["aimed_fire"].animations[0].marks("shoot", 7),
            [8.0]
        );

        let rem_700 = data.weapon(14).unwrap();
        let reload = &rem_700.states["reload"];
        assert_eq!(reload.state_type, 310);
        assert_eq!(reload.substates.len(), 3);
        assert_eq!(
            rem_700.states["chamber_a_round"].animations[0].length_frames,
            40.0
        );

        let toz_34 = data.weapon(16).unwrap();
        let reload = &toz_34.states["reload"].animations;
        assert_eq!(
            (reload[0].length_frames, reload[1].length_frames),
            (80.0, 110.0)
        );
    }

    #[test]
    fn loads_the_human_damage_model() {
        let Some(data) = game_data() else {
            return;
        };
        let body = &data.body;
        assert_eq!(body.parts.len(), 15);
        let head = &body.parts[body.index_of("head").unwrap()];
        assert_eq!(head.health, 0.3);
        assert_eq!(head.regeneration_timeout_ms, 5000);
        assert_eq!(
            head.damage_group,
            body.parts[body.index_of("face").unwrap()].damage_group
        );
        let pain = body.index_of("pain").unwrap();
        let injury = head.hit_types.iter().find(|t| t.name == "injury").unwrap();
        assert_eq!(injury.bdb, [(pain, 1.0)]);
        assert_eq!(head.thresholds[1].affects, [2]);
        assert_eq!(body.parts[pain].health, 3.0);
    }

    #[test]
    fn loads_materials_with_the_default_fallback() {
        let Some(data) = game_data() else {
            return;
        };
        assert_eq!(data.materials.get(19).resistance, 1.0); // concrete
        assert_eq!(data.materials.get(ACTOR_MATERIAL).resistance, 0.3);
        assert_eq!(data.materials.get(9999), data.materials.get(11));
    }

    #[test]
    fn loads_where_traps_may_be_set() {
        let Some(data) = game_data() else {
            return;
        };
        let concrete = data.materials.get(19);
        assert!(concrete.can_place_mine && concrete.can_stick_mine);
        let mud = data.materials.get(23);
        assert!(!mud.can_place_mine && !mud.can_stick_mine);
        // The player's own material takes a stuck trap, not a placed one.
        let actor = data.materials.get(ACTOR_MATERIAL);
        assert!(!actor.can_place_mine && actor.can_stick_mine);
    }

    #[test]
    fn the_items_without_a_spawn_state_are_the_lifebones() {
        let Some(data) = game_data() else {
            return;
        };
        let mut lifebones: Vec<u16> = data
            .items
            .iter()
            .filter(|(_, item)| matches!(item, Item::Lifebone))
            .map(|(dict_id, _)| *dict_id)
            .collect();
        lifebones.sort_unstable();
        assert_eq!(lifebones, crate::game::NO_SPAWN_STATE);
    }

    #[test]
    fn loads_the_trap_set() {
        let Some(data) = game_data() else {
            return;
        };
        // The lifebone artefact has a type of its own.
        assert!(matches!(data.items.get(&54), Some(Item::Lifebone)));
        let Some(Item::Trap(trap)) = data.items.get(&68) else {
            panic!("item 68 is not a trap set");
        };
        assert_eq!(trap.max_deploy_distance, 2.0);
        assert!((trap.max_slope_cos - 30f32.to_radians().cos()).abs() < 1e-6);
        assert_eq!(
            (
                trap.armed_life_ms,
                trap.fired_life_ms,
                trap.disarmed_life_ms,
                trap.defuse_ms
            ),
            (0, 3000, 3000, 5000)
        );
        assert!(trap.defuse_by_hit);
        assert!(trap.material_can_place_test && !trap.material_can_stick_test);
        assert_eq!(
            trap.damage
                .iter()
                .map(|d| (d.body_part.as_str(), d.hit_type.as_str(), d.amount))
                .collect::<Vec<_>>(),
            [
                ("right_foot", "injury", 1.0),
                ("left_foot", "injury", 1.0),
                ("pain", "injury", 1.5)
            ]
        );
        assert!(trap.damage.iter().all(|d| d.armor_piercing == 1.0));

        // A flat cylinder 0.25 m across its radius, 0.12 m up the normal.
        let sensor = &trap.sensor;
        assert_eq!((sensor.group, sensor.mask), (513, 1056));
        assert_eq!(sensor.shapes.len(), 1);
        assert_eq!(
            sensor.shapes[0].0,
            Shape::Cylinder {
                radius: 0.25,
                half_height: 0.1
            }
        );
        assert_eq!(sensor.shapes[0].1.c, Vec3::new(0.0, 0.12, 0.0));
        assert_eq!((trap.usable.group, trap.usable.mask), (128, 256));
        let hittable = trap.hittable.as_ref().unwrap();
        assert_eq!((hittable.group, hittable.mask), (8, 16));
        assert_eq!(hittable.shapes[0].2, 18); // metall_thick
    }
}
