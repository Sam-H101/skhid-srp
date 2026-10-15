// SPDX-License-Identifier: GPL-3.0-or-later
//! `srp-game-data`: the server's game data ([`resources_db::server`]), the
//! files `srp` reads of `resources.db` with its patches over it, as the
//! players' game mounts them, and what it was made of: `resources.db`, and
//! each patch with the URL the players download it from.
//!
//! ```text
//! srp-game-data build <archives> --out <file>
//! srp-game-data info <file>
//! srp-game-data check <file> <archives>
//!
//! <archives>: --dir <game folder> [--patch-url <base URL>]
//!           | --db <resources.db> [--patch <file>[=<URL>]]... [--patch-url <base URL>]
//! ```
//!
//! See [`USAGE`].

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use resources_db::Archive;
use resources_db::server::{
    Data, PatchFile, Provenance, archives_in, build, provenance_of, read, url_in,
};

const USAGE: &str = "usage:
  srp-game-data build <archives> --out <file>
  srp-game-data info <file>
  srp-game-data check <file> <archives>

<archives> is one of
  --dir <game folder> [--patch-url <base URL>]
  --db <resources.db> [--patch <file>[=<URL>]]... [--patch-url <base URL>]

--dir takes what the game mounts from that folder: resources.db, then every
resources.db.patch_<n> there, n from 1 to 255 (a missing n is skipped, other
names are not mounted). --db and --patch name the files; a patch must be
called resources.db.patch_<n>, and the patches are laid over resources.db in
ascending n, as the game mounts them, whatever their order here.

Every patch needs the URL the players' updater downloads it from (http:// or
https://): <file>=<URL>, or --patch-url <base URL>, which gives a patch
without one the URL <base URL>/<its file name>.";

/// The archives the options name: `resources.db`, its patches with their
/// URLs; and `--out`.
fn archives(options: &[String]) -> Result<(PathBuf, Vec<PatchFile>, Option<PathBuf>), String> {
    let mut dir = None;
    let mut db = None;
    let mut base = None;
    let mut out = None;
    let mut patches: Vec<(PathBuf, Option<String>)> = Vec::new();
    let mut rest = options.iter();
    while let Some(key) = rest.next() {
        let value = rest
            .next()
            .ok_or_else(|| format!("{key} needs a value\n{USAGE}"))?;
        let once = |slot: &mut Option<String>| match slot.replace(value.clone()) {
            None => Ok(()),
            Some(_) => Err(format!("{key} is given twice")),
        };
        match key.as_str() {
            "--dir" => once(&mut dir)?,
            "--db" => once(&mut db)?,
            "--patch-url" => once(&mut base)?,
            "--out" => once(&mut out)?,
            "--patch" => patches.push(match value.split_once('=') {
                Some((path, url)) => (path.into(), Some(url.to_owned())),
                None => (value.into(), None),
            }),
            _ => return Err(format!("unknown option {key}\n{USAGE}")),
        }
    }
    let db = match (dir, db) {
        (Some(dir), None) if patches.is_empty() => {
            let (db, found) = archives_in(Path::new(&dir)).map_err(|e| e.to_string())?;
            patches = found.into_iter().map(|path| (path, None)).collect();
            db
        }
        (None, Some(db)) => PathBuf::from(db),
        _ => return Err(format!("either --dir, or --db with any --patch\n{USAGE}")),
    };
    let patches = patches
        .into_iter()
        .map(|(path, url)| {
            let url = match (url, &base) {
                (Some(url), _) => url,
                (None, Some(base)) => {
                    let name = path.file_name().and_then(|name| name.to_str());
                    url_in(base, name.unwrap_or_default())
                }
                (None, None) => {
                    return Err(format!(
                        "{}: no URL (<file>=<URL> or --patch-url)",
                        path.display()
                    ));
                }
            };
            Ok(PatchFile { path, url })
        })
        .collect::<Result<_, String>>()?;
    Ok((db, patches, out.map(PathBuf::from)))
}

fn open(data: &Path) -> Result<Data, String> {
    let file = std::fs::File::open(data).map_err(|cause| format!("{}: {cause}", data.display()))?;
    let mut archive = Archive::new(file).map_err(|e| format!("{}: {e}", data.display()))?;
    read(&mut archive).map_err(|e| format!("{}: {e}", data.display()))
}

fn print(provenance: &Provenance) {
    let db = &provenance.db;
    println!("  {} {} bytes sha256 {}", db.name, db.size, db.sha256);
    for patch in &provenance.patches {
        let input = &patch.input;
        println!(
            "  {} {} bytes sha256 {} url {}",
            input.name, input.size, input.sha256, patch.url
        );
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (command, options) = args.split_first().ok_or(USAGE)?;
    match command.as_str() {
        "build" => {
            let (db, patches, out) = archives(options)?;
            let out = out.ok_or_else(|| format!("build needs --out\n{USAGE}"))?;
            let built = build(&db, &patches).map_err(|e| e.to_string())?;
            let names = std::iter::once(&built.provenance.db)
                .chain(built.provenance.patches.iter().map(|patch| &patch.input));
            for (input, given) in names.zip(&built.given) {
                println!(
                    "{}: {} of the server's files, {} of them over a file below",
                    input.name, given.files, given.replaced
                );
            }
            let mut temporary = out.clone().into_os_string();
            temporary.push(".part");
            std::fs::write(&temporary, &built.bytes)
                .and_then(|()| std::fs::rename(&temporary, &out))
                .map_err(|cause| format!("{}: {cause}", out.display()))?;
            let made = resources_db::server::Input::of(&out).map_err(|e| e.to_string())?;
            println!(
                "{}: {} bytes, sha256 {}, {} gameplay files, {} projects, made of:",
                out.display(),
                made.size,
                made.sha256,
                built.gameplay,
                built.projects,
            );
            print(&built.provenance);
            Ok(())
        }
        "info" => {
            let [data] = options else {
                return Err(USAGE.into());
            };
            let data = PathBuf::from(data);
            let found = open(&data)?;
            println!(
                "{}: {} gameplay files, {} projects, made of:",
                data.display(),
                found.gameplay,
                found.projects
            );
            print(&found.provenance);
            Ok(())
        }
        "check" => {
            let (data, options) = options.split_first().ok_or(USAGE)?;
            let data = PathBuf::from(data);
            let (db, patches, out) = archives(options)?;
            if out.is_some() {
                return Err(format!("check takes no --out\n{USAGE}"));
            }
            let found = open(&data)?;
            let given = provenance_of(&db, &patches).map_err(|e| e.to_string())?;
            if found.provenance == given {
                println!("{}: made of these, as given:", data.display());
                print(&given);
                Ok(())
            } else {
                println!("{} was made of:", data.display());
                print(&found.provenance);
                println!("what is given:");
                print(&given);
                Err(format!("{} was made of something else", data.display()))
            }
        }
        _ => Err(USAGE.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("srp-game-data: {message}");
            ExitCode::FAILURE
        }
    }
}
