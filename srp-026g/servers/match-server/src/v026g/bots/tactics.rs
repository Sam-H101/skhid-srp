//! Where to stand and how to get there: the `tactics` layer
//! (`player_bots_research.md` B1, B6). Plays ask; they never walk the grid
//! themselves.
//!
//! - **Routes** keep off the floors a body must not be on: a damage zone
//!   while it is active (`hazards`), and a spawn area (the owner's rule:
//!   never into the enemy's; never back into one's own once out of it).
//! - **Spots** ([`SpotQuery`]): floors at a distance band from a point,
//!   with a line of sight to it or hidden from another, higher or lower,
//!   off the hazards and spawn areas. Cheap filters first, rays last
//!   (research B5).
//! - **Exits** of a side's base: points 30-60 m out on the ways from its
//!   spawns toward the other side (play 15).

use std::sync::Arc;

use mapdata::regions::Regions;
use mapdata::{NodeId, Vec3, World};

use super::hazards::Hazards;
use super::path::{self, Route};
use super::rng::Rng;
use super::view::{EYE_CROUCHED, EYE_STANDING};

/// A spawn area: this far round a side's spawn points, metres (GUESS from
/// the maps: spawn points of a side lie in a few clusters 10-20 m across).
pub const SPAWN_AREA_M: f32 = 20.0;
/// A point is looked for on the grid within this, metres.
const LOCATE_M: f32 = 3.0;

/// What one map's tactics hold for one hazard margin: the same for every
/// match on the map, so matches share it ([`Tactics::of`]). Read-only once
/// built; a match's own state (the anomalies' clocks, the human routes)
/// is in [`Tactics`].
pub struct TacticsBase {
    pub world: Arc<World>,
    /// The zones and the floors they mark, every zone counting as active
    /// (no anomalies attached): each match clones it and attaches its own.
    hazards: Hazards,
    /// The bases' exits by map side, worked out once.
    exits: Vec<(i32, Vec<Vec3>)>,
    /// Per floor, a bit per side (`1 << side`, sides 0-7) whose spawn area
    /// it lies in ([`Tactics::allowed`]).
    spawn_areas: Vec<u8>,
    /// The grid's regions cut by the floors' labels (the zones' set and the
    /// spawn areas), so [`Tactics::allowed`] is the same on every floor of
    /// a region (`mapdata::regions`).
    regions: Regions,
}

/// One map's tactics for a match: the shared [`TacticsBase`] and the
/// match's own hazard clocks and human routes.
pub struct Tactics {
    base: Arc<TacticsBase>,
    pub world: Arc<World>,
    pub hazards: Hazards,
    /// The routes humans took on this map and mode (play 17), on its
    /// floors; empty without `SRP_BOT_ROUTES`.
    pub routes: Vec<super::routes::HumanRoute>,
}

/// The label's bits for the sides' spawn areas.
const SPAWN_BITS: u32 = 8;

/// What a spot is for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotQuery {
    pub around: Vec3,
    pub min_m: f32,
    pub max_m: f32,
    /// The spot must see this point (from crouched eyes).
    pub sight_to: Option<Vec3>,
    /// The spot must not be seen from this point.
    pub hidden_from: Option<Vec3>,
    /// Prefer spots higher than `around` (by this weight per metre).
    pub prefer_high: f32,
    /// The bot's map side: the other side's spawn area is out.
    pub side: Option<i32>,
    /// Candidates sampled before the rays.
    pub samples: usize,
}

impl SpotQuery {
    pub fn ring(around: Vec3, min_m: f32, max_m: f32) -> Self {
        Self {
            around,
            min_m,
            max_m,
            sight_to: None,
            hidden_from: None,
            prefer_high: 0.0,
            side: None,
            samples: 48,
        }
    }
}

/// A spot found, with its score (higher is better).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub at: Vec3,
    pub node: NodeId,
    pub score: f32,
}

impl TacticsBase {
    /// Builds them: the hazard marks, the floors' labels, the regions, the
    /// bases' exits. 0.15-0.3 s on the big maps on a fast machine, so off
    /// the match loop.
    pub fn build(world: Arc<World>, hazard_margin: f32) -> Self {
        let hazards = Hazards::build(&world, hazard_margin);
        let grid = &world.grid;
        // The zones' set in the high bits, the sides' spawn areas in the low.
        let mut labels: Vec<u32> = (0..grid.len() as NodeId)
            .map(|node| u32::from(hazards.set_of(node)) << SPAWN_BITS)
            .collect();
        for spawn in &world.map.spawns {
            let bit = 1u32 << (spawn.team.clamp(0, SPAWN_BITS as i32 - 1));
            let p = spawn.position;
            let area = [
                p.x - SPAWN_AREA_M,
                p.z - SPAWN_AREA_M,
                p.x + SPAWN_AREA_M,
                p.z + SPAWN_AREA_M,
            ];
            for node in grid.floors_in(area) {
                if grid.position(node).distance_xz(p) < SPAWN_AREA_M {
                    labels[node as usize] |= bit;
                }
            }
        }
        let regions = Regions::build(grid, Some(&labels));
        // Only the spawn areas are read from here on: the zones' set is
        // the hazards' own.
        let spawn_areas = labels.iter().map(|&label| label as u8).collect();
        drop(labels);
        let mut base = Self {
            world,
            hazards,
            exits: Vec::new(),
            spawn_areas,
            regions,
        };
        let sides: Vec<i32> = {
            let mut sides: Vec<i32> = base.world.map.spawns.iter().map(|s| s.team).collect();
            sides.sort_unstable();
            sides.dedup();
            sides
        };
        for side in sides {
            let exits = base.find_exits(side);
            base.exits.push((side, exits));
        }
        base
    }

    /// The hazard margin it was built for.
    pub fn margin(&self) -> f32 {
        self.hazards.margin
    }

    /// Bytes held, beyond the world.
    pub fn memory_bytes(&self) -> usize {
        self.hazards.marks().memory_bytes()
            + self.spawn_areas.len()
            + self.regions.memory_bytes()
            + self.exits.iter().map(|(_, exits)| exits.len() * 12).sum::<usize>()
    }

    /// What the tactics answer, as one number (`mapdata::digest`): the
    /// zones, their marks, the spawn areas, the regions and the exits.
    /// Equal digests, equal answers.
    pub fn digest(&self) -> u64 {
        let mut digest = mapdata::digest::Digest::new();
        digest.f32(self.hazards.margin);
        digest.u32(self.hazards.pass_ms);
        digest.len(self.hazards.zones.len());
        for zone in &self.hazards.zones {
            digest.len(zone.zone);
            for v in [zone.centre.x, zone.centre.y, zone.centre.z] {
                digest.f32(v);
            }
        }
        self.hazards.marks().digest_into(&mut digest);
        digest.len(self.spawn_areas.len());
        self.spawn_areas
            .iter()
            .for_each(|&bits| digest.u32(u32::from(bits)));
        self.regions.digest_into(&mut digest);
        digest.len(self.exits.len());
        for (side, exits) in &self.exits {
            digest.u32(*side as u32);
            digest.len(exits.len());
            for exit in exits {
                for v in [exit.x, exit.y, exit.z] {
                    digest.f32(v);
                }
            }
        }
        digest.finish()
    }

    /// [`Tactics::in_spawn_area`].
    fn in_spawn_area(&self, point: Vec3, side: Option<i32>) -> bool {
        self.world
            .map
            .spawns
            .iter()
            .filter(|spawn| side.is_none_or(|side| spawn.team == side))
            .any(|spawn| spawn.position.distance_xz(point) < SPAWN_AREA_M)
    }

    fn find_exits(&self, side: i32) -> Vec<Vec3> {
        let map = &self.world.map;
        let ours: Vec<Vec3> = map
            .spawns
            .iter()
            .filter(|s| s.team == side)
            .map(|s| s.position)
            .collect();
        let theirs: Vec<Vec3> = map
            .spawns
            .iter()
            .filter(|s| s.team != side)
            .map(|s| s.position)
            .collect();
        let Some(target) = centroid(&theirs) else {
            return Vec::new();
        };
        let mut exits: Vec<Vec3> = Vec::new();
        for spawn in ours.iter().step_by(ours.len().div_ceil(6).max(1)) {
            // The whole grid's search, the very cheapest way (once a map).
            let grid = &self.world.grid;
            let Some(route) = grid
                .locate(*spawn, 1.5)
                .zip(grid.locate(target, LOCATE_M))
                .and_then(|(from, to)| {
                    // Every zone counts as active here: no anomalies are
                    // attached to the base.
                    path::plan_where(grid, from, to, |node| self.hazards.allowed(node, 0))
                })
            else {
                continue;
            };
            // The first point 30-60 m along that is out of every spawn area.
            let mut s = 30.0;
            while s <= 60.0_f32.min(route.length()) {
                let point = route.point_at(s);
                if !self.in_spawn_area(point, None) {
                    if exits.iter().all(|e| e.distance_xz(point) > 10.0) {
                        exits.push(point);
                    }
                    break;
                }
                s += 5.0;
            }
        }
        exits
    }
}

impl Tactics {
    /// A match's tactics, built for it alone ([`TacticsBase::build`]).
    pub fn new(world: Arc<World>, hazard_margin: f32) -> Self {
        Self::of(Arc::new(TacticsBase::build(world, hazard_margin)))
    }

    /// A match's tactics on a shared base: its own copy of the zones (the
    /// marks stay shared), no routes yet.
    pub fn of(base: Arc<TacticsBase>) -> Self {
        Self {
            world: base.world.clone(),
            hazards: base.hazards.clone(),
            routes: Vec::new(),
            base,
        }
    }

    pub fn base(&self) -> &Arc<TacticsBase> {
        &self.base
    }

    /// The human routes of this map and mode, put on its floors; a route
    /// left with no point is dropped.
    pub fn set_routes(&mut self, routes: Vec<super::routes::HumanRoute>) {
        self.routes = routes
            .into_iter()
            .map(|route| route.on_floors(&self.world.grid))
            .filter(|route| !route.points.is_empty())
            .collect();
    }

    /// The human routes out of `side`'s base.
    pub fn routes_of(&self, side: i32) -> impl Iterator<Item = &super::routes::HumanRoute> {
        self.routes.iter().filter(move |route| route.team == side)
    }

    /// Whether `point` is in a spawn area of `side` (any side if `None`).
    pub fn in_spawn_area(&self, point: Vec3, side: Option<i32>) -> bool {
        self.base.in_spawn_area(point, side)
    }

    /// The map side whose spawn is nearest `point`.
    pub fn side_at(&self, point: Vec3) -> Option<i32> {
        self.world
            .map
            .spawns
            .iter()
            .min_by(|a, b| {
                a.position
                    .distance_xz(point)
                    .total_cmp(&b.position.distance_xz(point))
            })
            .map(|spawn| spawn.team)
    }

    /// Whether a body of map side `side` may walk on `node` at `time`: no
    /// active hazard, and not the other side's spawn area unless the goal
    /// is in their base (`objective`: their container, which lies 6-23 m
    /// from their spawns on the shipped maps). Its own spawn area is open:
    /// its container is there.
    pub fn allowed(&self, node: NodeId, time: u32, side: Option<i32>, objective: bool) -> bool {
        if !self.hazards.allowed(node, time) {
            return false;
        }
        let Some(side) = side.filter(|_| !objective) else {
            return true;
        };
        // No spawn area of another side.
        let own = 1u32 << side.clamp(0, SPAWN_BITS as i32 - 1);
        let spawn_areas = self
            .base
            .spawn_areas
            .get(node as usize)
            .map_or(0, |&bits| u32::from(bits));
        spawn_areas & !own == 0
    }

    /// [`Self::allowed`] for a whole region (its floors share their label).
    pub fn region_allowed(
        &self,
        region: u32,
        time: u32,
        side: Option<i32>,
        objective: bool,
    ) -> bool {
        self.allowed(self.base.regions.middle(region), time, side, objective)
    }

    /// The regions the routes go by.
    pub fn regions(&self) -> &Regions {
        &self.base.regions
    }

    /// The route from `from` to `to` for a body of map side `side`, off
    /// their spawn area.
    pub fn route(&self, from: Vec3, to: Vec3, time: u32, side: Option<i32>) -> Option<Route> {
        self.route_for(from, to, time, side, false)
    }

    /// [`Self::route`], into their base if `objective`.
    pub fn route_for(
        &self,
        from: Vec3,
        to: Vec3,
        time: u32,
        side: Option<i32>,
        objective: bool,
    ) -> Option<Route> {
        let grid = &self.world.grid;
        let start = grid.locate(from, 1.5)?;
        let goal = grid.locate(to, LOCATE_M)?;
        self.route_nodes(start, goal, time, side, objective, |_| true)
    }

    /// [`Self::route_for`] between floors, keeping off the floors `avoid`
    /// shuts too (the spots a bot got stuck at).
    pub fn route_nodes(
        &self,
        start: NodeId,
        goal: NodeId,
        time: u32,
        side: Option<i32>,
        objective: bool,
        open: impl Fn(NodeId) -> bool,
    ) -> Option<Route> {
        path::plan_in(
            &self.world.grid,
            &self.base.regions,
            start,
            goal,
            |node| self.allowed(node, time, side, objective) && open(node),
            |region| self.region_allowed(region, time, side, objective),
        )
    }

    /// Whether a body walks from `a` to `b` in a line, on allowed floors.
    pub fn clear_line(&self, a: Vec3, b: Vec3, time: u32, side: Option<i32>) -> bool {
        self.clear_line_for(a, b, time, side, false)
    }

    /// [`Self::clear_line`], into their base if `objective`.
    pub fn clear_line_for(
        &self,
        a: Vec3,
        b: Vec3,
        time: u32,
        side: Option<i32>,
        objective: bool,
    ) -> bool {
        let grid = &self.world.grid;
        let (Some(from), Some(to)) = (grid.locate(a, 1.0), grid.locate(b, 0.6)) else {
            return false;
        };
        path::straight_where(grid, from, to, |node| {
            self.allowed(node, time, side, objective)
        })
    }

    /// Whether `point` is a place a goal may be: on a floor, off hazards
    /// and the other side's spawn area.
    pub fn fine_goal(&self, point: Vec3, time: u32, side: Option<i32>) -> bool {
        self.world
            .grid
            .locate(point, LOCATE_M)
            .is_some_and(|node| self.allowed(node, time, side, false))
    }

    /// Whether `b` is seen from `a` (points at the eyes).
    pub fn sees(&self, a: Vec3, b: Vec3) -> bool {
        self.world.sight.visible(a, b)
    }

    /// The best spots for `query`, best first, at most `n`.
    pub fn spots(&self, query: &SpotQuery, time: u32, n: usize, rng: &mut Rng) -> Vec<Spot> {
        let grid = &self.world.grid;
        let mut found: Vec<Spot> = Vec::new();
        for _ in 0..query.samples {
            let angle = rng.range(0.0, std::f32::consts::TAU);
            let r = rng.range(query.min_m, query.max_m);
            let probe = Vec3::new(
                query.around.x + r * angle.cos(),
                query.around.y,
                query.around.z + r * angle.sin(),
            );
            let Some(node) = grid.locate(Vec3::new(probe.x, probe.y + 1.0, probe.z), 2.0) else {
                continue;
            };
            let at = grid.position(node);
            let d = at.distance_xz(query.around);
            if d < query.min_m || d > query.max_m || !self.allowed(node, time, query.side, false) {
                continue;
            }
            if found.iter().any(|spot| spot.node == node) {
                continue;
            }
            let score = query.prefer_high * (at.y - query.around.y) - d * 0.01;
            found.push(Spot { at, node, score });
        }
        found.sort_by(|a, b| b.score.total_cmp(&a.score));
        // Rays lazily, from the best down.
        let mut out = Vec::new();
        for spot in found {
            if out.len() >= n {
                break;
            }
            let eye = spot.at + Vec3::new(0.0, EYE_CROUCHED, 0.0);
            if let Some(target) = query.sight_to
                && !self.sees(eye, target + Vec3::new(0.0, 1.0, 0.0))
            {
                continue;
            }
            if let Some(from) = query.hidden_from
                && self.sees(
                    from + Vec3::new(0.0, EYE_STANDING, 0.0),
                    spot.at + Vec3::new(0.0, EYE_STANDING, 0.0),
                )
            {
                continue;
            }
            out.push(spot);
        }
        out
    }

    /// The exits of `side`'s base: points 30-60 m out on the ways from its
    /// spawns toward the other side's, outside the spawn areas.
    pub fn exits(&self, side: i32) -> &[Vec3] {
        self.base
            .exits
            .iter()
            .find(|(s, _)| *s == side)
            .map_or(&[], |(_, exits)| exits.as_slice())
    }

}

fn centroid(points: &[Vec3]) -> Option<Vec3> {
    if points.is_empty() {
        return None;
    }
    let n = points.len() as f32;
    let sum = points.iter().fold(Vec3::ZERO, |a, &b| a + b);
    Some(Vec3::new(sum.x / n, sum.y / n, sum.z / n))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use mapdata::{MapData, RayTree, Spawn, WalkGrid};

    use crate::v026g::bots::body::tests::{floor, wall_x};

    /// An 80 x 40 m field: side 0 spawns at x = 5, side 1 at x = 75, a
    /// wall across x = 40 from z = 0 to 25 (the way round is past z = 25).
    pub fn field() -> Arc<World> {
        field_with(|_| {})
    }

    /// [`field`] with its map data changed by `edit` (containers,
    /// batteries).
    pub fn field_with(edit: impl FnOnce(&mut MapData)) -> Arc<World> {
        let mut triangles = floor(0.0, 0.0, 80.0, 40.0, 0.0).to_vec();
        triangles.extend(wall_x(40.0, 0.0, 25.0));
        let grid = WalkGrid::build(
            &triangles,
            [0.0, 0.0, 80.0, 40.0],
            0.25,
            mapdata::Body::default(),
        );
        let spawn = |team: i32, x: f32, z: f32| Spawn {
            position: Vec3::new(x, 0.0, z),
            rotation: Vec3::ZERO,
            team,
            priority: 0,
            point_id: 0,
            layer: "default".into(),
        };
        let mut map = MapData {
            spawns: vec![
                spawn(0, 5.0, 10.0),
                spawn(0, 5.0, 20.0),
                spawn(1, 75.0, 10.0),
                spawn(1, 75.0, 20.0),
            ],
            ..MapData::default()
        };
        edit(&mut map);
        let sight = RayTree::build(&triangles);
        Arc::new(World::assemble(map, grid.into(), sight.into()))
    }

    #[test]
    fn routes_keep_out_of_the_other_sides_spawn_area_unless_bound_for_its_container() {
        let tactics = Tactics::new(field(), 1.0);
        // From side 0's spawn to beside side 1's area: out of its own, never
        // into theirs.
        let route = tactics
            .route(
                Vec3::new(5.0, 0.0, 10.0),
                Vec3::new(52.0, 0.0, 10.0),
                0,
                Some(0),
            )
            .unwrap();
        let mut s = 0.0;
        while s < route.length() {
            let at = route.point_at(s);
            assert!(!tactics.in_spawn_area(at, Some(1)), "{at:?}");
            s += 0.5;
        }
        // Bound for their container (inside their area): it may go in.
        let into = tactics
            .route_for(
                Vec3::new(5.0, 0.0, 10.0),
                Vec3::new(72.0, 0.0, 12.0),
                0,
                Some(0),
                true,
            )
            .unwrap();
        assert!(into.end().distance_xz(Vec3::new(72.0, 0.0, 12.0)) < 1.0);
        assert!(
            tactics
                .route(
                    Vec3::new(5.0, 0.0, 10.0),
                    Vec3::new(72.0, 0.0, 12.0),
                    0,
                    Some(0)
                )
                .is_none()
        );
        // A goal in their spawn area is no goal.
        assert!(!tactics.fine_goal(Vec3::new(75.0, 0.0, 15.0), 0, Some(0)));
        assert!(tactics.fine_goal(Vec3::new(40.0, 0.0, 35.0), 0, Some(0)));
        // The exits of side 1 are out of every spawn area, 30-60 m from it.
        let exits = tactics.exits(1);
        assert!(!exits.is_empty());
        for exit in exits {
            assert!(!tactics.in_spawn_area(*exit, None));
            assert!(exit.distance_xz(Vec3::new(75.0, 0.0, 15.0)) < 62.0);
        }
    }

    #[test]
    fn matches_share_one_base_and_keep_their_own_clocks() {
        let world = field();
        let base = Arc::new(TacticsBase::build(world.clone(), 1.0));
        assert_eq!(base.digest(), TacticsBase::build(world, 1.0).digest());
        assert_ne!(
            base.digest(),
            TacticsBase::build(base.world.clone(), 2.0).digest()
        );
        let (mut one, two) = (Tactics::of(base.clone()), Tactics::of(base.clone()));
        assert!(Arc::ptr_eq(one.base(), two.base()));
        assert_eq!(Arc::strong_count(&base), 3);
        // One match's clock is its own.
        one.hazards.pass_ms += 1;
        assert_ne!(one.hazards.pass_ms, two.hazards.pass_ms);
        // A match built alone answers as one on the shared base.
        let alone = Tactics::new(base.world.clone(), 1.0);
        assert_eq!(alone.base().digest(), base.digest());
        assert_eq!(alone.exits(1), two.exits(1));
    }

    #[test]
    fn spots_see_their_point_and_hide_from_another() {
        let tactics = Tactics::new(field(), 1.0);
        let target = Vec3::new(30.0, 0.0, 10.0);
        let from = Vec3::new(50.0, 0.0, 10.0);
        let query = SpotQuery {
            sight_to: Some(target),
            hidden_from: Some(from),
            samples: 200,
            ..SpotQuery::ring(target, 10.0, 25.0)
        };
        let spots = tactics.spots(&query, 0, 5, &mut Rng::new(5));
        assert!(!spots.is_empty());
        for spot in &spots {
            let d = spot.at.distance_xz(target);
            assert!((10.0..=25.0).contains(&d));
            // Behind the wall from the enemy's side: x < 40.
            assert!(spot.at.x < 40.0, "{spot:?}");
        }
    }
}
