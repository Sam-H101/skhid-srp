//! Map knowledge for server-driven player bots (0.26g): spawns and
//! objectives, a walk grid with paths, and line of sight, from the map
//! projects in the game's `resources.db`. The notes are
//! `docs/notes/player_bots_mapdata.md` (how to dump, the file format, the
//! API, timings) and `docs/notes/player_bots.md` §5.
//!
//! - [`MapData`]: one project's objectives and placed collision. Loaded
//!   from `resources.db` ([`MapData::load`], the dump step) or from the
//!   file `mapdata-dump` writes ([`MapData::read`], the match server). The
//!   game's data is never part of this repository.
//! - [`WalkGrid`]: where the walking body stands and how it moves, from
//!   the walker mesh; [`WalkGrid::distances_to`] floods a target once for
//!   every bot heading there, [`WalkGrid::path`] is A* for one path.
//! - [`RayTree`]: line of sight on the bullet mesh ([`RayTree::visible`]).
//! - [`World`]: the three together for one match.
//!
//! Everything is in the map frame: left-handed, Y up, positions at the
//! feet. Values from the physics engine (the uploaded worlds' `m_last_pos`)
//! are in another frame, z negated, at the capsule's centre: see
//! [`math`] and [`PhysicsPoint`].
//!
//! ```no_run
//! use mapdata::{Body, MapData, Vec3, World};
//!
//! let map = MapData::read("maps/level_01_day_4.mapdata".as_ref()).unwrap();
//! let world = World::build(map, mapdata::CELL, Body::default());
//! let spawn = world.map.spawns[0].position;
//! let enemy = world.map.spawns.iter().find(|s| s.team == 1).unwrap().position;
//! let (from, to) = (world.grid.locate(spawn, 1.0).unwrap(), world.grid.locate(enemy, 1.0).unwrap());
//! let to_enemy = world.grid.distances_to(&[to]);
//! let (next, _how) = to_enemy.next_step(&world.grid, from).unwrap();
//! let eye = Vec3::new(0.0, 1.6, 0.0);
//! let sees = world.sight.visible(spawn + eye, enemy + eye);
//! # let _ = (next, sees);
//! ```

mod config;
pub mod format;
pub mod grid;
pub mod library;
pub mod math;
pub mod project;
pub mod resources;
pub mod sight;

pub use grid::{Body, BuildStats, DistanceMap, LinkKind, NodeId, Path, WalkGrid};
pub use library::{Library, Materials, WalkerBehaviour};
pub use math::{PhysicsPoint, Rotation, Transform, Vec3};
pub use project::{
    Anomaly, AnomalyArtefacts, AnomalyEnergy, AnomalyState, BatterySpawner, CollisionFilter,
    Container, Damage, MapData, Placement, Shape, ShapeKind, Spawn, Triangle, Zone, ZoneGroup,
    ZoneKind,
};
pub use resources::{Error, Resources};
pub use sight::RayTree;

/// A play area margin around the `levelmap` box, metres.
pub const AREA_MARGIN: f32 = 5.0;

/// The walk grid's cell size to use, metres. 0.5 m cells shut 1 m doors
/// (level_02 and level_04 fall apart); 0.25 m cells keep them.
pub const CELL: f32 = 0.25;

/// One map ready for bots: its data, walk grid and sight tree.
pub struct World {
    pub map: MapData,
    pub grid: WalkGrid,
    pub sight: RayTree,
}

impl World {
    /// Builds the grid (cells of `cell` metres over the play area, normally
    /// [`CELL`]) and the sight tree: 0.3-0.8 s for a 0.26g map at 0.25 m
    /// (timings in the notes).
    pub fn build(map: MapData, cell: f32, body: Body) -> Self {
        let grid = WalkGrid::build(
            &map.walker_triangles(),
            map.play_area(AREA_MARGIN),
            cell,
            body,
        );
        let sight = RayTree::build(&map.bullet_triangles());
        Self { map, grid, sight }
    }
}
