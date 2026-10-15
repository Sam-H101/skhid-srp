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
//! A path that is a file in one archive and a folder in another is refused:
//! what the game would show there is not known.

use std::collections::BTreeMap;
use std::io::{Read, Seek};

use crate::{Archive, Entry, Error};

/// Archives over one another: for each path the file of the last archive
/// that has one.
pub struct Overlay<R> {
    archives: Vec<Archive<R>>,
    /// Each file seen, by path: the archive it is from (its index) and its
    /// entry there.
    files: BTreeMap<Vec<u8>, (usize, Entry)>,
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

impl<R> Overlay<R> {
    /// `archives`, the first at the bottom; `names` names them in errors.
    pub fn new(archives: Vec<Archive<R>>, names: &[String]) -> Result<Self, Error> {
        let name = |index: usize| names.get(index).map_or("?", String::as_str);
        let mut files: BTreeMap<Vec<u8>, (usize, Entry)> = BTreeMap::new();
        for (index, archive) in archives.iter().enumerate() {
            for (path, entry) in &archive.files {
                // A folder of the path is a file below, or the path is a
                // folder below.
                let mut folder = path.clone();
                folder.push(b'/');
                let clash = path
                    .iter()
                    .enumerate()
                    .filter(|&(_, &byte)| byte == b'/')
                    .find_map(|(at, _)| files.get_key_value(&path[..at]))
                    .or_else(|| {
                        files
                            .range(folder.clone()..)
                            .next()
                            .filter(|(seen, _)| seen.starts_with(&folder))
                    });
                if let Some((seen, &(below, _))) = clash {
                    return Err(content(format!(
                        "{} has the file {}, {} has {} there: a file and a folder at one path",
                        name(index),
                        String::from_utf8_lossy(path),
                        name(below),
                        String::from_utf8_lossy(seen),
                    )));
                }
                files.insert(path.clone(), (index, *entry));
            }
        }
        Ok(Self { archives, files })
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
        self.files
            .get(path.as_bytes())
            .map(|(index, entry)| (*index, entry))
    }

    /// Every file seen whose path starts with `prefix`, in path order, with
    /// the index of its archive. A path that is not UTF-8 is left out.
    pub fn files_under<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = (&'a str, usize, &'a Entry)> + 'a {
        self.files
            .range(prefix.as_bytes().to_vec()..)
            .take_while(move |(path, _)| path.starts_with(prefix.as_bytes()))
            .filter_map(|(path, (index, entry))| {
                Some((std::str::from_utf8(path).ok()?, *index, entry))
            })
    }
}

impl<R: Read + Seek> Overlay<R> {
    /// The bytes of the file seen at `path`; `None` if no archive has one.
    pub fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, Error> {
        let Some(&(index, _)) = self.files.get(path.as_bytes()) else {
            return Ok(None);
        };
        self.archives[index].read(path)
    }
}

#[cfg(test)]
pub(crate) mod tests;
