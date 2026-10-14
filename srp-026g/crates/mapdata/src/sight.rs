//! Line of sight: a ray tree (bounding volume hierarchy) over triangles,
//! normally the bullet mesh ([`crate::MapData::bullet_triangles`]).
//!
//! Built by median splits on the widest axis of the triangles' centres, four
//! triangles a leaf. A test stops at the first hit.

use crate::math::Vec3;
use crate::project::Triangle;

/// Triangles a leaf holds at most.
const LEAF_SIZE: usize = 4;
/// Traversal stack: median splits keep the depth near log2(n / 4).
const STACK: usize = 64;

#[derive(Clone, Copy, Debug)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// A leaf's first triangle, or a branch's right child (the left child
    /// follows the branch).
    first_or_right: u32,
    /// Triangles in a leaf; 0 for a branch.
    count: u32,
}

/// A triangle kept as its corner and two edges, ready for the ray test.
#[derive(Clone, Copy, Debug)]
struct Prepared {
    a: Vec3,
    e1: Vec3,
    e2: Vec3,
}

pub struct RayTree {
    nodes: Vec<Node>,
    triangles: Vec<Prepared>,
}

impl RayTree {
    pub fn build(triangles: &[Triangle]) -> Self {
        Self::from_corners(triangles.iter().map(|t| t.v))
    }

    pub fn from_corners(corners: impl IntoIterator<Item = [Vec3; 3]>) -> Self {
        let corners: Vec<[Vec3; 3]> = corners
            .into_iter()
            .filter(|t| t.iter().all(|v| v.is_finite()))
            .collect();
        let centres: Vec<Vec3> = corners
            .iter()
            .map(|t| (t[0] + t[1] + t[2]) * (1.0 / 3.0))
            .collect();
        let mut order: Vec<u32> = (0..corners.len() as u32).collect();
        let mut nodes = Vec::with_capacity(corners.len() / 2 + 1);
        if !order.is_empty() {
            build_node(&mut nodes, &corners, &centres, &mut order, 0);
        }
        let triangles = order
            .iter()
            .map(|&i| {
                let [a, b, c] = corners[i as usize];
                Prepared {
                    a,
                    e1: b - a,
                    e2: c - a,
                }
            })
            .collect();
        Self { nodes, triangles }
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Bytes the tree holds.
    pub fn memory_bytes(&self) -> usize {
        self.nodes.len() * std::mem::size_of::<Node>()
            + self.triangles.len() * std::mem::size_of::<Prepared>()
    }

    /// Whether nothing lies between `a` and `b` (both excluded by 1 cm).
    pub fn visible(&self, a: Vec3, b: Vec3) -> bool {
        let d = b - a;
        let length = d.length();
        if !length.is_finite() {
            return false;
        }
        if length <= 0.02 {
            return true;
        }
        !self.any_hit(a, d * (1.0 / length), 0.01, length - 0.01)
    }

    /// The distance to the first hit along `dir` (unit) within `max_t`.
    pub fn ray_cast(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<f32> {
        let mut best = None;
        self.walk(origin, dir, 0.0, max_t, &mut |t| {
            best = Some(t);
            false
        });
        best
    }

    fn any_hit(&self, origin: Vec3, dir: Vec3, min_t: f32, max_t: f32) -> bool {
        let mut hit = false;
        self.walk(origin, dir, min_t, max_t, &mut |_| {
            hit = true;
            true
        });
        hit
    }

    /// Visits hits nearer than the best so far; `found(t)` returns whether
    /// to stop.
    fn walk(
        &self,
        origin: Vec3,
        dir: Vec3,
        min_t: f32,
        mut max_t: f32,
        found: &mut dyn FnMut(f32) -> bool,
    ) {
        if self.nodes.is_empty() || !origin.is_finite() || !dir.is_finite() {
            return;
        }
        let inv = Vec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let mut stack = [0u32; STACK];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let index = stack[top] as usize;
            let node = &self.nodes[index];
            if !slab(origin, inv, node.min, node.max, max_t) {
                continue;
            }
            if node.count > 0 {
                let first = node.first_or_right as usize;
                for tri in &self.triangles[first..first + node.count as usize] {
                    if let Some(t) = ray_triangle(origin, dir, tri)
                        && t > min_t
                        && t < max_t
                    {
                        max_t = t;
                        if found(t) {
                            return;
                        }
                    }
                }
            } else if top + 2 <= STACK {
                stack[top] = node.first_or_right;
                stack[top + 1] = index as u32 + 1;
                top += 2;
            }
        }
    }
}

fn build_node(
    nodes: &mut Vec<Node>,
    corners: &[[Vec3; 3]],
    centres: &[Vec3],
    order: &mut [u32],
    first: usize,
) -> usize {
    let mut min = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut max = -min;
    let mut centre_min = min;
    let mut centre_max = max;
    for &i in order.iter() {
        for v in corners[i as usize] {
            min = min.min(v);
            max = max.max(v);
        }
        centre_min = centre_min.min(centres[i as usize]);
        centre_max = centre_max.max(centres[i as usize]);
    }
    let index = nodes.len();
    nodes.push(Node {
        min,
        max,
        first_or_right: first as u32,
        count: order.len() as u32,
    });
    if order.len() <= LEAF_SIZE {
        return index;
    }
    let extent = centre_max - centre_min;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let middle = order.len() / 2;
    order.select_nth_unstable_by(middle, |&a, &b| {
        centres[a as usize]
            .axis(axis)
            .total_cmp(&centres[b as usize].axis(axis))
    });
    let (lower, upper) = order.split_at_mut(middle);
    build_node(nodes, corners, centres, lower, first);
    let right = build_node(nodes, corners, centres, upper, first + middle);
    nodes[index].first_or_right = right as u32;
    nodes[index].count = 0;
    index
}

/// Whether the ray meets the box before `max_t`.
fn slab(origin: Vec3, inv: Vec3, min: Vec3, max: Vec3, max_t: f32) -> bool {
    let mut near = 0.0f32;
    let mut far = max_t;
    for axis in 0..3 {
        let (o, i) = (origin.axis(axis), inv.axis(axis));
        let t1 = (min.axis(axis) - o) * i;
        let t2 = (max.axis(axis) - o) * i;
        // NaN (a flat box along a zero direction) leaves the bounds as they are.
        near = near.max(t1.min(t2));
        far = far.min(t1.max(t2));
    }
    near <= far
}

/// Möller-Trumbore; both faces count.
fn ray_triangle(origin: Vec3, dir: Vec3, tri: &Prepared) -> Option<f32> {
    let p = dir.cross(tri.e2);
    let det = tri.e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - tri.a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(tri.e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(tri.e2.dot(q) * inv)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wall 2 m high in the plane z = 0, from x = -1 to 1.
    fn wall() -> [[Vec3; 3]; 2] {
        let (a, b) = (Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
        let (c, d) = (Vec3::new(1.0, 2.0, 0.0), Vec3::new(-1.0, 2.0, 0.0));
        [[a, b, c], [a, c, d]]
    }

    #[test]
    fn a_wall_blocks_sight_through_it_only() {
        let tree = RayTree::from_corners(wall());
        let eye = Vec3::new(0.0, 1.0, -5.0);
        assert!(!tree.visible(eye, Vec3::new(0.0, 1.0, 5.0)));
        assert!(tree.visible(eye, Vec3::new(0.0, 3.5, 5.0)));
        assert!(tree.visible(eye, Vec3::new(3.0, 1.0, 5.0)));
        // Short of the wall, and from the other side.
        assert!(tree.visible(eye, Vec3::new(0.0, 1.0, -0.5)));
        assert!(!tree.visible(Vec3::new(0.0, 1.0, 5.0), eye));
        let t = tree.ray_cast(eye, Vec3::new(0.0, 0.0, 1.0), 100.0).unwrap();
        assert!((t - 5.0).abs() < 1e-5);
        assert_eq!(tree.ray_cast(eye, Vec3::new(0.0, 0.0, 1.0), 4.0), None);
    }

    #[test]
    fn the_tree_agrees_with_testing_every_triangle() {
        // A field of small random triangles; a few hundred rays.
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32
        };
        let mut corners = Vec::new();
        for _ in 0..2000 {
            let c = Vec3::new(next() * 100.0, next() * 20.0, next() * 100.0);
            corners.push([
                c,
                c + Vec3::new(next() * 3.0, next() * 3.0, next() * 3.0),
                c + Vec3::new(next() * 3.0, next() * 3.0, next() * 3.0),
            ]);
        }
        let tree = RayTree::from_corners(corners.clone());
        assert_eq!(tree.triangle_count(), 2000);
        let prepared: Vec<Prepared> = corners
            .iter()
            .map(|&[a, b, c]| Prepared {
                a,
                e1: b - a,
                e2: c - a,
            })
            .collect();
        let mut blocked = 0;
        for _ in 0..500 {
            let a = Vec3::new(next() * 100.0, next() * 20.0, next() * 100.0);
            let b = Vec3::new(next() * 100.0, next() * 20.0, next() * 100.0);
            let d = b - a;
            let length = d.length();
            let dir = d * (1.0 / length);
            let brute = prepared
                .iter()
                .any(|t| ray_triangle(a, dir, t).is_some_and(|t| t > 0.01 && t < length - 0.01));
            assert_eq!(tree.visible(a, b), !brute);
            blocked += brute as usize;
        }
        assert!(blocked > 50 && blocked < 450, "{blocked}");
    }

    #[test]
    fn an_empty_tree_sees_everything() {
        let tree = RayTree::from_corners([]);
        assert!(tree.visible(Vec3::ZERO, Vec3::Y));
        assert_eq!(tree.ray_cast(Vec3::ZERO, Vec3::Y, 10.0), None);
    }
}
