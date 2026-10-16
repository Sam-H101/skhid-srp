//! Writes a top view of each map for drawing, from `.mapdata` files:
//!
//! ```text
//! mapdata-layout --out <dir> <file.mapdata>...
//! ```
//!
//! Per project two files:
//! - `<project>.layout`: magic `SRPLAYT\0`, `u32 nx, u32 nz, f32 x0, f32 z0,
//!   f32 cell`, then per cell (z major) `f32 top floor height` (NaN: no
//!   floor), then per cell `u8 floors` (how many floors one above another),
//!   from a walk grid at [`mapdata::CELL`], floors reached from a spawn
//!   only, gathered into cells of `--cell` metres;
//! - `<project>.objects.csv`: `kind,team,id,x,y,z` for spawns, containers,
//!   battery spawners (the battery itself), stations and zones, in the map's
//!   frame.
//!
//! The output is the game's content: keep it out of the repository.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use mapdata::{Body, MapData, World};

#[derive(Parser)]
#[command(about = "Top view of 0.26g maps (floors and objectives) for drawing")]
struct Cli {
    /// Where the files go.
    #[arg(long)]
    out: PathBuf,
    /// Cell of the top view in metres.
    #[arg(long, default_value_t = 0.5)]
    cell: f32,
    /// `.mapdata` files.
    files: Vec<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&cli.out)?;
    for path in &cli.files {
        let map = MapData::read(path)?;
        let project = map.project.replace('/', "_");
        write_objects(&map, &cli.out.join(format!("{project}.objects.csv")))?;
        let world = World::build(map, mapdata::CELL, Body::default());
        let grid = &world.grid;

        let area = world.map.play_area(mapdata::AREA_MARGIN);
        let cell = cli.cell;
        let nx = ((area[2] - area[0]) / cell).ceil().max(1.0) as usize;
        let nz = ((area[3] - area[1]) / cell).ceil().max(1.0) as usize;
        let mut top = vec![f32::NAN; nx * nz];
        let mut floors = vec![0u8; nx * nz];
        // Floors one above another in a fine cell, then the most of them
        // in the coarse cell.
        let mut fine = std::collections::HashMap::<(i32, i32), u8>::new();
        // Only floors a player gets to from some spawn: roofs and the
        // invisible ceilings some maps have (level_02's at 48 m) stay out.
        let spawns: Vec<u32> = world
            .map
            .spawns
            .iter()
            .filter_map(|s| grid.locate(s.position, 1.0))
            .collect();
        let reach = grid.distances_from(&spawns);
        for node in 0..grid.len() as u32 {
            if !reach.reaches(node) {
                continue;
            }
            let p = grid.position(node);
            let i = ((p.x - area[0]) / cell).floor();
            let k = ((p.z - area[1]) / cell).floor();
            if i < 0.0 || k < 0.0 || i as usize >= nx || k as usize >= nz {
                continue;
            }
            let at = k as usize * nx + i as usize;
            if top[at].is_nan() || p.y > top[at] {
                top[at] = p.y;
            }
            let key = (
                ((p.x - area[0]) / grid.cell_size()) as i32,
                ((p.z - area[1]) / grid.cell_size()) as i32,
            );
            let n = fine.entry(key).or_insert(0);
            *n = n.saturating_add(1);
            floors[at] = floors[at].max(*n);
        }

        let file = cli.out.join(format!("{project}.layout"));
        let mut out = BufWriter::new(File::create(&file)?);
        out.write_all(b"SRPLAYT\0")?;
        out.write_all(&(nx as u32).to_le_bytes())?;
        out.write_all(&(nz as u32).to_le_bytes())?;
        out.write_all(&area[0].to_le_bytes())?;
        out.write_all(&area[1].to_le_bytes())?;
        out.write_all(&cell.to_le_bytes())?;
        for h in &top {
            out.write_all(&h.to_le_bytes())?;
        }
        out.write_all(&floors)?;
        out.flush()?;
        println!(
            "{}: {nx}x{nz} cells of {cell} m, {} floors",
            world.map.project,
            grid.len()
        );
    }
    Ok(())
}

fn write_objects(map: &MapData, path: &std::path::Path) -> std::io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    writeln!(out, "kind,team,id,x,y,z")?;
    for s in &map.spawns {
        let p = s.position;
        writeln!(out, "spawn,{},{},{},{},{}", s.team, s.point_id, p.x, p.y, p.z)?;
    }
    for c in &map.containers {
        let p = c.position;
        writeln!(out, "container,{},{},{},{},{}", c.team, c.id, p.x, p.y, p.z)?;
    }
    for (i, s) in map.battery_spawners.iter().enumerate() {
        let p = s.item;
        writeln!(out, "battery,-1,{i},{},{},{}", p.x, p.y, p.z)?;
    }
    for (i, p) in map.stations.iter().enumerate() {
        writeln!(out, "station,-1,{i},{},{},{}", p.x, p.y, p.z)?;
    }
    for (i, z) in map.zones.iter().enumerate() {
        let p = z.position;
        let kind = format!("{:?}", z.kind).to_lowercase();
        writeln!(out, "zone_{kind},{},{i},{},{},{}", z.team, p.x, p.y, p.z)?;
    }
    out.flush()
}
