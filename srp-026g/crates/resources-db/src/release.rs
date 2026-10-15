// SPDX-License-Identifier: GPL-3.0-or-later
//! The players' update plan of a release, read off the release's final
//! `resources.db`: what differs from 0.26g's, as the updater's `region` and
//! `tail` lines (`client/updater/README.md`, "Resources").
//!
//! Both files are read in chunks, side by side. Every byte that differs must
//! be in a file of the original's table, or in its header, table, table
//! signature and what follows them before the first file:
//! - when the table differs, the head: `[0, H)`, the header, the new table
//!   and its 256-byte signature, to the end of the last original file they
//!   cover (a release moves those to its end). Its region is
//!   `resources_head`;
//! - every other file that differs, at the same place and size, is a region
//!   named after its file (`flash_movies/lobby_menu.swf`: `lobby_menu`);
//!   with the table unchanged, each must keep the table's CRC-32 of the
//!   original, or the game refuses it;
//! - what the final file has past the original's end is the tail,
//!   `resources_tail`, written with the head; a file that grows needs a new
//!   table, and one that shrinks is refused.
//!
//! Anything else is an error. The same two files give the same plan.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use updater_logic::sha256::{Sha256, to_hex};

use crate::plan::RESOURCES_DB;
use crate::{Archive, Error, HEADER_SIZE, crc32};

/// The head region's name, and the tail's.
pub const HEAD: &str = "resources_head";
pub const TAIL: &str = "resources_tail";

const CHUNK: usize = 8 << 20;

/// A range of the original or the final file a blob is made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    pub sha256: String,
    /// In the final file (`false`: the original).
    pub in_final: bool,
    pub offset: u64,
    pub size: u64,
}

/// The plan of a release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Release {
    /// The plan's lines for `game/resources.db`.
    pub lines: Vec<String>,
    /// The blobs the lines name: each region's original and target, the tail.
    pub blobs: Vec<Blob>,
}

/// A file of the original's table.
#[derive(Clone, Debug)]
struct Placed {
    offset: u64,
    size: u64,
    hash: u32,
    path: String,
}

fn hex(digest: &[u8; 32]) -> String {
    String::from_utf8_lossy(&to_hex(digest)).into_owned()
}

fn sha256_of(file: &mut File, offset: u64, size: u64) -> Result<String, Error> {
    file.seek(SeekFrom::Start(offset))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; CHUNK];
    let mut left = size;
    while left > 0 {
        let n = buffer
            .len()
            .min(usize::try_from(left).unwrap_or(usize::MAX));
        file.read_exact(&mut buffer[..n])?;
        hasher.update(&buffer[..n]);
        left -= n as u64;
    }
    Ok(hex(&hasher.finish()))
}

fn read_at(file: &mut File, offset: u64, size: u64) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![0; usize::try_from(size).map_err(|_| crate::format("a file too large"))?];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Fills `buffer` from `file`, or to its end; the bytes read.
fn fill(file: &mut File, buffer: &mut [u8]) -> Result<usize, Error> {
    let mut done = 0;
    while done < buffer.len() {
        match file.read(&mut buffer[done..])? {
            0 => break,
            n => done += n,
        }
    }
    Ok(done)
}

/// The ranges of `[0, len)` where `original` and `final_` differ, merged.
fn differences(original: &mut File, final_: &mut File, len: u64) -> Result<Vec<(u64, u64)>, Error> {
    original.seek(SeekFrom::Start(0))?;
    final_.seek(SeekFrom::Start(0))?;
    let (mut a, mut b) = (vec![0; CHUNK], vec![0; CHUNK]);
    let mut runs: Vec<(u64, u64)> = Vec::new();
    let mut at = 0u64;
    while at < len {
        let want = CHUNK.min(usize::try_from(len - at).unwrap_or(CHUNK));
        let (n, m) = (
            fill(original, &mut a[..want])?,
            fill(final_, &mut b[..want])?,
        );
        if n != want || m != want {
            return Err(crate::format("a file ended while it was compared"));
        }
        if a[..want] != b[..want] {
            let mut i = 0;
            while i < want {
                if a[i] == b[i] {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < want && a[i] != b[i] {
                    i += 1;
                }
                let (s, e) = (at + start as u64, at + i as u64);
                match runs.last_mut() {
                    Some(last) if last.1 == s => last.1 = e,
                    _ => runs.push((s, e)),
                }
            }
        }
        at += want as u64;
    }
    Ok(runs)
}

/// The plan's name of the file at `path`: its name without the extension,
/// in the characters a plan name may have; the whole path if that is taken.
fn region_name(path: &str, taken: &BTreeSet<String>) -> String {
    let clean = |text: &str| -> String {
        text.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    let mut name = clean(stem);
    if name.is_empty() || taken.contains(&name) || name == HEAD || name == TAIL {
        name = clean(path);
    }
    name.chars().take(64).collect()
}

/// The plan that makes `final_db` of the original `original_db`.
pub fn release(original_db: &Path, final_db: &Path) -> Result<Release, Error> {
    let mut original = File::open(original_db)?;
    let mut final_ = File::open(final_db)?;
    let (original_size, final_size) = (original.metadata()?.len(), final_.metadata()?.len());
    if final_size < original_size {
        return Err(crate::format(format!(
            "{} is {final_size} bytes, shorter than the original's {original_size}: a release only \
             changes bytes in place and appends",
            final_db.display()
        )));
    }
    // The original's files, by place: a hard link is the same place twice.
    let mut files: Vec<Placed> = Archive::new(File::open(original_db)?)?
        .files_under("")
        .filter(|(_, entry)| entry.size > 0)
        .map(|(path, entry)| Placed {
            offset: entry.offset,
            size: u64::from(entry.size),
            hash: entry.hash,
            path: path.to_owned(),
        })
        .collect();
    files.sort_by(|a, b| a.offset.cmp(&b.offset).then_with(|| a.path.cmp(&b.path)));
    files.dedup_by_key(|file| file.offset);
    for pair in files.windows(2) {
        if pair[0].offset + pair[0].size > pair[1].offset {
            return Err(crate::format(format!(
                "{} and {} overlap in the original",
                pair[0].path, pair[1].path
            )));
        }
    }
    // The final file must be an archive too.
    Archive::new(File::open(final_db)?)?;
    let first = files.first().map_or(original_size, |file| file.offset);
    let table_end = |file: &mut File| -> Result<u64, Error> {
        let header = read_at(file, 0, HEADER_SIZE)?;
        Ok(HEADER_SIZE
            + u64::from(u32::from_le_bytes(
                header[20..24].try_into().expect("4 bytes"),
            )))
    };
    let (original_table, final_table) = (table_end(&mut original)?, table_end(&mut final_)?);

    let runs = differences(&mut original, &mut final_, original_size)?;
    let head_changed = runs.first().is_some_and(|run| run.0 < first);
    // The header, the new table and its signature, and every original file
    // they cover (a release moves those to its end).
    let mut head_end = 0;
    if head_changed {
        head_end = first.max(final_table + 256);
        for file in files
            .iter()
            .take_while(|file| file.offset < final_table + 256)
        {
            head_end = head_end.max(file.offset + file.size);
        }
    } else if original_table != final_table {
        return Err(crate::format(
            "the file table's size differs, but none of its bytes do",
        ));
    }
    // The other files each run touches.
    let mut touched = BTreeSet::new();
    for &(start, end) in &runs {
        let mut at = start.max(first).max(head_end);
        while at < end {
            let i = files.partition_point(|file| file.offset + file.size <= at);
            match files.get(i) {
                Some(file) if file.offset <= at => {
                    touched.insert(i);
                    at = file.offset + file.size;
                }
                _ => {
                    return Err(crate::format(format!(
                        "{} differs from the original at {at}, outside every file of its table",
                        final_db.display()
                    )));
                }
            }
        }
    }
    let mut out = Release::default();
    let mut taken = BTreeSet::new();
    let region = |out: &mut Release,
                  original: &mut File,
                  final_: &mut File,
                  offset: u64,
                  size: u64,
                  name: &str|
     -> Result<(), Error> {
        let before = sha256_of(original, offset, size)?;
        let after = sha256_of(final_, offset, size)?;
        out.lines.push(format!(
            "region {RESOURCES_DB} {original_size} {offset} {size} {before} {after} {name}"
        ));
        out.blobs.push(Blob {
            sha256: after,
            in_final: true,
            offset,
            size,
        });
        out.blobs.push(Blob {
            sha256: before,
            in_final: false,
            offset,
            size,
        });
        Ok(())
    };
    if head_changed {
        region(&mut out, &mut original, &mut final_, 0, head_end, HEAD)?;
        taken.insert(HEAD.to_owned());
    }
    for i in touched {
        let file = &files[i];
        if !head_changed {
            let bytes = read_at(&mut final_, file.offset, file.size)?;
            if crc32(&bytes) != file.hash {
                return Err(crate::format(format!(
                    "{}: it differs from the original but keeps the original table, which names \
                     CRC-32 {:08x}, and its bytes have {:08x}: the game would refuse it",
                    file.path,
                    file.hash,
                    crc32(&bytes)
                )));
            }
        }
        let name = region_name(&file.path, &taken);
        taken.insert(name.clone());
        region(
            &mut out,
            &mut original,
            &mut final_,
            file.offset,
            file.size,
            &name,
        )?;
    }
    if final_size > original_size {
        if !head_changed {
            return Err(crate::format(format!(
                "{} is longer than the original, but its file table is the original's: nothing \
                 would read what it appends",
                final_db.display()
            )));
        }
        let size = final_size - original_size;
        let sha = sha256_of(&mut final_, original_size, size)?;
        out.lines.push(format!(
            "tail {RESOURCES_DB} {original_size} {final_size} {sha} {TAIL} {HEAD}"
        ));
        out.blobs.push(Blob {
            sha256: sha,
            in_final: true,
            offset: original_size,
            size,
        });
    }
    Ok(out)
}

/// Writes each blob of `release` into `folder` as `<sha256>`; a blob that is
/// there must have its bytes. The names of those it wrote.
pub fn write_blobs(
    release: &Release,
    original_db: &Path,
    final_db: &Path,
    folder: &Path,
) -> Result<Vec<String>, Error> {
    std::fs::create_dir_all(folder)?;
    let (mut original, mut final_) = (File::open(original_db)?, File::open(final_db)?);
    let mut written = Vec::new();
    for blob in &release.blobs {
        let path = folder.join(&blob.sha256);
        let source = if blob.in_final {
            &mut final_
        } else {
            &mut original
        };
        if path.exists() {
            let mut there = File::open(&path)?;
            let len = there.metadata()?.len();
            if len != blob.size || sha256_of(&mut there, 0, len)? != blob.sha256 {
                return Err(crate::format(format!(
                    "{}: already there with other bytes",
                    path.display()
                )));
            }
            continue;
        }
        let part = path.with_extension("part");
        {
            let mut out = File::create(&part)?;
            source.seek(SeekFrom::Start(blob.offset))?;
            std::io::copy(&mut source.by_ref().take(blob.size), &mut out)?;
            out.sync_all()?;
        }
        std::fs::rename(&part, &path)?;
        written.push(blob.sha256.clone());
    }
    Ok(written)
}

/// Writes `bytes` over the file `path` of the archive `db`, in place: it
/// must be that file's size, and its CRC-32 the one the archive's table
/// names, or the game would refuse it. How a release's final `resources.db`
/// gets a file made outside it (a menu: `exchange_026g.py`, ...).
pub fn put(db: &Path, path: &str, bytes: &[u8]) -> Result<(), Error> {
    let entry = *Archive::new(File::open(db)?)?
        .entry(path)
        .ok_or_else(|| crate::format(format!("{} has no {path}", db.display())))?;
    if bytes.len() as u64 != u64::from(entry.size) {
        return Err(crate::format(format!(
            "{path}: {} bytes; the archive's is {}: it must be the same size",
            bytes.len(),
            entry.size
        )));
    }
    if crc32(bytes) != entry.hash {
        return Err(crate::format(format!(
            "{path}: its CRC-32 is {:08x}, the table names {:08x}: the game would refuse it",
            crc32(bytes),
            entry.hash
        )));
    }
    let mut file = std::fs::OpenOptions::new().write(true).open(db)?;
    file.seek(SeekFrom::Start(entry.offset))?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests;
