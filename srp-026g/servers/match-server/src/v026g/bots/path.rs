//! Routes a body can walk: the walk grid's A* path, pulled straight.
//!
//! A grid path goes cell to cell in eight directions: followed as it is,
//! a body would zig-zag and turn every few steps, which no player does.
//! So the path is pulled tight: from each corner the route goes straight
//! to the farthest later floor it can walk to in a line, and only real
//! corners are left (a door, the end of a wall). A jump or a drop is
//! always a corner of its own: the line may not cross it.
//!
//! A [`Route`] is followed by looking ahead along it ([`Route::progress`],
//! [`Route::point_at`]): the body steers to a point a few metres on, so it
//! rounds the corners in a curve as a player does, instead of reaching
//! each one and turning on the spot.

use mapdata::{LinkKind, NodeId, Vec3, WalkGrid};

/// A straight line is not tried farther than this (metres): the check
/// walks every cell of it.
const MAX_STRAIGHT_M: f32 = 40.0;

/// A corner of a route.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waypoint {
    pub at: Vec3,
    pub node: NodeId,
    /// Reached by a jump from the waypoint before.
    pub jump: bool,
}

#[derive(Clone, Debug)]
pub struct Route {
    points: Vec<Waypoint>,
    /// The distance along the route to each point.
    along: Vec<f32>,
    /// The segment the body was last found on.
    segment: usize,
}

/// The route from one floor to another, if there is one.
pub fn plan(grid: &WalkGrid, from: NodeId, to: NodeId) -> Option<Route> {
    plan_where(grid, from, to, |_| true)
}

/// [`plan`] over the floors `allowed` lets through (an anomaly's, a spawn
/// area's are not): the A* keeps off them, and so do the straight legs.
pub fn plan_where(
    grid: &WalkGrid,
    from: NodeId,
    to: NodeId,
    allowed: impl Fn(NodeId) -> bool,
) -> Option<Route> {
    let path = grid.path_where(from, to, &allowed)?;
    let nodes = path.nodes;
    // The kind of the link into each node.
    let mut into = vec![LinkKind::Walk; nodes.len()];
    for k in 1..nodes.len() {
        into[k] = grid
            .links(nodes[k - 1])
            .find(|(next, _)| *next == nodes[k])
            .map_or(LinkKind::Walk, |(_, kind)| kind);
    }
    let mut points = vec![Waypoint {
        at: grid.position(nodes[0]),
        node: nodes[0],
        jump: false,
    }];
    let mut i = 0;
    while i + 1 < nodes.len() {
        // A jump or a drop is taken as it is, corner to corner.
        if into[i + 1] != LinkKind::Walk {
            i += 1;
            points.push(Waypoint {
                at: grid.position(nodes[i]),
                node: nodes[i],
                jump: into[i] == LinkKind::Jump,
            });
            continue;
        }
        let mut j = i + 1;
        while j + 1 < nodes.len()
            && into[j + 1] == LinkKind::Walk
            && straight_where(grid, nodes[i], nodes[j + 1], &allowed)
        {
            j += 1;
        }
        i = j;
        points.push(Waypoint {
            at: grid.position(nodes[i]),
            node: nodes[i],
            jump: false,
        });
    }
    Some(Route::new(points))
}

/// Whether a body walks from `a` to `b` in a straight line: every half
/// cell of the line is on a floor walked to from the one before (no jump,
/// no drop), and the line stays within a step's height of the floors.
pub fn straight(grid: &WalkGrid, a: NodeId, b: NodeId) -> bool {
    straight_where(grid, a, b, |_| true)
}

/// [`straight`] over allowed floors only.
pub fn straight_where(
    grid: &WalkGrid,
    a: NodeId,
    b: NodeId,
    allowed: impl Fn(NodeId) -> bool,
) -> bool {
    let (from, to) = (grid.position(a), grid.position(b));
    let length = from.distance_xz(to);
    if length > MAX_STRAIGHT_M {
        return false;
    }
    let half = grid.cell_size() * 0.5;
    let steps = ((length / half).ceil() as usize).max(1);
    let mut node = a;
    for k in 1..=steps {
        let t = k as f32 / steps as f32;
        let point = Vec3::new(
            from.x + (to.x - from.x) * t,
            0.0,
            from.z + (to.z - from.z) * t,
        );
        let within = |n: NodeId| {
            let p = grid.position(n);
            (p.x - point.x).abs() <= half + 1e-4 && (p.z - point.z).abs() <= half + 1e-4
        };
        if within(node) {
            continue;
        }
        match grid
            .links(node)
            .filter(|(_, kind)| *kind == LinkKind::Walk)
            .map(|(next, _)| next)
            .find(|&next| within(next))
        {
            Some(next) if allowed(next) || next == b => node = next,
            _ => return false,
        }
    }
    node == b
}

impl Route {
    pub fn new(points: Vec<Waypoint>) -> Self {
        let mut along = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (k, point) in points.iter().enumerate() {
            if k > 0 {
                total += points[k - 1].at.distance_xz(point.at);
            }
            along.push(total);
        }
        Self {
            points,
            along,
            segment: 0,
        }
    }

    pub fn points(&self) -> &[Waypoint] {
        &self.points
    }

    pub fn length(&self) -> f32 {
        self.along.last().copied().unwrap_or(0.0)
    }

    pub fn end(&self) -> Vec3 {
        self.points.last().map_or(Vec3::ZERO, |point| point.at)
    }

    /// How far along the route the body at `feet` is, and how far it is
    /// off the route (both on the ground plane). The body is looked for on
    /// the segment it was last on and the next few, never behind: a route
    /// that passes the same place twice is followed in order.
    pub fn progress(&mut self, feet: Vec3) -> (f32, f32) {
        if self.points.len() < 2 {
            return (0.0, self.end().distance_xz(feet));
        }
        let last = self.points.len() - 2;
        let mut best = (f32::INFINITY, self.segment, 0.0);
        for segment in self.segment..=(self.segment + 3).min(last) {
            let (a, b) = (self.points[segment].at, self.points[segment + 1].at);
            let (dx, dz) = (b.x - a.x, b.z - a.z);
            let length2 = dx * dx + dz * dz;
            let t = if length2 < 1e-6 {
                0.0
            } else {
                (((feet.x - a.x) * dx + (feet.z - a.z) * dz) / length2).clamp(0.0, 1.0)
            };
            let (px, pz) = (a.x + dx * t, a.z + dz * t);
            let off = ((feet.x - px).powi(2) + (feet.z - pz).powi(2)).sqrt();
            if off < best.0 - 1e-3 {
                best = (off, segment, t * length2.sqrt());
            }
        }
        self.segment = best.1;
        (self.along[best.1] + best.2, best.0)
    }

    /// The point `s` metres along the route (the end past it).
    pub fn point_at(&self, s: f32) -> Vec3 {
        if s <= 0.0 || self.points.len() < 2 {
            return self.points.first().map_or(Vec3::ZERO, |point| point.at);
        }
        let k = self.along.partition_point(|&a| a <= s);
        if k >= self.points.len() {
            return self.end();
        }
        let (a, b) = (self.points[k - 1].at, self.points[k].at);
        let span = self.along[k] - self.along[k - 1];
        let t = if span < 1e-6 {
            1.0
        } else {
            (s - self.along[k - 1]) / span
        };
        a + (b - a) * t
    }

    /// From `s` on: the distance to the next corner that turns by more than
    /// `angle` radians (or to the end), and that corner's waypoint index.
    pub fn straight_ahead(&self, s: f32, angle: f32) -> (f32, Option<usize>) {
        let first = self.along.partition_point(|&a| a <= s);
        for k in first..self.points.len().saturating_sub(1) {
            if self.points[k + 1].jump || self.turn_at(k) > angle {
                return (self.along[k] - s, Some(k));
            }
        }
        ((self.length() - s).max(0.0), None)
    }

    /// The next waypoint after `s` that is reached by a jump: its index and
    /// the distance to the waypoint before it (where the jump starts).
    pub fn next_jump(&self, s: f32) -> Option<(usize, f32)> {
        let first = self.along.partition_point(|&a| a <= s).max(1);
        (first..self.points.len())
            .find(|&k| self.points[k].jump)
            .map(|k| (k, self.along[k - 1] - s))
    }

    /// How much the route turns at waypoint `k`, radians (0 at the ends).
    pub fn turn_at(&self, k: usize) -> f32 {
        if k == 0 || k + 1 >= self.points.len() {
            return 0.0;
        }
        let (a, b, c) = (
            self.points[k - 1].at,
            self.points[k].at,
            self.points[k + 1].at,
        );
        let (ux, uz, vx, vz) = (b.x - a.x, b.z - a.z, c.x - b.x, c.z - b.z);
        let (nu, nv) = ((ux * ux + uz * uz).sqrt(), (vx * vx + vz * vz).sqrt());
        if nu < 1e-4 || nv < 1e-4 {
            return 0.0;
        }
        ((ux * vx + uz * vz) / (nu * nv)).clamp(-1.0, 1.0).acos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v026g::bots::body::tests::walled;

    #[test]
    fn a_path_round_a_wall_is_pulled_to_a_few_corners() {
        let grid = walled();
        let from = grid.locate(Vec3::new(10.0, 0.0, 10.0), 1.0).unwrap();
        let to = grid.locate(Vec3::new(30.0, 0.0, 10.0), 1.0).unwrap();
        let path = grid.path(from, to).unwrap();
        let route = plan(&grid, from, to).unwrap();
        // Cell by cell the path has a hundred-odd floors; pulled straight,
        // a few corners at the wall's end.
        assert!(path.nodes.len() > 80, "{}", path.nodes.len());
        assert!(
            (3..=6).contains(&route.points().len()),
            "{:?}",
            route.points()
        );
        // Each leg is walkable straight, and it goes round past z = 30.
        for pair in route.points().windows(2) {
            assert!(straight(&grid, pair[0].node, pair[1].node));
        }
        assert!(route.points().iter().any(|point| point.at.z > 30.0));
        // About as long as the grid path, shorter than its zig-zag.
        assert!(route.length() <= path.cost + 0.01);
        assert!(route.length() > 2.0 * (20.0_f32.hypot(10.0)) - 1.0);
        // A line through the wall is not straight.
        assert!(!straight(&grid, from, to));
        // On open floor the route is one line.
        let near = grid.locate(Vec3::new(12.0, 0.0, 25.0), 1.0).unwrap();
        assert_eq!(plan(&grid, from, near).unwrap().points().len(), 2);
    }

    #[test]
    fn progress_and_points_along_a_route() {
        let p = |x, z| Waypoint {
            at: Vec3::new(x, 0.0, z),
            node: 0,
            jump: false,
        };
        let mut route = Route::new(vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)]);
        assert_eq!(route.length(), 20.0);
        let (s, off) = route.progress(Vec3::new(4.0, 0.0, 1.0));
        assert!((s - 4.0).abs() < 1e-4 && (off - 1.0).abs() < 1e-4);
        assert_eq!(route.point_at(15.0), Vec3::new(10.0, 0.0, 5.0));
        assert_eq!(route.point_at(99.0), Vec3::new(10.0, 0.0, 10.0));
        // A right angle at the corner, 6 m ahead.
        let (ahead, corner) = route.straight_ahead(4.0, 0.5);
        assert!((ahead - 6.0).abs() < 1e-4 && corner == Some(1));
        assert!((route.turn_at(1) - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        // Progress never goes back to an earlier segment.
        let (s, _) = route.progress(Vec3::new(10.5, 0.0, 6.0));
        assert!((s - 16.0).abs() < 1e-4);
        let (s, _) = route.progress(Vec3::new(5.0, 0.0, 0.0));
        assert!(s >= 10.0, "{s}");
    }
}
