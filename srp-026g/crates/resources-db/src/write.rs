// SPDX-License-Identifier: GPL-3.0-or-later
//! Writes an archive of the format [`crate::Archive`] reads, from files by
//! path: the same inputs give the same bytes.
//!
//! Only what an archive needs, after resource-porter's packer
//! (`deps/vostok-resources-db/src/pack.rs` at resource-porter 5d3b53e): the
//! mount root, folders and files, each folder's children in name order,
//! every node's own folder, the table's checksum of each file
//! ([`crate::crc32`]), the zeroed signature after the table, and the files'
//! bytes from the next 2048-byte boundary after it, in the table's order.
//! That boundary is this writer's choice (resource-porter's packer has it),
//! not something the engine asks for: it reads a file's bytes wherever its
//! node says they are. No hard links: files that [`write_to`] is told are
//! the same bytes ([`Stored::share`]) are separate nodes that point at one
//! copy of them.
//!
//! [`write()`] makes a small archive of bytes in memory (a patch);
//! [`write_to`] writes one whose files are read one at a time while it is
//! written, so a whole game archive (0.26g's 6.3 GB, a donor build's) can be
//! written again with some files changed, and empty folders kept.
//!
//! The layout is otherwise not 0.26g's (its packer orders and places nodes
//! and bytes its own way); it is like 0.26g's archive only in the two things
//! the game needs and a reader does not: a node's `m_parent` is the start of the
//! class of the folder it is in (the mount root's folder, at 936, for the
//! root's children), which the engine follows; and the 256 bytes after the
//! table are the archive's signature, which the 0.26g mounter reads
//! (revision 14 of the client checks none, so they are left zero).
//!
//! A path is at most [`crate::MAX_PATH`] bytes, the longest the game looks
//! up.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use crate::{
    ARCHIVE, Error, FILE_BASE, FLAGS, FOLDER, FOLDER_BASE, HEADER_SIZE, MAX_PATH, MOUNT_ROOT, NAME,
    NEXT, ROOT_BASE, ROOT_FIRST_CHILD, ROOT_NODE, crc32,
};

fn unwritable(what: impl Into<String>) -> Error {
    Error::Write(what.into())
}

/// `base_node`'s size: a node's class is its prefix, this, then its name.
const BASE_NODE: usize = 56;
/// `base_node::m_parent`: the start of the class of the node's folder.
const PARENT: usize = 32;
/// The mount root's `mount_type` and its value for an archive.
const MOUNT_TYPE: usize = 92;
const MOUNT_TYPE_ARCHIVE: u32 = 2;
/// The mount root's name field: a `string_path`.
const ROOT_NAME: usize = 260;
const NODE_ALIGN: usize = 8;
/// The signature's room after the table.
pub const SIGNATURE_SIZE: u64 = 256;
/// Where the files' bytes start: the next multiple of this after the table
/// and the signature.
const DATA_ALIGN: u64 = 2048;

enum Node<'a> {
    Folder(BTreeMap<&'a [u8], Node<'a>>),
    File(&'a str, Stored),
}

/// A file as the table has it, known before its bytes are written: its
/// size, its checksum ([`crate::crc32`]), and, if it is the same bytes as
/// other files, a key it shares with them. Files of one key are written
/// once, and must be of one size and checksum.
///
/// For files of an archive read, the key can be where their bytes are
/// ([`crate::Entry::offset`], and which archive): files at one offset are
/// one copy of their bytes, a hard link and its target. But an empty file
/// takes no bytes, so its offset is any other file's, often the next one's
/// (this writer gives it that); so a file of size 0 shares nothing, and its
/// key is not looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stored {
    pub size: u32,
    pub hash: u32,
    pub share: Option<u64>,
}

/// The bytes a node takes in the table: its prefix, `base_node`, its name
/// field, to the next 8 bytes.
fn node_size(prefix: usize, name_field: usize) -> usize {
    (prefix + BASE_NODE.max(NAME + name_field)).div_ceil(NODE_ALIGN) * NODE_ALIGN
}

fn table_size(children: &BTreeMap<&[u8], Node<'_>>) -> usize {
    children
        .iter()
        .map(|(name, child)| match child {
            Node::Folder(grandchildren) => {
                node_size(FOLDER_BASE, name.len() + 1) + table_size(grandchildren)
            }
            Node::File(..) => node_size(FILE_BASE, name.len() + 1),
        })
        .sum()
}

struct Writer<'a> {
    table: Vec<u8>,
    /// The files whose bytes are written, in the order their nodes are.
    payloads: Vec<(&'a str, Stored)>,
    /// Where the bytes of each share key went.
    shared: BTreeMap<u64, (u64, &'a str, Stored)>,
    /// Where the next file's bytes go.
    pos: u64,
    nodes: u32,
}

impl<'a> Writer<'a> {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.table[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A node of `prefix` bytes before its `base_node`, named `name`, in
    /// the folder whose class starts at `parent`, at the table's end: where
    /// its `base_node` is.
    fn node(
        &mut self,
        prefix: usize,
        name: &[u8],
        name_field: usize,
        flags: u16,
        parent: usize,
    ) -> usize {
        let start = self.table.len();
        self.table.resize(start + node_size(prefix, name_field), 0);
        let base = start + prefix;
        self.put(base + PARENT, &(parent as u64).to_le_bytes());
        self.put(base + FLAGS, &flags.to_le_bytes());
        self.put(base + NAME, name);
        self.nodes += 1;
        base
    }

    /// The children of the folder whose class starts at `parent`, depth
    /// first, in name order: the first one's `base_node`.
    fn children(
        &mut self,
        children: &BTreeMap<&'a [u8], Node<'a>>,
        parent: usize,
    ) -> Result<usize, Error> {
        let mut first = 0;
        let mut previous: Option<usize> = None;
        for (name, child) in children {
            let base = match child {
                Node::Folder(grandchildren) => {
                    let base =
                        self.node(FOLDER_BASE, name, name.len() + 1, FOLDER | ARCHIVE, parent);
                    let first_child = self.children(grandchildren, base - FOLDER_BASE)?;
                    self.put(base - FOLDER_BASE, &(first_child as u64).to_le_bytes());
                    base
                }
                Node::File(path, stored) => {
                    let base = self.node(FILE_BASE, name, name.len() + 1, ARCHIVE, parent);
                    let start = base - FILE_BASE;
                    // An empty file takes no bytes: it shares none.
                    let share = stored.share.filter(|_| stored.size != 0);
                    let at = match share.and_then(|key| self.shared.get(&key)) {
                        Some(&(at, first, first_stored)) => {
                            if (first_stored.size, first_stored.hash) != (stored.size, stored.hash)
                            {
                                return Err(unwritable(format!(
                                    "{path} and {first} share their bytes but differ in size or checksum"
                                )));
                            }
                            at
                        }
                        None => {
                            let at = self.pos;
                            if let Some(key) = share {
                                self.shared.insert(key, (at, path, *stored));
                            }
                            self.pos += u64::from(stored.size);
                            self.payloads.push((path, *stored));
                            at
                        }
                    };
                    self.put(start, &stored.size.to_le_bytes());
                    self.put(start + 8, &at.to_le_bytes());
                    self.put(start + 16, &stored.hash.to_le_bytes());
                    base
                }
            };
            match previous {
                Some(previous) => self.put(previous + NEXT, &(base as u64).to_le_bytes()),
                None => first = base,
            }
            previous = Some(base);
        }
        Ok(first)
    }
}

/// The archive of `files` (path, bytes). A path is its folders' names and
/// its own, by `/`; no name may be empty, and no path may be a folder of
/// another.
pub fn write(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, Error> {
    let stored = files
        .iter()
        .map(|(path, bytes)| {
            let size =
                u32::try_from(bytes.len()).map_err(|_| unwritable(format!("{path}: over 4 GB")))?;
            let hash = crc32(bytes);
            Ok((
                path.clone(),
                Stored {
                    size,
                    hash,
                    share: None,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, Error>>()?;
    let mut out = Vec::new();
    write_to(&mut out, &stored, &BTreeSet::new(), |path| {
        Ok(files[path].clone())
    })?;
    Ok(out)
}

/// Writes to `out` the archive of `files` (path, as the table has it) and of
/// the folders `empty_folders` (a folder named there that holds a file of
/// `files` is written once, with it). Paths are as for [`write()`]. The table
/// goes first, so every file's size and checksum are given; its bytes are
/// asked of `bytes` (by path) when their turn comes, once per share key, and
/// must be that size and checksum. Gives how many bytes were written.
pub fn write_to<W: Write>(
    out: &mut W,
    files: &BTreeMap<String, Stored>,
    empty_folders: &BTreeSet<String>,
    mut bytes: impl FnMut(&str) -> Result<Vec<u8>, Error>,
) -> Result<u64, Error> {
    let mut root = BTreeMap::new();
    fn names_of(path: &str) -> Result<Vec<&[u8]>, Error> {
        if path.len() > MAX_PATH {
            return Err(unwritable(format!(
                "{path}: a path of {} bytes, longer than the game takes ({MAX_PATH})",
                path.len()
            )));
        }
        let names: Vec<&[u8]> = path.split('/').map(str::as_bytes).collect();
        if names
            .iter()
            .any(|name| name.is_empty() || name.contains(&0))
        {
            return Err(unwritable(format!("{path}: not a path of an archive")));
        }
        Ok(names)
    }
    fn folder_of<'a, 'b>(
        mut folder: &'b mut BTreeMap<&'a [u8], Node<'a>>,
        names: &[&'a [u8]],
        path: &str,
    ) -> Result<&'b mut BTreeMap<&'a [u8], Node<'a>>, Error> {
        for name in names {
            let node = folder
                .entry(*name)
                .or_insert_with(|| Node::Folder(BTreeMap::new()));
            folder = match node {
                Node::Folder(children) => children,
                Node::File(..) => return Err(unwritable(format!("{path}: a file is in its way"))),
            };
        }
        Ok(folder)
    }
    for (path, stored) in files {
        let names = names_of(path)?;
        let (file, folders) = names.split_last().expect("split gives one name at least");
        let folder = folder_of(&mut root, folders, path)?;
        if folder.insert(*file, Node::File(path, *stored)).is_some() {
            return Err(unwritable(format!(
                "{path}: a folder of that name is there"
            )));
        }
    }
    for path in empty_folders {
        folder_of(&mut root, &names_of(path)?, path)?;
    }

    let size = node_size(ROOT_BASE, ROOT_NAME) + table_size(&root);
    let table_size = u32::try_from(size).map_err(|_| unwritable("the table is over 4 GB"))?;
    let origin =
        (HEADER_SIZE + u64::from(table_size) + SIGNATURE_SIZE).div_ceil(DATA_ALIGN) * DATA_ALIGN;
    let mut writer = Writer {
        table: Vec::with_capacity(size),
        payloads: Vec::new(),
        shared: BTreeMap::new(),
        pos: origin,
        nodes: 0,
    };
    // The mount root is in no folder; its own folder's class starts at its
    // first child's pointer.
    let base = writer.node(ROOT_BASE, b"", ROOT_NAME, FOLDER | ARCHIVE | MOUNT_ROOT, 0);
    writer.put(ROOT_NODE, &(base as u64).to_le_bytes());
    writer.put(MOUNT_TYPE, &MOUNT_TYPE_ARCHIVE.to_le_bytes());
    let first = writer.children(&root, ROOT_FIRST_CHILD)?;
    writer.put(ROOT_FIRST_CHILD, &(first as u64).to_le_bytes());
    debug_assert_eq!(writer.table.len(), size);

    let mut head = Vec::with_capacity(origin as usize);
    head.extend_from_slice(b"little-endian\0\0\0");
    head.extend_from_slice(&writer.nodes.to_le_bytes());
    head.extend_from_slice(&table_size.to_le_bytes());
    head.extend_from_slice(&writer.table);
    head.resize(origin as usize, 0);
    out.write_all(&head)?;
    for (path, stored) in writer.payloads {
        let payload = bytes(path)?;
        if payload.len() != stored.size as usize || crc32(&payload) != stored.hash {
            return Err(unwritable(format!(
                "{path}: its bytes are not the size and checksum given for it"
            )));
        }
        out.write_all(&payload)?;
    }
    Ok(writer.pos)
}

#[cfg(test)]
mod tests;
