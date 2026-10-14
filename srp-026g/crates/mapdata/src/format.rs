//! The file `mapdata-dump` writes, one per project: everything in
//! [`MapData`], so a server loads maps without the game's `resources.db`.
//!
//! Little endian. A string is `u32 length` + UTF-8; a list is `u32 count` +
//! items; a vector is three `f32`.
//!
//! ```text
//! magic "SRPMAPD\0", u32 version (1)
//! string project
//! vec3 levelmap_min, vec3 levelmap_max
//! list spawns:      vec3 position, vec3 rotation, i32 team, i32 priority,
//!                   i32 point_id, string layer
//! list containers:  vec3 position, i32 team, i32 id, list shapes
//! list spawners:    vec3 position, vec3 item, string layer
//! list stations:    vec3
//! list zones:       u8 kind, string name, i32 team, vec3 position,
//!                   string damage_type, list shapes
//! list anomalies:   string name, vec3 position, list string zones
//! list libraries:   string name, u32 primitives, list vec3 vertices,
//!                   list [u32; 3] triangles, then one u8 walker behaviour
//!                   per triangle
//! list placements:  u32 library, vec3 position, vec3 rotation,
//!                   u16 cgroup, u16 cmask
//! magic "END\0"
//! shape:            u8 kind, vec3 center, 3 × vec3 rotation rows, vec3 half
//! ```
//!
//! Reading checks every count against the bytes left, so a damaged file is
//! an error, never a panic or a huge allocation.

use std::path::Path;

use crate::library::{Library, WalkerBehaviour};
use crate::math::{Rotation, Vec3};
use crate::project::{
    Anomaly, BatterySpawner, Container, MapData, Placement, Shape, ShapeKind, Spawn, Zone, ZoneKind,
};
use crate::resources::Error;

const MAGIC: &[u8; 8] = b"SRPMAPD\0";
const END: &[u8; 4] = b"END\0";
pub const VERSION: u32 = 1;
/// The file name extension.
pub const EXTENSION: &str = "mapdata";

/// The file name for a project: `level_01/day_4` → `level_01_day_4.mapdata`.
pub fn file_name(project: &str) -> String {
    format!("{}.{EXTENSION}", project.replace(['/', '\\'], "_"))
}

impl MapData {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer(Vec::with_capacity(1 << 20));
        w.0.extend_from_slice(MAGIC);
        w.u32(VERSION);
        w.str(&self.project);
        w.vec3(self.levelmap_min);
        w.vec3(self.levelmap_max);
        w.list(&self.spawns, |w, s| {
            w.vec3(s.position);
            w.vec3(s.rotation);
            w.i32(s.team);
            w.i32(s.priority);
            w.i32(s.point_id);
            w.str(&s.layer);
        });
        w.list(&self.containers, |w, c| {
            w.vec3(c.position);
            w.i32(c.team);
            w.i32(c.id);
            w.shapes(&c.use_shapes);
        });
        w.list(&self.battery_spawners, |w, s| {
            w.vec3(s.position);
            w.vec3(s.item);
            w.str(&s.layer);
        });
        w.list(&self.stations, |w, &s| w.vec3(s));
        w.list(&self.zones, |w, z| {
            w.0.push(z.kind as u8);
            w.str(&z.name);
            w.i32(z.team);
            w.vec3(z.position);
            w.str(&z.damage_type);
            w.shapes(&z.shapes);
        });
        w.list(&self.anomalies, |w, a| {
            w.str(&a.name);
            w.vec3(a.position);
            w.list(&a.zones, |w, z| w.str(z));
        });
        w.list(&self.libraries, |w, l| {
            w.str(&l.name);
            w.u32(l.primitives);
            w.list(&l.vertices, |w, &v| w.vec3(v));
            w.list(&l.triangles, |w, t| t.iter().for_each(|&i| w.u32(i)));
            w.0.extend(l.walker.iter().map(|&b| b as u8));
        });
        w.list(&self.placements, |w, p| {
            w.u32(p.library);
            w.vec3(p.position);
            w.vec3(p.rotation);
            w.0.extend_from_slice(&p.cgroup.to_le_bytes());
            w.0.extend_from_slice(&p.cmask.to_le_bytes());
        });
        w.0.extend_from_slice(END);
        w.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(8)? != MAGIC {
            return Err(Error("not a map data file".into()));
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(Error(format!(
                "map data version {version}, this build reads {VERSION}"
            )));
        }
        let mut map = MapData {
            project: r.str()?,
            levelmap_min: r.vec3()?,
            levelmap_max: r.vec3()?,
            ..MapData::default()
        };
        map.spawns = r.list(40, |r| {
            Ok(Spawn {
                position: r.vec3()?,
                rotation: r.vec3()?,
                team: r.i32()?,
                priority: r.i32()?,
                point_id: r.i32()?,
                layer: r.str()?,
            })
        })?;
        map.containers = r.list(24, |r| {
            Ok(Container {
                position: r.vec3()?,
                team: r.i32()?,
                id: r.i32()?,
                use_shapes: r.shapes()?,
            })
        })?;
        map.battery_spawners = r.list(28, |r| {
            Ok(BatterySpawner {
                position: r.vec3()?,
                item: r.vec3()?,
                layer: r.str()?,
            })
        })?;
        map.stations = r.list(12, Reader::vec3)?;
        map.zones = r.list(29, |r| {
            let kind = r.u8()?;
            Ok(Zone {
                kind: ZoneKind::from_u8(kind)
                    .ok_or_else(|| Error(format!("unknown zone kind {kind}")))?,
                name: r.str()?,
                team: r.i32()?,
                position: r.vec3()?,
                damage_type: r.str()?,
                shapes: r.shapes()?,
            })
        })?;
        map.anomalies = r.list(20, |r| {
            Ok(Anomaly {
                name: r.str()?,
                position: r.vec3()?,
                zones: r.list(4, Reader::str)?,
            })
        })?;
        map.libraries = r.list(16, |r| {
            let name = r.str()?;
            let primitives = r.u32()?;
            let vertices = r.list(12, Reader::vec3)?;
            let triangles = r.list(12, |r| Ok([r.u32()?, r.u32()?, r.u32()?]))?;
            if triangles
                .iter()
                .flatten()
                .any(|&i| i as usize >= vertices.len())
            {
                return Err(Error(format!("{name}: a triangle names no vertex")));
            }
            let walker = r
                .take(triangles.len())?
                .iter()
                .map(|&b| WalkerBehaviour::from_u8(b))
                .collect();
            Ok(Library {
                name,
                vertices,
                triangles,
                walker,
                primitives,
            })
        })?;
        map.placements = r.list(32, |r| {
            Ok(Placement {
                library: r.u32()?,
                position: r.vec3()?,
                rotation: r.vec3()?,
                cgroup: r.u16()?,
                cmask: r.u16()?,
            })
        })?;
        if map
            .placements
            .iter()
            .any(|p| p.library as usize >= map.libraries.len())
        {
            return Err(Error("a placement names no library".into()));
        }
        if r.take(4)? != END || r.at != bytes.len() {
            return Err(Error("map data file has a bad end".into()));
        }
        Ok(map)
    }

    pub fn write(&self, path: &Path) -> Result<(), Error> {
        std::fs::write(path, self.to_bytes())
            .map_err(|error| Error(format!("{}: {error}", path.display())))
    }

    pub fn read(path: &Path) -> Result<Self, Error> {
        let bytes =
            std::fs::read(path).map_err(|error| Error(format!("{}: {error}", path.display())))?;
        Self::from_bytes(&bytes).map_err(|error| Error(format!("{}: {error}", path.display())))
    }
}

struct Writer(Vec<u8>);

impl Writer {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }

    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }

    fn list<T>(&mut self, items: &[T], mut each: impl FnMut(&mut Self, &T)) {
        self.u32(items.len() as u32);
        for item in items {
            each(self, item);
        }
    }

    fn shapes(&mut self, shapes: &[Shape]) {
        self.list(shapes, |w, s| {
            w.0.push(s.kind as u8);
            w.vec3(s.center);
            w.vec3(s.rotation.i);
            w.vec3(s.rotation.j);
            w.vec3(s.rotation.k);
            w.vec3(s.half);
        });
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self
            .at
            .checked_add(n)
            .filter(|&end| end <= self.bytes.len())
            .ok_or_else(|| Error(format!("map data file ends early (at {})", self.at)))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32, Error> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn vec3(&mut self) -> Result<Vec3, Error> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }

    fn str(&mut self) -> Result<String, Error> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| Error("map data file has a bad string".into()))
    }

    /// A list whose items take at least `min_size` bytes each.
    fn list<T>(
        &mut self,
        min_size: usize,
        mut each: impl FnMut(&mut Self) -> Result<T, Error>,
    ) -> Result<Vec<T>, Error> {
        let n = self.u32()? as usize;
        if n.saturating_mul(min_size) > self.bytes.len() - self.at {
            return Err(Error(format!(
                "map data list of {n} is longer than the file"
            )));
        }
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(each(self)?);
        }
        Ok(out)
    }

    fn shapes(&mut self) -> Result<Vec<Shape>, Error> {
        self.list(61, |r| {
            let kind = match r.u8()? {
                0 => ShapeKind::Sphere,
                1 => ShapeKind::Box,
                other => return Err(Error(format!("unknown shape kind {other}"))),
            };
            Ok(Shape {
                kind,
                center: r.vec3()?,
                rotation: Rotation {
                    i: r.vec3()?,
                    j: r.vec3()?,
                    k: r.vec3()?,
                },
                half: r.vec3()?,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MapData {
        let shape = Shape {
            kind: ShapeKind::Box,
            center: Vec3::new(1.0, 2.0, 3.0),
            rotation: Rotation::from_angles(Vec3::new(0.0, 0.5, 0.0)),
            half: Vec3::new(0.5, 1.0, 0.5),
        };
        MapData {
            project: "level_01/day_4".into(),
            levelmap_min: Vec3::new(-1.0, 0.0, -2.0),
            levelmap_max: Vec3::new(10.0, 1.0, 20.0),
            spawns: vec![Spawn {
                position: Vec3::new(1.0, 2.0, 3.0),
                rotation: Vec3::new(0.0, 1.0, 0.0),
                team: 1,
                priority: 2,
                point_id: 3,
                layer: "default".into(),
            }],
            containers: vec![Container {
                position: Vec3::ZERO,
                team: 0,
                id: 4,
                use_shapes: vec![shape],
            }],
            battery_spawners: vec![BatterySpawner {
                position: Vec3::Y,
                item: Vec3::ONE,
                layer: "mode_2".into(),
            }],
            stations: vec![Vec3::new(5.0, 0.0, 5.0)],
            zones: vec![Zone {
                kind: ZoneKind::Damage,
                name: "logic/zone".into(),
                team: -1,
                position: Vec3::ONE,
                damage_type: "toxic".into(),
                shapes: vec![shape, shape],
            }],
            anomalies: vec![Anomaly {
                name: "core".into(),
                position: Vec3::ZERO,
                zones: vec!["logic/zone".into()],
            }],
            libraries: vec![Library {
                name: "models/a.model/walker_collision#[2][2][2]".into(),
                vertices: vec![Vec3::ZERO, Vec3::Y, Vec3::ONE],
                triangles: vec![[0, 1, 2]],
                walker: vec![WalkerBehaviour::DontWalk],
                primitives: 1,
            }],
            placements: vec![Placement {
                library: 0,
                position: Vec3::new(3.0, 4.0, 5.0),
                rotation: Vec3::new(0.1, 0.2, 0.3),
                cgroup: 10,
                cmask: 20,
            }],
        }
    }

    #[test]
    fn round_trips() {
        let map = sample();
        let bytes = map.to_bytes();
        assert_eq!(MapData::from_bytes(&bytes).unwrap(), map);
    }

    #[test]
    fn damaged_files_are_errors() {
        let bytes = sample().to_bytes();
        for len in 0..bytes.len() {
            assert!(MapData::from_bytes(&bytes[..len]).is_err());
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(MapData::from_bytes(&longer).is_err());
        // A placement naming library 7.
        let mut bad = sample();
        bad.placements[0].library = 7;
        assert!(MapData::from_bytes(&bad.to_bytes()).is_err());
        // Every single flipped byte: an error or a file, never a panic.
        for at in 0..bytes.len() {
            let mut flipped = bytes.clone();
            flipped[at] ^= 0xFF;
            let _ = MapData::from_bytes(&flipped);
        }
    }

    #[test]
    fn names_files_by_project() {
        assert_eq!(file_name("level_01/day_4"), "level_01_day_4.mapdata");
    }
}
