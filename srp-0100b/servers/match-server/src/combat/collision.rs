// Ray casts against the map's static collision and the four primitive shapes
// the engine uses (`physics/sources/collision_shapes.cpp`: Bullet spheres,
// boxes, Y-axis cylinders and Y-axis capsules).
//
// Bullet reports a triangle hit with the normal turned toward the ray, so a
// bullet sees both faces of a wall it pierces; convex shapes report only the
// entry. Both are reproduced here.

use super::math::{Mat4, Vec3};
use super::resources::{ResourceError, Resources};

/// The map srp serves (see `map.rs`).
pub const SERVER_PROJECT: &str = "projects/level_03_evn/server_project";

/// The collision filter of a ray cast: Bullet keeps a pair when each side's
/// group is in the other's mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RayFilter {
    pub group: u16,
    pub mask: u16,
}

impl RayFilter {
    /// Whether the ray meets an object of this group and mask.
    pub fn meets(self, group: u16, mask: u16) -> bool {
        self.group & mask != 0 && group & self.mask != 0
    }
}

/// `bullet::check_collision`.
pub const BULLET_RAY: RayFilter = RayFilter { group: 16, mask: 8 };
/// `booby_trap_set_core::get_visible_place_transform`: it meets what players
/// walk on (the map's `walker_collision`), not what only stops bullets.
pub const TRAP_PLACEMENT_RAY: RayFilter = RayFilter {
    group: 0x404,
    mask: 0x202,
};

const CHUNK_VERTICES: u32 = 0x19;
const CHUNK_INDICES: u32 = 0x1a;
const CHUNK_FACE_DATA_HEADER: u32 = 0x1b;
const CHUNK_FACE_DATA: u32 = 0x1c;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sphere {
        radius: f32,
    },
    Box {
        half: Vec3,
    },
    /// Along local Y.
    Cylinder {
        radius: f32,
        half_height: f32,
    },
    /// Along local Y; `half_height` excludes the end caps.
    Capsule {
        radius: f32,
        half_height: f32,
    },
}

impl Shape {
    /// `create_bt_primitive( type, dim )`.
    pub fn from_engine(kind: i32, dim: Vec3) -> Option<Self> {
        match kind {
            0 => Some(Self::Sphere { radius: dim.x }),
            1 => Some(Self::Box { half: dim }),
            2 => Some(Self::Cylinder {
                radius: dim.x,
                half_height: dim.y,
            }),
            // btCapsuleShape( radius, height ): height is between the caps.
            3 => Some(Self::Capsule {
                radius: dim.x,
                half_height: dim.y * 0.5,
            }),
            _ => None,
        }
    }

    fn local_bounds(&self) -> (Vec3, Vec3) {
        let half = match *self {
            Self::Sphere { radius } => Vec3::new(radius, radius, radius),
            Self::Box { half } => half,
            Self::Cylinder {
                radius,
                half_height,
            } => Vec3::new(radius, half_height, radius),
            Self::Capsule {
                radius,
                half_height,
            } => Vec3::new(radius, half_height + radius, radius),
        };
        (-half, half)
    }

    /// The farthest local point toward local direction `d` (Bullet's
    /// `localGetSupportingVertex`; these shapes keep their margins inside).
    fn support(&self, d: Vec3) -> Vec3 {
        let toward = |v: f32, extent: f32| if v < 0.0 { -extent } else { extent };
        match *self {
            Self::Sphere { radius } => d.normalized_or(Vec3::Y) * radius,
            Self::Box { half } => Vec3::new(
                toward(d.x, half.x),
                toward(d.y, half.y),
                toward(d.z, half.z),
            ),
            Self::Cylinder {
                radius,
                half_height,
            } => {
                let across = Vec3::new(d.x, 0.0, d.z).normalized_or(Vec3::ZERO) * radius;
                Vec3::new(across.x, toward(d.y, half_height), across.z)
            }
            Self::Capsule {
                radius,
                half_height,
            } => Vec3::new(0.0, toward(d.y, half_height), 0.0) + d.normalized_or(Vec3::Y) * radius,
        }
    }

    /// Entry hit of a local-space ray: (t, outward normal). A ray starting
    /// inside reports nothing.
    pub fn ray(&self, origin: Vec3, dir: Vec3) -> Option<(f32, Vec3)> {
        match *self {
            Self::Sphere { radius } => ray_sphere(origin, dir, Vec3::ZERO, radius),
            Self::Box { half } => ray_box(origin, dir, half),
            Self::Cylinder {
                radius,
                half_height,
            } => ray_cylinder(origin, dir, radius, half_height),
            Self::Capsule {
                radius,
                half_height,
            } => ray_capsule(origin, dir, radius, half_height),
        }
    }
}

fn ray_sphere(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> Option<(f32, Vec3)> {
    let offset = origin - center;
    let c = offset.squared_length() - radius * radius;
    if c <= 0.0 {
        return None;
    }
    let a = dir.squared_length();
    let b = offset.dot(dir);
    let discriminant = b * b - a * c;
    if a == 0.0 || discriminant < 0.0 {
        return None;
    }
    let t = (-b - discriminant.sqrt()) / a;
    (t >= 0.0).then(|| (t, (origin + dir * t - center).normalized_or(Vec3::Y)))
}

fn ray_box(origin: Vec3, dir: Vec3, half: Vec3) -> Option<(f32, Vec3)> {
    let mut t_near = f32::NEG_INFINITY;
    let mut t_far = f32::INFINITY;
    let mut axis_near = 0;
    for axis in 0..3 {
        let (o, d, h) = (origin.axis(axis), dir.axis(axis), half.axis(axis));
        if d.abs() < 1e-12 {
            if o < -h || o > h {
                return None;
            }
            continue;
        }
        let (mut t0, mut t1) = ((-h - o) / d, (h - o) / d);
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        if t0 > t_near {
            t_near = t0;
            axis_near = axis;
        }
        t_far = t_far.min(t1);
        if t_near > t_far {
            return None;
        }
    }
    if t_near < 0.0 {
        return None;
    }
    let mut normal = Vec3::ZERO;
    let sign = if dir.axis(axis_near) > 0.0 { -1.0 } else { 1.0 };
    match axis_near {
        0 => normal.x = sign,
        1 => normal.y = sign,
        _ => normal.z = sign,
    }
    Some((t_near, normal))
}

fn ray_cylinder(origin: Vec3, dir: Vec3, radius: f32, half_height: f32) -> Option<(f32, Vec3)> {
    let inside = origin.x * origin.x + origin.z * origin.z <= radius * radius
        && origin.y.abs() <= half_height;
    if inside {
        return None;
    }
    let mut best: Option<(f32, Vec3)> = None;
    let mut consider = |t: f32, normal: Vec3| {
        if t >= 0.0 && best.is_none_or(|(b, _)| t < b) {
            best = Some((t, normal));
        }
    };
    let a = dir.x * dir.x + dir.z * dir.z;
    if a > 1e-12 {
        let b = origin.x * dir.x + origin.z * dir.z;
        let c = origin.x * origin.x + origin.z * origin.z - radius * radius;
        let discriminant = b * b - a * c;
        if discriminant >= 0.0 {
            let t = (-b - discriminant.sqrt()) / a;
            let y = origin.y + dir.y * t;
            if y.abs() <= half_height {
                let p = origin + dir * t;
                consider(t, Vec3::new(p.x, 0.0, p.z).normalized_or(Vec3::Y));
            }
        }
    }
    if dir.y.abs() > 1e-12 {
        for cap in [-half_height, half_height] {
            let t = (cap - origin.y) / dir.y;
            let p = origin + dir * t;
            if p.x * p.x + p.z * p.z <= radius * radius {
                consider(t, Vec3::new(0.0, cap.signum(), 0.0));
            }
        }
    }
    best
}

fn ray_capsule(origin: Vec3, dir: Vec3, radius: f32, half_height: f32) -> Option<(f32, Vec3)> {
    let clamped = Vec3::new(0.0, origin.y.clamp(-half_height, half_height), 0.0);
    if (origin - clamped).squared_length() <= radius * radius {
        return None;
    }
    let mut best: Option<(f32, Vec3)> = None;
    let a = dir.x * dir.x + dir.z * dir.z;
    if a > 1e-12 {
        let b = origin.x * dir.x + origin.z * dir.z;
        let c = origin.x * origin.x + origin.z * origin.z - radius * radius;
        let discriminant = b * b - a * c;
        if discriminant >= 0.0 {
            let t = (-b - discriminant.sqrt()) / a;
            let p = origin + dir * t;
            if t >= 0.0 && p.y.abs() <= half_height {
                best = Some((t, Vec3::new(p.x, 0.0, p.z).normalized_or(Vec3::Y)));
            }
        }
    }
    for cap in [-half_height, half_height] {
        if let Some((t, normal)) = ray_sphere(origin, dir, Vec3::new(0.0, cap, 0.0), radius) {
            if best.is_none_or(|(b, _)| t < b) {
                best = Some((t, normal));
            }
        }
    }
    best
}

/// A shape placed in the world.
#[derive(Clone, Debug)]
pub struct Placed {
    pub shape: Shape,
    pub to_world: Mat4,
    pub to_local: Mat4,
}

impl Placed {
    pub fn new(shape: Shape, to_world: Mat4) -> Option<Self> {
        Some(Self {
            shape,
            to_world,
            to_local: to_world.inverse()?,
        })
    }

    /// Entry hit along a world ray: (t in world units along `dir`, world
    /// normal).
    pub fn ray(&self, origin: Vec3, dir: Vec3) -> Option<(f32, Vec3)> {
        let local_origin = self.to_local.transform_position(origin);
        let local_dir = self.to_local.transform_direction(dir);
        let (t, local_normal) = self.shape.ray(local_origin, local_dir)?;
        // Normals transform by the inverse transpose; with row vectors that is
        // `to_local` applied from the other side.
        let l = &self.to_local;
        let normal = Vec3::new(
            l.i.dot(local_normal),
            l.j.dot(local_normal),
            l.k.dot(local_normal),
        );
        Some((t, normal.normalized_or(-dir)))
    }

    /// The farthest world point toward world direction `d`.
    fn support(&self, d: Vec3) -> Vec3 {
        let m = &self.to_world;
        let local = Vec3::new(m.i.dot(d), m.j.dot(d), m.k.dot(d));
        m.transform_position(self.shape.support(local))
    }

    /// Whether the two shapes touch or overlap: GJK on their Minkowski
    /// difference, as Bullet's convex contact tests run it.
    pub fn overlaps(&self, other: &Placed) -> bool {
        let (lo, hi) = self.bounds();
        let (other_lo, other_hi) = other.bounds();
        if lo.x > other_hi.x
            || lo.y > other_hi.y
            || lo.z > other_hi.z
            || other_lo.x > hi.x
            || other_lo.y > hi.y
            || other_lo.z > hi.z
        {
            return false;
        }
        let support = |d: Vec3| self.support(d) - other.support(-d);
        let start = (self.to_world.c - other.to_world.c).normalized_or(Vec3::Y);
        let mut simplex = vec![support(start)];
        let mut d = -simplex[0];
        for _ in 0..GJK_ITERATIONS {
            if d.squared_length() < GJK_EPSILON {
                return true;
            }
            let point = support(d);
            if point.dot(d) < 0.0 {
                return false;
            }
            simplex.push(point);
            match gjk_next(&mut simplex) {
                Some(next) => d = next,
                None => return true,
            }
        }
        false
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        let (min, max) = self.shape.local_bounds();
        let mut lo = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
        let mut hi = -lo;
        for corner in 0..8 {
            let local = Vec3::new(
                if corner & 1 == 0 { min.x } else { max.x },
                if corner & 2 == 0 { min.y } else { max.y },
                if corner & 4 == 0 { min.z } else { max.z },
            );
            let world = self.to_world.transform_position(local);
            lo = lo.min(world);
            hi = hi.max(world);
        }
        (lo, hi)
    }
}

const GJK_ITERATIONS: usize = 64;
const GJK_EPSILON: f32 = 1e-12;

/// One GJK step on a simplex of the Minkowski difference whose last point is
/// the newest: keeps the part nearest the origin and returns the direction
/// to search next, or `None` when the simplex holds the origin.
fn gjk_next(simplex: &mut Vec<Vec3>) -> Option<Vec3> {
    match simplex.len() {
        2 => gjk_line(simplex),
        3 => gjk_triangle(simplex),
        _ => gjk_tetrahedron(simplex),
    }
}

fn gjk_line(simplex: &mut Vec<Vec3>) -> Option<Vec3> {
    let (b, a) = (simplex[0], simplex[1]);
    let (ab, ao) = (b - a, -a);
    if ab.dot(ao) > 0.0 {
        let d = ab.cross(ao).cross(ab);
        (d.squared_length() > GJK_EPSILON * ab.squared_length()).then_some(d)
    } else {
        *simplex = vec![a];
        Some(ao)
    }
}

fn gjk_triangle(simplex: &mut Vec<Vec3>) -> Option<Vec3> {
    let (c, b, a) = (simplex[0], simplex[1], simplex[2]);
    let (ab, ac, ao) = (b - a, c - a, -a);
    let abc = ab.cross(ac);
    if abc.cross(ac).dot(ao) > 0.0 {
        if ac.dot(ao) > 0.0 {
            *simplex = vec![c, a];
            let d = ac.cross(ao).cross(ac);
            return (d.squared_length() > GJK_EPSILON * ac.squared_length()).then_some(d);
        }
        *simplex = vec![b, a];
        return gjk_line(simplex);
    }
    if ab.cross(abc).dot(ao) > 0.0 {
        *simplex = vec![b, a];
        return gjk_line(simplex);
    }
    let side = abc.dot(ao);
    if side.abs() <= 1e-7 * abc.length() {
        return None;
    }
    if side > 0.0 {
        Some(abc)
    } else {
        *simplex = vec![b, c, a];
        Some(-abc)
    }
}

fn gjk_tetrahedron(simplex: &mut Vec<Vec3>) -> Option<Vec3> {
    let (d, c, b, a) = (simplex[0], simplex[1], simplex[2], simplex[3]);
    let ao = -a;
    // Each face through the newest point, turned away from the fourth.
    for (x, y, behind) in [(b, c, d), (c, d, b), (d, b, c)] {
        let mut normal = (x - a).cross(y - a);
        if normal.dot(behind - a) > 0.0 {
            normal = -normal;
        }
        if normal.dot(ao) > 0.0 {
            *simplex = vec![y, x, a];
            return gjk_triangle(simplex);
        }
    }
    None
}

#[derive(Clone, Copy, Debug)]
struct Triangle {
    a: Vec3,
    b: Vec3,
    c: Vec3,
    material: u16,
}

impl Triangle {
    /// Two-sided Möller–Trumbore; the normal faces the ray (Bullet's
    /// `btTriangleRaycastCallback`).
    fn ray(&self, origin: Vec3, dir: Vec3) -> Option<(f32, Vec3)> {
        let e1 = self.b - self.a;
        let e2 = self.c - self.a;
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let s = origin - self.a;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(e1);
        let v = dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = e2.dot(q) * inv;
        if t < 0.0 {
            return None;
        }
        let normal = e1.cross(e2).normalized_or(Vec3::Y);
        let normal = if normal.dot(dir) > 0.0 {
            -normal
        } else {
            normal
        };
        Some((t, normal))
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        (
            self.a.min(self.b).min(self.c),
            self.a.max(self.b).max(self.c),
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum Item {
    Triangle(u32),
    Primitive(u32),
}

#[derive(Clone, Debug)]
enum Node {
    Leaf {
        min: Vec3,
        max: Vec3,
        items: Vec<Item>,
    },
    Branch {
        min: Vec3,
        max: Vec3,
        left: usize,
        right: usize,
    },
}

impl Node {
    fn bounds(&self) -> (Vec3, Vec3) {
        match self {
            Self::Leaf { min, max, .. } | Self::Branch { min, max, .. } => (*min, *max),
        }
    }
}

/// Slab test: the entry distance of a ray into a box, if within `max_t`.
fn ray_aabb(origin: Vec3, inv_dir: Vec3, min: Vec3, max: Vec3, max_t: f32) -> Option<f32> {
    let mut t0: f32 = 0.0;
    let mut t1 = max_t;
    for axis in 0..3 {
        let (o, inv) = (origin.axis(axis), inv_dir.axis(axis));
        let (mut near, mut far) = ((min.axis(axis) - o) * inv, (max.axis(axis) - o) * inv);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        // NaN (0 * inf on a slab edge) keeps the current bounds.
        if near > t0 {
            t0 = near;
        }
        if far < t1 {
            t1 = far;
        }
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldHit {
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub material: u16,
}

/// A world's triangles and primitives before its tree is built.
type WorldParts = (Vec<Triangle>, Vec<(Placed, u16)>);

/// The map's static collision that one kind of ray meets (bullets, or the
/// trap placement ray).
pub struct StaticWorld {
    triangles: Vec<Triangle>,
    primitives: Vec<(Placed, u16)>,
    nodes: Vec<Node>,
    min: Vec3,
    max: Vec3,
}

impl StaticWorld {
    fn build(triangles: Vec<Triangle>, primitives: Vec<(Placed, u16)>) -> Self {
        let mut items: Vec<(Item, Vec3, Vec3)> = triangles
            .iter()
            .enumerate()
            .map(|(i, tri)| {
                let (min, max) = tri.bounds();
                (Item::Triangle(i as u32), min, max)
            })
            .chain(primitives.iter().enumerate().map(|(i, (placed, _))| {
                let (min, max) = placed.bounds();
                (Item::Primitive(i as u32), min, max)
            }))
            .collect();
        let mut nodes = Vec::new();
        let (min, max) = if items.is_empty() {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            build_node(&mut nodes, &mut items);
            nodes[0].bounds()
        };
        Self {
            triangles,
            primitives,
            nodes,
            min,
            max,
        }
    }

    /// Loads every collision object of the server project that stops
    /// bullets, with its per-face game materials.
    #[cfg(test)]
    pub fn load(resources: &mut Resources, project: &str) -> Result<Self, ResourceError> {
        let mut worlds = Self::load_for(resources, project, &[BULLET_RAY])?;
        Ok(worlds.remove(0))
    }

    /// One world per filter: the server project's collision objects that
    /// filter's ray meets, with their per-face game materials. Objects in
    /// several share their parsed collision files.
    pub fn load_for(
        resources: &mut Resources,
        project: &str,
        filters: &[RayFilter],
    ) -> Result<Vec<Self>, ResourceError> {
        let project = resources.config(project)?;
        let objects = project.req("objects")?.req("collision_objects")?.children();
        let mut libraries: std::collections::HashMap<String, Library> = Default::default();
        let mut worlds: Vec<WorldParts> = filters.iter().map(|_| Default::default()).collect();
        for object in objects {
            let filter_value = |key: &str| -> Result<u16, ResourceError> {
                u16::try_from(object.i32_at(key)?)
                    .map_err(|_| ResourceError(format!("collision object {key} out of range")))
            };
            let (group, mask) = (filter_value("cgroup")?, filter_value("cmask")?);
            let wanted: Vec<usize> = (0..filters.len())
                .filter(|&i| filters[i].meets(group, mask))
                .collect();
            if wanted.is_empty() {
                continue;
            }
            let lib_name = object.str_at("lib_name")?;
            let transform = Mat4::compose(
                Vec3::from_array(object.float3_at("scale")?),
                Vec3::from_array(object.float3_at("rotation")?),
                Vec3::from_array(object.float3_at("position")?),
            );
            if !libraries.contains_key(lib_name) {
                let library = match Library::load(resources, lib_name) {
                    Ok(library) => library,
                    Err(error) => {
                        log::warn!("collision {lib_name}: {error}");
                        Library::default()
                    }
                };
                libraries.insert(lib_name.to_owned(), library);
            }
            let library = &libraries[lib_name];
            for i in wanted {
                let (triangles, primitives) = &mut worlds[i];
                for tri in &library.triangles {
                    triangles.push(Triangle {
                        a: transform.transform_position(tri.a),
                        b: transform.transform_position(tri.b),
                        c: transform.transform_position(tri.c),
                        material: tri.material,
                    });
                }
                for (shape, local, material) in &library.primitives {
                    if let Some(placed) = Placed::new(*shape, *local * transform) {
                        primitives.push((placed, *material));
                    }
                }
            }
        }
        Ok(worlds
            .into_iter()
            .map(|(triangles, primitives)| Self::build(triangles, primitives))
            .collect())
    }

    /// A world of the given triangles (three corners each), all of one
    /// material.
    #[cfg(test)]
    pub fn from_triangles_for_tests(corners: &[Vec3], material: u16) -> Self {
        let triangles = corners
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[a, b, c]| Triangle { a, b, c, material })
            .collect();
        Self::build(triangles, Vec::new())
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    pub fn primitive_count(&self) -> usize {
        self.primitives.len()
    }

    /// The map's extent; bullets leaving it stop
    /// (`bullet_manager::is_inside_collision_db`).
    pub fn contains(&self, point: Vec3) -> bool {
        const MARGIN: f32 = 50.0;
        point.x >= self.min.x - MARGIN
            && point.x <= self.max.x + MARGIN
            && point.z >= self.min.z - MARGIN
            && point.z <= self.max.z + MARGIN
            && point.y >= self.min.y - MARGIN
            && point.y <= self.max.y + 4.0 * MARGIN
    }

    /// The closest hit along `dir` (unit) within `max_t`.
    pub fn ray_cast(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<WorldHit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv_dir = Vec3::new(1.0 / dir.x, 1.0 / dir.y, 1.0 / dir.z);
        let mut best: Option<(f32, Vec3, u16)> = None;
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let limit = best.map_or(max_t, |(t, _, _)| t);
            let (min, max) = self.nodes[index].bounds();
            if ray_aabb(origin, inv_dir, min, max, limit).is_none() {
                continue;
            }
            match &self.nodes[index] {
                Node::Branch { left, right, .. } => {
                    stack.push(*left);
                    stack.push(*right);
                }
                Node::Leaf { items, .. } => {
                    for item in items {
                        let hit = match *item {
                            Item::Triangle(i) => {
                                let tri = &self.triangles[i as usize];
                                tri.ray(origin, dir).map(|(t, n)| (t, n, tri.material))
                            }
                            Item::Primitive(i) => {
                                let (placed, material) = &self.primitives[i as usize];
                                placed.ray(origin, dir).map(|(t, n)| (t, n, *material))
                            }
                        };
                        if let Some((t, normal, material)) = hit {
                            if t <= max_t && best.is_none_or(|(b, _, _)| t < b) {
                                best = Some((t, normal, material));
                            }
                        }
                    }
                }
            }
        }
        best.map(|(t, normal, material)| WorldHit {
            t,
            point: origin + dir * t,
            normal,
            material,
        })
    }
}

fn build_node(nodes: &mut Vec<Node>, items: &mut [(Item, Vec3, Vec3)]) -> usize {
    const LEAF_SIZE: usize = 4;
    let mut min = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut max = -min;
    for (_, lo, hi) in items.iter() {
        min = min.min(*lo);
        max = max.max(*hi);
    }
    let index = nodes.len();
    if items.len() <= LEAF_SIZE {
        nodes.push(Node::Leaf {
            min,
            max,
            items: items.iter().map(|(item, _, _)| *item).collect(),
        });
        return index;
    }
    let extent = max - min;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let middle = items.len() / 2;
    items.select_nth_unstable_by(middle, |a, b| {
        let ca = a.1.axis(axis) + a.2.axis(axis);
        let cb = b.1.axis(axis) + b.2.axis(axis);
        ca.total_cmp(&cb)
    });
    nodes.push(Node::Leaf {
        min,
        max,
        items: Vec::new(),
    });
    let (lower, upper) = items.split_at_mut(middle);
    let left = build_node(nodes, lower);
    let right = build_node(nodes, upper);
    nodes[index] = Node::Branch {
        min,
        max,
        left,
        right,
    };
    index
}

/// One collision library (`<model>.model/collision`), in its own space.
#[derive(Default)]
struct Library {
    triangles: Vec<Triangle>,
    primitives: Vec<(Shape, Mat4, u16)>,
}

impl Library {
    /// `collision_shape_cook`: an optional `#[sx][sy][sz]` scale, the
    /// primitives, the triangle mesh, and materials from the model settings.
    fn load(resources: &mut Resources, lib_name: &str) -> Result<Self, ResourceError> {
        let (path, scale) = match lib_name.find("#[") {
            Some(at) => (&lib_name[..at], parse_scale(&lib_name[at..])),
            None => (lib_name, Vec3::new(1.0, 1.0, 1.0)),
        };
        let settings_path = match path.find(".model") {
            Some(at) => format!("{}/settings", &path[..at + ".model".len()]),
            None => format!("{path}/settings"),
        };
        let materials = if resources.exists(&settings_path) {
            resources
                .config(&settings_path)?
                .get("game_material_settings")
                .cloned()
        } else {
            None
        };
        let material_of = |shading_group: &str| -> u16 {
            materials
                .as_ref()
                .and_then(|m| m.get(shading_group))
                .and_then(|m| m.get("game_material_id"))
                .and_then(|id| id.as_i32())
                .and_then(|id| u16::try_from(id).ok())
                .unwrap_or(u16::MAX)
        };

        let mut library = Self::default();
        let primitives_path = format!("{path}/exported_primitives");
        if resources.exists(&primitives_path) {
            let config = resources.config(&primitives_path)?;
            let names: Vec<String> = config
                .get("mtl_list")
                .map(|list| {
                    list.children()
                        .iter()
                        .map(|name| name.as_str().unwrap_or_default().to_owned())
                        .collect()
                })
                .unwrap_or_default();
            let primitives = config.req("primitives")?.children();
            // A lone primitive at the origin is built without its rotation.
            let single = primitives.len() == 1
                && primitives[0]
                    .float3_at("position")
                    .map(|p| Vec3::from_array(p).length() < 1e-4)
                    .unwrap_or(false);
            for primitive in primitives {
                let Some(shape) = Shape::from_engine(
                    primitive.i32_at("type")?,
                    Vec3::from_array(primitive.float3_at("scale")?),
                ) else {
                    continue;
                };
                let local = if single {
                    Mat4::IDENTITY
                } else {
                    Mat4::rotation_xyz(Vec3::from_array(primitive.float3_at("rotation")?))
                        * Mat4::translation(Vec3::from_array(primitive.float3_at("position")?))
                };
                let shading_group = primitive
                    .get("mtl")
                    .and_then(|m| m.as_i32())
                    .and_then(|i| names.get(i as usize))
                    .map(String::as_str)
                    .unwrap_or("");
                let material = if materials.is_some() {
                    match material_of(shading_group) {
                        u16::MAX => 0,
                        id => id,
                    }
                } else {
                    0
                };
                library
                    .primitives
                    .push((shape, local * Mat4::scale(scale), material));
            }
        }

        let vertices_path = format!("{path}/vertices");
        if resources.exists(&vertices_path) {
            let vertices = resources.read(&vertices_path)?;
            let indices = resources.read(&format!("{path}/indices"))?;
            let vertices = chunk(&vertices, CHUNK_VERTICES)
                .ok_or_else(|| ResourceError(format!("{path}: no vertex chunk")))?;
            let indices = chunk(&indices, CHUNK_INDICES)
                .ok_or_else(|| ResourceError(format!("{path}: no index chunk")))?;
            let points: Vec<Vec3> = vertices
                .chunks_exact(12)
                .map(|v| {
                    let f = |i: usize| f32::from_le_bytes(v[i..i + 4].try_into().unwrap());
                    Vec3::new(f(0) * scale.x, f(4) * scale.y, f(8) * scale.z)
                })
                .collect();
            let indices: Vec<u32> = indices
                .chunks_exact(4)
                .map(|i| u32::from_le_bytes(i.try_into().unwrap()))
                .collect();
            let face_materials = if resources.exists(&format!("{path}/face_data")) {
                let face_data = resources.read(&format!("{path}/face_data"))?;
                face_materials(&face_data, &material_of)
            } else {
                Vec::new()
            };
            for (i, tri) in indices.chunks_exact(3).enumerate() {
                let corner = |k: usize| points.get(tri[k] as usize).copied();
                let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else {
                    continue;
                };
                library.triangles.push(Triangle {
                    a,
                    b,
                    c,
                    material: face_materials.get(i).copied().unwrap_or(u16::MAX),
                });
            }
        }
        Ok(library)
    }
}

fn parse_scale(suffix: &str) -> Vec3 {
    let values: Vec<f32> = suffix
        .split(['[', ']'])
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    match values[..] {
        [x, y, z, ..] => Vec3::new(x, y, z),
        _ => Vec3::new(1.0, 1.0, 1.0),
    }
}

/// The body of chunk `id` in a sequential chunk file (`u32 id, u32 size`).
fn chunk(bytes: &[u8], id: u32) -> Option<&[u8]> {
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let chunk_id = u32::from_le_bytes(bytes[at..at + 4].try_into().ok()?);
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..at + 8 + size)?;
        if chunk_id == id {
            return Some(body);
        }
        at += 8 + size;
    }
    None
}

/// Per-triangle game materials: a header of shading group names, then one
/// u16 index into it per triangle; unknown groups become `u16::MAX`, which
/// the material table resolves to `default`.
fn face_materials(bytes: &[u8], material_of: &dyn Fn(&str) -> u16) -> Vec<u16> {
    let (Some(header), Some(faces)) = (
        chunk(bytes, CHUNK_FACE_DATA_HEADER),
        chunk(bytes, CHUNK_FACE_DATA),
    ) else {
        return Vec::new();
    };
    if header.len() < 2 {
        return Vec::new();
    }
    let count = u16::from_le_bytes([header[0], header[1]]) as usize;
    let mut remap = Vec::with_capacity(count);
    let mut names = header[2..].split(|&b| b == 0);
    for _ in 0..count {
        let name = names.next().unwrap_or_default();
        remap.push(material_of(&String::from_utf8_lossy(name)));
    }
    faces
        .chunks_exact(2)
        .map(|f| {
            let index = u16::from_le_bytes([f[0], f[1]]) as usize;
            remap.get(index).copied().unwrap_or(u16::MAX)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::resources::tests::game_resources;

    fn placed(shape: Shape, angles: Vec3, at: Vec3) -> Placed {
        Placed::new(shape, Mat4::rotation_xyz(angles) * Mat4::translation(at)).unwrap()
    }

    #[test]
    fn convex_shapes_overlap_where_they_touch() {
        let sphere = |at| placed(Shape::Sphere { radius: 0.5 }, Vec3::ZERO, at);
        let origin = sphere(Vec3::ZERO);
        // Spheres overlap below a metre apart, whichever way.
        for step in 0..32 {
            let angle = step as f32 * 0.37;
            let dir =
                Vec3::new(angle.cos(), (angle * 1.7).sin(), angle.sin()).normalized_or(Vec3::Y);
            assert!(origin.overlaps(&sphere(dir * 0.98)), "{dir:?}");
            assert!(!origin.overlaps(&sphere(dir * 1.02)), "{dir:?}");
        }
        // A trap's flat cylinder and a box foot: touching from above and
        // aside, missed over its top and past its rim.
        let trap = placed(
            Shape::Cylinder {
                radius: 0.25,
                half_height: 0.1,
            },
            Vec3::ZERO,
            Vec3::new(0.0, 0.12, 0.0),
        );
        let foot = |angles, at| {
            placed(
                Shape::Box {
                    half: Vec3::new(0.05, 0.05, 0.12),
                },
                angles,
                at,
            )
        };
        assert!(trap.overlaps(&foot(Vec3::ZERO, Vec3::new(0.1, 0.26, 0.0))));
        assert!(!trap.overlaps(&foot(Vec3::ZERO, Vec3::new(0.1, 0.28, 0.0))));
        assert!(trap.overlaps(&foot(Vec3::ZERO, Vec3::new(0.0, 0.1, 0.36))));
        assert!(!trap.overlaps(&foot(Vec3::ZERO, Vec3::new(0.0, 0.1, 0.38))));
        // Turned 45 degrees about y, the box's corner reaches further out.
        let turned = Vec3::new(0.0, std::f32::consts::FRAC_PI_4, 0.0);
        assert!(trap.overlaps(&foot(turned, Vec3::new(0.0, 0.1, 0.33))));
        assert!(!trap.overlaps(&foot(turned, Vec3::new(0.0, 0.1, 0.38))));
        // Past the rim diagonally, where only the round edge decides.
        let diagonal = Vec3::new(1.0, 0.0, 1.0).normalized_or(Vec3::Y);
        let ball = |at| placed(Shape::Sphere { radius: 0.05 }, Vec3::ZERO, at);
        let middle = Vec3::new(0.0, 0.12, 0.0);
        assert!(trap.overlaps(&ball(diagonal * 0.29 + middle)));
        assert!(!trap.overlaps(&ball(diagonal * 0.31 + middle)));
        // A capsule lying across the top.
        let shin = |y| {
            placed(
                Shape::Capsule {
                    radius: 0.06,
                    half_height: 0.2,
                },
                Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0),
                Vec3::new(0.0, y, 0.0),
            )
        };
        assert!(trap.overlaps(&shin(0.27)));
        assert!(!trap.overlaps(&shin(0.29)));
    }

    #[test]
    fn primitive_rays_hit_the_near_surface_only_from_outside() {
        let down = Vec3::new(0.0, -1.0, 0.0);
        let from_above = Vec3::new(0.0, 5.0, 0.0);
        let shapes = [
            (Shape::Sphere { radius: 1.0 }, 4.0),
            (
                Shape::Box {
                    half: Vec3::new(1.0, 2.0, 1.0),
                },
                3.0,
            ),
            (
                Shape::Cylinder {
                    radius: 1.0,
                    half_height: 2.0,
                },
                3.0,
            ),
            (
                Shape::Capsule {
                    radius: 1.0,
                    half_height: 2.0,
                },
                2.0,
            ),
        ];
        for (shape, expected) in shapes {
            let (t, normal) = shape.ray(from_above, down).unwrap();
            assert!((t - expected).abs() < 1e-5, "{shape:?}: {t}");
            assert!((normal - Vec3::Y).length() < 1e-5, "{shape:?}: {normal:?}");
            assert!(
                shape.ray(Vec3::ZERO, down).is_none(),
                "{shape:?} from inside"
            );
            assert!(
                shape.ray(Vec3::new(5.0, 5.0, 0.0), down).is_none(),
                "{shape:?} miss"
            );
        }
    }

    #[test]
    fn triangles_report_the_face_toward_the_ray() {
        let tri = Triangle {
            a: Vec3::new(-1.0, 0.0, -1.0),
            b: Vec3::new(1.0, 0.0, -1.0),
            c: Vec3::new(0.0, 0.0, 1.0),
            material: 3,
        };
        let (t, n) = tri
            .ray(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, -1.0, 0.0))
            .unwrap();
        assert!((t - 2.0).abs() < 1e-6 && n.y > 0.99);
        let (_, n) = tri
            .ray(Vec3::new(0.0, -2.0, 0.0), Vec3::new(0.0, 1.0, 0.0))
            .unwrap();
        assert!(n.y < -0.99);
    }

    #[test]
    fn the_bvh_finds_the_closest_of_many() {
        let triangles: Vec<Triangle> = (0..100)
            .map(|i| {
                let z = i as f32;
                Triangle {
                    a: Vec3::new(-1.0, -1.0, z),
                    b: Vec3::new(1.0, -1.0, z),
                    c: Vec3::new(0.0, 1.0, z),
                    material: i,
                }
            })
            .collect();
        let world = StaticWorld::build(triangles, Vec::new());
        let hit = world
            .ray_cast(Vec3::new(0.0, 0.0, 41.5), Vec3::new(0.0, 0.0, 1.0), 1000.0)
            .unwrap();
        assert_eq!(hit.material, 42);
        assert!((hit.t - 0.5).abs() < 1e-5);
        assert!(
            world
                .ray_cast(Vec3::new(0.0, 0.0, 41.5), Vec3::new(0.0, 0.0, 1.0), 0.4)
                .is_none()
        );
    }

    #[test]
    fn loads_the_maps_bullet_collision() {
        let Some(mut resources) = game_resources() else {
            return;
        };
        let world = StaticWorld::load(&mut resources, SERVER_PROJECT).unwrap();
        assert!(
            world.triangle_count() > 10_000,
            "{}",
            world.triangle_count()
        );
        assert!(world.primitive_count() > 100, "{}", world.primitive_count());
        // Team 1's base is indoors: from head height at a spawn point (feet at
        // y 0.51) the floor is below and a roof above.
        let floor = world
            .ray_cast(
                Vec3::new(-34.0, 1.7, -76.0),
                Vec3::new(0.0, -1.0, 0.0),
                100.0,
            )
            .expect("the base has a floor");
        assert!((floor.point.y - 0.51).abs() < 0.3, "{floor:?}");
        assert!(floor.normal.y > 0.5, "{floor:?}");
        let roof = world
            .ray_cast(
                Vec3::new(-34.0, 1.7, -76.0),
                Vec3::new(0.0, 1.0, 0.0),
                100.0,
            )
            .expect("the base has a roof");
        assert!(roof.normal.y < -0.5, "{roof:?}");
    }

    #[test]
    fn the_trap_placement_ray_meets_what_players_walk_on() {
        // Bullet-only objects (group 8, mask 16) are left out; walker-only
        // (2, 4) and shared (10, 20) ones are in.
        assert!(!TRAP_PLACEMENT_RAY.meets(8, 16));
        assert!(TRAP_PLACEMENT_RAY.meets(2, 4));
        assert!(TRAP_PLACEMENT_RAY.meets(10, 20));
        assert!(!BULLET_RAY.meets(2, 4));
        // Another trap's sensor (513, 1056) stops it; players do not.
        assert!(TRAP_PLACEMENT_RAY.meets(513, 1056));
        assert!(!TRAP_PLACEMENT_RAY.meets(0x68, 0x11));
        assert!(!TRAP_PLACEMENT_RAY.meets(4, 2));

        let Some(mut resources) = game_resources() else {
            return;
        };
        let worlds = StaticWorld::load_for(
            &mut resources,
            SERVER_PROJECT,
            &[BULLET_RAY, TRAP_PLACEMENT_RAY],
        )
        .unwrap();
        let (bullets, walking) = (&worlds[0], &worlds[1]);
        assert_eq!(
            bullets.triangle_count(),
            StaticWorld::load(&mut resources, SERVER_PROJECT)
                .unwrap()
                .triangle_count()
        );
        assert!(
            walking.triangle_count() > 1_000,
            "{}",
            walking.triangle_count()
        );
        // The floor under a team 1 spawn point is there to walk on.
        let floor = walking
            .ray_cast(
                Vec3::new(-34.0, 1.7, -76.0),
                Vec3::new(0.0, -1.0, 0.0),
                100.0,
            )
            .expect("the base has a floor to walk on");
        assert!((floor.point.y - 0.51).abs() < 0.3, "{floor:?}");
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::combat::resources::tests::game_resources;

    /// `cargo test --release -p match-server ray_cast_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ray_cast_speed() {
        let Some(mut resources) = game_resources() else {
            return;
        };
        let started = std::time::Instant::now();
        let world = StaticWorld::load(&mut resources, SERVER_PROJECT).unwrap();
        println!(
            "loaded {} triangles and {} primitives in {:?}",
            world.triangle_count(),
            world.primitive_count(),
            started.elapsed()
        );
        // Bullet-like segments: from head height at random points, in random
        // directions, 7 m long (an 800 m/s round over one 120 Hz frame).
        let mut seed = 12345u32;
        let mut random = || {
            seed = seed.wrapping_mul(0x0808_8405).wrapping_add(1);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let rays: Vec<(Vec3, Vec3)> = (0..200_000)
            .map(|_| {
                let origin = Vec3::new(random() * 160.0 - 80.0, 1.6, random() * 200.0 - 120.0);
                let dir = Vec3::new(random() - 0.5, (random() - 0.5) * 0.2, random() - 0.5)
                    .normalized_or(Vec3::Y);
                (origin, dir)
            })
            .collect();
        let started = std::time::Instant::now();
        let mut hits = 0;
        for (origin, dir) in &rays {
            if world.ray_cast(*origin, *dir, 7.0).is_some() {
                hits += 1;
            }
        }
        let elapsed = started.elapsed();
        println!(
            "{} rays in {:?}: {:.2} us each, {hits} hits",
            rays.len(),
            elapsed,
            elapsed.as_secs_f64() * 1e6 / rays.len() as f64
        );
    }
}
