//! The walk grid's coarse layer: the floors of each 8 m block cut into
//! regions, and the links between regions (hierarchical path finding, as
//! HPA* and the navigation meshes of games do it).
//!
//! A region is the floors of one block joined by walk links both ways and
//! of one label (the caller's: which spawn area, which anomalies cover the
//! floor), so any floor of a region reaches any other inside it, and a
//! filter of allowed floors that goes by the labels lets a region through
//! whole or not at all. Region A links to
//! region B when any floor of A links to a floor of B (a walk, drop or
//! jump; one way when the floor link is). So a way between two floors
//! exists exactly when one exists between their regions, and a search on
//! the regions (a few thousand, against up to 2.3 million floors on the
//! real maps) answers it in well under a millisecond.
//!
//! [`WalkGrid::route_search`] uses it: the regions' search gives the
//! corridor, the floors' A* runs inside the corridor only (what it expands
//! is the corridor's few thousand floors, not the map's), and its route is
//! the floors' own, so it bends no worse than a search of the whole grid
//! round anything the corridor holds. The regions' search keeps off the
//! regions the caller's region filter shuts, so a filter that goes by the
//! labels is met exactly; one that does not (the spots a bot got stuck
//! at) can still shut the corridor: then it is widened by the regions next
//! to it, and last the whole grid is searched.

use std::cell::RefCell;
use std::collections::BinaryHeap;

use crate::grid::{Entry, LinkKind, NodeId, Search, WalkGrid};
use crate::math::Vec3;

/// A block's side, in cells (8 m at 0.25 m cells).
pub const BLOCK_CELLS: usize = 32;
/// Rings of neighbour regions the first corridor takes in: with one, the
/// route found is the whole grid's within 1-2 % (`real_route_search`).
pub const CORRIDOR_RINGS: usize = 1;
/// Rings more when the allowed floors shut the corridor.
pub const WIDE_RINGS: usize = 3;

const NONE: u32 = u32::MAX;

/// See the module documentation.
#[derive(Clone, Debug, Default)]
pub struct Regions {
    /// Per floor, its region.
    of: Vec<u32>,
    /// Per region, the floor nearest its middle.
    middle: Vec<NodeId>,
    /// Per region, where its links out start in `out`; one more entry.
    out_start: Vec<u32>,
    /// The links out: the region reached and the cost (between the
    /// regions' middles, plus the link kind's penalty).
    out: Vec<(u32, f32)>,
    /// The links in, as `out_start` and `out` (the region linking).
    in_start: Vec<u32>,
    inn: Vec<u32>,
}

impl Regions {
    /// The regions of `grid`, cut by `label` (one per floor) when given.
    pub fn build(grid: &WalkGrid, label: Option<&[u32]>) -> Self {
        let n = grid.len();
        let same = |a: NodeId, b: NodeId| label.is_none_or(|l| l[a as usize] == l[b as usize]);
        // Floors of one block joined by walk links both ways.
        let mut root: Vec<u32> = (0..n as u32).collect();
        for a in 0..n as NodeId {
            let block = grid.block_of(a, BLOCK_CELLS);
            for (b, kind) in grid.links(a) {
                if kind != LinkKind::Walk
                    || b < a
                    || grid.block_of(b, BLOCK_CELLS) != block
                    || !same(a, b)
                {
                    continue;
                }
                if !grid.links(b).any(|(c, k)| c == a && k == LinkKind::Walk) {
                    continue;
                }
                let (ra, rb) = (find(&mut root, a), find(&mut root, b));
                if ra != rb {
                    root[ra.max(rb) as usize] = ra.min(rb);
                }
            }
        }
        let mut of = vec![NONE; n];
        let mut count = 0u32;
        for a in 0..n as NodeId {
            let r = find(&mut root, a);
            if of[r as usize] == NONE {
                of[r as usize] = count;
                count += 1;
            }
            of[a as usize] = of[r as usize];
        }
        drop(root);
        // The middles: the mean of each region's floors, then the floor
        // nearest it.
        let regions = count as usize;
        let mut sum = vec![(0.0f64, 0.0f64, 0.0f64, 0u32); regions];
        for a in 0..n as NodeId {
            let p = grid.position(a);
            let s = &mut sum[of[a as usize] as usize];
            s.0 += f64::from(p.x);
            s.1 += f64::from(p.y);
            s.2 += f64::from(p.z);
            s.3 += 1;
        }
        let mean: Vec<Vec3> = sum
            .iter()
            .map(|s| {
                let k = f64::from(s.3.max(1));
                Vec3::new((s.0 / k) as f32, (s.1 / k) as f32, (s.2 / k) as f32)
            })
            .collect();
        drop(sum);
        let mut middle = vec![(NONE, f32::MAX); regions];
        for a in 0..n as NodeId {
            let r = of[a as usize] as usize;
            let d = (grid.position(a) - mean[r]).length();
            if d < middle[r].1 {
                middle[r] = (a, d);
            }
        }
        let middle: Vec<NodeId> = middle.into_iter().map(|(a, _)| a).collect();
        // The links between regions, the cheapest kind of each pair.
        let mut pairs: Vec<(u32, u32, u8)> = Vec::new();
        for a in 0..n as NodeId {
            let ra = of[a as usize];
            for (b, kind) in grid.links(a) {
                let rb = of[b as usize];
                if ra != rb {
                    pairs.push((ra, rb, kind as u8));
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup_by(|later, first| later.0 == first.0 && later.1 == first.1);
        let penalty = |kind: u8| {
            match kind {
                0 => LinkKind::Walk,
                1 => LinkKind::Drop,
                _ => LinkKind::Jump,
            }
            .penalty()
        };
        let mut out_start = Vec::with_capacity(regions + 1);
        let mut out = Vec::with_capacity(pairs.len());
        let mut k = 0;
        for r in 0..regions as u32 {
            out_start.push(out.len() as u32);
            while k < pairs.len() && pairs[k].0 == r {
                let (_, to, kind) = pairs[k];
                let cost = (grid.position(middle[to as usize]) - grid.position(middle[r as usize]))
                    .length()
                    + penalty(kind);
                out.push((to, cost));
                k += 1;
            }
        }
        out_start.push(out.len() as u32);
        let mut in_count = vec![0u32; regions + 1];
        for &(to, _) in &out {
            in_count[to as usize + 1] += 1;
        }
        for r in 0..regions {
            in_count[r + 1] += in_count[r];
        }
        let in_start = in_count.clone();
        let mut inn = vec![0u32; out.len()];
        for r in 0..regions as u32 {
            for &(to, _) in &out[out_start[r as usize] as usize..out_start[r as usize + 1] as usize]
            {
                inn[in_count[to as usize] as usize] = r;
                in_count[to as usize] += 1;
            }
        }
        Self {
            in_start,
            inn,
            of,
            middle,
            out_start,
            out,
        }
    }

    pub fn len(&self) -> usize {
        self.middle.len()
    }

    pub fn is_empty(&self) -> bool {
        self.middle.is_empty()
    }

    /// The region of `node`.
    pub fn of(&self, node: NodeId) -> u32 {
        self.of[node as usize]
    }

    /// The floor nearest region `r`'s middle (any floor of it stands for
    /// its label).
    pub fn middle(&self, r: u32) -> NodeId {
        self.middle[r as usize]
    }

    /// The links out of region `r`.
    pub fn out(&self, r: u32) -> &[(u32, f32)] {
        &self.out[self.out_start[r as usize] as usize..self.out_start[r as usize + 1] as usize]
    }

    /// The regions linking into region `r`.
    pub fn into(&self, r: u32) -> &[u32] {
        &self.inn[self.in_start[r as usize] as usize..self.in_start[r as usize + 1] as usize]
    }

    /// Into `digest` ([`crate::digest`]): every floor's region, every
    /// region's middle and links.
    pub fn digest_into(&self, digest: &mut crate::digest::Digest) {
        for array in [&self.of, &self.middle, &self.out_start, &self.in_start, &self.inn] {
            digest.len(array.len());
            array.iter().for_each(|&value| digest.u32(value));
        }
        digest.len(self.out.len());
        for &(region, cost) in &self.out {
            digest.u32(region);
            digest.f32(cost);
        }
    }

    pub fn memory_bytes(&self) -> usize {
        self.of.len() * 4
            + self.middle.len() * 4
            + (self.out_start.len() + self.in_start.len()) * 4
            + self.out.len() * 12
    }

    /// The regions of the cheapest way from region `from` to `to` (both
    /// included), by A* over the regions; `None` when there is no way.
    fn corridor(
        &self,
        grid: &WalkGrid,
        from: u32,
        to: u32,
        region_ok: &impl Fn(u32) -> bool,
        marks: &mut Marks,
    ) -> Option<Vec<u32>> {
        let regions = self.len();
        marks.fit(regions);
        let (seen, closed) = marks.next();
        let Marks {
            mark,
            cost,
            parent,
            open,
            ..
        } = marks;
        let goal = grid.position(self.middle[to as usize]);
        let h = |r: u32| grid.position(self.middle[r as usize]).distance_xz(goal);
        open.clear();
        mark[from as usize] = seen;
        cost[from as usize] = 0.0;
        parent[from as usize] = NONE;
        open.push(Entry(h(from), from));
        while let Some(Entry(_, u)) = open.pop() {
            if mark[u as usize] == closed {
                continue;
            }
            mark[u as usize] = closed;
            if u == to {
                let mut way = vec![to];
                let mut at = to;
                while at != from {
                    at = parent[at as usize];
                    way.push(at);
                }
                return Some(way);
            }
            let cu = cost[u as usize];
            for &(v, step) in self.out(u) {
                let m = mark[v as usize];
                if m == closed || (v != to && !region_ok(v)) {
                    continue;
                }
                let c = cu + step;
                if m != seen || c < cost[v as usize] {
                    mark[v as usize] = seen;
                    cost[v as usize] = c;
                    parent[v as usize] = u;
                    open.push(Entry(c + h(v), v));
                }
            }
        }
        None
    }
}

fn find(root: &mut [u32], mut a: u32) -> u32 {
    while root[a as usize] != a {
        let up = root[root[a as usize] as usize];
        root[a as usize] = up;
        a = up;
    }
    a
}

/// A search's per-region arrays, kept by the thread, marked by a search
/// number as the floors' are.
#[derive(Default)]
struct Marks {
    mark: Vec<u32>,
    cost: Vec<f32>,
    parent: Vec<u32>,
    search: u32,
    open: BinaryHeap<Entry>,
    /// Regions in the corridor: `== in_corridor`.
    corridor: Vec<u32>,
    in_corridor: u32,
}

impl Marks {
    fn fit(&mut self, n: usize) {
        if self.mark.len() < n {
            self.mark.resize(n, 0);
            self.cost.resize(n, f32::INFINITY);
            self.parent.resize(n, NONE);
            self.corridor.resize(n, 0);
        }
    }

    /// The marks of a new search: (seen, closed).
    fn next(&mut self) -> (u32, u32) {
        self.search += 1;
        if self.search >= u32::MAX / 2 - 1 {
            self.mark.fill(0);
            self.search = 1;
        }
        (2 * self.search, 2 * self.search + 1)
    }

    /// A new, empty corridor.
    fn new_corridor(&mut self) -> u32 {
        self.in_corridor += 1;
        if self.in_corridor == u32::MAX {
            self.corridor.fill(0);
            self.in_corridor = 1;
        }
        self.in_corridor
    }
}

thread_local! {
    static MARKS: RefCell<Marks> = RefCell::new(Marks::default());
}

/// How a [`WalkGrid::route_search`] went, for the logs and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// No way between the regions: answered without a floor searched.
    #[default]
    NoWay,
    /// Found inside the corridor.
    Corridor,
    /// Found inside the corridor and the regions next to it.
    Widened,
    /// Searched over the whole grid (the corridor shut by the filter).
    Whole,
}

impl Regions {
    /// The way from `from` to `to` on `grid` (these regions') over the
    /// floors `allowed` lets through (as [`WalkGrid::path_where`]), by the
    /// regions first (module documentation); `region_ok` shuts regions
    /// (the start's and the goal's never). With a `region_ok` that agrees
    /// with `allowed` on every floor of the region, a way is found whenever
    /// there is one. `budget` bounds only the last stage, the search of the
    /// whole grid when the corridor and its neighbours are shut.
    pub fn route_search(
        &self,
        grid: &WalkGrid,
        from: NodeId,
        to: NodeId,
        allowed: impl Fn(NodeId) -> bool,
        region_ok: impl Fn(u32) -> bool,
        budget: usize,
    ) -> (Search, Stage) {
        let n = grid.len();
        if from as usize >= n || to as usize >= n || !grid.may_reach(from, to) {
            return (Search::NoWay, Stage::NoWay);
        }
        let regions = self;
        let (rf, rt) = (regions.of(from), regions.of(to));
        let corridor = MARKS.with(|marks| {
            let mut marks = marks.borrow_mut();
            let way = regions.corridor(grid, rf, rt, &region_ok, &mut marks)?;
            Some(way)
        });
        let Some(way) = corridor else {
            return (Search::NoWay, Stage::NoWay);
        };
        let inside = |node: NodeId, mark: u32| {
            MARKS.with(|marks| marks.borrow().corridor[regions.of(node) as usize] == mark)
        };
        // The corridor and the regions next to it ([`CORRIDOR_RINGS`]), then
        // wider ([`WIDE_RINGS`]).
        let mut ring: Vec<u32> = way;
        for (rings, stage) in [
            (CORRIDOR_RINGS, Stage::Corridor),
            (WIDE_RINGS, Stage::Widened),
        ] {
            let mark = MARKS.with(|marks| {
                let mut marks = marks.borrow_mut();
                let mark = marks.new_corridor();
                let mut all = std::mem::take(&mut ring);
                for &r in &all {
                    marks.corridor[r as usize] = mark;
                }
                let mut edge = all.clone();
                for _ in 0..rings {
                    let mut next = Vec::new();
                    for &r in &edge {
                        let near = regions
                            .out(r)
                            .iter()
                            .map(|&(v, _)| v)
                            .chain(regions.into(r).iter().copied());
                        for v in near {
                            if marks.corridor[v as usize] != mark && region_ok(v) {
                                marks.corridor[v as usize] = mark;
                                next.push(v);
                            }
                        }
                    }
                    all.extend_from_slice(&next);
                    edge = next;
                }
                ring = all;
                mark
            });
            if let found @ Search::Found(_) =
                grid.search(from, to, |v| inside(v, mark) && allowed(v), usize::MAX)
            {
                return (found, stage);
            }
        }
        (grid.search(from, to, allowed, budget), Stage::Whole)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_way_through_regions_matches_the_whole_search() {
        // Two rooms joined by a door, 30 x 12 m: several blocks, a wall.
        let grid = crate::grid::tests::two_rooms();
        let regions = Regions::build(&grid, None);
        assert!(regions.len() >= 4, "{}", regions.len());
        let a = grid.locate(Vec3::new(2.0, 0.0, 2.0), 1.0).unwrap();
        let b = grid.locate(Vec3::new(28.0, 0.0, 10.0), 1.0).unwrap();
        let whole = grid.path(a, b).unwrap();
        let (found, stage) = regions.route_search(&grid, a, b, |_| true, |_| true, usize::MAX);
        let Search::Found(path) = found else {
            panic!("{found:?}");
        };
        assert_eq!(stage, Stage::Corridor);
        assert!(
            path.cost <= whole.cost * 1.05,
            "{} {}",
            path.cost,
            whole.cost
        );
        // The door's floors labelled apart and shut: no way, known from
        // the regions alone.
        let door = |n: NodeId| {
            let p = grid.position(n);
            (14.0..16.0).contains(&p.x) && (5.0..7.0).contains(&p.z)
        };
        let label: Vec<u32> = (0..grid.len() as NodeId)
            .map(|n| u32::from(door(n)))
            .collect();
        let cut = Regions::build(&grid, Some(&label));
        assert!(cut.len() > regions.len());
        let (found, stage) = cut.route_search(
            &grid,
            a,
            b,
            |n| !door(n),
            |r| !door(cut.middle(r)),
            usize::MAX,
        );
        assert_eq!((found, stage), (Search::NoWay, Stage::NoWay));
    }
}
