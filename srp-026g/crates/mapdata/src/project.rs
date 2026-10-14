//! One map project (`projects/<level>/<weather>_<mode>/client_project`):
//! its objectives and its placed collision.
//!
//! The project is a binary config. What a bot needs of it:
//! - `server_objects.respawn_points[]`: `position`, `rotation`, `team`,
//!   `priority`, `point_id`, `layer`;
//! - `server_objects.victory_items_containers[]` (modes 2, 3): `position`,
//!   `team`, `id`, and use boxes named in `collision_geometries[].name`;
//! - `server_objects.victory_item_spawners[]` (modes 2, 3): where a battery
//!   lies (`spawned_item.position`);
//! - `server_objects.victory_item_spawn_points[]` (mode 3): stations;
//! - `client_objects[]` by `game_object_type`: `resupply_zone`,
//!   `damage_zone`, `effect_zone`, `artefact_container` with their
//!   `collision_geometries`, which name `collision_geometry` objects whose
//!   `meshes[]` are spheres (type 0) and boxes (type 1, half sizes);
//!   `generic_anomaly_core` names the damage zones it drives;
//! - `collision_objects[]`: `lib_name`, `position`, `rotation`, `scale`,
//!   `cgroup`, `cmask`;
//! - `levelmap_min` / `levelmap_max`: the play area's x/z box (its y means
//!   nothing).
//!
//! All positions are in the map frame ([`crate::math`]).

use std::collections::HashMap;

use binary_config_parser::ConfigValue;

use crate::config::{children, float3, float3_or, float3_req, i32_or, str_or};
use crate::library::{Library, Materials, WalkerBehaviour};
use crate::math::{Rotation, Transform, Vec3};
use crate::resources::{Error, Resources};

/// A body's collision filter: it meets an object when
/// `object.cgroup & body.mask != 0` and `object.cmask & body.group != 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionFilter {
    pub group: u16,
    pub mask: u16,
}

impl CollisionFilter {
    /// The character controller (`character_controller.cpp:45-51`): meets
    /// `cgroup & 2` (the walker meshes and shared props). PROVEN.
    pub const WALKER: Self = Self { group: 4, mask: 2 };
    /// The bullet ray (`bullet.cpp:402`): meets `cgroup & 8`. PROVEN.
    pub const BULLET: Self = Self { group: 16, mask: 8 };

    pub fn meets(self, cgroup: u16, cmask: u16) -> bool {
        cgroup & self.mask != 0 && cmask & self.group != 0
    }
}

/// A respawn point. `team` is 0 or 1 (once 2, on level_02 TDM).
#[derive(Clone, Debug, PartialEq)]
pub struct Spawn {
    pub position: Vec3,
    /// Euler angles ([`Rotation::from_angles`]); [`crate::math::forward`]
    /// gives the facing.
    pub rotation: Vec3,
    pub team: i32,
    pub priority: i32,
    pub point_id: i32,
    pub layer: String,
}

/// A team's battery container (modes 2 and 3).
#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    pub position: Vec3,
    pub team: i32,
    pub id: i32,
    /// Where a carrier must stand to hand a battery in.
    pub use_shapes: Vec<Shape>,
}

/// Where a battery lies at the start (modes 2 and 3).
#[derive(Clone, Debug, PartialEq)]
pub struct BatterySpawner {
    pub position: Vec3,
    /// `spawned_item.position`: the battery itself.
    pub item: Vec3,
    pub layer: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ZoneKind {
    Resupply = 0,
    Damage = 1,
    Effect = 2,
    ArtefactContainer = 3,
}

impl ZoneKind {
    pub fn from_game_object_type(name: &str) -> Option<Self> {
        Some(match name {
            "resupply_zone" => Self::Resupply,
            "damage_zone" => Self::Damage,
            "effect_zone" => Self::Effect,
            "artefact_container" => Self::ArtefactContainer,
            _ => return None,
        })
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Resupply,
            1 => Self::Damage,
            2 => Self::Effect,
            3 => Self::ArtefactContainer,
            _ => return None,
        })
    }
}

/// A zone: resupply (one per team), damage (anomaly, toxic), effect, or an
/// artefact container's pick-up place.
#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub kind: ZoneKind,
    /// `full_name`; anomalies name their damage zones by it.
    pub name: String,
    /// -1 when the zone has no team.
    pub team: i32,
    pub position: Vec3,
    /// `damage_type` of a damage zone (`toxic`, ...), else empty.
    pub damage_type: String,
    pub shapes: Vec<Shape>,
}

/// A `generic_anomaly_core`: its centre and the damage zones it drives.
#[derive(Clone, Debug, PartialEq)]
pub struct Anomaly {
    pub name: String,
    pub position: Vec3,
    pub zones: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ShapeKind {
    Sphere = 0,
    Box = 1,
}

/// A trigger shape of a `collision_geometry`, placed in the map frame:
/// the mesh's own rotation and position, then the object's scale, rotation
/// and position (the order of the engine's matrices; LIKELY, every shape
/// of the shipped maps has scale 1 and most no rotation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub kind: ShapeKind,
    pub center: Vec3,
    /// Local x, y, z axes in the map frame.
    pub rotation: Rotation,
    /// A box's half sizes; a sphere's radius is `half.x`.
    pub half: Vec3,
}

impl Shape {
    pub fn contains(&self, point: Vec3) -> bool {
        let d = point - self.center;
        match self.kind {
            ShapeKind::Sphere => d.length() <= self.half.x,
            ShapeKind::Box => {
                let local = self.rotation.transposed().apply(d);
                local.x.abs() <= self.half.x
                    && local.y.abs() <= self.half.y
                    && local.z.abs() <= self.half.z
            }
        }
    }
}

/// One `collision_objects[]` entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Index into [`MapData::libraries`].
    pub library: u32,
    pub position: Vec3,
    pub rotation: Vec3,
    pub cgroup: u16,
    pub cmask: u16,
}

impl Placement {
    pub fn transform(&self) -> Transform {
        Transform::new(self.rotation, self.position)
    }
}

/// Metres added around the play area when a project has no `levelmap`.
pub const NO_LEVELMAP_MARGIN: f32 = 15.0;

/// A triangle placed in the map frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub v: [Vec3; 3],
    pub walker: WalkerBehaviour,
}

/// Everything this crate knows of one project; what [`crate::format`]
/// stores.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapData {
    /// `level_01/day_4`.
    pub project: String,
    /// The `levelmap` box (zero when the project has none, as level_06).
    pub levelmap_min: Vec3,
    pub levelmap_max: Vec3,
    pub spawns: Vec<Spawn>,
    pub containers: Vec<Container>,
    pub battery_spawners: Vec<BatterySpawner>,
    /// Mode 3's charging stations.
    pub stations: Vec<Vec3>,
    pub zones: Vec<Zone>,
    pub anomalies: Vec<Anomaly>,
    /// Each collision library a placement uses, once, in its own space.
    pub libraries: Vec<Library>,
    pub placements: Vec<Placement>,
}

impl MapData {
    /// Loads `projects/<project>/client_project` and every library its
    /// walker and bullet placements use.
    pub fn load(
        resources: &mut Resources,
        materials: &Materials,
        project: &str,
    ) -> Result<Self, Error> {
        let root = resources.config(&format!("projects/{project}/client_project"))?;
        let mut map = Self::from_config(project, &root)?;
        let mut names: HashMap<String, u32> = HashMap::new();
        let mut placements = Vec::new();
        for object in children(&root, "collision_objects") {
            let filter_value = |key: &str| -> Result<u16, Error> {
                u16::try_from(object.i32_at(key)?)
                    .map_err(|_| Error(format!("collision object {key} out of range")))
            };
            let (cgroup, cmask) = (filter_value("cgroup")?, filter_value("cmask")?);
            if !CollisionFilter::WALKER.meets(cgroup, cmask)
                && !CollisionFilter::BULLET.meets(cgroup, cmask)
            {
                continue;
            }
            let lib_name = object.str_at("lib_name")?;
            let library = match names.get(lib_name) {
                Some(&index) => index,
                None => {
                    let index = map.libraries.len() as u32;
                    map.libraries
                        .push(Library::load(resources, materials, lib_name)?);
                    names.insert(lib_name.to_owned(), index);
                    index
                }
            };
            placements.push(Placement {
                library,
                position: float3_req(object, "position")?,
                rotation: float3_req(object, "rotation")?,
                cgroup,
                cmask,
            });
        }
        map.placements = placements;
        Ok(map)
    }

    /// The objectives and the level box of a parsed `client_project`
    /// (no collision).
    pub fn from_config(project: &str, root: &ConfigValue) -> Result<Self, Error> {
        let mut map = Self {
            project: project.to_owned(),
            levelmap_min: point_at(root, "levelmap_min"),
            levelmap_max: point_at(root, "levelmap_max"),
            ..Self::default()
        };
        // `collision_geometry` client objects by `full_name`.
        let geometries: HashMap<&str, &ConfigValue> = children(root, "client_objects")
            .into_iter()
            .filter(|o| str_or(o, "game_object_type") == "collision_geometry")
            .map(|o| (str_or(o, "full_name"), o))
            .collect();
        let shapes_of = |object: &ConfigValue| -> Vec<Shape> {
            let mut shapes = Vec::new();
            for reference in children(object, "collision_geometries") {
                let name = reference
                    .as_str()
                    .unwrap_or_else(|| str_or(reference, "name"));
                if let Some(geometry) = geometries.get(name) {
                    shapes.extend(geometry_shapes(geometry));
                }
            }
            shapes
        };

        let empty = ConfigValue::Named(Vec::new());
        let server = root.get("server_objects").unwrap_or(&empty);
        for point in children(server, "respawn_points") {
            map.spawns.push(Spawn {
                position: point_at(point, "position"),
                rotation: point_at(point, "rotation"),
                team: i32_or(point, "team", -1),
                priority: i32_or(point, "priority", 0),
                point_id: i32_or(point, "point_id", 0),
                layer: str_or(point, "layer").to_owned(),
            });
        }
        for container in children(server, "victory_items_containers") {
            map.containers.push(Container {
                position: point_at(container, "position"),
                team: i32_or(container, "team", -1),
                id: i32_or(container, "id", 0),
                use_shapes: shapes_of(container),
            });
        }
        for spawner in children(server, "victory_item_spawners") {
            let position = point_at(spawner, "position");
            map.battery_spawners.push(BatterySpawner {
                position,
                item: spawner
                    .get("spawned_item")
                    .and_then(|item| float3(item, "position"))
                    .unwrap_or(position),
                layer: str_or(spawner, "layer").to_owned(),
            });
        }
        // Stations sit under `server_objects`; a few projects keep an
        // (empty) copy at the top too.
        for source in [server, root] {
            for station in children(source, "victory_item_spawn_points") {
                map.stations.push(point_at(station, "position"));
            }
        }
        for object in children(root, "client_objects") {
            let kind = str_or(object, "game_object_type");
            if let Some(kind) = ZoneKind::from_game_object_type(kind) {
                map.zones.push(Zone {
                    kind,
                    name: str_or(object, "full_name").to_owned(),
                    team: i32_or(object, "team", -1),
                    position: point_at(object, "position"),
                    damage_type: str_or(object, "damage_type").to_owned(),
                    shapes: shapes_of(object),
                });
            } else if kind == "generic_anomaly_core" {
                let mut zones = Vec::new();
                for state in children(object, "states") {
                    for group in children(state, "groups") {
                        for zone in children(group, "zones") {
                            if let Some(name) = zone.as_str() {
                                zones.push(name.to_owned());
                            }
                        }
                    }
                }
                map.anomalies.push(Anomaly {
                    name: str_or(object, "full_name").to_owned(),
                    position: point_at(object, "position"),
                    zones,
                });
            }
        }
        Ok(map)
    }

    /// Every triangle a body with `filter` meets, placed in the map frame.
    pub fn triangles(&self, filter: CollisionFilter) -> Vec<Triangle> {
        let count = self
            .placements
            .iter()
            .filter(|p| filter.meets(p.cgroup, p.cmask))
            .map(|p| self.libraries[p.library as usize].triangles.len())
            .sum();
        let mut out = Vec::with_capacity(count);
        for placement in &self.placements {
            if !filter.meets(placement.cgroup, placement.cmask) {
                continue;
            }
            let library = &self.libraries[placement.library as usize];
            let transform = placement.transform();
            let world: Vec<Vec3> = library
                .vertices
                .iter()
                .map(|&v| transform.apply(v))
                .collect();
            for (t, &walker) in library.triangles.iter().zip(&library.walker) {
                out.push(Triangle {
                    v: t.map(|i| world[i as usize]),
                    walker,
                });
            }
        }
        out
    }

    /// The walker world: what the walking body stands on and bumps into.
    pub fn walker_triangles(&self) -> Vec<Triangle> {
        self.triangles(CollisionFilter::WALKER)
    }

    /// The bullet world: what stops shots and sight.
    pub fn bullet_triangles(&self) -> Vec<Triangle> {
        self.triangles(CollisionFilter::BULLET)
    }

    /// The area a walk grid should cover, as `[x0, z0, x1, z1]`: the
    /// `levelmap` box (the walker mesh reaches far outside play, to -666 m),
    /// widened by `margin` and to hold every spawn and objective. A box
    /// that is no box (level_03's is 2 m; level_06, 07 and 08 have none)
    /// gives way to where the collision objects stand (the middle 96 % of
    /// their positions on each axis), widened by a further
    /// [`NO_LEVELMAP_MARGIN`]: the objectives alone leave out ground
    /// between the bases (level_06 mode 3).
    pub fn play_area(&self, margin: f32) -> [f32; 4] {
        let mut lo = self.levelmap_min;
        let mut hi = self.levelmap_max;
        let mut points: Vec<Vec3> = self.spawns.iter().map(|s| s.position).collect();
        points.extend(self.containers.iter().map(|c| c.position));
        points.extend(self.battery_spawners.iter().map(|s| s.item));
        points.extend(self.stations.iter().copied());
        points.extend(self.zones.iter().map(|z| z.position));
        let mut margin = margin;
        if ((hi.x - lo.x) < 20.0 || (hi.z - lo.z) < 20.0) && !points.is_empty() {
            lo = points[0];
            hi = points[0];
            margin += NO_LEVELMAP_MARGIN;
            if !self.placements.is_empty() {
                let mut xs: Vec<f32> = self.placements.iter().map(|p| p.position.x).collect();
                let mut zs: Vec<f32> = self.placements.iter().map(|p| p.position.z).collect();
                xs.sort_by(f32::total_cmp);
                zs.sort_by(f32::total_cmp);
                let at = |v: &[f32], share: f32| v[((v.len() - 1) as f32 * share) as usize];
                points.push(Vec3::new(at(&xs, 0.02), lo.y, at(&zs, 0.02)));
                points.push(Vec3::new(at(&xs, 0.98), lo.y, at(&zs, 0.98)));
            }
        }
        for p in points {
            lo = lo.min(p);
            hi = hi.max(p);
        }
        [lo.x - margin, lo.z - margin, hi.x + margin, hi.z + margin]
    }
}

fn geometry_shapes(geometry: &ConfigValue) -> Vec<Shape> {
    let object_scale = float3_or(geometry, "scale", Vec3::ONE);
    let object = Transform::new(
        point_at(geometry, "rotation"),
        point_at(geometry, "position"),
    );
    let mut shapes = Vec::new();
    for mesh in children(geometry, "meshes") {
        let kind = match i32_or(mesh, "type", -1) {
            0 => ShapeKind::Sphere,
            1 => ShapeKind::Box,
            _ => continue,
        };
        let local = Rotation::from_angles(point_at(mesh, "rotation"));
        shapes.push(Shape {
            kind,
            center: object.apply(point_at(mesh, "position").scaled(object_scale)),
            rotation: local.then(&object.rotation),
            half: point_at(mesh, "scale").scaled(object_scale),
        });
    }
    shapes
}

/// A position or angles, zero when missing.
fn point_at(value: &ConfigValue, key: &str) -> Vec3 {
    float3_or(value, key, Vec3::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_follow_the_engine_groups() {
        // (8, 16) bullet-only, (2, 4) walker-only, (10, 20) both.
        assert!(CollisionFilter::BULLET.meets(8, 16));
        assert!(!CollisionFilter::WALKER.meets(8, 16));
        assert!(CollisionFilter::WALKER.meets(2, 4));
        assert!(!CollisionFilter::BULLET.meets(2, 4));
        assert!(CollisionFilter::WALKER.meets(10, 20));
        assert!(CollisionFilter::BULLET.meets(10, 20));
    }

    fn project_json() -> serde_json::Value {
        serde_json::json!({
            "levelmap_min": [-10.0, 0.0, -10.0],
            "levelmap_max": [10.0, 1.0, 10.0],
            "collision_objects": [],
            "server_objects": {
                "respawn_points": [
                    { "position": [1.0, 2.0, 3.0], "rotation": [0.0, 1.0, 0.0], "team": 1, "priority": 2, "point_id": 5, "layer": "default" },
                ],
                "victory_items_containers": [
                    { "position": [4.0, 0.0, 4.0], "team": 0, "id": 7, "collision_geometries": [ { "name": "use_box" } ] },
                ],
                "victory_item_spawners": [
                    { "position": [0.0, 0.0, 0.0], "spawned_item": { "position": [0.5, 0.2, 0.5] }, "layer": "mode_2" },
                ],
            },
            "client_objects": [
                { "game_object_type": "collision_geometry", "full_name": "use_box", "position": [4.0, 1.0, 4.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0],
                  "meshes": [ { "type": 1, "position": [0.0, 0.0, 1.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 0.5, 1.0] } ] },
                { "game_object_type": "collision_geometry", "full_name": "pool", "position": [-5.0, 0.0, -5.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0],
                  "meshes": [ { "type": 0, "position": [0.0, 0.0, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": [2.0, 2.0, 2.0] } ] },
                { "game_object_type": "damage_zone", "full_name": "logic/pool_zone", "damage_type": "toxic", "position": [-5.0, 0.0, -5.0],
                  "collision_geometries": [ { "name": "pool" } ] },
                { "game_object_type": "resupply_zone", "full_name": "resupply", "team": 1, "position": [8.0, 0.0, 8.0], "collision_geometries": [] },
                { "game_object_type": "generic_anomaly_core", "full_name": "core", "position": [-5.0, 0.0, -5.0],
                  "states": [ { "groups": [ { "zones": ["logic/pool_zone"] } ] } ] },
                { "game_object_type": "ambient_light", "full_name": "light", "position": [0.0, 0.0, 0.0] },
            ],
        })
    }

    #[test]
    fn reads_objectives_from_a_project() {
        let config = binary_config_parser::BinaryConfig::parse_json(&project_json());
        let root = ConfigValue::parse(config.as_bytes()).unwrap();
        let map = MapData::from_config("test/day_4", &root).unwrap();
        assert_eq!(map.spawns.len(), 1);
        assert_eq!(map.spawns[0].team, 1);
        assert_eq!(map.spawns[0].position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(map.containers[0].id, 7);
        let use_box = map.containers[0].use_shapes[0];
        assert!(use_box.contains(Vec3::new(4.5, 1.2, 5.5)));
        assert!(!use_box.contains(Vec3::new(4.0, 1.0, 3.5)));
        assert_eq!(map.battery_spawners[0].item, Vec3::new(0.5, 0.2, 0.5));
        assert_eq!(map.zones.len(), 2);
        let pool = &map.zones[0];
        assert_eq!(
            (pool.kind, pool.damage_type.as_str()),
            (ZoneKind::Damage, "toxic")
        );
        assert!(pool.shapes[0].contains(Vec3::new(-6.5, 0.0, -5.0)));
        assert!(!pool.shapes[0].contains(Vec3::new(-7.5, 0.0, -5.0)));
        assert_eq!(map.zones[1].team, 1);
        assert_eq!(map.anomalies[0].zones, ["logic/pool_zone"]);
        assert_eq!(map.play_area(5.0), [-15.0, -15.0, 15.0, 15.0]);
    }

    #[test]
    fn places_library_triangles_without_the_object_scale() {
        let library = Library {
            name: "lib".into(),
            vertices: vec![
                Vec3::ZERO,
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
            triangles: vec![[0, 1, 2]],
            walker: vec![WalkerBehaviour::Walk],
            primitives: 0,
        };
        let map = MapData {
            libraries: vec![library],
            placements: vec![
                Placement {
                    library: 0,
                    position: Vec3::new(10.0, 1.0, 0.0),
                    rotation: Vec3::new(0.0, std::f32::consts::PI, 0.0),
                    cgroup: 2,
                    cmask: 4,
                },
                Placement {
                    library: 0,
                    position: Vec3::ZERO,
                    rotation: Vec3::ZERO,
                    cgroup: 8,
                    cmask: 16,
                },
            ],
            ..MapData::default()
        };
        let walker = map.walker_triangles();
        assert_eq!(walker.len(), 1);
        assert_eq!(walker[0].walker, WalkerBehaviour::Walk);
        let b = walker[0].v[1];
        assert!((b - Vec3::new(9.0, 1.0, 0.0)).length() < 1e-5, "{b:?}");
        assert_eq!(map.bullet_triangles().len(), 1);
    }
}
