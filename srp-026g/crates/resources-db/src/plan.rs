// SPDX-License-Identifier: GPL-3.0-or-later
//! The archive the players run: the installed `resources.db` with the
//! players' update plan laid over it in memory, nothing written.
//!
//! The plan is the one the players' ZIP carries
//! (`game/binaries/x86/updater_skhid.resources`), read by the updater's own
//! code (`client/updater/logic`), so it cannot be read otherwise here. Of it,
//! the lines of `game/resources.db` count: a `region` puts a blob's bytes in
//! place of a range of the installed file, a `tail` appends a blob to it
//! (`client/updater/README.md`, "Resources"). A blob is a file named by its
//! sha256 in a folder, as the server offers them (`download/resources/`).
//!
//! Every size and checksum is checked first, as the updater checks them:
//! the installed file is the size the plan was made for, each range of it a
//! region replaces holds the plan's original bytes, and each blob is the
//! plan's. Anything else is an error, and nothing is read.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use updater_logic::resources::{PLAN_NAME, Plan, parse_plan};
use updater_logic::sha256::{Sha256, to_hex};

use crate::{Archive, Error};

/// The plan's name for the archive.
pub const RESOURCES_DB: &str = "game/resources.db";

/// The plan file of the players' ZIP `zip` as it is, if it has one.
pub fn plan_text_of_zip(zip: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    let entries = updater_logic::zip::entries(zip)
        .map_err(|cause| plan_error(format!("the ZIP cannot be read ({cause:?})")))?;
    let Some(entry) = entries.iter().find(|entry| entry.name == PLAN_NAME) else {
        return Ok(None);
    };
    let text = entry
        .read()
        .map_err(|cause| plan_error(format!("its {PLAN_NAME} cannot be read ({cause:?})")))?;
    Ok(Some(text))
}

/// The plan of the players' ZIP `zip`; no lines if it has none.
pub fn plan_of_zip(zip: &[u8]) -> Result<Plan, Error> {
    match plan_text_of_zip(zip)? {
        Some(text) => {
            parse_plan(&text).map_err(|cause| plan_error(format!("{PLAN_NAME}: {cause:?}")))
        }
        None => Ok(Plan::default()),
    }
}

fn plan_error(what: String) -> Error {
    Error::Plan(what)
}

fn hex(digest: &[u8; 32]) -> String {
    String::from_utf8_lossy(&to_hex(digest)).into_owned()
}

/// Where a range of the archive comes from.
#[derive(Debug)]
enum Source {
    /// The installed file, at the same offset.
    Installed,
    /// A blob, from its start.
    Blob(usize),
}

#[derive(Debug)]
struct Part {
    start: u64,
    end: u64,
    source: Source,
}

/// The installed archive with a plan's regions and tail laid over it: reads
/// as the file the players' updater makes.
#[derive(Debug)]
pub struct Patched {
    installed: File,
    blobs: Vec<File>,
    parts: Vec<Part>,
    len: u64,
    pos: u64,
}

/// What [`Patched::open`] checked and laid over the installed file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The regions, by name.
    pub regions: Vec<String>,
    /// The tail, by name, if the plan has one.
    pub tail: Option<String>,
    /// The archive's size with them.
    pub size: u64,
}

/// The sha256 of `len` bytes of `file` from `at`.
fn digest_of(file: &mut File, at: u64, len: u64) -> io::Result<[u8; 32]> {
    file.seek(SeekFrom::Start(at))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    let mut left = len;
    while left > 0 {
        let n = buffer
            .len()
            .min(usize::try_from(left).unwrap_or(usize::MAX));
        file.read_exact(&mut buffer[..n])?;
        hasher.update(&buffer[..n]);
        left -= n as u64;
    }
    Ok(hasher.finish())
}

/// The blob `sha256` of `blobs`, checked to be `size` bytes of that sha256.
fn blob(blobs: &Path, sha256: &[u8; 32], size: u64, name: &str) -> Result<File, Error> {
    let path: PathBuf = blobs.join(hex(sha256));
    let mut file = File::open(&path)
        .map_err(|cause| plan_error(format!("{name}: no blob {} ({cause})", path.display())))?;
    let len = file.metadata()?.len();
    if len != size {
        return Err(plan_error(format!(
            "{name}: the blob {} is {len} bytes, the plan says {size}",
            path.display()
        )));
    }
    if digest_of(&mut file, 0, len)? != *sha256 {
        return Err(plan_error(format!(
            "{name}: the blob {} is not the plan's (sha256)",
            path.display()
        )));
    }
    Ok(file)
}

impl Patched {
    /// The installed archive at `installed` with `plan`'s lines for
    /// `game/resources.db` laid over it, every one checked; the blobs are
    /// in `blobs`.
    pub fn open(installed: &Path, plan: &Plan, blobs: &Path) -> Result<(Self, Applied), Error> {
        let mut file = File::open(installed)?;
        let size = file.metadata()?.len();
        let mut applied = Applied::default();
        let mut opened = Vec::new();
        let mut parts = Vec::new();
        for region in plan.regions.iter().filter(|r| r.path == RESOURCES_DB) {
            let name = &region.name;
            if region.file_size != size {
                return Err(plan_error(format!(
                    "{name}: the plan is for a {} of {} bytes; {} has {size}",
                    RESOURCES_DB,
                    region.file_size,
                    installed.display()
                )));
            }
            let end = region
                .offset
                .checked_add(region.size)
                .filter(|&end| end <= size)
                .ok_or_else(|| plan_error(format!("{name}: past the end of the file")))?;
            if digest_of(&mut file, region.offset, region.size)? != region.original {
                return Err(plan_error(format!(
                    "{name}: {} at {} is not the plan's original (sha256): another build",
                    installed.display(),
                    region.offset
                )));
            }
            opened.push(blob(blobs, &region.target, region.size, name)?);
            parts.push(Part {
                start: region.offset,
                end,
                source: Source::Blob(opened.len() - 1),
            });
            applied.regions.push(name.clone());
        }
        let mut len = size;
        let tails: Vec<_> = plan
            .tails
            .iter()
            .filter(|t| t.path == RESOURCES_DB)
            .collect();
        if tails.len() > 1 {
            return Err(plan_error(format!(
                "{RESOURCES_DB} has {} tails",
                tails.len()
            )));
        }
        if let Some(tail) = tails.first() {
            let name = &tail.name;
            if tail.file_size != size {
                return Err(plan_error(format!(
                    "{name}: the plan grows a {RESOURCES_DB} of {} bytes; {} has {size}",
                    tail.file_size,
                    installed.display()
                )));
            }
            if !applied.regions.contains(&tail.region) {
                return Err(plan_error(format!(
                    "{name}: its region {} is not in the plan",
                    tail.region
                )));
            }
            opened.push(blob(blobs, &tail.sha256, tail.size(), name)?);
            parts.push(Part {
                start: size,
                end: tail.new_size,
                source: Source::Blob(opened.len() - 1),
            });
            len = tail.new_size;
            applied.tail = Some(name.clone());
        }
        // The installed file fills what no blob does.
        parts.sort_by_key(|part| part.start);
        let mut whole = Vec::new();
        let mut at = 0;
        for part in parts {
            if part.start < at {
                return Err(plan_error("two regions overlap".into()));
            }
            if part.start > at {
                whole.push(Part {
                    start: at,
                    end: part.start,
                    source: Source::Installed,
                });
            }
            at = part.end;
            whole.push(part);
        }
        if at < size {
            whole.push(Part {
                start: at,
                end: size,
                source: Source::Installed,
            });
        }
        applied.size = len;
        let patched = Self {
            installed: file,
            blobs: opened,
            parts: whole,
            len,
            pos: 0,
        };
        Ok((patched, applied))
    }

    /// The size of the archive the players have.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Read for Patched {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.len {
            return Ok(0);
        }
        let i = self.parts.partition_point(|part| part.end <= self.pos);
        let part = &self.parts[i];
        let n = usize::try_from(part.end - self.pos)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let file = match part.source {
            Source::Installed => {
                self.installed.seek(SeekFrom::Start(self.pos))?;
                &mut self.installed
            }
            Source::Blob(blob) => {
                let file = &mut self.blobs[blob];
                file.seek(SeekFrom::Start(self.pos - part.start))?;
                file
            }
        };
        let n = file.read(&mut buf[..n])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Patched {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::End(by) => self.len.checked_add_signed(by),
            SeekFrom::Current(by) => self.pos.checked_add_signed(by),
        };
        self.pos = pos.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        Ok(self.pos)
    }
}

/// The archive the players run: `installed` with the plan of the players'
/// ZIP `zip` laid over it, the blobs in `blobs`.
pub fn open_installed(
    installed: &Path,
    zip: &Path,
    blobs: &Path,
) -> Result<(Archive<Patched>, Applied), Error> {
    let bytes = std::fs::read(zip)
        .map_err(|cause| plan_error(format!("cannot read {} ({cause})", zip.display())))?;
    let plan = plan_of_zip(&bytes)?;
    let (patched, applied) = Patched::open(installed, &plan, blobs)?;
    Ok((Archive::new(patched)?, applied))
}

#[cfg(test)]
pub(crate) mod tests;
