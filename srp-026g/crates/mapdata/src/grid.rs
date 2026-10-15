//! The walk grid: where the walking body can stand and how it gets from
//! one place to the next, built from the walker mesh.
//!
//! Recast-style, on a square grid over the play area:
//! 1. Every walker triangle is clipped to each cell it covers; the clipped
//!    piece gives a solid span (its lowest and highest y) in that column.
//! 2. Spans that touch are merged. The top of a span is a floor when its
//!    faces can be stood on (walker behaviour [`WalkerBehaviour::Walk`], or
//!    [`WalkerBehaviour::BySlope`] no steeper than 60°) and the next span
//!    above leaves at least the crouched height. One cell can hold several
//!    floors (bridges, buildings).
//! 3. A floor links to the floor of each of its 8 neighbour cells within a
//!    step (0.6 m) with headroom for a crouched body ([`LinkKind::Walk`]).
//!    Two `walk` faces (stairs) may differ by up to 3 cells' width. Where no
//!    step exists, a floor links one way to a lower floor within the
//!    harmless fall ([`LinkKind::Drop`]) or to a higher one within the jump
//!    height ([`LinkKind::Jump`]).
//! 4. Erosion by the body's radius: a floor with a wall beside it (a side
//!    with no link of any kind) goes, round after round
//!    ([`erosion_rounds`]). A floor `k` cells from a wall cell is at least
//!    `(k - 0.5) × cell` from the wall; it stays when that is the radius
//!    (within 1 cm). At 0.25 m cells that is 1 round, which keeps 0.9-1 m
//!    doors open; 2 rounds (`ceil(radius / cell)`, the prototype) shut some.
//!
//! Positions are in the map frame, at the feet ([`crate::math`]).

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::built;
use crate::digest::Digest;
use crate::library::WalkerBehaviour;
use crate::math::Vec3;
use crate::project::Triangle;
use crate::resources::Error;

/// The body and how it moves. Defaults: the 0.26g client's character
/// (`gameplay/players/default.player`; step 0xa7ab08), except the jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    /// `s_step_height`: the highest step taken by walking. PROVEN 0.6 m.
    pub step: f32,
    /// `stand_capsule_height`. PROVEN 1.8 m.
    pub stand_height: f32,
    /// The crouched capsule's height: the lowest gap the body passes.
    /// PROVEN 1.1 m.
    pub crouch_height: f32,
    /// Half of `stand_capsule_width` (0.76 m). PROVEN.
    pub radius: f32,
    /// The cosine of the steepest ground (60°, LIKELY).
    pub min_ground_normal_y: f32,
    /// The highest drop taken (`fall_params.harmless_height`, 5 m).
    pub max_drop: f32,
    /// The highest ledge reached by a jump. GUESS: about 1.7 m in the
    /// engine's numbers; 1.3 m keeps a margin.
    pub max_jump: f32,
}

impl Body {
    /// Its numbers in order, for digests and files.
    pub(crate) fn values(&self) -> [f32; 7] {
        [
            self.step,
            self.stand_height,
            self.crouch_height,
            self.radius,
            self.min_ground_normal_y,
            self.max_drop,
            self.max_jump,
        ]
    }

    fn from_values(v: [f32; 7]) -> Self {
        Self {
            step: v[0],
            stand_height: v[1],
            crouch_height: v[2],
            radius: v[3],
            min_ground_normal_y: v[4],
            max_drop: v[5],
            max_jump: v[6],
        }
    }
}

impl Default for Body {
    fn default() -> Self {
        Self {
            step: 0.6,
            stand_height: 1.8,
            crouch_height: 1.1,
            radius: 0.38,
            min_ground_normal_y: 0.5,
            max_drop: 5.0,
            max_jump: 1.3,
        }
    }
}

/// How one floor reaches the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LinkKind {
    Walk = 0,
    /// Down a ledge; one way.
    Drop = 1,
    /// Up a ledge higher than a step; one way.
    Jump = 2,
}

impl LinkKind {
    /// From its two bits in [`WalkGrid`]'s packed links.
    fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Self::Walk,
            1 => Self::Drop,
            _ => Self::Jump,
        }
    }

    /// Extra path cost, in metres, so paths prefer walking (tuning, not
    /// game data).
    pub fn penalty(self) -> f32 {
        match self {
            Self::Walk => 0.0,
            Self::Drop => 0.5,
            Self::Jump => 1.5,
        }
    }
}

/// A floor of the grid.
pub type NodeId = u32;

const NONE: u32 = u32::MAX;
/// No link in a direction ([`WalkGrid`]'s `to`).
const NO_LINK: u8 = u8::MAX;
/// The 8 neighbour directions (dx, dz); `DIRS[d ^ 1]` is the opposite.
const DIRS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
];
/// Spans this close in y are one solid.
const MERGE_GAP: f32 = 0.05;
/// Spans whose tops differ less than this are one floor.
const SAME_TOP: f32 = 0.1;

#[derive(Clone, Copy, Debug)]
struct Span {
    low: f32,
    high: f32,
    /// 0 not a floor, 1 a floor by slope, 2 a `walk` face.
    walk: u8,
}

/// See the module documentation.
pub struct WalkGrid {
    x0: f32,
    z0: f32,
    cell: f32,
    nx: usize,
    nz: usize,
    body: Body,
    /// Per cell, its first node; `nx * nz + 1` entries.
    start: Vec<u32>,
    /// Per node.
    cell_of: Vec<u32>,
    height: Vec<f32>,
    ceiling: Vec<f32>,
    /// Per node and direction ([`DIRS`]), the floor it links to: that
    /// floor's index among the floors of its cell, [`NO_LINK`] for none.
    to: Vec<[u8; 8]>,
    /// Per node, 4 bits a direction: the link's kind (bits 0-1) and how
    /// many cells along the direction it lands, less one (bits 2-3; a drop
    /// or a jump past an eroded floor lands further on).
    how: Vec<u32>,
    /// Per node, its piece of the grid: floors joined by links either way
    /// ([`Self::may_reach`]).
    piece: Vec<u32>,
}

/// The links while the grid is built, a whole floor id a direction; packed
/// into `to` and `how` at the end (28 B a floor instead of 56).
struct Wide {
    links: Vec<[u32; 8]>,
    kinds: Vec<[LinkKind; 8]>,
}

/// Counts of a build, for logs and the docs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildStats {
    pub triangles_used: usize,
    pub spans: usize,
    pub floors: usize,
    pub floors_after_erosion: usize,
}

impl WalkGrid {
    /// Builds the grid over `area` = `[x0, z0, x1, z1]` (map frame) with
    /// square cells of `cell` metres (0.25-0.5 m).
    pub fn build(triangles: &[Triangle], area: [f32; 4], cell: f32, body: Body) -> Self {
        Self::build_with_stats(triangles, area, cell, body).0
    }

    pub fn build_with_stats(
        triangles: &[Triangle],
        area: [f32; 4],
        cell: f32,
        body: Body,
    ) -> (Self, BuildStats) {
        assert!(cell > 0.0, "cell size must be positive");
        let (x0, z0) = (area[0], area[1]);
        let nx = (((area[2] - x0) / cell).max(0.0) as usize) + 1;
        let nz = (((area[3] - z0) / cell).max(0.0) as usize) + 1;
        let mut stats = BuildStats::default();
        let columns = rasterize(triangles, x0, z0, cell, nx, nz, body, &mut stats);

        // Floors per cell.
        let mut start = Vec::with_capacity(nx * nz + 1);
        let mut cell_of = Vec::new();
        let mut height = Vec::new();
        let mut ceiling = Vec::new();
        let mut forced = Vec::new();
        for (index, mut column) in columns.into_iter().enumerate() {
            start.push(height.len() as u32);
            if column.is_empty() {
                continue;
            }
            column.sort_by(|a, b| a.low.total_cmp(&b.low));
            let mut merged: Vec<Span> = Vec::with_capacity(column.len());
            for span in column {
                if let Some(last) = merged.last_mut()
                    && span.low <= last.high + MERGE_GAP
                {
                    if (span.high - last.high).abs() <= SAME_TOP {
                        last.walk = last.walk.max(span.walk);
                        last.high = last.high.max(span.high);
                    } else if span.high > last.high {
                        last.high = span.high;
                        last.walk = span.walk;
                    }
                    continue;
                }
                merged.push(span);
            }
            for (k, span) in merged.iter().enumerate() {
                if span.walk == 0 {
                    continue;
                }
                let above = merged.get(k + 1).map_or(f32::MAX, |next| next.low);
                if above - span.high >= body.crouch_height {
                    cell_of.push(index as u32);
                    height.push(span.high);
                    ceiling.push(above);
                    forced.push(span.walk == 2);
                }
            }
        }
        start.push(height.len() as u32);
        stats.floors = height.len();

        let mut grid = Self {
            x0,
            z0,
            cell,
            nx,
            nz,
            body,
            start,
            cell_of,
            height,
            ceiling,
            to: Vec::new(),
            how: Vec::new(),
            piece: Vec::new(),
        };
        let mut wide = grid.link(&forced);
        grid.erode(&mut wide);
        grid.pieces(&wide);
        grid.pack(wide);
        stats.floors_after_erosion = grid.len();
        (grid, stats)
    }

    fn link(&self, forced: &[bool]) -> Wide {
        let n = self.height.len();
        let body = self.body;
        let stair_rise = body.step.max(3.0 * self.cell);
        let mut links = vec![[NONE; 8]; n];
        let mut kinds = vec![[LinkKind::Walk; 8]; n];
        for a in 0..n {
            let (i, k) = self.cell_xz(self.cell_of[a] as usize);
            for (d, &(di, dk)) in DIRS.iter().enumerate() {
                let Some(c2) = self.cell_index(i + di, k + dk) else {
                    continue;
                };
                let (ha, ca) = (self.height[a], self.ceiling[a]);
                let mut best = (NONE, f32::MAX, LinkKind::Walk);
                // A step first.
                for b in self.start[c2]..self.start[c2 + 1] {
                    let b = b as usize;
                    let (hb, cb) = (self.height[b], self.ceiling[b]);
                    let rise = (hb - ha).abs();
                    let gap = ca.min(cb) - ha.max(hb);
                    let limit = if forced[a] && forced[b] {
                        stair_rise
                    } else {
                        body.step
                    };
                    if rise <= limit && gap >= body.crouch_height && rise < best.1 {
                        best = (b as u32, rise, LinkKind::Walk);
                    }
                }
                if best.0 == NONE {
                    // Else the nearest drop or jump.
                    if let Some((b, kind)) = self.ledge(a, c2, None) {
                        best = (b, 0.0, kind);
                    }
                }
                links[a][d] = best.0;
                kinds[a][d] = best.2;
            }
        }
        Wide { links, kinds }
    }

    /// The floor in `cell` nearest in height that `a` drops or jumps to,
    /// among the `alive` ones.
    fn ledge(&self, a: usize, cell: usize, alive: Option<&[bool]>) -> Option<(u32, LinkKind)> {
        let body = &self.body;
        let (ha, ca) = (self.height[a], self.ceiling[a]);
        let mut best: Option<(u32, f32, LinkKind)> = None;
        for b in self.start[cell]..self.start[cell + 1] {
            if alive.is_some_and(|alive| !alive[b as usize]) {
                continue;
            }
            let (hb, cb) = (self.height[b as usize], self.ceiling[b as usize]);
            let rise = hb - ha;
            let kind = if rise < 0.0 && -rise <= body.max_drop && cb - ha >= body.crouch_height {
                LinkKind::Drop
            } else if rise > 0.0
                && rise <= body.max_jump
                && ca - hb >= body.crouch_height
                && cb - hb >= body.crouch_height
            {
                LinkKind::Jump
            } else {
                continue;
            };
            if best.is_none_or(|(_, r, _)| rise.abs() < r) {
                best = Some((b, rise.abs(), kind));
            }
        }
        best.map(|(b, _, kind)| (b, kind))
    }

    /// Removes floors within the body's radius of a wall, then compacts.
    ///
    /// A drop or jump whose landing went lands further on, up to the
    /// rounds of erosion past it: a body that drops beside a wall lands
    /// clear of it.
    fn erode(&mut self, wide: &mut Wide) {
        let n = self.height.len();
        let mut alive = vec![true; n];
        let rounds = erosion_rounds(self.body.radius, self.cell);
        for _ in 0..rounds {
            let doomed: Vec<usize> = (0..n)
                .filter(|&a| {
                    alive[a]
                        && (0..4).any(|d| {
                            let b = wide.links[a][d];
                            b == NONE || (wide.kinds[a][d] == LinkKind::Walk && !alive[b as usize])
                        })
                })
                .collect();
            for a in doomed {
                alive[a] = false;
            }
        }
        for a in 0..n {
            if !alive[a] {
                continue;
            }
            let (i, k) = self.cell_xz(self.cell_of[a] as usize);
            for (d, &(di, dk)) in DIRS.iter().enumerate() {
                let b = wide.links[a][d];
                if b == NONE || alive[b as usize] || wide.kinds[a][d] == LinkKind::Walk {
                    continue;
                }
                wide.links[a][d] = NONE;
                for reach in 2..=rounds as i32 + 1 {
                    let Some(cell) = self.cell_index(i + di * reach, k + dk * reach) else {
                        break;
                    };
                    if let Some((b, kind)) = self.ledge(a, cell, Some(&alive)) {
                        wide.links[a][d] = b;
                        wide.kinds[a][d] = kind;
                        break;
                    }
                }
            }
        }
        // Compact: new ids in the same (cell) order.
        let mut new_id = vec![NONE; n];
        let mut next = 0u32;
        for a in 0..n {
            if alive[a] {
                new_id[a] = next;
                next += 1;
            }
        }
        let keep = |v: &mut Vec<f32>| {
            let mut k = 0;
            v.retain(|_| {
                k += 1;
                alive[k - 1]
            });
        };
        keep(&mut self.height);
        keep(&mut self.ceiling);
        let mut k = 0;
        self.cell_of.retain(|_| {
            k += 1;
            alive[k - 1]
        });
        let mut links = Vec::with_capacity(next as usize);
        let mut kinds = Vec::with_capacity(next as usize);
        for a in 0..n {
            if !alive[a] {
                continue;
            }
            let mut l = wide.links[a];
            for to in l.iter_mut() {
                if *to != NONE {
                    *to = new_id[*to as usize];
                }
            }
            links.push(l);
            kinds.push(wide.kinds[a]);
        }
        wide.links = links;
        wide.kinds = kinds;
        let mut start = Vec::with_capacity(self.start.len());
        let mut node = 0usize;
        for c in 0..self.nx * self.nz {
            start.push(node as u32);
            while node < self.cell_of.len() && self.cell_of[node] as usize == c {
                node += 1;
            }
        }
        start.push(node as u32);
        self.start = start;
    }

    pub fn len(&self) -> usize {
        self.height.len()
    }

    pub fn is_empty(&self) -> bool {
        self.height.is_empty()
    }

    pub fn cell_size(&self) -> f32 {
        self.cell
    }

    pub fn body(&self) -> &Body {
        &self.body
    }

    /// Cells along x and z.
    pub fn dimensions(&self) -> (usize, usize) {
        (self.nx, self.nz)
    }

    /// Bytes the grid holds.
    pub fn memory_bytes(&self) -> usize {
        self.start.len() * 4 + self.len() * (4 + 4 + 4 + 8 + 4 + 4)
    }

    /// What the grid answers, as one number ([`crate::digest`]): its frame
    /// and body, the cells' floors, and each floor's height, ceiling,
    /// piece and links as (target, kind). Equal digests, equal answers.
    pub fn digest(&self) -> u64 {
        let mut digest = Digest::new();
        for value in [self.x0, self.z0, self.cell] {
            digest.f32(value);
        }
        digest.len(self.nx);
        digest.len(self.nz);
        for value in self.body.values() {
            digest.f32(value);
        }
        digest.len(self.start.len());
        self.start.iter().for_each(|&first| digest.u32(first));
        digest.len(self.len());
        for node in 0..self.len() {
            digest.u32(self.cell_of[node]);
            digest.f32(self.height[node]);
            digest.f32(self.ceiling[node]);
            digest.u32(self.piece[node]);
            let (i, k) = self.cell_xz(self.cell_of[node] as usize);
            for d in 0..8 {
                match self.link_of(node, d, i, k) {
                    Some((to, kind)) => {
                        digest.u32(to);
                        digest.u32(kind as u32);
                    }
                    None => digest.u32(NONE),
                }
            }
        }
        digest.finish()
    }

    /// Writes the grid to `path` as the grid of `key` ([`crate::built`],
    /// [`crate::grid_key`]).
    pub fn write(&self, path: &std::path::Path, key: u64) -> Result<(), Error> {
        built::write(path, built::GRID_MAGIC, key, self.digest(), |out| {
            for value in [self.x0, self.z0, self.cell] {
                out.f32(value)?;
            }
            out.u64(self.nx as u64)?;
            out.u64(self.nz as u64)?;
            for value in self.body.values() {
                out.f32(value)?;
            }
            out.plain(&self.start, |v| v.to_le_bytes())?;
            out.plain(&self.cell_of, |v| v.to_le_bytes())?;
            out.plain(&self.height, |v| v.to_bits().to_le_bytes())?;
            out.plain(&self.ceiling, |v| v.to_bits().to_le_bytes())?;
            out.plain(&self.to, |v| *v)?;
            out.plain(&self.how, |v| v.to_le_bytes())?;
            out.plain(&self.piece, |v| v.to_le_bytes())
        })
    }

    /// Reads the grid of `key` that [`Self::write`] wrote: checked index by
    /// index, then by its digest, so it answers as the grid written did.
    pub fn read(path: &std::path::Path, key: u64) -> Result<Self, Error> {
        let float = |bytes: [u8; 4]| f32::from_bits(u32::from_le_bytes(bytes));
        built::read(
            path,
            built::GRID_MAGIC,
            key,
            |input| {
                let (x0, z0, cell) = (input.f32()?, input.f32()?, input.f32()?);
                let (nx, nz) = (input.u64()?, input.u64()?);
                let mut body = [0.0; 7];
                for value in &mut body {
                    *value = input.f32()?;
                }
                let grid = Self {
                    x0,
                    z0,
                    cell,
                    nx: usize::try_from(nx).map_err(|_| "too many cells")?,
                    nz: usize::try_from(nz).map_err(|_| "too many cells")?,
                    body: Body::from_values(body),
                    start: input.plain(u32::from_le_bytes)?,
                    cell_of: input.plain(u32::from_le_bytes)?,
                    height: input.plain(float)?,
                    ceiling: input.plain(float)?,
                    to: input.plain(|bytes: [u8; 8]| bytes)?,
                    how: input.plain(u32::from_le_bytes)?,
                    piece: input.plain(u32::from_le_bytes)?,
                };
                grid.check()?;
                Ok(grid)
            },
            Self::digest,
        )
    }

    /// Whether every index of a grid read from a file is in range, so no
    /// query can panic on it.
    fn check(&self) -> Result<(), String> {
        let cells = self
            .nx
            .checked_mul(self.nz)
            .filter(|&cells| cells > 0 && cells < i32::MAX as usize && self.nx < i32::MAX as usize)
            .ok_or("bad dimensions")?;
        let n = self.height.len();
        if self.start.len() != cells + 1
            || self.start[0] != 0
            || self.start[cells] as usize != n
            || self.start.windows(2).any(|w| w[0] > w[1])
        {
            return Err("bad cell starts".into());
        }
        let lengths = [
            self.cell_of.len(),
            self.ceiling.len(),
            self.to.len(),
            self.how.len(),
            self.piece.len(),
        ];
        if lengths.iter().any(|&len| len != n) {
            return Err("arrays of different lengths".into());
        }
        for a in 0..n {
            let c = self.cell_of[a] as usize;
            if c >= cells || a < self.start[c] as usize || a >= self.start[c + 1] as usize {
                return Err(format!("floor {a} is not in its cell"));
            }
            if self.piece[a] as usize >= n {
                return Err(format!("floor {a}'s piece is out of range"));
            }
            let (i, k) = self.cell_xz(c);
            for (d, &(di, dk)) in DIRS.iter().enumerate() {
                let index = self.to[a][d];
                let bits = self.how[a] >> (4 * d);
                if index == NO_LINK {
                    // As packed: nothing for no link.
                    if bits & 0xF != 0 {
                        return Err(format!("floor {a}'s direction {d} has no link but bits"));
                    }
                    continue;
                }
                let reach = ((bits >> 2) & 3) as i32 + 1;
                let target = self.cell_index(i + di * reach, k + dk * reach);
                let fits =
                    target.is_some_and(|t| u32::from(index) < self.start[t + 1] - self.start[t]);
                if bits & 3 == 3 || !fits {
                    return Err(format!("floor {a}'s link {d} is out of range"));
                }
            }
        }
        Ok(())
    }

    /// The block of `block` cells a side that `node` lies in.
    pub(crate) fn block_of(&self, node: NodeId, block: usize) -> (usize, usize) {
        let (i, k) = self.cell_xz(self.cell_of[node as usize] as usize);
        (i as usize / block, k as usize / block)
    }

    /// Labels each floor with its piece: the floors it is joined to by
    /// links taken either way (union-find over the links).
    fn pieces(&mut self, wide: &Wide) {
        let n = self.len();
        let mut root: Vec<u32> = (0..n as u32).collect();
        fn find(root: &mut [u32], mut a: u32) -> u32 {
            while root[a as usize] != a {
                let up = root[root[a as usize] as usize];
                root[a as usize] = up;
                a = up;
            }
            a
        }
        for (a, links) in wide.links.iter().enumerate() {
            for &b in links {
                if b != NONE {
                    let (ra, rb) = (find(&mut root, a as u32), find(&mut root, b));
                    if ra != rb {
                        root[ra.max(rb) as usize] = ra.min(rb);
                    }
                }
            }
        }
        self.piece = (0..n as u32).map(|a| find(&mut root, a)).collect();
    }

    /// Packs the links: each as its floor's index in the cell it lands in,
    /// its kind and its reach ([`WalkGrid`]'s `to` and `how`).
    fn pack(&mut self, wide: Wide) {
        let n = self.len();
        let mut to = Vec::with_capacity(n);
        let mut how = Vec::with_capacity(n);
        for a in 0..n {
            let (i, k) = self.cell_xz(self.cell_of[a] as usize);
            let (mut floors, mut bits) = ([NO_LINK; 8], 0u32);
            for (d, &(di, dk)) in DIRS.iter().enumerate() {
                let b = wide.links[a][d];
                if b == NONE {
                    continue;
                }
                let cell = self.cell_of[b as usize] as usize;
                let index = b - self.start[cell];
                let (ib, kb) = self.cell_xz(cell);
                let reach = (ib - i).abs().max((kb - k).abs());
                assert!(
                    index < u32::from(NO_LINK)
                        && (1..=4).contains(&reach)
                        && (ib - i, kb - k) == (di * reach, dk * reach),
                    "a link the packed grid cannot hold: floor {a} to {b}"
                );
                floors[d] = index as u8;
                bits |= (wide.kinds[a][d] as u32 | ((reach as u32 - 1) << 2)) << (4 * d);
            }
            to.push(floors);
            how.push(bits);
        }
        self.to = to;
        self.how = how;
    }

    /// The link out of floor `a` (in cell `i`, `k`) in direction `d`.
    #[inline]
    fn link_of(&self, a: usize, d: usize, i: i32, k: i32) -> Option<(NodeId, LinkKind)> {
        let index = self.to[a][d];
        if index == NO_LINK {
            return None;
        }
        let bits = self.how[a] >> (4 * d);
        let reach = ((bits >> 2) & 3) as i32 + 1;
        let (di, dk) = DIRS[d];
        let cell = (k + dk * reach) as usize * self.nx + (i + di * reach) as usize;
        Some((
            self.start[cell] + u32::from(index),
            LinkKind::from_bits(bits),
        ))
    }

    /// Whether a way from `from` to `to` may exist: false when they lie in
    /// pieces no link joins (one floor on a roof, the other on the ground
    /// with no way up), so no search need prove it. True does not promise a
    /// way (a drop is one way).
    pub fn may_reach(&self, from: NodeId, to: NodeId) -> bool {
        match (self.piece.get(from as usize), self.piece.get(to as usize)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// The floor's point: the cell's centre at the floor's height.
    pub fn position(&self, node: NodeId) -> Vec3 {
        let (i, k) = self.cell_xz(self.cell_of[node as usize] as usize);
        Vec3::new(
            self.x0 + (i as f32 + 0.5) * self.cell,
            self.height[node as usize],
            self.z0 + (k as f32 + 0.5) * self.cell,
        )
    }

    /// The free height above the floor (`f32::MAX` under the open sky).
    pub fn headroom(&self, node: NodeId) -> f32 {
        self.ceiling[node as usize] - self.height[node as usize]
    }

    /// Whether a standing body fits (else only a crouched one).
    pub fn can_stand(&self, node: NodeId) -> bool {
        self.headroom(node) >= self.body.stand_height
    }

    /// The links out of a floor.
    pub fn links(&self, node: NodeId) -> impl Iterator<Item = (NodeId, LinkKind)> + '_ {
        let a = node as usize;
        let (i, k) = self.cell_xz(self.cell_of[a] as usize);
        (0..8).filter_map(move |d| self.link_of(a, d, i, k))
    }

    /// The links into a floor.
    pub fn incoming(&self, node: NodeId) -> impl Iterator<Item = (NodeId, LinkKind)> + '_ {
        let cell = self.cell_of[node as usize] as usize;
        let (i, k) = self.cell_xz(cell);
        let index = node - self.start[cell];
        DIRS.iter().enumerate().flat_map(move |(d, &(di, dk))| {
            // A floor in the cell behind direction d linking forward one
            // cell.
            let from = self.cell_index(i - di, k - dk);
            let range = from.map_or(0..0, |c| self.start[c]..self.start[c + 1]);
            range.filter_map(move |b| {
                let bits = self.how[b as usize] >> (4 * d);
                (u32::from(self.to[b as usize][d]) == index && bits & 0b1100 == 0)
                    .then(|| (b, LinkKind::from_bits(bits)))
            })
        })
    }

    /// The cost of a link: its length plus the kind's penalty.
    pub fn link_cost(&self, from: NodeId, to: NodeId, kind: LinkKind) -> f32 {
        (self.position(to) - self.position(from)).length() + kind.penalty()
    }

    /// Every floor whose cell overlaps the ground box `[x0, z0, x1, z1]`.
    pub fn floors_in(&self, area: [f32; 4]) -> impl Iterator<Item = NodeId> + '_ {
        let index = |v: f32, origin: f32, n: usize| {
            ((v - origin) / self.cell).floor().clamp(-1.0, n as f32) as i64
        };
        let (i0, i1) = (
            index(area[0], self.x0, self.nx).max(0),
            index(area[2], self.x0, self.nx).min(self.nx as i64 - 1),
        );
        let (k0, k1) = (
            index(area[1], self.z0, self.nz).max(0),
            index(area[3], self.z0, self.nz).min(self.nz as i64 - 1),
        );
        (k0..=k1).flat_map(move |k| {
            let row = k as usize * self.nx;
            let (a, b) = if i0 <= i1 {
                (
                    self.start[row + i0 as usize],
                    self.start[row + i1 as usize + 1],
                )
            } else {
                (0, 0)
            };
            a..b
        })
    }

    /// The nearest floor to a point at the feet: within `max_distance` on
    /// the ground plane, from 1 m above the point to 2.5 m below it.
    pub fn locate(&self, point: Vec3, max_distance: f32) -> Option<NodeId> {
        if !point.is_finite() {
            return None;
        }
        let i = ((point.x - self.x0) / self.cell).floor() as i32;
        let k = ((point.z - self.z0) / self.cell).floor() as i32;
        let reach = (max_distance / self.cell).ceil() as i32;
        let mut best: Option<(NodeId, f32)> = None;
        for ring in 0..=reach {
            for dk in -ring..=ring {
                for di in -ring..=ring {
                    if di.abs() != ring && dk.abs() != ring {
                        continue;
                    }
                    let Some(c) = self.cell_index(i + di, k + dk) else {
                        continue;
                    };
                    for node in self.start[c]..self.start[c + 1] {
                        let h = self.height[node as usize];
                        if h > point.y + 1.0 || point.y - h > 2.5 {
                            continue;
                        }
                        let p = self.position(node);
                        if p.distance_xz(point) > max_distance {
                            continue;
                        }
                        let d = (p - point).length();
                        if best.is_none_or(|(_, bd)| d < bd) {
                            best = Some((node, d));
                        }
                    }
                }
            }
            // A farther ring cannot beat a hit within this one's inner
            // radius.
            if let Some((_, d)) = best
                && d <= ring as f32 * self.cell
            {
                break;
            }
        }
        best.map(|(node, _)| node)
    }

    /// The cheapest path by A* (heuristic: the ground distance), `from` and
    /// `to` included.
    pub fn path(&self, from: NodeId, to: NodeId) -> Option<Path> {
        self.path_where(from, to, |_| true)
    }

    /// [`Self::path`] over the floors `allowed` lets through only (`from`
    /// and `to` are always allowed): a bot's way round an anomaly.
    pub fn path_where(
        &self,
        from: NodeId,
        to: NodeId,
        allowed: impl Fn(NodeId) -> bool,
    ) -> Option<Path> {
        match self.search(from, to, allowed, usize::MAX) {
            Search::Found(path) => Some(path),
            Search::NoWay | Search::TooFar => None,
        }
    }

    /// [`Self::path_where`], giving up after `budget` floors expanded
    /// ([`Search::TooFar`]): a search that cannot end (a goal the allowed
    /// floors shut off) otherwise floods the whole piece, a million floors
    /// on the big maps. Floors in different pieces ([`Self::may_reach`])
    /// are [`Search::NoWay`] at once. The search's arrays are kept by the
    /// thread between searches and marked by a search number, so a search
    /// costs what it expands, not the grid's size.
    pub fn search(
        &self,
        from: NodeId,
        to: NodeId,
        allowed: impl Fn(NodeId) -> bool,
        budget: usize,
    ) -> Search {
        let n = self.len();
        if from as usize >= n || to as usize >= n || !self.may_reach(from, to) {
            return Search::NoWay;
        }
        SCRATCH.with(|scratch| {
            let mut scratch = scratch.borrow_mut();
            let Scratch {
                mark,
                cost,
                parent,
                search,
                open,
            } = &mut *scratch;
            if mark.len() < n {
                mark.resize(n, 0);
                cost.resize(n, f32::INFINITY);
                parent.resize(n, NONE);
            }
            // A mark of `2 * search` is a floor reached in this search,
            // `2 * search + 1` one expanded (closed).
            *search += 1;
            if *search >= u32::MAX / 2 - 1 {
                mark.fill(0);
                *search = 1;
            }
            let (seen, closed) = (2 * *search, 2 * *search + 1);
            open.clear();
            let goal = self.position(to);
            mark[from as usize] = seen;
            cost[from as usize] = 0.0;
            parent[from as usize] = NONE;
            open.push(Entry(self.position(from).distance_xz(goal), from));
            let mut expanded = 0usize;
            while let Some(Entry(_, u)) = open.pop() {
                // A floor met again by a cheaper way is in the heap twice:
                // the later entry is stale.
                if mark[u as usize] == closed {
                    continue;
                }
                mark[u as usize] = closed;
                if u == to {
                    let mut nodes = vec![to];
                    let mut at = to;
                    while at != from {
                        at = parent[at as usize];
                        nodes.push(at);
                    }
                    nodes.reverse();
                    return Search::Found(Path {
                        nodes,
                        cost: cost[to as usize],
                    });
                }
                expanded += 1;
                if expanded > budget {
                    return Search::TooFar;
                }
                let cu = cost[u as usize];
                for (v, kind) in self.links(u) {
                    let m = mark[v as usize];
                    if m == closed || (v != to && !allowed(v)) {
                        continue;
                    }
                    let c = cu + self.link_cost(u, v, kind);
                    if m != seen || c < cost[v as usize] {
                        mark[v as usize] = seen;
                        cost[v as usize] = c;
                        parent[v as usize] = u;
                        open.push(Entry(c + self.position(v).distance_xz(goal), v));
                    }
                }
            }
            Search::NoWay
        })
    }

    /// The cost from every floor to the nearest of `targets` (a flood
    /// backwards along the links): a bot at any floor follows
    /// [`DistanceMap::next_step`] downhill. One flood serves every bot that
    /// heads for the same place.
    pub fn distances_to(&self, targets: &[NodeId]) -> DistanceMap {
        self.flood(targets, true)
    }

    /// The cost from the nearest of `sources` to every floor (forwards):
    /// where a team can get, and how far.
    pub fn distances_from(&self, sources: &[NodeId]) -> DistanceMap {
        self.flood(sources, false)
    }

    fn flood(&self, seeds: &[NodeId], backwards: bool) -> DistanceMap {
        let n = self.len();
        let mut cost = vec![f32::INFINITY; n];
        let mut open = BinaryHeap::new();
        for &s in seeds {
            if (s as usize) < n {
                cost[s as usize] = 0.0;
                open.push(Entry(0.0, s));
            }
        }
        let mut next: Vec<(NodeId, LinkKind)> = Vec::with_capacity(16);
        while let Some(Entry(c, u)) = open.pop() {
            if c > cost[u as usize] {
                continue;
            }
            next.clear();
            if backwards {
                next.extend(self.incoming(u));
            } else {
                next.extend(self.links(u));
            }
            for &(v, kind) in &next {
                let step = if backwards {
                    self.link_cost(v, u, kind)
                } else {
                    self.link_cost(u, v, kind)
                };
                let cv = c + step;
                if cv < cost[v as usize] {
                    cost[v as usize] = cv;
                    open.push(Entry(cv, v));
                }
            }
        }
        DistanceMap { cost }
    }

    fn cell_xz(&self, cell: usize) -> (i32, i32) {
        ((cell % self.nx) as i32, (cell / self.nx) as i32)
    }

    fn cell_index(&self, i: i32, k: i32) -> Option<usize> {
        (i >= 0 && k >= 0 && (i as usize) < self.nx && (k as usize) < self.nz)
            .then(|| k as usize * self.nx + i as usize)
    }
}

/// Rounds of erosion for a body of `radius` on cells of `cell` metres: the
/// `k` with `(k - 0.5) × cell < radius - 1 cm` (the nearest a floor `k`
/// cells from a wall cell can be to the wall).
pub fn erosion_rounds(radius: f32, cell: f32) -> usize {
    (((radius - 0.01) / cell + 0.5).ceil() - 1.0).max(0.0) as usize
}

/// Steps 1 and 2 of the build: each column's spans.
#[allow(clippy::too_many_arguments)]
fn rasterize(
    triangles: &[Triangle],
    x0: f32,
    z0: f32,
    cell: f32,
    nx: usize,
    nz: usize,
    body: Body,
    stats: &mut BuildStats,
) -> Vec<Vec<Span>> {
    let (x1, z1) = (x0 + nx as f32 * cell, z0 + nz as f32 * cell);
    let mut columns: Vec<Vec<Span>> = vec![Vec::new(); nx * nz];
    let mut row = Vec::with_capacity(8);
    let mut piece = Vec::with_capacity(8);
    let mut scratch = Vec::with_capacity(8);
    for t in triangles {
        let [a, b, c] = t.v;
        let min = a.min(b).min(c);
        let max = a.max(b).max(c);
        if !(min.is_finite() && max.is_finite())
            || max.x < x0
            || min.x > x1
            || max.z < z0
            || min.z > z1
        {
            continue;
        }
        let normal = (b - a).cross(c - a);
        let length = normal.length();
        if length < 1e-9 {
            continue;
        }
        stats.triangles_used += 1;
        let up = normal.y.abs() / length;
        let walk = match t.walker {
            WalkerBehaviour::Walk => 2,
            WalkerBehaviour::BySlope if up >= body.min_ground_normal_y => 1,
            _ => 0,
        };
        // Cells are half-open: a mesh ending on a cell line stops before it,
        // a wall standing on one belongs to one column.
        let first = |low: f32, origin: f32| ((low - origin) / cell).floor();
        let last = |low: f32, high: f32, origin: f32| {
            (((high - origin) / cell).ceil() - 1.0).max(first(low, origin))
        };
        let i0 = first(min.x, x0).max(0.0) as usize;
        let i1 = (last(min.x, max.x, x0) as isize).min(nx as isize - 1);
        let k0 = first(min.z, z0).max(0.0) as usize;
        let k1 = (last(min.z, max.z, z0) as isize).min(nz as isize - 1);
        if i1 < 0 || k1 < 0 {
            continue;
        }
        let corners = [a, b, c];
        for k in k0..=k1 as usize {
            let cz = z0 + k as f32 * cell;
            clip(&corners, 2, cz, true, &mut scratch);
            clip(&scratch, 2, cz + cell, false, &mut row);
            if row.len() < 3 {
                continue;
            }
            for i in i0..=i1 as usize {
                let cx = x0 + i as f32 * cell;
                clip(&row, 0, cx, true, &mut scratch);
                clip(&scratch, 0, cx + cell, false, &mut piece);
                if piece.len() < 3 {
                    continue;
                }
                let (mut low, mut high) = (f32::MAX, f32::MIN);
                for p in &piece {
                    low = low.min(p.y);
                    high = high.max(p.y);
                }
                columns[k * nx + i].push(Span { low, high, walk });
                stats.spans += 1;
            }
        }
    }
    columns
}

/// Sutherland-Hodgman against one axis-aligned plane: keeps the side
/// `>= value` (`keep_above`) or `<= value`.
fn clip(polygon: &[Vec3], axis: usize, value: f32, keep_above: bool, out: &mut Vec<Vec3>) {
    out.clear();
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let side = |p: Vec3| {
            if keep_above {
                p.axis(axis) - value
            } else {
                value - p.axis(axis)
            }
        };
        let (da, db) = (side(a), side(b));
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
}

/// How a search ([`WalkGrid::search`]) ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Search {
    Found(Path),
    /// There is no way over the allowed floors.
    NoWay,
    /// The budget ran out first.
    TooFar,
}

/// A search's arrays, kept by each thread between searches (a few MB on the
/// big maps, once).
#[derive(Default)]
struct Scratch {
    mark: Vec<u32>,
    cost: Vec<f32>,
    parent: Vec<u32>,
    search: u32,
    open: BinaryHeap<Entry>,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

/// A path of floors and its cost (metres plus link penalties).
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub nodes: Vec<NodeId>,
    pub cost: f32,
}

/// Costs per floor from a flood ([`WalkGrid::distances_to`],
/// [`WalkGrid::distances_from`]); `f32::INFINITY` where it never got.
#[derive(Clone, Debug, PartialEq)]
pub struct DistanceMap {
    pub cost: Vec<f32>,
}

impl DistanceMap {
    pub fn get(&self, node: NodeId) -> f32 {
        self.cost
            .get(node as usize)
            .copied()
            .unwrap_or(f32::INFINITY)
    }

    pub fn reaches(&self, node: NodeId) -> bool {
        self.get(node).is_finite()
    }

    /// For a map made by [`WalkGrid::distances_to`]: the link out of `node`
    /// that leads downhill fastest, `None` at a target or where no link
    /// leads to it.
    pub fn next_step(&self, grid: &WalkGrid, node: NodeId) -> Option<(NodeId, LinkKind)> {
        let here = self.get(node);
        if !here.is_finite() || here == 0.0 {
            return None;
        }
        grid.links(node)
            .map(|(to, kind)| (to, kind, grid.link_cost(node, to, kind) + self.get(to)))
            .filter(|&(_, _, total)| total.is_finite())
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(to, kind, _)| (to, kind))
    }

    /// Floors reached.
    pub fn reached(&self) -> usize {
        self.cost.iter().filter(|c| c.is_finite()).count()
    }
}

/// A heap entry, smallest cost first.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Entry(pub(crate) f32, pub(crate) NodeId);

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Two 15 x 12 m rooms side by side, the wall between them at x = 15
    /// with a 2 m door at z 5-7.
    pub(crate) fn two_rooms() -> WalkGrid {
        let mut tris = Vec::new();
        tris.extend(floor(0.0, 0.0, 30.0, 12.0, 0.0));
        tris.extend(wall_x(15.0, 0.0, 5.0, 3.0));
        tris.extend(wall_x(15.0, 7.0, 12.0, 3.0));
        WalkGrid::build(&tris, [0.0, 0.0, 30.0, 12.0], 0.25, Body::default())
    }

    /// The build gives the same grid every time, and the digest is pinned:
    /// a change to the build or to the digest fails here until the pin
    /// (and any stored grid it would make stale) is updated on purpose.
    #[test]
    fn the_build_and_its_digest_are_fixed() {
        let grid = two_rooms();
        assert_eq!(grid.digest(), two_rooms().digest());
        assert_eq!(grid.digest(), PINNED_TWO_ROOMS);
        // Any answer changes it.
        let mut moved = two_rooms();
        moved.height[0] += 0.001;
        assert_ne!(moved.digest(), grid.digest());
        let mut relinked = two_rooms();
        let d = (0..8).find(|&d| relinked.to[0][d] != NO_LINK).unwrap();
        relinked.how[0] = (relinked.how[0] & !(3 << (4 * d))) | (2 << (4 * d));
        assert_ne!(relinked.digest(), grid.digest());
        // A link that goes nowhere has no kind to tell.
        let mut unlinked = two_rooms();
        let d = (0..8).find(|&d| unlinked.to[0][d] == NO_LINK).unwrap();
        unlinked.how[0] |= 1 << (4 * d);
        assert_eq!(unlinked.digest(), grid.digest());
    }

    const PINNED_TWO_ROOMS: u64 = 11_945_545_835_143_266_436;

    #[test]
    fn a_written_grid_reads_back_the_same_and_a_damaged_one_is_refused() {
        let dir = std::env::temp_dir().join(format!("srp-grid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(crate::built::grid_file_name(7));
        let grid = two_rooms();
        grid.write(&path, 7).unwrap();
        let read = WalkGrid::read(&path, 7).unwrap();
        assert_eq!(read.digest(), grid.digest());
        assert_eq!(read.memory_bytes(), grid.memory_bytes());
        // Another key's file is not this one.
        assert!(WalkGrid::read(&path, 8).is_err());
        // Every byte matters: a flipped one is refused, never a panic.
        let bytes = std::fs::read(&path).unwrap();
        for at in (0..bytes.len()).step_by(97).chain([bytes.len() - 1]) {
            let mut damaged = bytes.clone();
            damaged[at] ^= 0x41;
            std::fs::write(&path, &damaged).unwrap();
            assert!(WalkGrid::read(&path, 7).is_err(), "byte {at}");
        }
        // Cut short.
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(WalkGrid::read(&path, 7).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn quad(min: Vec3, max_x: f32, max_z: f32, walker: WalkerBehaviour) -> [Triangle; 2] {
        let y = min.y;
        let a = Vec3::new(min.x, y, min.z);
        let b = Vec3::new(max_x, y, min.z);
        let c = Vec3::new(max_x, y, max_z);
        let d = Vec3::new(min.x, y, max_z);
        [
            Triangle {
                v: [a, b, c],
                walker,
            },
            Triangle {
                v: [a, c, d],
                walker,
            },
        ]
    }

    fn floor(x0: f32, z0: f32, x1: f32, z1: f32, y: f32) -> [Triangle; 2] {
        quad(Vec3::new(x0, y, z0), x1, z1, WalkerBehaviour::BySlope)
    }

    /// A vertical wall across x at `x` from z0 to z1, height h.
    fn wall_x(x: f32, z0: f32, z1: f32, h: f32) -> [Triangle; 2] {
        let a = Vec3::new(x, 0.0, z0);
        let b = Vec3::new(x, 0.0, z1);
        let c = Vec3::new(x, h, z1);
        let d = Vec3::new(x, h, z0);
        [
            Triangle {
                v: [a, b, c],
                walker: WalkerBehaviour::BySlope,
            },
            Triangle {
                v: [a, c, d],
                walker: WalkerBehaviour::BySlope,
            },
        ]
    }

    const AREA: [f32; 4] = [0.0, 0.0, 10.0, 10.0];

    #[test]
    fn erosion_keeps_floors_a_radius_from_walls() {
        assert_eq!(erosion_rounds(0.38, 0.5), 1);
        assert_eq!(erosion_rounds(0.38, 0.25), 1);
        assert_eq!(erosion_rounds(0.38, 0.2), 2);
        assert_eq!(erosion_rounds(0.38, 0.1), 4);
        assert_eq!(erosion_rounds(0.0, 0.5), 0);
    }

    #[test]
    fn a_path_keeps_off_the_floors_it_is_not_allowed() {
        let tris = floor(0.0, 0.0, 10.0, 10.0, 0.0);
        let grid = WalkGrid::build(&tris, AREA, 0.25, Body::default());
        let a = grid.locate(Vec3::new(1.0, 0.0, 5.0), 1.0).unwrap();
        let b = grid.locate(Vec3::new(9.0, 0.0, 5.0), 1.0).unwrap();
        // A round 1.5 m anomaly in the middle.
        let centre = Vec3::new(5.0, 0.0, 5.0);
        let clear = |n: NodeId| grid.position(n).distance_xz(centre) > 1.5;
        let path = grid.path_where(a, b, clear).unwrap();
        assert!(path.nodes.iter().all(|&n| clear(n)));
        assert!(path.cost > grid.path(a, b).unwrap().cost);
        // Walled in: no way.
        assert!(
            grid.path_where(a, b, |n| grid.position(n).x < 3.0)
                .is_none()
        );
    }

    #[test]
    fn a_one_metre_door_is_open_at_quarter_metre_cells() {
        // A wall across x = 5 with a 1 m door from z = 4.5 to 5.5.
        let mut tris = floor(0.0, 0.0, 10.0, 10.0, 0.0).to_vec();
        tris.extend(wall_x(5.0, 0.0, 4.5, 3.0));
        tris.extend(wall_x(5.0, 5.5, 10.0, 3.0));
        let grid = WalkGrid::build(&tris, AREA, 0.25, Body::default());
        let a = grid.locate(Vec3::new(2.0, 0.0, 5.0), 1.0).unwrap();
        let b = grid.locate(Vec3::new(8.0, 0.0, 5.0), 1.0).unwrap();
        assert!(grid.path(a, b).is_some());
        // A 0.5 m gap is shut.
        let mut tris = floor(0.0, 0.0, 10.0, 10.0, 0.0).to_vec();
        tris.extend(wall_x(5.0, 0.0, 4.75, 3.0));
        tris.extend(wall_x(5.0, 5.25, 10.0, 3.0));
        let grid = WalkGrid::build(&tris, AREA, 0.25, Body::default());
        let a = grid.locate(Vec3::new(2.0, 0.0, 5.0), 1.0).unwrap();
        let b = grid.locate(Vec3::new(8.0, 0.0, 5.0), 1.0).unwrap();
        assert!(grid.path(a, b).is_none());
    }

    #[test]
    fn a_flat_floor_is_walkable_and_eroded_at_its_edge() {
        let tris = floor(0.0, 0.0, 10.0, 10.0, 0.0);
        let (grid, stats) = WalkGrid::build_with_stats(&tris, AREA, 0.5, Body::default());
        // 20 x 20 cells with floor (the 21st row/column is past the mesh);
        // one round of erosion takes the outer ring.
        assert_eq!(stats.floors, 400);
        assert_eq!(grid.len(), 18 * 18);
        let a = grid.locate(Vec3::new(1.2, 0.0, 1.2), 1.0).unwrap();
        let b = grid.locate(Vec3::new(8.8, 0.0, 8.7), 1.0).unwrap();
        let path = grid.path(a, b).unwrap();
        // The diagonal, 7.5 m by 7.5 m in cells.
        assert!(
            (path.cost - 7.5 * 2f32.sqrt()).abs() < 0.01,
            "{}",
            path.cost
        );
        assert!(grid.can_stand(a));
    }

    #[test]
    fn a_wall_splits_the_floor_and_paths_go_round_it() {
        let mut tris = floor(0.0, 0.0, 10.0, 10.0, 0.0).to_vec();
        // A wall at x = 5 from z = 0 to z = 8: a gap from 8 to 10.
        tris.extend(wall_x(5.0, 0.0, 8.0, 3.0));
        let grid = WalkGrid::build(&tris, AREA, 0.25, Body::default());
        let a = grid.locate(Vec3::new(2.0, 0.0, 2.0), 1.0).unwrap();
        let b = grid.locate(Vec3::new(8.0, 0.0, 2.0), 1.0).unwrap();
        let path = grid.path(a, b).unwrap();
        assert!(path.cost > 12.0, "{}", path.cost);
        // Never within the radius of the wall (less the 1 cm allowed).
        for &node in &path.nodes {
            let p = grid.position(node);
            if p.z < 8.0 {
                assert!((p.x - 5.0).abs() >= 0.37, "{p:?}");
            }
        }
        // The flood agrees with A*, and its steps lead there.
        let to_b = grid.distances_to(&[b]);
        assert!((to_b.get(a) - path.cost).abs() < 1e-3);
        let mut at = a;
        let mut steps = 0;
        while let Some((next, _)) = to_b.next_step(&grid, at) {
            at = next;
            steps += 1;
            assert!(steps < 1000);
        }
        assert_eq!(at, b);
    }

    #[test]
    fn ledges_link_one_way_by_drop_and_both_by_jump() {
        // Low floor x 0..5 at y 0, a ledge x 5..10 at y 1.0 (a jump) and
        // a high deck z 0..10, x 10..15 at y 3.0 (only a drop down).
        let mut tris = floor(0.0, 0.0, 5.0, 10.0, 0.0).to_vec();
        tris.extend(floor(5.0, 0.0, 10.0, 10.0, 1.0));
        tris.extend(wall_x(5.0, 0.0, 10.0, 1.0));
        tris.extend(floor(10.0, 0.0, 15.0, 10.0, 3.0));
        tris.extend(wall_x(10.0, 0.0, 10.0, 3.0));
        let area = [0.0, 0.0, 15.0, 10.0];
        let grid = WalkGrid::build(&tris, area, 0.5, Body::default());
        let low = grid.locate(Vec3::new(2.0, 0.0, 5.0), 1.0).unwrap();
        let ledge = grid.locate(Vec3::new(7.5, 1.0, 5.0), 1.0).unwrap();
        let deck = grid.locate(Vec3::new(12.5, 3.0, 5.0), 1.0).unwrap();
        let from_low = grid.distances_from(&[low]);
        assert!(from_low.reaches(ledge));
        assert!(!from_low.reaches(deck));
        let from_deck = grid.distances_from(&[deck]);
        assert!(from_deck.reaches(ledge) && from_deck.reaches(low));
        let kinds: Vec<LinkKind> = grid
            .path(deck, low)
            .unwrap()
            .nodes
            .windows(2)
            .filter_map(|w| grid.links(w[0]).find(|l| l.0 == w[1]).map(|l| l.1))
            .collect();
        assert!(kinds.contains(&LinkKind::Drop));
        let up: Vec<LinkKind> = grid
            .path(low, ledge)
            .unwrap()
            .nodes
            .windows(2)
            .filter_map(|w| grid.links(w[0]).find(|l| l.0 == w[1]).map(|l| l.1))
            .collect();
        assert!(up.contains(&LinkKind::Jump));
        // No jumps: the ledge is out of reach.
        let body = Body {
            max_jump: 0.0,
            ..Body::default()
        };
        let grid = WalkGrid::build(&tris, area, 0.5, body);
        let low = grid.locate(Vec3::new(2.0, 0.0, 5.0), 1.0).unwrap();
        let ledge = grid.locate(Vec3::new(7.5, 1.0, 5.0), 1.0).unwrap();
        assert!(!grid.distances_from(&[low]).reaches(ledge));
    }

    #[test]
    fn steps_and_steep_or_dont_walk_faces() {
        // A 0.4 m step is walked; a slope of 70° is no floor; `dont_walk`
        // is no floor; a `walk` face is a floor at any slope.
        let mut tris = floor(0.0, 0.0, 5.0, 10.0, 0.0).to_vec();
        tris.extend(floor(5.0, 0.0, 10.0, 10.0, 0.4));
        let grid = WalkGrid::build(&tris, AREA, 0.5, Body::default());
        let a = grid.locate(Vec3::new(2.0, 0.0, 5.0), 1.0).unwrap();
        let b = grid.locate(Vec3::new(8.0, 0.4, 5.0), 1.0).unwrap();
        let path = grid.path(a, b).unwrap();
        assert!(
            path.nodes
                .windows(2)
                .all(|w| grid.links(w[0]).any(|l| l == (w[1], LinkKind::Walk)))
        );

        let steep = |walker| {
            let rise = 5.0 * 70f32.to_radians().tan();
            let a = Vec3::new(0.0, 0.0, 0.0);
            let b = Vec3::new(5.0, rise, 0.0);
            let c = Vec3::new(5.0, rise, 10.0);
            let d = Vec3::new(0.0, 0.0, 10.0);
            [
                Triangle {
                    v: [a, b, c],
                    walker,
                },
                Triangle {
                    v: [a, c, d],
                    walker,
                },
            ]
        };
        let none = WalkGrid::build(&steep(WalkerBehaviour::BySlope), AREA, 0.5, Body::default());
        assert_eq!(none.len(), 0);
        let flat_dont = quad(Vec3::ZERO, 10.0, 10.0, WalkerBehaviour::DontWalk);
        assert_eq!(
            WalkGrid::build(&flat_dont, AREA, 0.5, Body::default()).len(),
            0
        );
        let stairs = WalkGrid::build(&steep(WalkerBehaviour::Walk), AREA, 0.5, Body::default());
        assert!(!stairs.is_empty());
    }

    #[test]
    fn low_ceilings_need_a_crouch_and_lower_ones_block() {
        // A floor with a roof 1.4 m over x 4..6 (crouch only) and a block
        // from 0.8 m over x 7..9 (no way through).
        let mut tris = floor(0.0, 0.0, 10.0, 10.0, 0.0).to_vec();
        let roof = |x0: f32, x1: f32, bottom: f32, top: f32| {
            let mut r = floor(x0, 0.0, x1, 10.0, bottom).to_vec();
            r.extend(floor(x0, 0.0, x1, 10.0, top));
            r
        };
        tris.extend(roof(4.0, 6.0, 1.4, 1.6));
        // A closed block, too high to jump on.
        tris.extend(roof(7.0, 9.0, 0.8, 3.0));
        for x in [7.0, 9.0] {
            let (a, b) = (Vec3::new(x, 0.8, 0.0), Vec3::new(x, 0.8, 10.0));
            let (c, d) = (Vec3::new(x, 3.0, 10.0), Vec3::new(x, 3.0, 0.0));
            let walker = WalkerBehaviour::BySlope;
            tris.push(Triangle {
                v: [a, b, c],
                walker,
            });
            tris.push(Triangle {
                v: [a, c, d],
                walker,
            });
        }
        let grid = WalkGrid::build(&tris, AREA, 0.5, Body::default());
        let under = grid.locate(Vec3::new(5.0, 0.0, 5.0), 0.5).unwrap();
        assert!(!grid.can_stand(under));
        assert!((grid.headroom(under) - 1.4).abs() < 0.01);
        assert!(grid.locate(Vec3::new(8.0, 0.0, 5.0), 0.3).is_none());
        let a = grid.locate(Vec3::new(2.0, 0.0, 5.0), 1.0).unwrap();
        let b = grid.locate(Vec3::new(9.5, 0.0, 5.0), 1.0);
        // x 9..10 is cut off by the low roof (and eroded at the edge).
        assert!(b.is_none_or(|b| !grid.distances_from(&[a]).reaches(b)));
    }

    #[test]
    fn two_floors_in_one_column() {
        // A bridge at 4 m over a floor: both are floors, not linked.
        let mut tris = floor(0.0, 0.0, 10.0, 10.0, 0.0).to_vec();
        tris.extend(floor(0.0, 4.0, 10.0, 6.0, 4.0));
        let grid = WalkGrid::build(&tris, AREA, 0.5, Body::default());
        let below = grid.locate(Vec3::new(5.0, 0.0, 5.0), 0.5).unwrap();
        let above = grid.locate(Vec3::new(5.0, 4.0, 5.0), 0.5).unwrap();
        assert_ne!(below, above);
        assert!((grid.position(above).y - 4.0).abs() < 1e-5);
        assert!((grid.headroom(below) - 4.0).abs() < 1e-5);
        // From the bridge, a drop of 4 m at its sides reaches the ground;
        // there is no way back up.
        assert!(grid.distances_from(&[above]).reaches(below));
        assert!(!grid.distances_from(&[below]).reaches(above));
    }
}
