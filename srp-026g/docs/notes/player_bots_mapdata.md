# Player bots: map data (0.26g)

The map knowledge server-driven bots need (`player_bots.md` §5), built as
the crate `crates/mapdata`: spawns and objectives, a walk grid with paths,
and line of sight, from the map projects in the game's `resources.db`. Tags
as elsewhere: PROVEN, LIKELY, GUESS. Nothing in the match server uses it
yet.

## 1. Dumping

The game's data never goes into this repository. A server loads one file
per project, written once from a `resources.db`:

```text
cargo run --release -p mapdata --bin mapdata-dump -- --db <resources.db> --out <dir>
cargo run --release -p mapdata --bin mapdata-dump -- --db <resources.db> --out <dir> --map 117 --map 87 --stats 0.25
```

- `--db` may come from `SRP_RESOURCES_DB`.
- No `--map` / `--project`: every playable project (72; the table is
  `survarium::v026g::maps`). `--map <id>` takes a `maps_dict` id,
  `--project level_01/day_4` a project.
- Output: `<dir>/level_01_day_4.mapdata` etc. 7.1-16.3 MB a project,
  817 MB for all 72. All 72 in 6.4 s with a warm disk cache.
- `--stats <cell>` also builds the grid and the sight tree and prints
  counts and timings (§5).

Mode variants of one map place different props (level_01: 1157 / 1159 /
1015 collision objects), so a match loads the file of its own project.

## 2. Facts the crate rests on

- **Frame.** The map frame: left-handed, Y up, Z forward, positions at the
  feet. The physics frame is the map frame with z negated; the uploaded
  worlds' `m_last_pos` is in it, at the capsule's centre 0.9 m above the
  feet. Type `PhysicsPoint` converts. PROVEN (`player_bots.md` §4).
- **Placement.** World point = (local point × the `#[sx][sy][sz]` scale of
  the library name) × `create_rotation(rotation)` + position, row vectors,
  `create_rotation` = Rx·Ry·Rz (unit test). The object's own `scale` is not
  applied. All 43 spawns of level_01/day_4 stand -0.036..+0.120 m above
  the walker mesh this way, those of level_05/evening_2 within 0.15 m
  (tests on the game data). PROVEN.
- **Groups.** The walking body (group 4, mask 2) meets objects with
  `cgroup & 2`, a bullet (16, 8) `cgroup & 8`; an object meets a body when
  `cgroup & mask` and `cmask & group`. PROVEN.
- **Hard links.** Many meshes are hard links in the archive's index. The
  `resources-db` listing resolves a link to its target's payload under the
  link's own path; links to links would be counted in
  `Resources::unresolved_links` (0 in the shipped archive). PROVEN.
- **Walker behaviour** per face: `face_data` names a shading group, the
  model's `settings` give its game material, `game.materials` its
  `physic.walker_behaviour`: 0 ground up to 60°, 1 (`walk`) always ground,
  2 (`dont_walk`) never. Values and slope LIKELY (`player_bots.md` §5).
- **Primitives** (`exported_primitives`: box, sphere, cylinder, capsule)
  are turned into triangles (box exact, round shapes 12 sides × 6 rings),
  with the material of their `mtl`.
- **Body** (`grid::Body`): radius 0.38, stand 1.8, crouch 1.1, step 0.6
  (PROVEN); 60° (LIKELY); drop ≤ 5 m (`fall_params.harmless_height`);
  jump ≤ 1.3 m (GUESS: the engine's numbers give about 1.7 m).
- **Play area.** The walker mesh reaches far off the map (to -666 m); the
  grid covers the `levelmap` x/z box, widened by 5 m and to hold every
  spawn and objective. level_03's box is 2 m and level_06, 07 and 08 have
  none: there the area is the middle 96 % of the collision objects'
  positions with the objectives, widened by a further 15 m (with the
  objectives alone, level_06 mode 3 lost the ground between the bases).

## 3. The file

One per project, little endian; strings `u32 length` + UTF-8, lists
`u32 count` + items, vectors 3 × f32. The libraries are stored once each in
their own space (scale suffix applied), the placements separately; placing
all triangles at load takes about 10 ms.

```text
magic "SRPMAPD\0", u32 version (1)
string project
vec3 levelmap_min, vec3 levelmap_max          (zero when the project has none)
list spawns:      vec3 position, vec3 rotation, i32 team, i32 priority,
                  i32 point_id, string layer
list containers:  vec3 position, i32 team, i32 id, list shapes (use boxes)
list spawners:    vec3 position, vec3 item, string layer
list stations:    vec3
list zones:       u8 kind (0 resupply, 1 damage, 2 effect, 3 artefact
                  container), string name, i32 team (-1 none), vec3 position,
                  string damage_type, list shapes
list anomalies:   string name, vec3 position, list string zones
list libraries:   string name, u32 primitives, list vec3 vertices,
                  list [u32; 3] triangles, then one u8 walker behaviour
                  per triangle
list placements:  u32 library, vec3 position, vec3 rotation,
                  u16 cgroup, u16 cmask
magic "END\0"
shape:            u8 kind (0 sphere, 1 box), vec3 center, 3 × vec3 rotation
                  rows, vec3 half (a sphere's radius is half.x)
```

A shape is placed as: the mesh's rotation and position, then the
object's scale, rotation and position (LIKELY; every shape of the shipped
maps has scale 1). Reading checks every count against the bytes left: a
damaged file is an error, never a panic.

## 4. API

```rust
let map = MapData::read(path)?;                       // or MapData::load(&mut resources, &materials, project)
let world = World::build(map, mapdata::CELL, Body::default());
let node = world.grid.locate(feet, 1.0)?;             // nearest floor within 1 m
let to_flag = world.grid.distances_to(&[goal]);       // one flood for every bot heading there
let (next, how) = to_flag.next_step(&world.grid, node)?;  // how: Walk, Drop or Jump
let path = world.grid.path(a, b)?;                    // A*, nodes and cost
let sees = world.sight.visible(eye_a, chest_b);       // bullet mesh
```

- `MapData`: `spawns`, `containers` (with use boxes), `battery_spawners`,
  `stations`, `zones`, `anomalies`; `walker_triangles()`,
  `bullet_triangles()`, `play_area(margin)`.
- `WalkGrid`: `locate`, `position`, `headroom` / `can_stand` (crouch-only
  floors), `links` / `incoming`, `path`, `distances_to` (cost from every
  floor to the targets) and `distances_from` (where a team gets).
- `RayTree`: `visible(a, b)`, `ray_cast(origin, dir, max)`.

### The walk grid

Port of the prototype, with these changes:
- Cells are half-open: a mesh ending on a cell line stops before it, a wall
  standing on one goes to one column.
- Erosion keeps a floor that is at least the radius (less 1 cm) from the
  near edge of a wall cell: `k` cells from it, `(k - 0.5) × cell`. That is
  1 round at 0.25 m cells; the prototype's `ceil(radius / cell)` = 2 rounds
  shut a 1 m diagonal door on level_01/day_4 and left a spawn in a 4 m²
  pocket.
- A drop or jump whose landing was eroded lands up to the rounds of erosion
  further on (a body dropping beside a wall lands clear of it).
- Paths cost metres plus 0.5 for a drop and 1.5 for a jump (tuning).

**Use 0.25 m cells** (`mapdata::CELL`). 0.5 m cells shut 1 m doors: on
level_02 and level_04 the teams then reach 13-64 of the spawns and A*
finds 0-1 of 25 paths.

## 5. Measured (2026-10-14, release build, one thread)

0.25 m cells, all 72 projects; the day_4 project of each map, and one
battery map:

| project | spawns | file | walker / bullet tris | floors | grid | tree | team floods | A* (5×5 spawns) | ray |
|---|---|---|---|---|---|---|---|---|---|
| level_01/day_4 | 43 | 9.5 MB | 230k / 457k | 852k | 0.45 s, 49 MB | 0.08 s, 25 MB | 57 ms | 38 ms, 1.20× straight | 1.9 µs |
| level_02/day_4 | 31 | 9.4 MB | 153k / 466k | 2,288k | 0.61 s, 122 MB | 0.08 s, 25 MB | 74 ms | 58 ms, 1.22× | 1.3 µs |
| level_03/day_4 | 22 | 9.4 MB | 150k / 561k | 672k | 0.29 s, 38 MB | 0.10 s, 31 MB | 37 ms | 14 ms, 1.14× | 1.8 µs |
| level_04/day_4 | 75 | 14.9 MB | 298k / 814k | 1,260k | 0.49 s, 69 MB | 0.14 s, 46 MB | 55 ms | 47 ms, 1.25× | 2.1 µs |
| level_05/day_4 | 29 | 10.2 MB | 267k / 691k | 228k | 0.27 s, 15 MB | 0.12 s, 42 MB | 16 ms | 5 ms, 1.14× | 1.8 µs |
| level_05/evening_2 | 29 | 10.3 MB | 269k / 699k | 228k | 0.27 s, 15 MB | 0.12 s, 42 MB | 16 ms | none (one-way) | 1.7 µs |
| level_06/day_4 | 30 | 15.9 MB | 633k / 810k | 1,482k | 0.58 s, 80 MB | 0.15 s, 46 MB | 40 ms | 34 ms, 1.41× | 1.3 µs |
| level_07/day_4 | 31 | 12.4 MB | 390k / 1,025k | 339k | 0.24 s, 19 MB | 0.18 s, 54 MB | 23 ms | 15 ms, 1.22× | 2.3 µs |
| level_08/day_4 | 26 | 7.1 MB | 158k / 518k | 1,328k | 0.58 s, 71 MB | 0.09 s, 27 MB | 22 ms | 14 ms, 1.10× | 1.8 µs |

- Over all 72: grid 0.23-0.63 s, tree 0.07-0.19 s, file read 5-16 ms, a
  team's flood 15-81 ms, A* 5-60 ms, a ray (5-150 m, eye to chest) 1.3-2.4
  µs. 20 bots × 19 targets × 2 rays × 10 Hz ≈ 2 % of a core.
- Every spawn stands on a floor (located within 1 m) on all 72.
- Reach (with jumps): on 60 projects each team's spawns reach every spawn.
  On level_01 and level_05 in modes 2 and 3 the bases are one-way (team 0
  of level_01 reaches 30 of 39 spawns; on level_05 neither team reaches
  the other's), as the prototype found for level_05 (players' word:
  plausible).
- level_01/day_4 at 0.25 m: each of the 43 spawns reaches every other one
  (with or without jumps). At 0.5 m: 34 of 43 in one piece without jumps,
  as the prototype; with jumps each team reaches all 43, but 5 spawns sit
  in small pockets (behind narrow doors) that the 0.5 m grid enters and
  cannot leave.
- level_05/evening_2 at 0.25 m: each team reaches all 50 batteries.
- Grid memory is 52 bytes a floor (8 links of 4 bytes dominate); level_02
  needs 122 MB at 0.25 m.

Tests on the game data: `SRP_RESOURCES_DB=<resources.db> cargo test -p
mapdata --release -- --nocapture` (skipped without the variable).

## 6. Not done, not proven

- Nothing in the match server loads these files yet; no bot moves by them.
- The grid against the client's real movement (step, slope, jump height,
  narrow doors, the one-way bases) is not checked in a game.
- Grid memory is high at 0.25 m (up to 122 MB); links could shrink to a
  byte each (index in the neighbour cell) or be recomputed. Not done.
- Files are not compressed (`lzma-rust2` is in the workspace).
- The objectives' reach is only checked for level_05 batteries; a
  container's use box is a trigger shape, not a floor (`Shape::contains`
  tells whether a point is inside).
- The yaw of a spawn (`math::forward`, `math::heading`) follows the
  engine's `create_rotation_y`; how a client's input yaw maps to it is not
  checked.
