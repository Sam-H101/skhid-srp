//! The resource step (`client/updater/README.md`, "Resources"): the plan the
//! players' ZIP brings, the record of what the step changed, and what is to
//! be done with each part of them.
//!
//! The plan is `game/binaries/x86/updater_skhid.resources`, a file of the
//! ZIP like any other:
//!
//! ```text
//! skhid-resources 2
//! region <path> <file size> <offset> <size> <original sha256> <target sha256> <name>
//! tail <path> <file size> <new size> <tail sha256> <name> <region>
//! file <sha256> <size> <path>
//! end
//! ```
//!
//! (Format 1 is the same without `tail` lines; a plan without one is still
//! written as format 1, so the updaters already out there read it.)
//!
//! - `region`: the bytes `[offset, offset + size)` of the file at `<path>`
//!   (a name as the ZIP has them: `game/resources.db`) are the blob
//!   `<original>` in the game as it is shipped, and are to become the blob
//!   `<target>`. `<file size>` is the whole file's size in that build of the
//!   game: a file of another size is of another build and is left alone.
//!   `<name>` is for the log and the launcher's line.
//! - `tail`: the file at `<path>`, `<file size>` bytes as shipped, is to
//!   grow to `<new size>` bytes: the blob `<tail sha256>` appended at its
//!   end. `<region>` names the region of the plan that must be written with
//!   it (the file's table, which says where the new entries are): that
//!   region is written only once the tail is in place, and if it cannot be
//!   written the tail is cut off again. A tail makes the plan *required*
//!   for its file: every region of the plan on that file, and the tail,
//!   must be in place before the game may be played (the game with a new
//!   exe and an old file crashes on what the server sends). One tail per
//!   file; the region it names must be of the same build (`<file size>`).
//!   The regions of a file keep their `<file size>`, the size as shipped:
//!   a region's file is of its build when it has that size, or that size
//!   grown by a tail the plan or the record names ([`file_sizes`]).
//! - `file`: a whole file, fetched by its checksum and put at `<path>`.
//!
//! The record beside the updater, `updater_skhid.resources.state`, is the
//! same lines under `skhid-resources-state 1` (2 when it holds a tail):
//! the regions the step wrote (a region is recorded before its first byte
//! is written), each with its original, so the original can be put back
//! when the plan no longer has it; the tails it appended (recorded before
//! the first byte is appended), so they can be cut off; and the files it
//! placed, so they can be removed. The journal,
//! `updater_skhid.resources.journal`, is a record of the one region or tail
//! being written.
//!
//! As in `manifest`, a line with an unknown word is skipped (a later server
//! may say more) and a higher format number is refused. Unlike the server's
//! answer, a file without its `end` line is cut short and is nothing at all,
//! and so is a file with a line that cannot be read or that names what it
//! may not: the step then changes nothing.
//!
//! What a region holds is told by the SHA-256 of its bytes alone: the
//! original, the target, or neither (damaged: a write that was cut short,
//! a disk error, another program). A journal left by a write that was cut
//! short is told in the log; the checksum decides what is done.

use alloc::string::String;
use alloc::vec::Vec;

use crate::manifest::{lines, number, push_number, word};
use crate::sha256::{from_hex, to_hex};
use crate::zip::safe_name;

/// The plan's name in the ZIP.
pub const PLAN_NAME: &str = "game/binaries/x86/updater_skhid.resources";
const PLAN_MAGIC: &[u8] = b"skhid-resources";
const RECORD_MAGIC: &[u8] = b"skhid-resources-state";
/// The most regions, tails and files one plan may have.
pub const MAX_ENTRIES: usize = 256;
/// The longest region name.
const MAX_LABEL: usize = 64;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PlanError {
    /// The first line is not this format's.
    NotPlan,
    /// The format's number is higher than this updater knows.
    NewerFormat,
    /// No `end` line: the file was cut short.
    CutShort,
    /// This line (counted from 1) cannot be read, or names a file the step
    /// may not change.
    BadLine(usize),
    /// This line claims bytes or a file an earlier line claims too.
    Conflict(usize),
    /// More than [`MAX_ENTRIES`] lines.
    TooMany,
}

/// Bytes of a file that are to become other bytes.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Region {
    pub path: String,
    pub file_size: u64,
    pub offset: u64,
    pub size: u64,
    pub original: [u8; 32],
    pub target: [u8; 32],
    pub name: String,
}

impl Region {
    /// The same bytes of the same file of the same build, the same original
    /// in them: the same region, whatever its target is.
    pub fn same_place(&self, other: &Region) -> bool {
        self.path.eq_ignore_ascii_case(&other.path)
            && self.file_size == other.file_size
            && self.offset == other.offset
            && self.size == other.size
            && self.original == other.original
    }

    /// Whether the two share a byte. (`offset + size` was checked when the
    /// region was read.)
    pub fn overlaps(&self, other: &Region) -> bool {
        self.path.eq_ignore_ascii_case(&other.path)
            && self.offset < other.offset + other.size
            && other.offset < self.offset + self.size
    }
}

/// Bytes appended to a file: it grows from `file_size` to `new_size`.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Tail {
    pub path: String,
    pub file_size: u64,
    pub new_size: u64,
    pub sha256: [u8; 32],
    pub name: String,
    /// The name of the region written with it.
    pub region: String,
}

impl Tail {
    /// The same bytes appended to the same file of the same build.
    pub fn same_place(&self, other: &Tail) -> bool {
        self.path.eq_ignore_ascii_case(&other.path)
            && self.file_size == other.file_size
            && self.new_size == other.new_size
            && self.sha256 == other.sha256
    }

    /// The tail's bytes.
    pub fn size(&self) -> u64 {
        self.new_size - self.file_size
    }

    /// Whether `region` is the one written with it.
    pub fn goes_with(&self, region: &Region) -> bool {
        self.path.eq_ignore_ascii_case(&region.path)
            && self.file_size == region.file_size
            && self.region.eq_ignore_ascii_case(&region.name)
    }
}

/// A whole file put in its place by its checksum.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct WholeFile {
    pub sha256: [u8; 32],
    pub size: u64,
    pub path: String,
}

/// A plan, or a record: its regions, tails and files.
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct Plan {
    pub regions: Vec<Region>,
    pub files: Vec<WholeFile>,
    pub tails: Vec<Tail>,
}

/// Files of the updater itself (its state, its log, its cache, the plan)
/// and the names of a half-done install: no line may name them.
fn updaters_own(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("game/binaries/x86/updater_skhid") || lower.contains(".skhid-")
}

/// The original game's files (`game::FILES`): never replaced whole.
fn original_game_file(path: &str) -> bool {
    path.len() > 5
        && path[..5].eq_ignore_ascii_case("game/")
        && crate::game::FILES
            .iter()
            .any(|(name, _)| path[5..].eq_ignore_ascii_case(name))
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

fn path(text: &[u8]) -> Option<String> {
    let path = core::str::from_utf8(text).ok()?;
    (safe_name(path) && !updaters_own(path)).then(|| path.into())
}

/// A region's name: what the log and the launcher's line say.
fn label(text: &[u8]) -> Option<String> {
    let ok = !text.is_empty()
        && text.len() <= MAX_LABEL
        && text
            .iter()
            .all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'));
    ok.then(|| String::from_utf8_lossy(text).into())
}

fn region(values: &[u8]) -> Option<Region> {
    let [path_text, file_size, offset, size, original, target, name] = fields::<7>(values)?;
    let region = Region {
        path: path(path_text)?,
        file_size: number(file_size)?,
        offset: number(offset)?,
        size: number(size)?,
        original: from_hex(original)?,
        target: from_hex(target)?,
        name: label(name)?,
    };
    let fits = region.size > 0
        && region
            .offset
            .checked_add(region.size)
            .is_some_and(|end| end <= region.file_size);
    (fits && region.original != region.target).then_some(region)
}

fn tail(values: &[u8]) -> Option<Tail> {
    let [path_text, file_size, new_size, sha256, name, region] = fields::<6>(values)?;
    let tail = Tail {
        path: path(path_text)?,
        file_size: number(file_size)?,
        new_size: number(new_size)?,
        sha256: from_hex(sha256)?,
        name: label(name)?,
        region: label(region)?,
    };
    (tail.new_size > tail.file_size).then_some(tail)
}

fn whole_file(values: &[u8]) -> Option<WholeFile> {
    let [sha256, size, path_text] = fields::<3>(values)?;
    let file = WholeFile {
        sha256: from_hex(sha256)?,
        size: number(size)?,
        path: path(path_text)?,
    };
    (file.size > 0 && !original_game_file(&file.path)).then_some(file)
}

impl Plan {
    /// Adds a region of the plan. `unique_names`: the plan's names are its
    /// own; a record may hold a region that left the plan under the name a
    /// new one has. `false` when it claims bytes or a file already claimed.
    fn add_region(&mut self, region: Region, unique_names: bool) -> bool {
        let clash = self.regions.iter().any(|other| {
            other.overlaps(&region)
                || (unique_names && other.name.eq_ignore_ascii_case(&region.name))
        }) || self
            .files
            .iter()
            .any(|file| file.path.eq_ignore_ascii_case(&region.path))
            || (unique_names
                && self
                    .tails
                    .iter()
                    .any(|tail| tail.name.eq_ignore_ascii_case(&region.name)));
        if !clash {
            self.regions.push(region);
        }
        !clash
    }

    fn add_file(&mut self, file: WholeFile) -> bool {
        let clash = self
            .files
            .iter()
            .any(|other| other.path.eq_ignore_ascii_case(&file.path))
            || self
                .regions
                .iter()
                .any(|region| region.path.eq_ignore_ascii_case(&file.path))
            || self
                .tails
                .iter()
                .any(|tail| tail.path.eq_ignore_ascii_case(&file.path));
        if !clash {
            self.files.push(file);
        }
        !clash
    }

    /// One tail per file, none on a whole file of the plan, and (a plan's)
    /// a name of its own.
    fn add_tail(&mut self, tail: Tail, unique_names: bool) -> bool {
        let clash = self
            .tails
            .iter()
            .any(|other| other.path.eq_ignore_ascii_case(&tail.path))
            || self
                .files
                .iter()
                .any(|file| file.path.eq_ignore_ascii_case(&tail.path))
            || (unique_names
                && (self
                    .regions
                    .iter()
                    .any(|region| region.name.eq_ignore_ascii_case(&tail.name))
                    || self
                        .tails
                        .iter()
                        .any(|other| other.name.eq_ignore_ascii_case(&tail.name))));
        if !clash {
            self.tails.push(tail);
        }
        !clash
    }

    /// The plan's tail on `path`, if it has one.
    pub fn tail_of(&self, path: &str) -> Option<&Tail> {
        self.tails
            .iter()
            .find(|tail| tail.path.eq_ignore_ascii_case(path))
    }

    /// Whether the plan must be in place before the game is played: it has
    /// a tail (see the module's text).
    pub fn required(&self) -> bool {
        !self.tails.is_empty()
    }

    /// Records `region` in the place of the one of the same place (another
    /// target), or as a new one. `false`, and nothing recorded, when it
    /// would share bytes with another recorded region: the record never
    /// holds two claims on one byte.
    pub fn record_region(&mut self, region: &Region) -> bool {
        if self
            .regions
            .iter()
            .any(|other| !other.same_place(region) && other.overlaps(region))
        {
            return false;
        }
        match self
            .regions
            .iter_mut()
            .find(|other| other.same_place(region))
        {
            Some(other) => *other = region.clone(),
            None => self.regions.push(region.clone()),
        }
        true
    }

    pub fn forget_region(&mut self, region: &Region) {
        self.regions.retain(|other| !other.same_place(region));
    }

    pub fn record_file(&mut self, file: &WholeFile) {
        self.forget_file(&file.path);
        self.files.push(file.clone());
    }

    pub fn forget_file(&mut self, path: &str) {
        self.files
            .retain(|other| !other.path.eq_ignore_ascii_case(path));
    }

    /// Records `tail` in the place of the one of its file (one tail per
    /// file).
    pub fn record_tail(&mut self, tail: &Tail) {
        self.tails
            .retain(|other| !other.path.eq_ignore_ascii_case(&tail.path));
        self.tails.push(tail.clone());
    }

    pub fn forget_tail(&mut self, tail: &Tail) {
        self.tails.retain(|other| !other.same_place(tail));
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty() && self.files.is_empty() && self.tails.is_empty()
    }
}

fn parse(text: &[u8], magic: &[u8], unique_names: bool) -> Result<Plan, PlanError> {
    let mut lines = lines(text);
    let (name, version) = word(lines.next().ok_or(PlanError::NotPlan)?);
    if name != magic {
        return Err(PlanError::NotPlan);
    }
    let version = match number(version) {
        Some(version @ (1 | 2)) => version,
        Some(version) if version > 2 => return Err(PlanError::NewerFormat),
        _ => return Err(PlanError::NotPlan),
    };
    let mut plan = Plan::default();
    let mut tail_lines = Vec::new();
    for (index, line) in lines.enumerate() {
        let at = index + 2;
        let added = match word(line) {
            (b"region", values) => {
                let region = region(values).ok_or(PlanError::BadLine(at))?;
                plan.add_region(region, unique_names)
            }
            // An updater of format 1 would skip the line and write the
            // region that needs it: a tail comes in format 2 only.
            (b"tail", _) if version < 2 => return Err(PlanError::BadLine(at)),
            (b"tail", values) => {
                let tail = tail(values).ok_or(PlanError::BadLine(at))?;
                tail_lines.push(at);
                plan.add_tail(tail, unique_names)
            }
            (b"file", values) => {
                let file = whole_file(values).ok_or(PlanError::BadLine(at))?;
                plan.add_file(file)
            }
            // What follows the end is not the plan's.
            (b"end", _) => {
                if unique_names {
                    check_tails(&plan, &tail_lines)?;
                }
                return Ok(plan);
            }
            _ => true,
        };
        if !added {
            return Err(PlanError::Conflict(at));
        }
        if plan.regions.len() + plan.files.len() + plan.tails.len() > MAX_ENTRIES {
            return Err(PlanError::TooMany);
        }
    }
    Err(PlanError::CutShort)
}

/// A plan's tail must name a region of the plan, of its build; every region
/// of its file must be of that build. (A record may hold a tail whose
/// region was put back already.)
fn check_tails(plan: &Plan, tail_lines: &[usize]) -> Result<(), PlanError> {
    for (tail, &at) in plan.tails.iter().zip(tail_lines) {
        if !plan.regions.iter().any(|region| tail.goes_with(region)) {
            return Err(PlanError::BadLine(at));
        }
        if plan.regions.iter().any(|region| {
            region.path.eq_ignore_ascii_case(&tail.path) && region.file_size != tail.file_size
        }) {
            return Err(PlanError::Conflict(at));
        }
    }
    Ok(())
}

/// Reads the plan.
pub fn parse_plan(text: &[u8]) -> Result<Plan, PlanError> {
    parse(text, PLAN_MAGIC, true)
}

/// Reads the record (or a journal). `None` for anything that is not a
/// whole, well-formed one.
pub fn parse_record(text: &[u8]) -> Option<Plan> {
    parse(text, RECORD_MAGIC, false).ok()
}

fn write(plan: &Plan, magic: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(magic);
    out.extend_from_slice(if plan.tails.is_empty() {
        b" 1\r\n"
    } else {
        b" 2\r\n"
    });
    for region in &plan.regions {
        out.extend_from_slice(b"region ");
        out.extend_from_slice(region.path.as_bytes());
        for value in [region.file_size, region.offset, region.size] {
            out.push(b' ');
            push_number(&mut out, value);
        }
        for hash in [&region.original, &region.target] {
            out.push(b' ');
            out.extend_from_slice(&to_hex(hash));
        }
        out.push(b' ');
        out.extend_from_slice(region.name.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    for tail in &plan.tails {
        out.extend_from_slice(b"tail ");
        out.extend_from_slice(tail.path.as_bytes());
        for value in [tail.file_size, tail.new_size] {
            out.push(b' ');
            push_number(&mut out, value);
        }
        out.push(b' ');
        out.extend_from_slice(&to_hex(&tail.sha256));
        for text in [&tail.name, &tail.region] {
            out.push(b' ');
            out.extend_from_slice(text.as_bytes());
        }
        out.extend_from_slice(b"\r\n");
    }
    for file in &plan.files {
        out.extend_from_slice(b"file ");
        out.extend_from_slice(&to_hex(&file.sha256));
        out.push(b' ');
        push_number(&mut out, file.size);
        out.push(b' ');
        out.extend_from_slice(file.path.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"end\r\n");
    out
}

/// The plan's text (`make_client_026g.py` writes the same).
pub fn write_plan(plan: &Plan) -> Vec<u8> {
    write(plan, PLAN_MAGIC)
}

/// The record's (or a journal's) text.
pub fn write_record(record: &Plan) -> Vec<u8> {
    write(record, RECORD_MAGIC)
}

/// The first name of a plan line that is a file of the overlay (the ZIP's
/// files are the overlay step's to write, not this step's). `None` when
/// there is none.
pub fn names_overlay_file<'a>(plan: &'a Plan, overlay: &[&str]) -> Option<&'a str> {
    plan.regions
        .iter()
        .map(|region| region.path.as_str())
        .chain(plan.tails.iter().map(|tail| tail.path.as_str()))
        .chain(plan.files.iter().map(|file| file.path.as_str()))
        .find(|path| overlay.iter().any(|name| name.eq_ignore_ascii_case(path)))
}

/// What a look at a region's bytes found.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Look {
    /// The file is not there.
    Missing,
    /// The file has another size than in the region's build of the game.
    OtherSize(u64),
    /// The file cannot be read now.
    Unreadable,
    /// The SHA-256 of the region's bytes.
    Bytes([u8; 32]),
}

/// What is to be done with a region of the plan.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Todo {
    /// Nothing, now: the file is missing, of another build, or unreadable.
    Skip,
    /// The region holds its target already.
    Done,
    /// The target is to be written. `backup`: the region holds the original
    /// and no checked copy of it is kept yet, so one is made first.
    /// `damaged`: the region holds neither the original nor the target (a
    /// write cut short, another target of an earlier plan, another
    /// program); the original, if it is ever needed, then comes from the
    /// copy kept or from the server.
    Write { backup: bool, damaged: bool },
}

/// What to do with `region` of the plan, by what its bytes are and whether
/// a checked copy of its original is kept.
pub fn todo(region: &Region, look: Look, backup_kept: bool) -> Todo {
    match look {
        Look::Missing | Look::OtherSize(_) | Look::Unreadable => Todo::Skip,
        Look::Bytes(hash) if hash == region.target => Todo::Done,
        Look::Bytes(hash) if hash == region.original => Todo::Write {
            backup: !backup_kept,
            damaged: false,
        },
        Look::Bytes(_) => Todo::Write {
            backup: false,
            damaged: true,
        },
    }
}

/// What to do with a recorded region the plan no longer has.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Undo {
    /// Nothing to put back (the original is there, or the file is gone or
    /// of another build): the record of it goes.
    Forget,
    /// The original is to be put back.
    Restore,
    /// The file cannot be read now: the record stays for the next run.
    Wait,
}

pub fn undo(region: &Region, look: Look) -> Undo {
    match look {
        Look::Missing | Look::OtherSize(_) => Undo::Forget,
        Look::Unreadable => Undo::Wait,
        Look::Bytes(hash) if hash == region.original => Undo::Forget,
        Look::Bytes(_) => Undo::Restore,
    }
}

/// The recorded regions the plan no longer has (a region of the plan of
/// the same place with another target is no removal: its new target is
/// written over the old one).
pub fn leaving(plan: &Plan, record: &Plan) -> Vec<Region> {
    record
        .regions
        .iter()
        .filter(|recorded| {
            !plan
                .regions
                .iter()
                .any(|planned| planned.same_place(recorded))
        })
        .cloned()
        .collect()
}

/// Whether a region of the plan must wait: it shares bytes with a recorded
/// region of another place whose original could not be put back yet.
/// Writing it would leave the old region's bytes with no way back.
pub fn blocked(region: &Region, record: &Plan) -> bool {
    record
        .regions
        .iter()
        .any(|other| !other.same_place(region) && other.overlaps(region))
}

/// The sizes `region`'s file may have and still be of its build: the size
/// as shipped, and that size grown by a tail of the plan or the record.
pub fn file_sizes(region: &Region, plan: &Plan, record: &Plan) -> Vec<u64> {
    let mut sizes = alloc::vec![region.file_size];
    for tail in plan.tails.iter().chain(record.tails.iter()) {
        if tail.path.eq_ignore_ascii_case(&region.path)
            && tail.file_size == region.file_size
            && !sizes.contains(&tail.new_size)
        {
            sizes.push(tail.new_size);
        }
    }
    sizes
}

/// What a look at a tail's file found.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TailLook {
    Missing,
    Unreadable,
    /// Shorter than as shipped: another build.
    Smaller(u64),
    /// The size as shipped: nothing appended.
    Original,
    /// Longer than as shipped. `tail`: the SHA-256 of the bytes past the
    /// size as shipped, when the file has the tail's new size.
    Grown {
        size: u64,
        tail: Option<[u8; 32]>,
    },
}

/// What is to be done with a tail of the plan.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TailTodo {
    /// Nothing can be done: the file is missing, unreadable, of another
    /// build, or grown by something that is not this step's.
    Skip,
    /// The tail is in place.
    Done,
    /// The tail is to be appended. `cut_first`: the file is longer than as
    /// shipped by what this step appended before (a tail cut short, an
    /// older tail): it is cut to its size as shipped first.
    Append { cut_first: bool },
}

/// `recorded`: the record has a tail of this file and build
/// ([`tail_recorded`]), so what is past the size as shipped is this step's.
pub fn tail_todo(tail: &Tail, look: TailLook, recorded: bool) -> TailTodo {
    match look {
        TailLook::Missing | TailLook::Unreadable | TailLook::Smaller(_) => TailTodo::Skip,
        TailLook::Original => TailTodo::Append { cut_first: false },
        TailLook::Grown {
            size,
            tail: Some(hash),
        } if size == tail.new_size && hash == tail.sha256 => TailTodo::Done,
        TailLook::Grown { .. } if recorded => TailTodo::Append { cut_first: true },
        TailLook::Grown { .. } => TailTodo::Skip,
    }
}

/// Whether the record has a tail of `tail`'s file and build.
pub fn tail_recorded(tail: &Tail, record: &Plan) -> bool {
    record.tails.iter().any(|other| {
        other.path.eq_ignore_ascii_case(&tail.path) && other.file_size == tail.file_size
    })
}

/// What to do with a recorded tail the plan no longer has.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum TailUndo {
    /// Nothing to cut (the file is as shipped, gone, or of another build):
    /// the record of it goes.
    Forget,
    /// The file is cut to its size as shipped.
    Cut,
    /// The file cannot be read now: the record stays.
    Wait,
}

pub fn tail_undo(look: TailLook) -> TailUndo {
    match look {
        TailLook::Missing | TailLook::Smaller(_) | TailLook::Original => TailUndo::Forget,
        TailLook::Unreadable => TailUndo::Wait,
        TailLook::Grown { .. } => TailUndo::Cut,
    }
}

/// The recorded tails the plan no longer has.
pub fn tails_leaving(plan: &Plan, record: &Plan) -> Vec<Tail> {
    record
        .tails
        .iter()
        .filter(|recorded| {
            !plan
                .tails
                .iter()
                .any(|planned| planned.same_place(recorded))
        })
        .cloned()
        .collect()
}

/// Whether a recorded tail that leaves must wait: the region written with
/// it leaves too and is not put back yet (its table would point past the
/// file's end). A region the plan keeps is written again after the plan's
/// own tail.
pub fn tail_waits(tail: &Tail, plan: &Plan, record: &Plan) -> bool {
    record.regions.iter().any(|region| {
        tail.goes_with(region)
            && !plan
                .regions
                .iter()
                .any(|planned| planned.same_place(region))
    })
}

/// What a look at a whole file found.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FileLook {
    Missing,
    Unreadable,
    /// Its size and SHA-256.
    Found(u64, [u8; 32]),
}

/// Whether the file of the plan is in its place already.
pub fn file_done(file: &WholeFile, look: FileLook) -> bool {
    look == FileLook::Found(file.size, file.sha256)
}

/// What to do with a recorded file the plan no longer has.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FileUndo {
    /// It is still the file that was put there: it goes.
    Delete,
    /// It is gone, or something else is there now (not the step's to
    /// delete): the record of it goes.
    Forget,
    /// It cannot be read now: the record stays.
    Wait,
}

pub fn file_undo(file: &WholeFile, look: FileLook) -> FileUndo {
    match look {
        FileLook::Missing => FileUndo::Forget,
        FileLook::Unreadable => FileUndo::Wait,
        found if file_done(file, found) => FileUndo::Delete,
        FileLook::Found(..) => FileUndo::Forget,
    }
}

/// The recorded files the plan no longer has, by their path.
pub fn files_leaving(plan: &Plan, record: &Plan) -> Vec<WholeFile> {
    record
        .files
        .iter()
        .filter(|recorded| {
            !plan
                .files
                .iter()
                .any(|planned| planned.path.eq_ignore_ascii_case(&recorded.path))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::digest;

    fn hex(data: &[u8]) -> String {
        String::from_utf8(to_hex(&digest(data)).to_vec()).unwrap()
    }

    /// The two regions of build 2777's resources.db the first plan has,
    /// with made-up targets.
    fn plan() -> Plan {
        Plan {
            regions: vec![
                Region {
                    path: "game/resources.db".into(),
                    file_size: 6_285_533_906,
                    offset: 19_165_044,
                    size: 38_985_769,
                    original: digest(b"lobby original"),
                    target: digest(b"lobby target"),
                    name: "lobby_menu".into(),
                },
                Region {
                    path: "game/resources.db".into(),
                    file_size: 6_285_533_906,
                    offset: 58_150_813,
                    size: 3_508_744,
                    original: digest(b"login original"),
                    target: digest(b"login target"),
                    name: "login_menu".into(),
                },
            ],
            files: vec![WholeFile {
                sha256: digest(b"map"),
                size: 123_456_789,
                path: "game/resources/maps/new_map.db".into(),
            }],
            tails: Vec::new(),
        }
    }

    /// The plan of a release that grows resources.db: its file table (the
    /// head) is written with the tail of new entries.
    fn grown() -> Plan {
        let mut plan = plan();
        plan.regions.insert(
            0,
            Region {
                path: "game/resources.db".into(),
                file_size: 6_285_533_906,
                offset: 0,
                size: 10_618_257,
                original: digest(b"table original"),
                target: digest(b"table target"),
                name: "file_table".into(),
            },
        );
        plan.tails.push(Tail {
            path: "game/resources.db".into(),
            file_size: 6_285_533_906,
            new_size: 6_285_533_906 + 43_000_000,
            sha256: digest(b"tail"),
            name: "new_items".into(),
            region: "file_table".into(),
        });
        plan
    }

    #[test]
    fn a_plan_with_a_tail_is_of_format_2() {
        let plan = grown();
        let text = write_plan(&plan);
        assert!(text.starts_with(b"skhid-resources 2\r\n"));
        let line = format!(
            "tail game/resources.db 6285533906 6328533906 {} new_items file_table\r\n",
            hex(b"tail")
        );
        assert!(String::from_utf8(text.clone()).unwrap().contains(&line));
        assert_eq!(parse_plan(&text), Ok(plan.clone()));
        assert!(plan.required());
        assert!(!self::plan().required());
        // Without a tail, format 1 still: the updaters out there read it.
        assert!(write_plan(&self::plan()).starts_with(b"skhid-resources 1\r\n"));
        // The record holds tails the same way; one whose region was put
        // back already is fine there.
        let mut record = plan.clone();
        record.regions.remove(0);
        assert!(write_record(&record).starts_with(b"skhid-resources-state 2\r\n"));
        assert_eq!(parse_record(&write_record(&record)), Some(record.clone()));
        // A plan of format 1 with a tail line: an old updater would skip
        // the line and write the table alone. Refused.
        let old = String::from_utf8(text)
            .unwrap()
            .replace("skhid-resources 2", "skhid-resources 1");
        assert_eq!(parse_plan(old.as_bytes()), Err(PlanError::BadLine(5)));
        // Format 3 is newer.
        assert_eq!(
            parse_plan(b"skhid-resources 3\nend\n"),
            Err(PlanError::NewerFormat)
        );
        // Cut short anywhere: nothing.
        let text = write_plan(&plan);
        for cut in 0..text.len() - 4 {
            assert!(parse_plan(&text[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn a_tail_must_come_with_its_region() {
        let text = String::from_utf8(write_plan(&grown())).unwrap();
        for (from, to, error) in [
            // No region of that name, or of another file.
            (
                " new_items file_table",
                " new_items lobby_table",
                PlanError::BadLine(5),
            ),
            (
                "tail game/resources.db",
                "tail game/resources2.db",
                PlanError::BadLine(5),
            ),
            // Not longer than as shipped.
            (
                " 6285533906 6328533906 ",
                " 6285533906 6285533906 ",
                PlanError::BadLine(5),
            ),
            (
                " 6285533906 6328533906 ",
                " 6285533906 5 ",
                PlanError::BadLine(5),
            ),
            // Another build than its region's (and the other regions').
            (
                "tail game/resources.db 6285533906",
                "tail game/resources.db 6285533907",
                PlanError::BadLine(5),
            ),
            // A name of a region.
            (
                " new_items file_table",
                " lobby_menu file_table",
                PlanError::Conflict(5),
            ),
            (
                " new_items file_table",
                " new_items lobby/table",
                PlanError::BadLine(5),
            ),
        ] {
            assert!(text.contains(from), "{from}");
            assert_eq!(
                parse_plan(text.replacen(from, to, 1).as_bytes()),
                Err(error),
                "{from} -> {to}"
            );
        }
        // Another build's region on the tail's file.
        let mut mixed = grown();
        mixed.regions[2].file_size = 70_000_000;
        assert_eq!(parse_plan(&write_plan(&mixed)), Err(PlanError::Conflict(5)));
        // Two tails on one file; a tail on a whole file of the plan.
        let mut two = grown();
        let mut other = two.tails[0].clone();
        other.name = "more".into();
        two.tails.push(other);
        assert_eq!(parse_plan(&write_plan(&two)), Err(PlanError::Conflict(6)));
        let mut whole = grown();
        whole.tails[0].path = whole.files[0].path.clone();
        whole.regions.clear();
        assert_eq!(parse_plan(&write_plan(&whole)), Err(PlanError::Conflict(3)));
        // The overlay may not have the tail's file either.
        let mut alone = grown();
        alone.regions.clear();
        alone.files.clear();
        assert_eq!(
            names_overlay_file(&alone, &["game/RESOURCES.db"]),
            Some("game/resources.db")
        );
    }

    #[test]
    fn a_grown_file_is_still_of_its_build() {
        let plan = grown();
        let lobby = &plan.regions[1];
        let empty = Plan::default();
        assert_eq!(
            file_sizes(lobby, &self::plan(), &empty),
            vec![6_285_533_906]
        );
        assert_eq!(
            file_sizes(lobby, &plan, &empty),
            vec![6_285_533_906, 6_328_533_906]
        );
        // An older tail of the record counts as well, once.
        let mut record = Plan::default();
        let mut older = plan.tails[0].clone();
        older.new_size += 5;
        older.sha256 = digest(b"older tail");
        record.record_tail(&older);
        record.record_tail(&plan.tails[0]);
        assert_eq!(record.tails, vec![plan.tails[0].clone()]);
        record.record_tail(&older);
        assert_eq!(
            file_sizes(lobby, &plan, &record),
            vec![6_285_533_906, 6_328_533_906, 6_328_533_911]
        );
        // Another file's or another build's tail does not.
        let mut elsewhere = plan.tails[0].clone();
        elsewhere.path = "game/other.db".into();
        let mut build = plan.tails[0].clone();
        build.file_size -= 1;
        let others = Plan {
            tails: vec![elsewhere, build],
            ..Plan::default()
        };
        assert_eq!(file_sizes(lobby, &others, &empty), vec![6_285_533_906]);
    }

    #[test]
    fn what_a_file_is_decides_what_is_done_with_its_tail() {
        let plan = grown();
        let tail = &plan.tails[0];
        let grown_by = |size: u64, hash: Option<[u8; 32]>| TailLook::Grown { size, tail: hash };
        for recorded in [false, true] {
            for look in [
                TailLook::Missing,
                TailLook::Unreadable,
                TailLook::Smaller(5),
            ] {
                assert_eq!(tail_todo(tail, look, recorded), TailTodo::Skip);
            }
            assert_eq!(
                tail_todo(tail, TailLook::Original, recorded),
                TailTodo::Append { cut_first: false }
            );
            assert_eq!(
                tail_todo(tail, grown_by(tail.new_size, Some(tail.sha256)), recorded),
                TailTodo::Done
            );
            // Cut short, another tail of this step, or more than it wrote.
            for look in [
                grown_by(tail.new_size - 1, None),
                grown_by(tail.new_size, Some(digest(b"x"))),
                grown_by(tail.new_size + 1, None),
            ] {
                assert_eq!(
                    tail_todo(tail, look, recorded),
                    if recorded {
                        TailTodo::Append { cut_first: true }
                    } else {
                        TailTodo::Skip
                    }
                );
            }
        }
        let mut record = Plan::default();
        assert!(!tail_recorded(tail, &record));
        let mut older = tail.clone();
        older.sha256 = digest(b"older");
        record.record_tail(&older);
        assert!(tail_recorded(tail, &record));
        // A tail that leaves.
        assert_eq!(tail_undo(TailLook::Original), TailUndo::Forget);
        assert_eq!(tail_undo(TailLook::Missing), TailUndo::Forget);
        assert_eq!(tail_undo(TailLook::Smaller(1)), TailUndo::Forget);
        assert_eq!(tail_undo(TailLook::Unreadable), TailUndo::Wait);
        assert_eq!(tail_undo(grown_by(7, None)), TailUndo::Cut);
        assert_eq!(tail.size(), 43_000_000);
    }

    #[test]
    fn a_tail_leaves_after_its_region() {
        let plan = grown();
        let mut record = plan.clone();
        assert_eq!(tails_leaving(&plan, &record), vec![]);
        // A release without the new items: the table goes back first.
        let without = self::plan();
        assert_eq!(tails_leaving(&without, &record), plan.tails);
        assert!(tail_waits(&plan.tails[0], &without, &record));
        record.forget_region(&plan.regions[0]);
        assert!(!tail_waits(&plan.tails[0], &without, &record));
        // A release with other new items: the old tail is cut, the new one
        // appended, then the table written over the old target.
        let mut next = grown();
        next.tails[0].sha256 = digest(b"next tail");
        next.tails[0].new_size += 1000;
        next.regions[0].target = digest(b"next table");
        let record = plan.clone();
        assert_eq!(tails_leaving(&next, &record), plan.tails);
        assert!(!tail_waits(&plan.tails[0], &next, &record));
        assert!(record.tails[0].goes_with(&next.regions[0]));
    }

    #[test]
    fn the_plan_goes_to_text_and_back() {
        let plan = plan();
        let text = write_plan(&plan);
        assert_eq!(parse_plan(&text), Ok(plan.clone()));
        // The text make_client_026g.py writes for one region (its test has
        // the same line).
        let line = format!(
            "region game/resources.db 6285533906 19165044 38985769 {} {} lobby_menu\r\n",
            hex(b"lobby original"),
            hex(b"lobby target")
        );
        assert!(String::from_utf8(text.clone()).unwrap().contains(&line));
        assert!(text.starts_with(b"skhid-resources 1\r\n"));
        assert!(text.ends_with(b"end\r\n"));
        // LF only, an unknown line, upper-case hex and lines after the end
        // are all fine.
        let other = String::from_utf8(text)
            .unwrap()
            .replace("\r\n", "\n")
            .replace(
                "region game/resources.db 6285533906 58150813",
                "note later servers may say more\nregion game/resources.db 6285533906 58150813",
            )
            .replace(
                &hex(b"login target"),
                &hex(b"login target").to_ascii_uppercase(),
            )
            + "region what follows the end is not read\n";
        assert_eq!(parse_plan(other.as_bytes()), Ok(plan));
        // The record is the same lines under its own first line.
        let record = plan_record();
        assert_eq!(parse_record(&write_record(&record)), Some(record.clone()));
        assert_eq!(parse_record(&write_plan(&record)), None);
        assert_eq!(parse_plan(&write_record(&record)), Err(PlanError::NotPlan));
        // An empty plan says "nothing", which is not "no plan".
        assert_eq!(parse_plan(b"skhid-resources 1\nend\n"), Ok(Plan::default()));
    }

    fn plan_record() -> Plan {
        let mut record = plan();
        // A record may hold a region that left the plan under a name a new
        // region has.
        let mut old = record.regions[0].clone();
        old.offset = 1_000;
        old.size = 10;
        record.regions.push(old);
        record
    }

    #[test]
    fn a_plan_cut_short_anywhere_is_no_plan() {
        let text = write_plan(&plan());
        for cut in 0..text.len() - 4 {
            assert!(parse_plan(&text[..cut]).is_err(), "cut at {cut}");
            assert_eq!(
                parse_record(&write_record(&plan())[..cut]),
                None,
                "cut at {cut}"
            );
        }
        assert_eq!(parse_plan(b"skhid-resources 1\n"), Err(PlanError::CutShort));
        assert_eq!(parse_plan(b""), Err(PlanError::NotPlan));
        assert_eq!(parse_plan(b"<html>"), Err(PlanError::NotPlan));
        assert_eq!(
            parse_plan(b"skhid-resources x\nend\n"),
            Err(PlanError::NotPlan)
        );
        assert_eq!(
            parse_plan(b"skhid-resources 0\nend\n"),
            Err(PlanError::NotPlan)
        );
        assert_eq!(
            parse_plan(b"skhid-resources 3\nsomething new\n"),
            Err(PlanError::NewerFormat)
        );
    }

    #[test]
    fn a_tampered_line_spoils_the_whole_plan() {
        let text = String::from_utf8(write_plan(&plan())).unwrap();
        let original = hex(b"lobby original");
        for (from, to) in [
            // Out of the game folder, or not a plain name.
            (
                "game/resources.db 6285533906 19165044",
                "../resources.db 6285533906 19165044",
            ),
            (
                "game/resources.db 6285533906 19165044",
                "c:/resources.db 6285533906 19165044",
            ),
            (
                "game/resources.db 6285533906 19165044",
                "game\\resources.db 6285533906 19165044",
            ),
            (
                "game/resources.db 6285533906 19165044",
                "game/aux.db 6285533906 19165044",
            ),
            (
                "game/resources/maps/new_map.db",
                "game/resources/maps/../../x.db",
            ),
            // The updater's own files and an install's half-done names.
            (
                "game/resources/maps/new_map.db",
                "game/binaries/x86/updater_skhid.state",
            ),
            (
                "game/resources/maps/new_map.db",
                "game/binaries/x86/UPDATER_SKHID.resources",
            ),
            (
                "game/resources/maps/new_map.db",
                "game/binaries/x86/updater_skhid.cache/x",
            ),
            (
                "game/resources/maps/new_map.db",
                "game/resources/maps/new_map.db.skhid-new",
            ),
            // The original game's files are never replaced whole.
            ("game/resources/maps/new_map.db", "game/resources.db"),
            (
                "game/resources/maps/new_map.db",
                "game/binaries/x86/survarium.exe",
            ),
            (
                "game/resources/maps/new_map.db",
                "GAME/binaries/X86/Survarium.exe",
            ),
            (
                "game/resources/maps/new_map.db",
                "game/binaries/x86/bugtrap.dll",
            ),
            // Numbers, checksums and names that cannot be read.
            (" 6285533906 19165044", " 6285533906 -19165044"),
            (" 6285533906 19165044", " 6285533906  19165044"),
            (" 38985769 ", " 0 "),
            (" 38985769 ", " 99999999999999999999 "),
            (" 123456789 ", " 0 "),
            (" lobby_menu", " lobby menu"),
            (" lobby_menu", " lobby/menu"),
            (" lobby_menu", " "),
            (&original, "zz"),
            // Past the file's end, or no change at all.
            (" 6285533906 19165044", " 58150812 19165044"),
            (&hex(b"lobby target"), &original),
        ] {
            assert!(text.contains(from), "{from}");
            let changed = text.replacen(from, to, 1);
            assert!(
                matches!(parse_plan(changed.as_bytes()), Err(PlanError::BadLine(_))),
                "{from} -> {to}: {:?}",
                parse_plan(changed.as_bytes())
            );
            let record = String::from_utf8(write_record(&plan())).unwrap();
            assert_eq!(
                parse_record(record.replacen(from, to, 1).as_bytes()),
                None,
                "{from} -> {to}"
            );
        }
        // A region that ends at the file's end is fine.
        let mut whole = plan();
        whole.regions[1].file_size = 58_150_813 + 3_508_744;
        whole.regions[0].file_size = whole.regions[1].file_size;
        assert_eq!(parse_plan(&write_plan(&whole)), Ok(whole));
    }

    #[test]
    fn two_lines_may_not_claim_the_same_bytes() {
        let mut overlapping = plan();
        overlapping.regions[1].offset = 19_165_044 + 38_985_769 - 1;
        assert_eq!(
            parse_plan(&write_plan(&overlapping)),
            Err(PlanError::Conflict(3))
        );
        assert_eq!(parse_record(&write_record(&overlapping)), None);
        // Touching is not sharing.
        let mut touching = plan();
        touching.regions[1].offset = 19_165_044 + 38_985_769;
        assert!(parse_plan(&write_plan(&touching)).is_ok());
        // The same bytes of another file are another file's.
        let mut other_file = overlapping.clone();
        other_file.regions[1].path = "game/other.db".into();
        assert!(parse_plan(&write_plan(&other_file)).is_ok());
        // The same name twice, in a plan.
        let mut named = plan();
        named.regions[1].name = "LOBBY_MENU".into();
        assert_eq!(parse_plan(&write_plan(&named)), Err(PlanError::Conflict(3)));
        // A file line and a region of one path, or one file twice.
        let mut both = plan();
        both.files[0].path = "game/RESOURCES2.db".into();
        both.regions[1].path = "game/resources2.db".into();
        assert_eq!(parse_plan(&write_plan(&both)), Err(PlanError::Conflict(4)));
        let mut twice = plan();
        twice.files.push(twice.files[0].clone());
        assert_eq!(parse_plan(&write_plan(&twice)), Err(PlanError::Conflict(5)));
        // Too many lines.
        let mut many = Plan::default();
        for index in 0..=MAX_ENTRIES {
            many.files.push(WholeFile {
                sha256: digest(b"x"),
                size: 1,
                path: format!("game/many/{index}.db"),
            });
        }
        assert_eq!(parse_plan(&write_plan(&many)), Err(PlanError::TooMany));
        many.files.pop();
        assert!(parse_plan(&write_plan(&many)).is_ok());
    }

    #[test]
    fn a_plan_may_not_name_a_file_of_the_overlay() {
        let plan = plan();
        let overlay = ["game/binaries/x86/survarium_skhid.exe", "README.txt"];
        assert_eq!(names_overlay_file(&plan, &overlay), None);
        let overlay = ["README.txt", "game/RESOURCES/maps/new_map.db"];
        assert_eq!(
            names_overlay_file(&plan, &overlay),
            Some("game/resources/maps/new_map.db")
        );
        let overlay = ["game/Resources.db"];
        assert_eq!(
            names_overlay_file(&plan, &overlay),
            Some("game/resources.db")
        );
        assert_eq!(names_overlay_file(&Plan::default(), &overlay), None);
    }

    #[test]
    fn what_a_region_holds_decides_what_is_done() {
        let region = &plan().regions[0];
        let (original, target, other) = (region.original, region.target, digest(b"x"));
        for kept in [false, true] {
            assert_eq!(todo(region, Look::Missing, kept), Todo::Skip);
            assert_eq!(todo(region, Look::OtherSize(5), kept), Todo::Skip);
            assert_eq!(todo(region, Look::Unreadable, kept), Todo::Skip);
            assert_eq!(todo(region, Look::Bytes(target), kept), Todo::Done);
            assert_eq!(
                todo(region, Look::Bytes(other), kept),
                Todo::Write {
                    backup: false,
                    damaged: true
                }
            );
        }
        // A copy of the original is made before its first write, once.
        assert_eq!(
            todo(region, Look::Bytes(original), false),
            Todo::Write {
                backup: true,
                damaged: false
            }
        );
        assert_eq!(
            todo(region, Look::Bytes(original), true),
            Todo::Write {
                backup: false,
                damaged: false
            }
        );
        // A region that left the plan.
        assert_eq!(undo(region, Look::Bytes(original)), Undo::Forget);
        assert_eq!(undo(region, Look::Bytes(target)), Undo::Restore);
        assert_eq!(undo(region, Look::Bytes(other)), Undo::Restore);
        assert_eq!(undo(region, Look::Missing), Undo::Forget);
        assert_eq!(undo(region, Look::OtherSize(1)), Undo::Forget);
        assert_eq!(undo(region, Look::Unreadable), Undo::Wait);
    }

    #[test]
    fn regions_that_leave_the_plan_are_put_back_first() {
        let plan = plan();
        let mut record = plan.clone();
        // The same place with another target: overwritten, not removed.
        record.regions[0].target = digest(b"older target");
        // A region of an older plan.
        let mut old = plan.regions[0].clone();
        old.offset = 19_165_044 + 10;
        old.size = 100;
        old.name = "older".into();
        assert!(!record.record_region(&old), "overlaps the one recorded");
        record.forget_region(&plan.regions[0]);
        assert!(record.record_region(&old));
        assert_eq!(leaving(&plan, &record), vec![old.clone()]);
        // While it is not put back, the new region over its bytes waits.
        assert!(blocked(&plan.regions[0], &record));
        assert!(!blocked(&plan.regions[1], &record));
        record.forget_region(&old);
        assert!(!blocked(&plan.regions[0], &record));
        assert_eq!(leaving(&plan, &record), vec![]);
        // Recording again replaces the one of the same place.
        assert!(record.record_region(&plan.regions[0]));
        let mut newer = plan.regions[0].clone();
        newer.target = digest(b"newer");
        assert!(record.record_region(&newer));
        assert_eq!(
            record
                .regions
                .iter()
                .filter(|r| r.same_place(&newer))
                .count(),
            1
        );
        assert_eq!(record.regions.last(), Some(&newer));
        assert!(!blocked(&newer, &record));
        // Another build of the file is another place.
        let mut build = newer.clone();
        build.file_size += 1;
        assert!(!build.same_place(&newer));
    }

    #[test]
    fn whole_files_are_placed_and_removed_by_their_checksum() {
        let plan = plan();
        let file = &plan.files[0];
        assert!(file_done(file, FileLook::Found(file.size, file.sha256)));
        assert!(!file_done(
            file,
            FileLook::Found(file.size + 1, file.sha256)
        ));
        assert!(!file_done(
            file,
            FileLook::Found(file.size, digest(b"other"))
        ));
        assert!(!file_done(file, FileLook::Missing));
        assert_eq!(
            file_undo(file, FileLook::Found(file.size, file.sha256)),
            FileUndo::Delete
        );
        assert_eq!(
            file_undo(file, FileLook::Found(file.size, digest(b"mine"))),
            FileUndo::Forget
        );
        assert_eq!(file_undo(file, FileLook::Missing), FileUndo::Forget);
        assert_eq!(file_undo(file, FileLook::Unreadable), FileUndo::Wait);
        let mut record = Plan::default();
        record.record_file(file);
        let mut other = file.clone();
        other.path = "game/resources/maps/OLD_map.db".into();
        record.record_file(&other);
        assert_eq!(files_leaving(&plan, &record), vec![other.clone()]);
        // A planned file with another checksum replaces the record of it.
        let mut newer = file.clone();
        newer.sha256 = digest(b"newer");
        record.record_file(&newer);
        assert_eq!(record.files, vec![other.clone(), newer]);
        record.forget_file("game/resources/maps/old_map.db");
        assert_eq!(files_leaving(&plan, &record), vec![]);
    }
}
