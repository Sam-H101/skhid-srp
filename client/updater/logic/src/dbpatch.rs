//! Database patches (`client/updater/README.md`, "The game's database").
//! The game's own `resources.db` is never changed; the updater builds
//! `resources_skhid.db` beside it: a copy with every patch of the release
//! applied in the order of their revisions, checked whole by its SHA-256.
//!
//! This module is the one definition of the format, for the updater and for
//! the tools that make patches (`srp-026g/client-patches/db_patches.py` is
//! its Python twin). No dependency, no `std`.
//!
//! A patch is a description and a blob. The description:
//!
//! ```text
//! skhid-db-patch 1
//! revision 14
//! from <sha256 of the whole input file> <size>
//! to <sha256 of the whole output file> <size>
//! blob <sha256 of the blob> <size>
//! write <offset> <length>
//! append <length>
//! end
//! ```
//!
//! - `write`: `length` bytes put over the input at `offset` (an entry
//!   rewritten in place, the file table). Inside the input, in rising
//!   order, not overlapping.
//! - `append`: `length` bytes added at the end. `to size` is `from size`
//!   and every append.
//! - `blob`: the zstd-compressed file (`/resources/<sha256>` on the server)
//!   that holds the bytes of every write and append, in the order of the
//!   lines, and nothing else.
//! - Applying: the input is `from`; the writes and appends are done; the
//!   whole file is hashed and must be `to`.
//!
//! Revision 14 (the first) is three writes and no append: the menus
//! `main_menu`, `lobby_menu` and `login_menu` in place.
//!
//! The list of a release, `game/binaries/x86/updater_skhid.patches` in the
//! players' ZIP, is the original and the descriptions of every patch, in
//! order (patches are never taken off the server: a player who missed
//! releases needs all of them):
//!
//! ```text
//! skhid-db-patches 1
//! original <sha256> <size>
//! skhid-db-patch 1
//! revision 14
//! ...
//! end
//! skhid-db-patch 1
//! revision 15
//! ...
//! end
//! end
//! ```
//!
//! Revisions rise; the first `from` is the original, each next `from` the
//! `to` before it. The last `to` is the release's `resources_skhid.db`; with
//! no patch, it is a copy of the original. As in the updater's other
//! formats, a line with an unknown word is skipped, a higher format number
//! is refused, and a text without its `end` line is nothing at all.

use alloc::vec::Vec;

use crate::manifest::{lines, number, push_number, word};
use crate::sha256::{from_hex, to_hex};

/// The list's name in the ZIP.
pub const MANIFEST_NAME: &str = "game/binaries/x86/updater_skhid.patches";
/// The game's own database (only read) and the one the updater builds.
pub const ORIGINAL_PATH: &str = "game/resources.db";
pub const SKHID_PATH: &str = "game/resources_skhid.db";
const LIST_MAGIC: &[u8] = b"skhid-db-patches";
const PATCH_MAGIC: &[u8] = b"skhid-db-patch";
const STATE_MAGIC: &[u8] = b"skhid-db-state";
/// The most patches a list may have, and writes and appends a patch may.
pub const MAX_PATCHES: usize = 256;
pub const MAX_OPS: usize = 4096;
/// What the quick look at a built database reads: this much at its start
/// (the file table) and at its end (the newest entries).
pub const PROBE: u64 = 16 << 20;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Error {
    /// The first line is not this format's.
    NotThis,
    NewerFormat,
    /// No `end` line.
    CutShort,
    /// This line (counted from 1) cannot be read, or breaks a rule.
    BadLine(usize),
    TooMany,
}

/// A whole file: its SHA-256 and size.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct FileId {
    pub sha256: [u8; 32],
    pub size: u64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Op {
    Write { offset: u64, length: u64 },
    Append { length: u64 },
}

impl Op {
    pub fn length(&self) -> u64 {
        match *self {
            Op::Write { length, .. } | Op::Append { length } => length,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Patch {
    pub revision: u64,
    pub from: FileId,
    pub to: FileId,
    /// The compressed blob.
    pub blob: FileId,
    pub ops: Vec<Op>,
}

impl Patch {
    /// The bytes the blob holds once decompressed.
    pub fn data_size(&self) -> u64 {
        self.ops.iter().map(Op::length).sum()
    }
}

/// The list of a release.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Manifest {
    pub original: FileId,
    pub patches: Vec<Patch>,
}

impl Manifest {
    /// The revision and the file the release's `resources_skhid.db` is.
    pub fn target(&self) -> (u64, FileId) {
        match self.patches.last() {
            Some(patch) => (patch.revision, patch.to),
            None => (0, self.original),
        }
    }

    /// The file the database is at `revision` (0: the original).
    pub fn at(&self, revision: u64) -> Option<FileId> {
        if revision == 0 {
            return Some(self.original);
        }
        self.patches
            .iter()
            .find(|patch| patch.revision == revision)
            .map(|patch| patch.to)
    }

    /// The newest revision whose database is `file`.
    pub fn revision_of(&self, file: &FileId) -> Option<u64> {
        self.patches
            .iter()
            .rev()
            .find(|patch| patch.to == *file)
            .map(|patch| patch.revision)
            .or((self.original == *file).then_some(0))
    }

    /// The patches newer than `revision`, in order.
    pub fn after(&self, revision: u64) -> &[Patch] {
        let first = self
            .patches
            .iter()
            .position(|patch| patch.revision > revision)
            .unwrap_or(self.patches.len());
        &self.patches[first..]
    }
}

/// Exactly `N` values, one space between each.
fn fields<const N: usize>(values: &[u8]) -> Option<[&[u8]; N]> {
    let mut out = [&values[..0]; N];
    let mut parts = values.split(|&byte| byte == b' ');
    for slot in &mut out {
        *slot = parts.next().filter(|part| !part.is_empty())?;
    }
    parts.next().is_none().then_some(out)
}

fn file_id(values: &[u8]) -> Option<FileId> {
    let [sha256, size] = fields::<2>(values)?;
    Some(FileId {
        sha256: from_hex(sha256)?,
        size: number(size)?,
    })
}

/// `<magic> 1`.
fn header(line: Option<&[u8]>, magic: &[u8]) -> Result<(), Error> {
    let (name, version) = word(line.ok_or(Error::NotThis)?);
    if name != magic {
        return Err(Error::NotThis);
    }
    match number(version) {
        Some(1) => Ok(()),
        Some(version) if version > 1 => Err(Error::NewerFormat),
        _ => Err(Error::NotThis),
    }
}

/// One patch, from the line after its first to its `end`. `at`: the number
/// of the line before the first one taken.
fn patch<'a>(lines: &mut impl Iterator<Item = &'a [u8]>, at: &mut usize) -> Result<Patch, Error> {
    let (mut revision, mut from, mut to, mut blob) = (None, None, None, None);
    let mut ops: Vec<Op> = Vec::new();
    for line in lines {
        *at += 1;
        let bad = Error::BadLine(*at);
        match word(line) {
            (b"revision", value) if revision.is_none() => {
                revision = Some(number(value).filter(|&r| r > 0).ok_or(bad)?);
            }
            (b"from", values) if from.is_none() => from = Some(file_id(values).ok_or(bad)?),
            (b"to", values) if to.is_none() => to = Some(file_id(values).ok_or(bad)?),
            (b"blob", values) if blob.is_none() => blob = Some(file_id(values).ok_or(bad)?),
            (b"revision" | b"from" | b"to" | b"blob", _) => return Err(bad),
            (b"write", values) => {
                let [offset, length] = fields::<2>(values).ok_or(bad)?;
                let offset = number(offset).ok_or(bad)?;
                let length = number(length).filter(|&l| l > 0).ok_or(bad)?;
                // In rising order, not overlapping, inside the input.
                let after_last = ops.iter().rev().find_map(|op| match *op {
                    Op::Write { offset, length } => Some(offset + length),
                    Op::Append { .. } => None,
                });
                if after_last.is_some_and(|end| end > offset)
                    || from.is_none_or(|from: FileId| offset + length > from.size)
                {
                    return Err(bad);
                }
                ops.push(Op::Write { offset, length });
            }
            (b"append", value) => {
                let length = number(value).filter(|&l| l > 0).ok_or(bad)?;
                ops.push(Op::Append { length });
            }
            (b"end", _) => {
                let (Some(revision), Some(from), Some(to), Some(blob)) = (revision, from, to, blob)
                else {
                    return Err(bad);
                };
                let appended: u64 = ops
                    .iter()
                    .filter_map(|op| match *op {
                        Op::Append { length } => Some(length),
                        Op::Write { .. } => None,
                    })
                    .sum();
                let fits = !ops.is_empty()
                    && from != to
                    && blob.size > 0
                    && from.size.checked_add(appended) == Some(to.size);
                return if fits {
                    Ok(Patch {
                        revision,
                        from,
                        to,
                        blob,
                        ops,
                    })
                } else {
                    Err(bad)
                };
            }
            _ => {}
        }
        if ops.len() > MAX_OPS {
            return Err(Error::TooMany);
        }
    }
    Err(Error::CutShort)
}

/// Reads one patch's description (a file of its own, as the tools write it).
pub fn parse_patch(text: &[u8]) -> Result<Patch, Error> {
    let mut lines = lines(text);
    header(lines.next(), PATCH_MAGIC)?;
    patch(&mut lines, &mut 1)
}

/// Reads the list. Every rule of the chain is checked.
pub fn parse_manifest(text: &[u8]) -> Result<Manifest, Error> {
    let mut lines = lines(text);
    header(lines.next(), LIST_MAGIC)?;
    let mut at = 1;
    let mut original = None;
    let mut patches: Vec<Patch> = Vec::new();
    while let Some(line) = lines.next() {
        at += 1;
        let bad = Error::BadLine(at);
        match word(line) {
            (b"original", values) if original.is_none() => {
                original = Some(file_id(values).ok_or(bad)?);
            }
            (b"original", _) => return Err(bad),
            (name, _) if name == PATCH_MAGIC => {
                header(Some(line), PATCH_MAGIC).map_err(|_| bad)?;
                let start = at;
                let patch = patch(&mut lines, &mut at)?;
                let before = match patches.last() {
                    Some(last) if patch.revision <= last.revision => {
                        return Err(Error::BadLine(start));
                    }
                    Some(last) => last.to,
                    None => original.ok_or(Error::BadLine(start))?,
                };
                if patch.from != before {
                    return Err(Error::BadLine(start));
                }
                patches.push(patch);
                if patches.len() > MAX_PATCHES {
                    return Err(Error::TooMany);
                }
            }
            (b"end", _) => {
                return Ok(Manifest {
                    original: original.ok_or(bad)?,
                    patches,
                });
            }
            _ => {}
        }
    }
    Err(Error::CutShort)
}

fn push_id(out: &mut Vec<u8>, word: &[u8], file: &FileId) {
    out.extend_from_slice(word);
    out.push(b' ');
    out.extend_from_slice(&to_hex(&file.sha256));
    out.push(b' ');
    push_number(out, file.size);
    out.extend_from_slice(b"\r\n");
}

/// A patch's description.
pub fn write_patch(patch: &Patch) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(PATCH_MAGIC);
    out.extend_from_slice(b" 1\r\nrevision ");
    push_number(&mut out, patch.revision);
    out.extend_from_slice(b"\r\n");
    push_id(&mut out, b"from", &patch.from);
    push_id(&mut out, b"to", &patch.to);
    push_id(&mut out, b"blob", &patch.blob);
    for op in &patch.ops {
        match *op {
            Op::Write { offset, length } => {
                out.extend_from_slice(b"write ");
                push_number(&mut out, offset);
                out.push(b' ');
                push_number(&mut out, length);
            }
            Op::Append { length } => {
                out.extend_from_slice(b"append ");
                push_number(&mut out, length);
            }
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"end\r\n");
    out
}

/// The list's text (`db_patches.py` writes the same).
pub fn write_manifest(manifest: &Manifest) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(LIST_MAGIC);
    out.extend_from_slice(b" 1\r\n");
    push_id(&mut out, b"original", &manifest.original);
    for patch in &manifest.patches {
        out.extend_from_slice(&write_patch(patch));
    }
    out.extend_from_slice(b"end\r\n");
    out
}

/// What the updater keeps about the `resources_skhid.db` it built and
/// checked whole: its revision, the file, and the SHA-256 of what
/// [`probe_ranges`] reads, for the quick look at the next starts.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct State {
    pub revision: u64,
    pub file: FileId,
    pub probe: [u8; 32],
}

pub fn parse_state(text: &[u8]) -> Option<State> {
    let mut lines = lines(text);
    header(lines.next(), STATE_MAGIC).ok()?;
    let (mut revision, mut file, mut probe) = (None, None, None);
    for line in lines {
        match word(line) {
            (b"revision", value) => revision = Some(number(value)?),
            (b"file", values) => file = Some(file_id(values)?),
            (b"probe", value) => probe = Some(from_hex(value)?),
            (b"end", _) => {
                return Some(State {
                    revision: revision?,
                    file: file?,
                    probe: probe?,
                });
            }
            _ => {}
        }
    }
    None
}

pub fn write_state(state: &State) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(STATE_MAGIC);
    out.extend_from_slice(b" 1\r\nrevision ");
    push_number(&mut out, state.revision);
    out.extend_from_slice(b"\r\n");
    push_id(&mut out, b"file", &state.file);
    out.extend_from_slice(b"probe ");
    out.extend_from_slice(&to_hex(&state.probe));
    out.extend_from_slice(b"\r\nend\r\n");
    out
}

/// The parts of a file of `size` bytes the quick look hashes, one after
/// the other: its first and its last [`PROBE`] bytes (all of a smaller
/// file).
pub fn probe_ranges(size: u64) -> [(u64, u64); 2] {
    if size <= 2 * PROBE {
        [(0, size), (size, 0)]
    } else {
        [(0, PROBE), (size - PROBE, PROBE)]
    }
}

/// What a quick look at `resources_skhid.db` found.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Look {
    Missing,
    /// Its size and the SHA-256 of its probe.
    Found {
        size: u64,
        probe: [u8; 32],
    },
    /// There, but it cannot be read now.
    Unreadable,
}

/// What is to be done with `resources_skhid.db`.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Todo {
    /// It is the release's.
    Done,
    /// It was checked whole at this revision: the newer patches are applied.
    From(u64),
    /// It is there, but nothing says what it is: it is hashed whole, and
    /// [`Manifest::revision_of`] tells.
    Hash,
    /// It is built from the original.
    Build,
}

/// What to do, by the state kept and what a quick look found.
pub fn todo(manifest: &Manifest, state: Option<&State>, look: Look) -> Todo {
    match look {
        Look::Missing => Todo::Build,
        Look::Unreadable => Todo::Hash,
        Look::Found { size, probe } => match state {
            Some(state)
                if state.file.size == size
                    && state.probe == probe
                    && manifest.at(state.revision) == Some(state.file) =>
            {
                if state.revision == manifest.target().0 {
                    Todo::Done
                } else {
                    Todo::From(state.revision)
                }
            }
            _ => Todo::Hash,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::digest;
    use alloc::format;
    use alloc::string::String;

    fn id(name: &[u8], size: u64) -> FileId {
        FileId {
            sha256: digest(name),
            size,
        }
    }

    fn hex(name: &[u8]) -> String {
        String::from_utf8(to_hex(&digest(name)).to_vec()).unwrap()
    }

    /// The stock db; revision 14, the three live menus in place (their
    /// stock offsets and sizes); a later one with new entries.
    fn manifest() -> Manifest {
        let original = id(b"stock", 6_285_533_906);
        let menus = id(b"menus", 6_285_533_906);
        let items = id(b"items", 6_328_475_738);
        Manifest {
            original,
            patches: vec![
                Patch {
                    revision: 14,
                    from: original,
                    to: menus,
                    blob: id(b"blob 14", 9_000_000),
                    ops: vec![
                        Op::Write {
                            offset: 17_970_448,
                            length: 1_194_596,
                        },
                        Op::Write {
                            offset: 19_165_044,
                            length: 38_985_769,
                        },
                        Op::Write {
                            offset: 58_150_813,
                            length: 3_508_744,
                        },
                    ],
                },
                Patch {
                    revision: 15,
                    from: menus,
                    to: items,
                    blob: id(b"blob 15", 20_000_000),
                    ops: vec![
                        Op::Write {
                            offset: 0,
                            length: 10_344_588,
                        },
                        Op::Append { length: 42_941_832 },
                    ],
                },
            ],
        }
    }

    #[test]
    fn a_patch_goes_to_text_and_back() {
        let patch = manifest().patches.remove(0);
        let text = write_patch(&patch);
        let expected = format!(
            "skhid-db-patch 1\r\nrevision 14\r\nfrom {} 6285533906\r\nto {} 6285533906\r\n\
             blob {} 9000000\r\nwrite 17970448 1194596\r\nwrite 19165044 38985769\r\n\
             write 58150813 3508744\r\nend\r\n",
            hex(b"stock"),
            hex(b"menus"),
            hex(b"blob 14")
        );
        assert_eq!(String::from_utf8(text.clone()).unwrap(), expected);
        assert_eq!(parse_patch(&text), Ok(patch.clone()));
        assert_eq!(patch.data_size(), 1_194_596 + 38_985_769 + 3_508_744);
        // LF and a line of a later tool are fine.
        let other = expected
            .replace("\r\n", "\n")
            .replace("write 1916", "made-by a tool\nwrite 1916");
        assert_eq!(parse_patch(other.as_bytes()), Ok(patch));
        for cut in 0..text.len() - 4 {
            assert!(parse_patch(&text[..cut]).is_err(), "cut at {cut}");
        }
        assert_eq!(
            parse_patch(b"skhid-db-patch 2\nend\n"),
            Err(Error::NewerFormat)
        );
    }

    #[test]
    fn a_patch_breaking_a_rule_is_refused() {
        let text = String::from_utf8(write_patch(&manifest().patches[1])).unwrap();
        for (from, to) in [
            // Not overlapping, rising, inside the input.
            ("write 0 10344588", "write 0 10344588\r\nwrite 10344587 1"),
            ("write 0 10344588", "write 100 10\r\nwrite 0 10"),
            ("write 0 10344588", "write 0 6285533907"),
            ("write 0 10344588", "write 0 0"),
            ("append 42941832", "append 0"),
            // The appends must make the new size.
            ("append 42941832", "append 42941831"),
            ("append 42941832", "append 42941832\r\nappend 1"),
            // Twice, missing, nothing done.
            ("revision 15", "revision 15\r\nrevision 16"),
            ("revision 15", "revision 0"),
            (
                &format!("blob {} 20000000\r\n", hex(b"blob 15")) as &str,
                "",
            ),
            (
                &format!("blob {} 20000000", hex(b"blob 15")),
                &format!("blob {} 0", hex(b"blob 15")),
            ),
            ("write 0 10344588\r\nappend 42941832\r\n", ""),
        ] {
            assert!(text.contains(from), "{from}");
            assert!(
                parse_patch(text.replacen(from, to, 1).as_bytes()).is_err(),
                "{from} -> {to}"
            );
        }
        // A write before `from` is known.
        let early = text
            .replace("write 0 10344588\r\n", "")
            .replace("revision 15\r\n", "revision 15\r\nwrite 0 1\r\n");
        assert!(parse_patch(early.as_bytes()).is_err());
    }

    #[test]
    fn the_list_goes_to_text_and_back() {
        let manifest = manifest();
        let text = write_manifest(&manifest);
        assert!(
            text.starts_with(
                format!(
                    "skhid-db-patches 1\r\noriginal {} 6285533906\r\nskhid-db-patch 1\r\n",
                    hex(b"stock")
                )
                .as_bytes()
            )
        );
        assert!(text.ends_with(b"end\r\nend\r\n"));
        assert_eq!(parse_manifest(&text), Ok(manifest.clone()));
        let other = String::from_utf8(text.clone())
            .unwrap()
            .replace("\r\n", "\n")
            + "skhid-db-patch 1\nwhat follows the end is not read\n";
        assert_eq!(parse_manifest(other.as_bytes()), Ok(manifest.clone()));
        let plain = Manifest {
            patches: Vec::new(),
            ..manifest.clone()
        };
        assert_eq!(parse_manifest(&write_manifest(&plain)), Ok(plain.clone()));
        assert_eq!(plain.target(), (0, manifest.original));
        for cut in 0..text.len() - 4 {
            assert!(parse_manifest(&text[..cut]).is_err(), "cut at {cut}");
        }
        assert_eq!(
            parse_manifest(b"skhid-db-patches 2\nend\n"),
            Err(Error::NewerFormat)
        );
        assert_eq!(
            parse_manifest(b"skhid-db-patches 1\nend\n"),
            Err(Error::BadLine(2))
        );
        assert_eq!(parse_manifest(b"<html>"), Err(Error::NotThis));
        // A patch of a newer format in the list: the whole list.
        let newer =
            String::from_utf8(text)
                .unwrap()
                .replacen("skhid-db-patch 1", "skhid-db-patch 2", 1);
        assert!(parse_manifest(newer.as_bytes()).is_err());
    }

    #[test]
    fn the_chain_must_hold() {
        let text = String::from_utf8(write_manifest(&manifest())).unwrap();
        for (from, to) in [
            ("revision 15", "revision 14"),
            ("revision 15", "revision 13"),
            (
                &format!("from {} 6285533906", hex(b"stock")) as &str,
                &format!("from {} 6285533906", hex(b"other")) as &str,
            ),
            (
                &format!("from {} 6285533906", hex(b"menus")),
                &format!("from {} 6285533906", hex(b"stock")),
            ),
            ("original", "x"),
        ] {
            assert!(text.contains(from), "{from}");
            assert!(
                parse_manifest(text.replacen(from, to, 1).as_bytes()).is_err(),
                "{from} -> {to}"
            );
        }
    }

    #[test]
    fn the_list_says_what_each_revision_is() {
        let manifest = manifest();
        let (p14, p15) = (&manifest.patches[0], &manifest.patches[1]);
        assert_eq!(manifest.target(), (15, p15.to));
        assert_eq!(manifest.at(0), Some(manifest.original));
        assert_eq!(manifest.at(14), Some(p14.to));
        assert_eq!(manifest.at(13), None);
        assert_eq!(manifest.revision_of(&p14.to), Some(14));
        assert_eq!(manifest.revision_of(&manifest.original), Some(0));
        assert_eq!(manifest.revision_of(&id(b"x", 1)), None);
        assert_eq!(manifest.after(0), &manifest.patches[..]);
        assert_eq!(manifest.after(14), &manifest.patches[1..]);
        assert_eq!(manifest.after(15), &[]);
    }

    #[test]
    fn the_state_goes_to_text_and_back() {
        let state = State {
            revision: 14,
            file: id(b"menus", 6_285_533_906),
            probe: digest(b"probe"),
        };
        let text = write_state(&state);
        assert_eq!(parse_state(&text), Some(state));
        for cut in 0..text.len() - 4 {
            assert_eq!(parse_state(&text[..cut]), None, "cut at {cut}");
        }
        assert_eq!(probe_ranges(100), [(0, 100), (100, 0)]);
        assert_eq!(
            probe_ranges(6_285_533_906),
            [(0, PROBE), (6_285_533_906 - PROBE, PROBE)]
        );
    }

    #[test]
    fn what_is_found_decides_what_is_done() {
        let manifest = manifest();
        let p14 = manifest.patches[0].to;
        let last = manifest.target().1;
        let found = |size| Look::Found {
            size,
            probe: digest(b"probe"),
        };
        let at = |revision, file| State {
            revision,
            file,
            probe: digest(b"probe"),
        };
        assert_eq!(todo(&manifest, None, Look::Missing), Todo::Build);
        assert_eq!(
            todo(&manifest, Some(&at(14, p14)), Look::Missing),
            Todo::Build
        );
        assert_eq!(todo(&manifest, None, found(p14.size)), Todo::Hash);
        assert_eq!(
            todo(&manifest, Some(&at(14, p14)), Look::Unreadable),
            Todo::Hash
        );
        assert_eq!(
            todo(&manifest, Some(&at(14, p14)), found(p14.size)),
            Todo::From(14)
        );
        assert_eq!(
            todo(&manifest, Some(&at(0, manifest.original)), found(p14.size)),
            Todo::From(0)
        );
        assert_eq!(
            todo(&manifest, Some(&at(15, last)), found(last.size)),
            Todo::Done
        );
        // Another size, another probe, a state the list does not have.
        assert_eq!(
            todo(&manifest, Some(&at(15, last)), found(last.size + 1)),
            Todo::Hash
        );
        let other = Look::Found {
            size: last.size,
            probe: digest(b"other"),
        };
        assert_eq!(todo(&manifest, Some(&at(15, last)), other), Todo::Hash);
        assert_eq!(
            todo(&manifest, Some(&at(15, p14)), found(p14.size)),
            Todo::Hash
        );
        assert_eq!(
            todo(&manifest, Some(&at(16, last)), found(last.size)),
            Todo::Hash
        );
    }
}
