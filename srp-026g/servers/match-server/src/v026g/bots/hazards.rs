//! Where a body gets hurt by the map: the damage zones (anomalies, toxic
//! ground) of the project, from `mapdata` (`Zone`, kind `Damage`, with its
//! trigger shapes; an anomaly core names the zones it drives).
//!
//! Each floor of the walk grid that lies in a damage zone, or within
//! [`Hazards::margin`] of one, is marked with that zone. Routes keep off
//! the marked floors while the zone is active (`path::plan` with
//! [`Hazards::allowed`]), the motor never steers through one, and goals
//! are never put in one.
//!
//! Some anomalies switch on and off as a match runs (players' word). Until
//! the server knows when ([`Activity`]), every zone counts as active: the
//! bots keep off all of them, which costs a detour now and then and never a
//! life.

use mapdata::{NodeId, Vec3, World, ZoneKind};

/// Whether a damage zone hurts at a match time.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Activity {
    /// Always (toxic ground, or an anomaly whose timing is not known).
    #[default]
    Always,
    /// On in these match-time spans, `[from, to)` ms, off between them.
    Spans(Vec<(u32, u32)>),
}

impl Activity {
    pub fn active(&self, time: u32) -> bool {
        match self {
            Self::Always => true,
            Self::Spans(spans) => spans.iter().any(|&(from, to)| (from..to).contains(&time)),
        }
    }

    /// Whether it stays off from `time` for `ms`.
    pub fn off_for(&self, time: u32, ms: u32) -> bool {
        match self {
            Self::Always => false,
            Self::Spans(spans) => !spans
                .iter()
                .any(|&(from, to)| from < time + ms && to > time),
        }
    }
}

/// One damage zone.
#[derive(Clone, Debug)]
pub struct Hazard {
    /// The zone's `full_name`.
    pub name: String,
    /// The anomaly core that drives it, if one does.
    pub anomaly: Option<String>,
    pub damage_type: String,
    pub centre: Vec3,
    pub activity: Activity,
}

/// The map's damage zones and the floors they cover.
#[derive(Clone, Debug, Default)]
pub struct Hazards {
    pub zones: Vec<Hazard>,
    /// By floor: 0 none, else the zone's index + 1.
    floor_zone: Vec<u16>,
    pub margin: f32,
}

impl Hazards {
    /// The damage zones of `world`, each floor within `margin` metres of
    /// one marked. A pass over every floor against each zone's shapes: a
    /// few ms to a few tens on the largest maps, once per match.
    pub fn build(world: &World, margin: f32) -> Self {
        let map = &world.map;
        let mut zones = Vec::new();
        let mut shapes = Vec::new();
        for zone in map
            .zones
            .iter()
            .filter(|zone| zone.kind == ZoneKind::Damage)
        {
            let anomaly = map
                .anomalies
                .iter()
                .find(|anomaly| anomaly.zones.contains(&zone.name))
                .map(|anomaly| anomaly.name.clone());
            zones.push(Hazard {
                name: zone.name.clone(),
                anomaly,
                damage_type: zone.damage_type.clone(),
                centre: zone.position,
                activity: Activity::Always,
            });
            for shape in &zone.shapes {
                // A bounding radius, to skip far floors cheaply.
                let reach = shape.half.length() + margin;
                shapes.push((zones.len(), *shape, reach));
            }
        }
        let grid = &world.grid;
        let mut floor_zone = vec![0u16; grid.len()];
        // The margin as points round the feet, at the body's chest height
        // too (a sphere's edge may stand above the floor).
        let offsets: Vec<Vec3> = (0..8)
            .map(|k| {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                Vec3::new(a.cos() * margin, 0.0, a.sin() * margin)
            })
            .chain([Vec3::ZERO])
            .collect();
        for (node, mark) in floor_zone.iter_mut().enumerate() {
            let feet = grid.position(node as NodeId);
            for (index, shape, reach) in &shapes {
                if feet.distance_xz(shape.center) > *reach {
                    continue;
                }
                let hit = offsets.iter().any(|offset| {
                    [0.1, 0.9, 1.6]
                        .iter()
                        .any(|up| shape.contains(feet + *offset + Vec3::new(0.0, *up, 0.0)))
                });
                if hit {
                    *mark = *index as u16;
                    break;
                }
            }
        }
        Self {
            zones,
            floor_zone,
            margin,
        }
    }

    /// The zone marking `node`, if any.
    pub fn zone_of(&self, node: NodeId) -> Option<&Hazard> {
        let mark = *self.floor_zone.get(node as usize)?;
        (mark > 0).then(|| &self.zones[usize::from(mark) - 1])
    }

    /// Whether a body may be on `node` at `time`: no active zone there.
    pub fn allowed(&self, node: NodeId, time: u32) -> bool {
        self.zone_of(node)
            .is_none_or(|zone| !zone.activity.active(time))
    }

    /// Floors marked, for the logs.
    pub fn marked(&self) -> usize {
        self.floor_zone.iter().filter(|&&mark| mark > 0).count()
    }

    /// Whether `point` is near a zone (the floor under it is marked).
    pub fn near(&self, world: &World, point: Vec3, time: u32) -> bool {
        world
            .grid
            .locate(point, 1.0)
            .is_some_and(|node| !self.allowed(node, time))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapdata::{MapData, RayTree, Rotation, Shape, ShapeKind, WalkGrid, Zone};

    use crate::v026g::bots::body::tests::floor;

    pub fn with_anomaly() -> World {
        let grid = WalkGrid::build(
            &floor(0.0, 0.0, 40.0, 40.0, 0.0),
            [0.0, 0.0, 40.0, 40.0],
            0.25,
            mapdata::Body::default(),
        );
        let map = MapData {
            zones: vec![Zone {
                kind: ZoneKind::Damage,
                name: "logic/pool_zone".into(),
                team: -1,
                position: Vec3::new(20.0, 0.0, 20.0),
                damage_type: "fire".into(),
                shapes: vec![Shape {
                    kind: ShapeKind::Sphere,
                    center: Vec3::new(20.0, 1.0, 20.0),
                    rotation: Rotation::IDENTITY,
                    half: Vec3::new(3.0, 3.0, 3.0),
                }],
            }],
            anomalies: vec![mapdata::Anomaly {
                name: "core".into(),
                position: Vec3::new(20.0, 0.0, 20.0),
                zones: vec!["logic/pool_zone".into()],
            }],
            ..MapData::default()
        };
        World {
            map,
            grid,
            sight: RayTree::build(&[]),
        }
    }

    #[test]
    fn floors_in_and_near_an_anomaly_are_marked_and_routes_go_round() {
        let world = with_anomaly();
        let hazards = Hazards::build(&world, 1.0);
        assert_eq!(hazards.zones.len(), 1);
        assert_eq!(hazards.zones[0].anomaly.as_deref(), Some("core"));
        let grid = &world.grid;
        let inside = grid.locate(Vec3::new(20.0, 0.0, 20.0), 1.0).unwrap();
        let margin = grid.locate(Vec3::new(23.6, 0.0, 20.0), 1.0).unwrap();
        let clear = grid.locate(Vec3::new(26.0, 0.0, 20.0), 1.0).unwrap();
        assert!(!hazards.allowed(inside, 0));
        assert!(!hazards.allowed(margin, 0), "within the margin");
        assert!(hazards.allowed(clear, 0));
        let a = grid.locate(Vec3::new(10.0, 0.0, 20.0), 1.0).unwrap();
        let b = grid.locate(Vec3::new(30.0, 0.0, 20.0), 1.0).unwrap();
        let path = grid.path_where(a, b, |n| hazards.allowed(n, 0)).unwrap();
        assert!(path.nodes.iter().all(|&n| hazards.allowed(n, 0)));
    }

    #[test]
    fn a_zone_with_known_spans_is_open_while_it_is_off() {
        let activity = Activity::Spans(vec![(10_000, 20_000), (40_000, 50_000)]);
        assert!(!activity.active(5_000));
        assert!(activity.active(15_000));
        assert!(activity.off_for(20_000, 15_000));
        assert!(!activity.off_for(30_000, 15_000));
        assert!(!Activity::Always.off_for(0, 1));
    }
}
