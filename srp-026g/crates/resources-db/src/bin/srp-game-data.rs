// SPDX-License-Identifier: GPL-3.0-or-later
//! `srp-game-data`: the server's game data ([`resources_db::server`]), the
//! files `srp` reads of `resources.db` with its patches over it, as the
//! players' game mounts them, and what it was made of: `resources.db`, and
//! each patch with the URLs the players download it from.
//!
//! ```text
//! srp-game-data build <archives> --out <file>
//! srp-game-data info <file>
//! srp-game-data check <file> <archives>
//! srp-game-data pack --dir <folder> --out <file>
//!
//! <archives>: --dir <game folder> [--patch-url <base URL>]
//!           | --db <resources.db> [--patch <file>[=<URL>[ <URL>]...]]... [--patch-url <base URL>]
//! ```
//!
//! See [`USAGE`].

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use resources_db::Archive;
use resources_db::server::{
    Data, LAST_PATCH, PATCH_PREFIX, PatchFile, Provenance, RESOURCES_DB, archives_in, build,
    patch_number, read, url_in,
};

const USAGE: &str = "usage:
  srp-game-data build <archives> --out <file>
  srp-game-data info <file>
  srp-game-data check <file> <archives>
  srp-game-data pack --dir <folder> --out <resources.db.patch_<n>>

<archives> is one of
  --dir <game folder> [--patch-url <base URL>]
  --db <resources.db> [--patch <file>[=<URL>[ <URL>]...]]... [--patch-url <base URL>]

--dir takes what the game mounts from that folder: resources.db, then every
resources.db.patch_<n> there, n from 1 to 255 (a missing n is skipped, other
names are not mounted). --db and --patch name the files; a patch must be
called resources.db.patch_<n>, and the patches are laid over resources.db in
ascending n, as the game mounts them, whatever their order here.

Every patch needs the URLs the players' updater downloads it from, 1 to 8,
tried in their order (the patches server first, then mirrors), each one the
updater takes (http:// or https://, a host name or a bare IPv4 address, an
optional port, a path; no credentials, no #, no spaces, quotes or
backslashes; at most 1024 bytes), none twice: <file>=<URL>, or
<file>=<URL> <URL>... as one argument, the URLs one space apart (a URL has
no space; the url column of release/patches.tsv is the same), or
--patch-url <base URL>, which gives a patch without any the URL
<base URL>/<its file name>.

build writes the server's game data; info reads it and says what it was made
of; check builds it again of the archives given and says whether <file> is
those bytes (exit status 1 if not).

pack makes a patch of every file under <folder>, at its path there
(<folder>/gameplay/x is gameplay/x of the archive), reads it back and checks
every file against the folder. --out must be called resources.db.patch_<n>
(or resources.db), the names the game mounts.

build and pack never overwrite --out.";

/// The archives the options name: `resources.db`, its patches with their
/// URLs; and `--out`.
fn archives(options: &[String]) -> Result<(PathBuf, Vec<PatchFile>, Option<PathBuf>), String> {
    let mut dir = None;
    let mut db = None;
    let mut base = None;
    let mut out = None;
    let mut patches: Vec<(PathBuf, Option<Vec<String>>)> = Vec::new();
    let mut rest = options.iter();
    while let Some(key) = rest.next() {
        if !matches!(
            key.as_str(),
            "--dir" | "--db" | "--patch-url" | "--out" | "--patch"
        ) {
            return Err(format!("unknown option {key}\n{USAGE}"));
        }
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
            // `<file>=<URL> <URL>...`: the URLs one space apart.
            "--patch" => patches.push(match value.split_once('=') {
                Some((path, urls)) => (
                    path.into(),
                    Some(urls.split(' ').map(str::to_owned).collect()),
                ),
                None => (value.into(), None),
            }),
            _ => unreachable!("an option above"),
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
        .map(|(path, urls)| {
            let urls = match (urls, &base) {
                (Some(urls), _) => urls,
                (None, Some(base)) => {
                    let name = path.file_name().and_then(|name| name.to_str());
                    vec![url_in(base, name.unwrap_or_default())]
                }
                (None, None) => {
                    return Err(format!(
                        "{}: no URL (<file>=<URL> or --patch-url)",
                        path.display()
                    ));
                }
            };
            Ok(PatchFile { path, urls })
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
            "  {} {} bytes sha256 {} urls {}",
            input.name,
            input.size,
            input.sha256,
            patch.urls.join(" ")
        );
    }
}

/// Refuses `out` if something is there: it is never overwritten.
fn is_new(out: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(out) {
        Ok(_) => Err(format!(
            "{}: already there; never overwritten",
            out.display()
        )),
        Err(_) => Ok(()),
    }
}

/// Writes `bytes` to `out`, which must not be there yet: to `<out>.part`,
/// which `check` reads back, then renamed to `out`. If any of it fails,
/// `<out>.part` is removed.
fn write_new(
    out: &Path,
    bytes: &[u8],
    check: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    is_new(out)?;
    let mut temporary = out.as_os_str().to_owned();
    temporary.push(".part");
    let temporary = PathBuf::from(temporary);
    let at = |cause: std::io::Error| format!("{}: {cause}", temporary.display());
    let written = std::fs::write(&temporary, bytes)
        .map_err(at)
        .and_then(|()| check(&temporary))
        .and_then(|()| is_new(out))
        .and_then(|()| std::fs::rename(&temporary, out).map_err(at));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

fn run(args: &[String]) -> Result<(), String> {
    let (command, options) = args.split_first().ok_or(USAGE)?;
    match command.as_str() {
        "build" => {
            let (db, patches, out) = archives(options)?;
            let out = out.ok_or_else(|| format!("build needs --out\n{USAGE}"))?;
            is_new(&out)?;
            let built = build(&db, &patches).map_err(|e| e.to_string())?;
            let names = std::iter::once(&built.provenance.db)
                .chain(built.provenance.patches.iter().map(|patch| &patch.input));
            for (input, given) in names.zip(&built.given) {
                println!(
                    "{}: {} of the server's files, {} of them over a file below",
                    input.name, given.files, given.replaced
                );
            }
            write_new(&out, &built.bytes, |written| {
                let found = open(written)?;
                (found.provenance == built.provenance)
                    .then_some(())
                    .ok_or_else(|| format!("{}: another provenance read back", written.display()))
            })?;
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
            check(&data, &db, &patches)
        }
        "pack" => pack(options),
        _ => Err(USAGE.into()),
    }
}

/// `check <file> <archives>`: whether `data` is, byte for byte, what
/// `build` makes of the archives now.
fn check(data: &Path, db: &Path, patches: &[PatchFile]) -> Result<(), String> {
    let found = std::fs::read(data).map_err(|cause| format!("{}: {cause}", data.display()))?;
    let built = build(db, patches).map_err(|e| e.to_string())?;
    if found == built.bytes {
        println!(
            "{}: the bytes these archives give, made of:",
            data.display()
        );
        print(&built.provenance);
        return Ok(());
    }
    match open(data) {
        Ok(read) if read.provenance != built.provenance => {
            println!("{} was made of:", data.display());
            print(&read.provenance);
            println!("what is given:");
            print(&built.provenance);
            Err(format!("{} was made of something else", data.display()))
        }
        Ok(_) => Err(format!(
            "{}: made of these archives, but not the bytes they give now ({} bytes, not {}): \
             another srp-game-data's, or changed",
            data.display(),
            found.len(),
            built.bytes.len()
        )),
        Err(cause) => Err(format!("{cause}; not the bytes these archives give")),
    }
}

/// `pack --dir <folder> --out <file>`.
fn pack(options: &[String]) -> Result<(), String> {
    let (dir, out) = match options {
        [a, dir, b, out] if a == "--dir" && b == "--out" => (dir, out),
        [b, out, a, dir] if a == "--dir" && b == "--out" => (dir, out),
        _ => return Err(format!("pack needs --dir <folder> --out <file>\n{USAGE}")),
    };
    let out = PathBuf::from(out);
    let name = out.file_name().and_then(|name| name.to_str());
    if !name.is_some_and(|name| name == RESOURCES_DB || patch_number(name).is_some()) {
        return Err(format!(
            "{}: the game mounts an archive only by the name {PATCH_PREFIX}<n>, n from 1 to \
             {LAST_PATCH} (or {RESOURCES_DB})",
            out.display()
        ));
    }
    is_new(&out)?;
    let (bytes, files) = resources_db::pack::pack(Path::new(dir)).map_err(|e| e.to_string())?;
    for (path, content) in &files {
        println!("  {path} {} bytes", content.len());
    }
    // Read back from the disk, as the game will.
    write_new(&out, &bytes, |written| {
        let written =
            std::fs::read(written).map_err(|cause| format!("{}: {cause}", written.display()))?;
        resources_db::pack::check(&written, &files)
            .map_err(|e| format!("{} read back: {e}", out.display()))
    })?;
    let made = resources_db::server::Input::of(&out).map_err(|e| e.to_string())?;
    println!(
        "{}: {} files, {} bytes, sha256 {}, read back: the same files",
        out.display(),
        files.len(),
        made.size,
        made.sha256
    );
    Ok(())
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

#[cfg(test)]
#[path = "srp-game-data/tests.rs"]
mod tests;
