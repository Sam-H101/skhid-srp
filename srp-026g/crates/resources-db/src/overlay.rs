// SPDX-License-Identifier: GPL-3.0-or-later
//! Archives laid over one another in order, as the game mounts
//! `resources.db` and then each `resources.db.patch_<n>` at the same virtual
//! path (`docs/notes/client_patch.md`, "resources.db's patches"): a later
//! archive's file replaces an earlier one's of the same path, its new files
//! are added, folders merge, and nothing is taken away.
//!
//! Only the archives' tables are read to lay them over one another; a file's
//! bytes are read from the one archive whose file is seen, when asked for.
//!
//! A path that is a file in one archive and a folder in another, an empty
//! one too, is refused: what the game would show there is not known.

use std::collections::BTreeMap;
use std::io::{Read, Seek};
use std::ops::Bound;

use crate::{Archive, Entry, Error};

/// Archives over one another: for each path the file of the last archive
/// that has one.
pub struct Overlay<R> {
    archives: Vec<Archive<R>>,
    /// Each file seen, by path: the archive it is from (its index) and its
    /// entry there.
    files: BTreeMap<String, (usize, Entry)>,
    /// The folders that hold nothing, of every archive, and the folders
    /// above them, are folders: the empty ones seen.
    empty_folders: Vec<String>,
}

impl<R> std::fmt::Debug for Overlay<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlay")
            .field("archives", &self.archives.len())
            .field("files", &self.files.len())
            .finish()
    }
}

fn content(what: String) -> Error {
    Error::Content(what)
}

/// The first path of `paths` under `path` (`path/...`).
fn under<'a, V>(paths: &'a BTreeMap<String, V>, path: &str) -> Option<(&'a String, &'a V)> {
    let folder = format!("{path}/");
    paths
        .range::<str, _>((Bound::Included(folder.as_str()), Bound::Unbounded))
        .next()
        .filter(|(seen, _)| seen.starts_with(&folder))
}

/// `path` in `paths`, or the first path under it.
fn at_or_under<'a, V>(paths: &'a BTreeMap<String, V>, path: &str) -> Option<(&'a String, &'a V)> {
    paths.get_key_value(path).or_else(|| under(paths, path))
}

/// The first folder of `path` (a path above it) that is in `paths`.
fn above<'a, V>(paths: &'a BTreeMap<String, V>, path: &str) -> Option<(&'a String, &'a V)> {
    path.match_indices('/')
        .find_map(|(at, _)| paths.get_key_value(&path[..at]))
}

impl<R> Overlay<R> {
    /// `archives`, the first at the bottom; `names` names them in errors.
    pub fn new(archives: Vec<Archive<R>>, names: &[String]) -> Result<Self, Error> {
        let name = |index: usize| names.get(index).map_or("?", String::as_str);
        let mut files: BTreeMap<String, (usize, Entry)> = BTreeMap::new();
        // Every empty folder of an archive below, and that archive.
        let mut empty: BTreeMap<String, usize> = BTreeMap::new();
        let clash =
            |index: usize, what: &str, path: &str, below: usize, seen: &str, there: &str| {
                content(format!(
                    "{} has the {what} {path}, {} has the {there} {seen} there: a file and a \
                 folder at one path",
                    name(index),
                    name(below),
                ))
            };
        for (index, archive) in archives.iter().enumerate() {
            for (path, entry) in &archive.files {
                // A folder of the path is a file below, or the path is a
                // folder below: one with files, or an empty one.
                if let Some((seen, &(below, _))) =
                    above(&files, path).or_else(|| under(&files, path))
                {
                    return Err(clash(index, "file", path, below, seen, "file"));
                }
                if let Some((seen, &below)) = at_or_under(&empty, path) {
                    return Err(clash(index, "file", path, below, seen, "empty folder"));
                }
                files.insert(path.clone(), (index, *entry));
            }
            for path in &archive.empty_folders {
                // The folder, or one it is in, is a file below.
                if let Some((seen, &(below, _))) = files
                    .get_key_value(path.as_str())
                    .or_else(|| above(&files, path))
                {
                    return Err(clash(index, "empty folder", path, below, seen, "file"));
                }
                empty.insert(path.clone(), index);
            }
        }
        // An empty folder of one archive may hold files, or another empty
        // folder, of another.
        let empty_folders = empty
            .keys()
            .filter(|path| under(&files, path).is_none() && under(&empty, path).is_none())
            .cloned()
            .collect();
        Ok(Self {
            archives,
            files,
            empty_folders,
        })
    }

    /// How many files are seen.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The file seen at `path`: the index of its archive and its entry there.
    pub fn entry(&self, path: &str) -> Option<(usize, &Entry)> {
        self.files.get(path).map(|(index, entry)| (*index, entry))
    }

    /// Every file seen whose path starts with `prefix`, in path order, with
    /// the index of its archive.
    pub fn files_under<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = (&'a str, usize, &'a Entry)> + 'a {
        self.files
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .take_while(move |(path, _)| path.starts_with(prefix))
            .map(|(path, (index, entry))| (path.as_str(), *index, entry))
    }

    /// The folders seen that hold nothing, in path order: an empty folder of
    /// an archive that no archive puts a file or a folder in.
    pub fn empty_folders(&self) -> impl Iterator<Item = &str> {
        self.empty_folders.iter().map(String::as_str)
    }
}

impl<R: Read + Seek> Overlay<R> {
    /// The bytes of the file seen at `path`; `None` if no archive has one.
    pub fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        let Some(&(index, _)) = self.files.get(path) else {
            return Ok(None);
        };
        self.archives[index].read(path)
    }
}

#[cfg(test)]
pub(crate) mod tests;
