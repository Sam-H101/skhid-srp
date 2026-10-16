// SPDX-License-Identifier: GPL-3.0-or-later
//! A patch from a folder: `srp-game-data pack`. Every file under the folder
//! goes into an archive ([`crate::write`]) at its path relative to the
//! folder, `/` between names, so `<folder>/gameplay/items/x.options` is
//! `gameplay/items/x.options` of the archive, the path the game looks it up
//! by. The archive is then read back and every file compared with the
//! folder's.
//!
//! Refused, rather than guessed at: a symbolic link or anything else that is
//! not a plain file or folder, a name that is not UTF-8 or has a `\`, an
//! empty folder (the writer makes none), and a folder with no file at all.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;

use crate::{Archive, Error, write::write};

/// Files by their archive path, with their bytes.
pub type Files = BTreeMap<String, Vec<u8>>;

fn content(what: impl Into<String>) -> Error {
    Error::Content(what.into())
}

/// Every file under `dir`, by its archive path, with its bytes.
pub fn files_of(dir: &Path) -> Result<Files, Error> {
    let mut files = BTreeMap::new();
    if !std::fs::metadata(dir)
        .map_err(|cause| content(format!("{}: {cause}", dir.display())))?
        .is_dir()
    {
        return Err(content(format!("{}: not a folder", dir.display())));
    }
    add(dir, "", &mut files)?;
    if files.is_empty() {
        return Err(content(format!("{}: no file in it", dir.display())));
    }
    Ok(files)
}

fn add(dir: &Path, prefix: &str, files: &mut Files) -> Result<(), Error> {
    let at = |path: &Path, cause: std::io::Error| content(format!("{}: {cause}", path.display()));
    let mut empty = true;
    for entry in std::fs::read_dir(dir).map_err(|cause| at(dir, cause))? {
        let entry = entry.map_err(|cause| at(dir, cause))?;
        let path = entry.path();
        empty = false;
        let name = entry
            .file_name()
            .into_string()
            .ok()
            .filter(|name| !name.contains('\\'))
            .ok_or_else(|| {
                content(format!(
                    "{}: a name of UTF-8 without `\\` is needed",
                    path.display()
                ))
            })?;
        let archive_path = format!("{prefix}{name}");
        let kind = entry.file_type().map_err(|cause| at(&path, cause))?;
        if kind.is_dir() {
            add(&path, &format!("{archive_path}/"), files)?;
        } else if kind.is_file() {
            let bytes = std::fs::read(&path).map_err(|cause| at(&path, cause))?;
            files.insert(archive_path, bytes);
        } else {
            return Err(content(format!(
                "{}: not a plain file or folder (a link?)",
                path.display()
            )));
        }
    }
    if empty && !prefix.is_empty() {
        return Err(content(format!(
            "{}: an empty folder (an archive written here has none; remove it)",
            dir.display()
        )));
    }
    Ok(())
}

/// The archive of the folder `dir`, read back and checked against it: its
/// bytes and the files' paths.
pub fn pack(dir: &Path) -> Result<(Vec<u8>, Files), Error> {
    let files = files_of(dir)?;
    let bytes = write(&files)?;
    check(&bytes, &files)?;
    Ok((bytes, files))
}

/// Whether `bytes`, read as an archive, has exactly `files`.
pub fn check(bytes: &[u8], files: &Files) -> Result<(), Error> {
    let mut archive = Archive::new(Cursor::new(bytes))?;
    if archive.len() != files.len() || archive.empty_folders().next().is_some() {
        return Err(content(format!(
            "the archive read back has {} files, not {}",
            archive.len(),
            files.len()
        )));
    }
    for (path, expected) in files {
        match archive.read(path)? {
            Some(found) if found == *expected => {}
            Some(_) => return Err(content(format!("{path}: other bytes read back"))),
            None => return Err(content(format!("{path}: not in the archive read back"))),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
