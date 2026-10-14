//! The maps the server's bots path on (`SRP_MAPDATA_DIR`,
//! `docs/notes/player_bots_mapdata.md` §6): one [`mapdata::World`] per
//! project, built from `<dir>/<project>.mapdata` (the file `mapdata-dump`
//! writes) on a thread of its own. A build takes 0.3-0.8 s and the match
//! loop is one thread, so a caller never waits: it asks early
//! ([`Maps::request`]) and looks again each tick ([`Maps::get`]) until the
//! world is [`MapState::Ready`].
//!
//! A world holds 63-149 MB (a build peaks near twice that) and the 0.26g
//! host has under 1 GB, so the store keeps only the last [`KEEP`] used
//! (`SRP_MAPDATA_KEEP`) and builds one world at a time. A match holds its
//! world by its `Arc`; one dropped from the store while a match still has
//! it is taken back without a new build. A file that is missing or fails
//! to load is reported once and stays absent until the server restarts.

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Instant;

use mapdata::{Body, MapData, World};

/// How many built worlds the store keeps by default, beyond those running
/// matches hold. With 72 projects the next match is rarely on the last
/// map, and a rebuild costs 0.3-0.8 s off the match thread.
pub const KEEP: usize = 1;

/// Where one project's world stands.
#[derive(Clone)]
pub enum MapState {
    /// No world for this project: no directory set, no file, or the load
    /// failed. The reason is for the log.
    Absent(String),
    /// Being built; look again later.
    Loading,
    Ready(Arc<World>),
}

impl fmt::Debug for MapState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent(reason) => f.debug_tuple("Absent").field(reason).finish(),
            Self::Loading => f.write_str("Loading"),
            Self::Ready(world) => write!(f, "Ready({})", world.map.project),
        }
    }
}

/// The store of built worlds, shared by every match of the server.
pub struct Maps {
    dir: Option<PathBuf>,
    keep: usize,
    cache: Arc<Mutex<Cache>>,
    /// Held by the build under way: one at a time, for memory and the one
    /// core of the host.
    builds: Arc<Mutex<()>>,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    /// Counts uses, for the least recently used.
    clock: u64,
}

enum Entry {
    Loading,
    Ready {
        world: Arc<World>,
        used: u64,
    },
    /// Dropped from the store; a match may still hold it.
    Dropped(Weak<World>),
    Failed(String),
}

impl Maps {
    /// A store loading from `dir`; `None`: every project is absent.
    pub fn new(dir: Option<PathBuf>) -> Arc<Self> {
        Self::with_keep(dir, KEEP)
    }

    /// As [`Maps::new`], keeping `keep` worlds (at least one).
    pub fn with_keep(dir: Option<PathBuf>, keep: usize) -> Arc<Self> {
        Arc::new(Self {
            dir,
            keep: keep.max(1),
            cache: Arc::default(),
            builds: Arc::default(),
        })
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Starts building `project`'s world (`level_01/day_4`) unless it is
    /// built, being built or known to be absent.
    pub fn request(&self, project: &str) {
        self.get(project);
    }

    /// Where `project`'s world stands; starts building it as
    /// [`Maps::request`] does. Never blocks on a build.
    pub fn get(&self, project: &str) -> MapState {
        let Some(dir) = &self.dir else {
            return MapState::Absent("SRP_MAPDATA_DIR is not set".into());
        };
        let mut cache = lock(&self.cache);
        cache.clock += 1;
        let clock = cache.clock;
        let revived = match cache.entries.get_mut(project) {
            Some(Entry::Loading) => return MapState::Loading,
            Some(Entry::Failed(reason)) => return MapState::Absent(reason.clone()),
            Some(Entry::Ready { world, used }) => {
                *used = clock;
                return MapState::Ready(world.clone());
            }
            Some(Entry::Dropped(world)) => world.upgrade(),
            None => None,
        };
        if let Some(world) = revived {
            cache.insert_ready(project, world.clone(), self.keep);
            return MapState::Ready(world);
        }
        let path = dir.join(mapdata::format::file_name(project));
        if !path.is_file() {
            let reason = format!("no map file {}", path.display());
            log::warn!("Bot map {project}: {reason}; bots on it keep their script");
            cache
                .entries
                .insert(project.to_owned(), Entry::Failed(reason.clone()));
            return MapState::Absent(reason);
        }
        cache.entries.insert(project.to_owned(), Entry::Loading);
        drop(cache);
        self.spawn_build(project, path);
        MapState::Loading
    }

    fn spawn_build(&self, project: &str, path: PathBuf) {
        let (cache, builds) = (self.cache.clone(), self.builds.clone());
        let (keep, name) = (self.keep, project.to_owned());
        let spawned = std::thread::Builder::new()
            .name(format!("map {project}"))
            .spawn(move || {
                let result = {
                    let _turn = builds.lock().unwrap_or_else(|p| p.into_inner());
                    build(&path)
                };
                let mut cache = lock(&cache);
                match result {
                    Ok((world, seconds)) => {
                        log::info!(
                            "Bot map {name} ready in {seconds:.2} s, {} MB",
                            world_bytes(&world) >> 20
                        );
                        cache.insert_ready(&name, Arc::new(world), keep);
                    }
                    Err(reason) => {
                        log::warn!("Bot map {name}: {reason}; bots on it keep their script");
                        cache.entries.insert(name, Entry::Failed(reason));
                    }
                }
            });
        if let Err(error) = spawned {
            let reason = format!("can't start its build: {error}");
            log::warn!("Bot map {project}: {reason}");
            lock(&self.cache)
                .entries
                .insert(project.to_owned(), Entry::Failed(reason));
        }
    }
}

impl Cache {
    /// Puts `world` in as the latest used, and drops the least recently
    /// used ready worlds over `keep`.
    fn insert_ready(&mut self, project: &str, world: Arc<World>, keep: usize) {
        self.clock += 1;
        let used = self.clock;
        self.entries
            .insert(project.to_owned(), Entry::Ready { world, used });
        loop {
            let ready = self.entries.iter().filter_map(|(name, entry)| match entry {
                Entry::Ready { used, .. } => Some((*used, name)),
                _ => None,
            });
            if ready.clone().count() <= keep {
                break;
            }
            let Some((_, oldest)) = ready.min() else {
                break;
            };
            let oldest = oldest.clone();
            if let Some(Entry::Ready { world, .. }) = self.entries.remove(&oldest) {
                log::info!("Bot map {oldest} dropped from the store (kept {keep})");
                self.entries
                    .insert(oldest, Entry::Dropped(Arc::downgrade(&world)));
            }
        }
    }
}

/// Reads and builds one world: the world and the seconds it took.
fn build(path: &Path) -> Result<(World, f32), String> {
    let started = Instant::now();
    let map = MapData::read(path).map_err(|error| error.to_string())?;
    let world = catch_unwind(AssertUnwindSafe(|| {
        World::build(map, mapdata::CELL, Body::default())
    }))
    .map_err(|_| format!("{}: the build panicked", path.display()))?;
    Ok((world, started.elapsed().as_secs_f32()))
}

/// Bytes a built world holds: the grid, the sight tree and the map's
/// meshes (the objectives are a few kB).
pub fn world_bytes(world: &World) -> usize {
    let meshes: usize = world
        .map
        .libraries
        .iter()
        .map(|library| {
            library.name.len()
                + library.vertices.len() * size_of::<mapdata::Vec3>()
                + library.triangles.len() * size_of::<[u32; 3]>()
                + library.walker.len() * size_of::<mapdata::WalkerBehaviour>()
        })
        .sum();
    world.grid.memory_bytes()
        + world.sight.memory_bytes()
        + meshes
        + world.map.placements.len() * size_of::<mapdata::Placement>()
}

fn lock(cache: &Mutex<Cache>) -> MutexGuard<'_, Cache> {
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl fmt::Debug for Maps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cache = lock(&self.cache);
        let mut ready: Vec<&str> = cache
            .entries
            .iter()
            .filter(|(_, entry)| matches!(entry, Entry::Ready { .. }))
            .map(|(name, _)| name.as_str())
            .collect();
        ready.sort_unstable();
        f.debug_struct("Maps")
            .field("dir", &self.dir)
            .field("keep", &self.keep)
            .field("ready", &ready)
            .finish()
    }
}

/// One store equals only itself (`Settings` compares its fields).
impl PartialEq for Maps {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for Maps {}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mapdata::{Library, Placement, Spawn, Vec3, WalkerBehaviour};

    use super::*;

    /// A 20 × 20 m floor that walkers and bullets both meet, with a spawn
    /// for each team.
    fn tiny_map(project: &str) -> MapData {
        let floor = Library {
            name: "floor".into(),
            vertices: vec![
                Vec3::new(-10.0, 0.0, -10.0),
                Vec3::new(10.0, 0.0, -10.0),
                Vec3::new(10.0, 0.0, 10.0),
                Vec3::new(-10.0, 0.0, 10.0),
            ],
            triangles: vec![[0, 2, 1], [0, 3, 2]],
            walker: vec![WalkerBehaviour::Walk; 2],
            primitives: 0,
        };
        let spawn = |team: i32, x: f32| Spawn {
            position: Vec3::new(x, 0.0, 0.0),
            rotation: Vec3::ZERO,
            team,
            priority: 0,
            point_id: team,
            layer: "default".into(),
        };
        MapData {
            project: project.into(),
            levelmap_min: Vec3::new(-10.0, 0.0, -10.0),
            levelmap_max: Vec3::new(10.0, 1.0, 10.0),
            spawns: vec![spawn(0, -5.0), spawn(1, 5.0)],
            libraries: vec![floor],
            placements: vec![Placement {
                library: 0,
                position: Vec3::ZERO,
                rotation: Vec3::ZERO,
                cgroup: 10,
                cmask: 20,
            }],
            ..MapData::default()
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("srp-maps-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait_ready(maps: &Maps, project: &str, limit: Duration) -> Arc<World> {
        let started = Instant::now();
        loop {
            match maps.get(project) {
                MapState::Ready(world) => return world,
                MapState::Loading if started.elapsed() < limit => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                other => panic!("{project}: {other:?} after {:?}", started.elapsed()),
            }
        }
    }

    #[test]
    fn no_directory_means_no_map() {
        let maps = Maps::new(None);
        maps.request("level_01/day_4");
        assert!(matches!(maps.get("level_01/day_4"), MapState::Absent(_)));
    }

    #[test]
    fn a_missing_file_is_absent_with_its_name() {
        let dir = temp_dir("missing");
        let maps = Maps::new(Some(dir.clone()));
        let MapState::Absent(reason) = maps.get("level_09/day_4") else {
            panic!("a missing file must be absent");
        };
        assert!(reason.contains("level_09_day_4.mapdata"), "{reason}");
        // Remembered: still absent, nothing started.
        assert!(matches!(maps.get("level_09/day_4"), MapState::Absent(_)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_file_is_absent() {
        let dir = temp_dir("damaged");
        std::fs::write(dir.join("broken_day_4.mapdata"), b"SRPMAPD\0nonsense").unwrap();
        let maps = Maps::new(Some(dir.clone()));
        let started = Instant::now();
        let reason = loop {
            match maps.get("broken/day_4") {
                MapState::Absent(reason) => break reason,
                MapState::Loading if started.elapsed() < Duration::from_secs(10) => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                other => panic!("{other:?}"),
            }
        };
        assert!(reason.contains("broken_day_4.mapdata"), "{reason}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_map_loads_in_the_background_and_is_shared() {
        let dir = temp_dir("tiny");
        let project = "tiny/day_4";
        tiny_map(project)
            .write(&dir.join(mapdata::format::file_name(project)))
            .unwrap();
        let maps = Maps::new(Some(dir.clone()));
        maps.request(project);
        // The build is on its own thread: the first look finds it loading
        // or (on a fast machine, rarely) done.
        assert!(matches!(
            maps.get(project),
            MapState::Loading | MapState::Ready(_)
        ));
        let world = wait_ready(&maps, project, Duration::from_secs(10));
        assert_eq!(world.map.project, project);
        let a = world.grid.locate(Vec3::new(-5.0, 0.0, 0.0), 1.0).unwrap();
        let b = world.grid.locate(Vec3::new(5.0, 0.0, 0.0), 1.0).unwrap();
        assert!(world.grid.path(a, b).is_some());
        let MapState::Ready(again) = maps.get(project) else {
            panic!("a built world stays ready");
        };
        assert!(Arc::ptr_eq(&world, &again));
        assert!(format!("{maps:?}").contains(project));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_least_recently_used_world_is_dropped_but_kept_while_held() {
        let dir = temp_dir("lru");
        let projects = ["a/day_4", "b/day_4", "c/day_4"];
        for project in projects {
            tiny_map(project)
                .write(&dir.join(mapdata::format::file_name(project)))
                .unwrap();
        }
        let maps = Maps::with_keep(Some(dir.clone()), 2);
        let wait = Duration::from_secs(10);
        let a = wait_ready(&maps, "a/day_4", wait);
        let b = wait_ready(&maps, "b/day_4", wait);
        drop(b);
        // a used again after b: b is the oldest when c comes.
        wait_ready(&maps, "a/day_4", wait);
        wait_ready(&maps, "c/day_4", wait);
        let cache = lock(&maps.cache);
        assert!(matches!(cache.entries["a/day_4"], Entry::Ready { .. }));
        assert!(matches!(cache.entries["c/day_4"], Entry::Ready { .. }));
        let Entry::Dropped(b) = &cache.entries["b/day_4"] else {
            panic!("b must be dropped");
        };
        assert!(b.upgrade().is_none(), "nothing holds b");
        drop(cache);
        // a dropped while held comes back without a build.
        wait_ready(&maps, "b/day_4", wait);
        let cache = lock(&maps.cache);
        let Entry::Dropped(dropped) = &cache.entries["a/day_4"] else {
            panic!("a must be dropped now");
        };
        assert!(Arc::ptr_eq(&dropped.upgrade().unwrap(), &a));
        drop(cache);
        let MapState::Ready(back) = maps.get("a/day_4") else {
            panic!("a is held, so it is ready at once");
        };
        assert!(Arc::ptr_eq(&back, &a));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The directory of real maps, if the environment names one.
    fn real_dir() -> Option<PathBuf> {
        let dir = std::env::var_os("SRP_MAPDATA_DIR").map(PathBuf::from);
        if dir.is_none() {
            eprintln!("skipping: SRP_MAPDATA_DIR is not set");
        }
        dir
    }

    /// One real map, only with `SRP_MAPDATA_DIR=<dir of mapdata-dump files>`.
    #[test]
    fn a_real_map_loads() {
        let Some(dir) = real_dir() else { return };
        let maps = Maps::new(Some(dir));
        let world = wait_ready(&maps, "level_01/day_4", Duration::from_secs(300));
        assert_eq!(world.map.project, "level_01/day_4");
        for spawn in &world.map.spawns {
            assert!(world.grid.locate(spawn.position, 1.0).is_some());
        }
    }

    /// Build time and memory of a few real maps (the numbers in the notes):
    /// `SRP_MAPDATA_DIR=<dir> cargo test -p match-server --release real_map_sizes
    /// -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement; needs SRP_MAPDATA_DIR"]
    fn real_map_sizes() {
        let Some(dir) = real_dir() else { return };
        let maps = Maps::with_keep(Some(dir), 1);
        for project in [
            "level_01/day_4",
            "level_02/day_4",
            "level_03/day_4",
            "level_04/day_4",
            "level_05/day_4",
            "level_06/day_4",
            "level_07/day_4",
            "level_08/day_4",
        ] {
            let started = Instant::now();
            let world = wait_ready(&maps, project, Duration::from_secs(300));
            let spawns = world.map.spawns.len();
            let located = world
                .map
                .spawns
                .iter()
                .filter(|s| world.grid.locate(s.position, 1.0).is_some())
                .count();
            eprintln!(
                "{project}: ready in {:.2} s; grid {} MB, tree {} MB, world {} MB; \
                 {located}/{spawns} spawns on a floor",
                started.elapsed().as_secs_f32(),
                world.grid.memory_bytes() >> 20,
                world.sight.memory_bytes() >> 20,
                world_bytes(&world) >> 20,
            );
            assert_eq!(located, spawns);
        }
    }
}
