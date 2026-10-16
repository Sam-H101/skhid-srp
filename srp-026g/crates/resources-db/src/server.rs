// SPDX-License-Identifier: GPL-3.0-or-later
//! The server's game data: the files `srp` reads of the archive the players'
//! game mounts, `resources.db` with its patches over it, in an archive of
//! their own ([`crate::write`]) that names what it was made of.
//!
//! [`build`] lays the patches over `resources.db` in the game's order
//! ([`crate::overlay`]): by their number n, `resources.db.patch_<n>`, the
//! lowest first. It takes the files the server reads ([`is_read`]): every
//! file under `gameplay/`, and each project's `client_project` (srp only asks
//! whether a project is there, so it is kept empty). Each file is read from
//! the archive whose file is seen, and checked against that archive's table:
//! its size, and the table's CRC-32 of its bytes ([`crate::crc32`]).
//!
//! [`PROVENANCE`] names what it was made of ([`Provenance`]): `resources.db`
//! (its file name, size and sha256) and every patch (its file name, size,
//! sha256 and the URL the players download it from, on the patches
//! server). The server tells the players' updater which patches to fetch
//! from it ([`Provenance::offered`]): it can only offer the patches it runs
//! on. The same inputs give the same bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::overlay::Overlay;
use crate::{Archive, Error, crc32};

/// The files of `gameplay/` are read whole.
pub const GAMEPLAY: &str = "gameplay/";
/// A project is there when `projects/<project>/client_project` is.
pub const PROJECTS: &str = "projects/";
pub const PROJECT_MARKER: &str = "/client_project";
/// What the data was made of ([`Provenance`]).
pub const PROVENANCE: &str = "srp/provenance";
/// The first line of [`PROVENANCE`].
pub const FORMAT: &str = "skhid server game data 2";

/// The game's archive, in its game folder.
pub const RESOURCES_DB: &str = "resources.db";
/// What the game's patch `n` is called: `resources.db.patch_<n>`.
pub const PATCH_PREFIX: &str = "resources.db.patch_";
/// The patches' numbers the game looks for: 1 to this.
pub const LAST_PATCH: u32 = 255;

/// Whether the server reads `path` of the players' archive.
pub fn is_read(path: &str) -> bool {
    path.starts_with(GAMEPLAY) || (path.starts_with(PROJECTS) && path.ends_with(PROJECT_MARKER))
}

fn content(what: impl Into<String>) -> Error {
    Error::Content(what.into())
}

/// The number of the game's patch called `name`, `resources.db.patch_<n>`
/// with n from 1 to 255 written as the game writes it (`%u`: no sign, no
/// leading zero); `None` for any other name, which the game does not mount.
pub fn patch_number(name: &str) -> Option<u32> {
    let digits = name.strip_prefix(PATCH_PREFIX)?;
    let n: u32 = digits.parse().ok()?;
    (n.to_string() == digits && (1..=LAST_PATCH).contains(&n)).then_some(n)
}

/// The archives the game in the folder `game` (the one with
/// `resources.db`) mounts, in its order: `resources.db`, and every
/// `resources.db.patch_<n>` there from n = 1 to 255, a missing number
/// skipped. A file of another name is not mounted, nor is it here.
pub fn archives_in(game: &Path) -> Result<(PathBuf, Vec<PathBuf>), Error> {
    let db = game.join(RESOURCES_DB);
    if !db.is_file() {
        return Err(content(format!("{}: no such file", db.display())));
    }
    let patches = (1..=LAST_PATCH)
        .map(|n| game.join(format!("{PATCH_PREFIX}{n}")))
        .filter(|patch| patch.is_file())
        .collect();
    Ok((db, patches))
}

/// The longest URL of a patch the players' updater takes.
pub const MAX_URL: usize = 1024;

/// Whether the players' updater takes `url` as a patch's address: exactly
/// the URLs its `url::patch` takes (client/updater/logic/src/url.rs), rule
/// for rule, so that a URL it would refuse, and with it PLAY, is refused
/// here when the server's data is built. Both are tested with
/// client/updater/logic/tests/data/patch_urls.txt: change the two and that
/// file together.
///
/// At most [`MAX_URL`] bytes of printable ASCII (no space, no control
/// byte), without `"`, `\` or `#` (no fragment; and it is one word of
/// [`PROVENANCE`] and a JSON string without escapes); `http://` or
/// `https://`, lowercase; a host: a name or a bare IPv4 address, of
/// letters, digits, `.` and `-`, neither first nor last, up to 253 (so no
/// credentials and no `[IPv6]`); an optional port, 1 to 65535 in up to 5
/// digits; and a path of `/` and at least one more byte.
pub fn is_url(url: &str) -> bool {
    if url.len() > MAX_URL
        || !url
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\' | b'#'))
    {
        return false;
    }
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return false;
    };
    let Some((authority, path)) = rest.find('/').map(|at| rest.split_at(at)) else {
        return false;
    };
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if is_port(port) => host,
        Some(_) => return false,
        None => authority,
    };
    path.len() >= 2 && is_host(host)
}

/// A port of a URL [`is_url`] takes: 1 to 65535, up to 5 digits.
fn is_port(port: &str) -> bool {
    (1..=5).contains(&port.len())
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port
            .parse::<u32>()
            .is_ok_and(|port| (1..=65535).contains(&port))
}

/// A host of a URL [`is_url`] takes: a name or a bare IPv4 address.
fn is_host(host: &str) -> bool {
    (1..=253).contains(&host.len())
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

/// The URL of the file `name` in the folder `base` of the patches server:
/// `base`, a `/` unless it ends with one, and `name`.
pub fn url_in(base: &str, name: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

/// A file the data was made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    /// Its file name.
    pub name: String,
    pub size: u64,
    /// The sha256 of the whole file, lowercase hex.
    pub sha256: String,
}

impl Input {
    /// The file at `path`: its name, its size and its sha256, read through
    /// once.
    pub fn of(path: &Path) -> Result<Self, Error> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty() && !name.chars().any(char::is_whitespace))
            .ok_or_else(|| {
                content(format!(
                    "{}: a file name of UTF-8 without spaces is needed",
                    path.display()
                ))
            })?
            .to_owned();
        let at = |cause: std::io::Error| content(format!("{}: {cause}", path.display()));
        let mut file = File::open(path).map_err(at)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1 << 20];
        let mut size = 0;
        loop {
            let read = file.read(&mut buffer).map_err(at)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            size += read as u64;
        }
        let sha256 = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self { name, size, sha256 })
    }
}

/// A patch the data was made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    /// Its n, of its name `resources.db.patch_<n>`.
    pub number: u32,
    pub input: Input,
    /// Where the players' updater downloads it.
    pub url: String,
}

impl Patch {
    /// What the server calls it to the players' updater: `patch_<n>`.
    pub fn key(&self) -> String {
        format!("patch_{}", self.number)
    }
}

/// What server game data was made of: `resources.db`, and the patches over
/// it in the game's order (ascending n).
///
/// As text ([`PROVENANCE`]), the [`FORMAT`] line, a line for `resources.db`
/// and a line for each patch, fields by one space:
///
/// ```text
/// skhid server game data 2
/// db resources.db 6285533906 <sha256>
/// patch resources.db.patch_14 43691157 <sha256> http://203.0.113.5/resources.db.patch_14
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub db: Input,
    pub patches: Vec<Patch>,
}

/// A size as [`Provenance::text`] writes it: decimal digits, no sign, no
/// leading zero.
fn size_of(text: &str) -> Option<u64> {
    let size: u64 = text.parse().ok()?;
    (size.to_string() == text).then_some(size)
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl Provenance {
    pub fn text(&self) -> String {
        let db = &self.db;
        let mut text = format!("{FORMAT}\ndb {} {} {}\n", db.name, db.size, db.sha256);
        for patch in &self.patches {
            let input = &patch.input;
            text.push_str(&format!(
                "patch {} {} {} {}\n",
                input.name, input.size, input.sha256, patch.url
            ));
        }
        text
    }

    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut lines = text.lines();
        if lines.next() != Some(FORMAT) {
            return Err(content(format!(
                "{PROVENANCE} does not start {FORMAT:?}: data of another srp-game-data"
            )));
        }
        let wrong = |line: &str| content(format!("{PROVENANCE}: a wrong line: {line:?}"));
        let input = |name: &str, size: &str, sha256: &str| {
            Some(Input {
                name: name.to_owned(),
                size: size_of(size)?,
                sha256: is_sha256(sha256).then(|| sha256.to_owned())?,
            })
        };
        let line = lines.next().unwrap_or_default();
        let db = match line.split(' ').collect::<Vec<_>>()[..] {
            ["db", name, size, sha256] if !name.is_empty() => input(name, size, sha256),
            _ => None,
        }
        .ok_or_else(|| wrong(line))?;
        let mut patches: Vec<Patch> = Vec::new();
        for line in lines {
            let patch = match line.split(' ').collect::<Vec<_>>()[..] {
                ["patch", name, size, sha256, url] if is_url(url) => patch_number(name)
                    .zip(input(name, size, sha256))
                    .map(|(number, input)| Patch {
                        number,
                        input,
                        url: url.to_owned(),
                    }),
                _ => None,
            }
            .filter(|patch| patches.last().is_none_or(|last| last.number < patch.number))
            .ok_or_else(|| wrong(line))?;
            patches.push(patch);
        }
        Ok(Self { db, patches })
    }

    /// What the server offers the players' updater: each patch, in the
    /// game's order, as `patch_<n>` ([`Patch::key`]) with its sha256 and
    /// URL.
    pub fn offered(&self) -> impl Iterator<Item = (String, &str, &str)> {
        self.patches
            .iter()
            .map(|patch| (patch.key(), patch.input.sha256.as_str(), patch.url.as_str()))
    }
}

/// A patch to build with: its file and the URL players download it from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchFile {
    pub path: PathBuf,
    pub url: String,
}

/// What one archive gave the server's data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Given {
    /// Files the server reads that it has.
    pub files: usize,
    /// Of those, the ones an archive below it has too: this one's is kept.
    pub replaced: usize,
}

/// What [`build`] made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The server's game data.
    pub bytes: Vec<u8>,
    pub provenance: Provenance,
    /// The files of `gameplay/`.
    pub gameplay: usize,
    /// The projects.
    pub projects: usize,
    /// What each archive gave: `resources.db`, then the patches in order.
    pub given: Vec<Given>,
}

/// `patches` in the game's order, by their n. A patch must be called as the
/// game calls one (`resources.db.patch_<n>`, [`patch_number`]), once each
/// n, with a URL ([`is_url`]).
fn in_order(patches: &[PatchFile]) -> Result<Vec<(u32, &PatchFile)>, Error> {
    let mut numbered = Vec::new();
    for patch in patches {
        let name = patch.path.file_name().and_then(|name| name.to_str());
        let number = name.and_then(patch_number).ok_or_else(|| {
            content(format!(
                "{}: the game mounts a patch only by the name {PATCH_PREFIX}<n>, n from 1 to \
                 {LAST_PATCH}",
                patch.path.display()
            ))
        })?;
        if !is_url(&patch.url) {
            return Err(content(format!(
                "{}: {:?} is not a URL the players' updater takes for a patch \
                 (http:// or https://, a host name or a bare IPv4 address, an optional \
                 port, a path; no credentials, no #, no spaces, quotes or backslashes; \
                 printable ASCII, at most {MAX_URL} bytes)",
                patch.path.display(),
                patch.url
            )));
        }
        numbered.push((number, patch));
    }
    numbered.sort_by_key(|(number, _)| *number);
    if let Some(pair) = numbered.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(content(format!(
            "{} and {}: two patches of one number",
            pair[0].1.path.display(),
            pair[1].1.path.display()
        )));
    }
    Ok(numbered)
}

/// What the data of `db` and `patches` says it was made of: the patches in
/// the game's order. Each file is read through once, to hash it.
pub fn provenance_of(db: &Path, patches: &[PatchFile]) -> Result<Provenance, Error> {
    provenance_in_order(db, &in_order(patches)?)
}

fn provenance_in_order(db: &Path, patches: &[(u32, &PatchFile)]) -> Result<Provenance, Error> {
    Ok(Provenance {
        db: Input::of(db)?,
        patches: patches
            .iter()
            .map(|&(number, patch)| {
                Ok(Patch {
                    number,
                    input: Input::of(&patch.path)?,
                    url: patch.url.clone(),
                })
            })
            .collect::<Result<_, Error>>()?,
    })
}

/// The server's game data of `db` with `patches` over it in the game's
/// order ([`provenance_of`]). Only the archives' tables and the server's
/// files are read from them, and each file once more to hash it.
pub fn build(db: &Path, patches: &[PatchFile]) -> Result<Built, Error> {
    let patches = in_order(patches)?;
    let provenance = provenance_in_order(db, &patches)?;
    let paths: Vec<&Path> = std::iter::once(db)
        .chain(patches.iter().map(|(_, patch)| patch.path.as_path()))
        .collect();
    let names: Vec<String> = std::iter::once(&provenance.db)
        .chain(provenance.patches.iter().map(|patch| &patch.input))
        .map(|input| input.name.clone())
        .collect();

    let mut opened = Vec::new();
    let mut given = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let at = |cause: &dyn std::fmt::Display| content(format!("{}: {cause}", path.display()));
        let file = File::open(path).map_err(|cause| at(&cause))?;
        let archive = Archive::new(file).map_err(|cause| at(&cause))?;
        let mut files = 0;
        let mut replaced = 0;
        for (file, _) in archive.files_under("").filter(|(file, _)| is_read(file)) {
            files += 1;
            if !seen.insert(file.to_owned()) {
                replaced += 1;
            }
        }
        given.push(Given { files, replaced });
        opened.push(archive);
    }
    let mut overlay = Overlay::new(opened, &names)?;

    let read: Vec<(String, usize, crate::Entry)> = overlay
        .files_under("")
        .filter(|(path, _, _)| is_read(path))
        .map(|(path, index, entry)| (path.to_owned(), index, *entry))
        .collect();
    let mut files = BTreeMap::new();
    for (path, index, entry) in read {
        let bytes = overlay
            .read(&path)
            .map_err(|cause| content(format!("{}: {cause}", names[index])))?
            .expect("a listed file");
        if bytes.len() != entry.size as usize || crc32(&bytes) != entry.hash {
            return Err(content(format!(
                "{}: {path}: its bytes are not those its table says (size {}, CRC-32 {:08x})",
                names[index], entry.size, entry.hash
            )));
        }
        let kept = if path.starts_with(PROJECTS) {
            Vec::new()
        } else {
            bytes
        };
        files.insert(path, kept);
    }
    let gameplay = files
        .keys()
        .filter(|path| path.starts_with(GAMEPLAY))
        .count();
    let projects = files
        .keys()
        .filter(|path| path.starts_with(PROJECTS))
        .count();
    if gameplay == 0 {
        return Err(content(format!("{} has no {GAMEPLAY}", names.join(" + "))));
    }
    files.insert(PROVENANCE.to_owned(), provenance.text().into_bytes());
    Ok(Built {
        bytes: crate::write::write(&files)?,
        provenance,
        gameplay,
        projects,
        given,
    })
}

/// What [`read`] found in server game data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Data {
    pub provenance: Provenance,
    pub gameplay: usize,
    pub projects: usize,
}

/// The [`PROVENANCE`] of server game data `archive`, every file of it
/// checked against its table.
pub fn read<R: Read + Seek>(archive: &mut Archive<R>) -> Result<Data, Error> {
    let text = archive.read(PROVENANCE)?.ok_or_else(|| {
        content(format!(
            "no {PROVENANCE}: not server game data (srp-game-data build)"
        ))
    })?;
    let provenance = Provenance::parse(&String::from_utf8_lossy(&text))?;
    let paths: Vec<(String, crate::Entry)> = archive
        .files_under("")
        .map(|(path, entry)| (path.to_owned(), *entry))
        .collect();
    for (path, entry) in &paths {
        let bytes = archive.read(path)?.expect("a listed file");
        if crc32(&bytes) != entry.hash {
            return Err(content(format!(
                "{path}: its bytes are not those its table says"
            )));
        }
    }
    let count = |prefix: &str| {
        paths
            .iter()
            .filter(|(path, _)| path.starts_with(prefix))
            .count()
    };
    Ok(Data {
        provenance,
        gameplay: count(GAMEPLAY),
        projects: count(PROJECTS),
    })
}

#[cfg(test)]
mod tests;
