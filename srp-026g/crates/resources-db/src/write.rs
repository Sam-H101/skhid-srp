// SPDX-License-Identifier: GPL-3.0-or-later
//! Writes an archive of the format [`crate::Archive`] reads, from files by
//! path: the same inputs give the same bytes.
//!
//! Only what a small archive needs, after resource-porter's packer
//! (`deps/vostok-resources-db/src/pack.rs` at resource-porter 5d3b53e): the
//! mount root, folders and files, each folder's children in name order,
//! every file's bytes once (no hard links, no shared payloads), the table's
//! checksum of each ([`crate::crc32`]), and the files' bytes after the table
//! from the next 2048-byte boundary on.

use std::collections::BTreeMap;

use crate::{
    ARCHIVE, Error, FILE_BASE, FLAGS, FOLDER, FOLDER_BASE, HEADER_SIZE, MOUNT_ROOT, NAME, NEXT,
    ROOT_BASE, ROOT_FIRST_CHILD, ROOT_NODE, crc32, format,
};

/// `base_node`'s size: a node's class is its prefix, this, then its name.
const BASE_NODE: usize = 56;
/// `base_node::m_parent`; the engine writes the mount root's folder there
/// for every node.
const PARENT: usize = 32;
/// The mount root's `mount_type` and its value for an archive.
const MOUNT_TYPE: usize = 92;
const MOUNT_TYPE_ARCHIVE: u32 = 2;
/// The mount root's name field: a `string_path`.
const ROOT_NAME: usize = 260;
const NODE_ALIGN: usize = 8;
/// Where the files' bytes start: the next multiple of this after the table.
const DATA_ALIGN: u64 = 2048;

enum Node<'a> {
    Folder(BTreeMap<&'a [u8], Node<'a>>),
    File(&'a [u8]),
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
            Node::File(_) => node_size(FILE_BASE, name.len() + 1),
        })
        .sum()
}

struct Writer<'a> {
    table: Vec<u8>,
    /// The files' bytes, in the order their nodes are written.
    payloads: Vec<&'a [u8]>,
    /// Where the next file's bytes go.
    pos: u64,
    nodes: u32,
}

impl<'a> Writer<'a> {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.table[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A node of `prefix` bytes before its `base_node`, named `name`, at
    /// the table's end: where its `base_node` is.
    fn node(&mut self, prefix: usize, name: &[u8], name_field: usize, flags: u16) -> usize {
        let start = self.table.len();
        self.table.resize(start + node_size(prefix, name_field), 0);
        let base = start + prefix;
        self.put(base + PARENT, &(ROOT_FIRST_CHILD as u64).to_le_bytes());
        self.put(base + FLAGS, &flags.to_le_bytes());
        self.put(base + NAME, name);
        self.nodes += 1;
        base
    }

    /// The children of a folder, depth first, in name order: the first
    /// one's `base_node`.
    fn children(&mut self, children: &BTreeMap<&'a [u8], Node<'a>>) -> usize {
        let mut first = 0;
        let mut previous: Option<usize> = None;
        for (name, child) in children {
            let base = match child {
                Node::Folder(grandchildren) => {
                    let base = self.node(FOLDER_BASE, name, name.len() + 1, FOLDER | ARCHIVE);
                    let first_child = self.children(grandchildren);
                    self.put(base - FOLDER_BASE, &(first_child as u64).to_le_bytes());
                    base
                }
                Node::File(bytes) => {
                    let base = self.node(FILE_BASE, name, name.len() + 1, ARCHIVE);
                    let start = base - FILE_BASE;
                    self.put(start, &(bytes.len() as u32).to_le_bytes());
                    self.put(start + 8, &self.pos.to_le_bytes());
                    self.put(start + 16, &crc32(bytes).to_le_bytes());
                    self.pos += bytes.len() as u64;
                    self.payloads.push(bytes);
                    base
                }
            };
            match previous {
                Some(previous) => self.put(previous + NEXT, &(base as u64).to_le_bytes()),
                None => first = base,
            }
            previous = Some(base);
        }
        first
    }
}

/// The archive of `files` (path, bytes). A path is its folders' names and
/// its own, by `/`; no name may be empty, and no path may be a folder of
/// another.
pub fn write(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, Error> {
    let mut root = BTreeMap::new();
    for (path, bytes) in files {
        let names: Vec<&[u8]> = path.split('/').map(str::as_bytes).collect();
        if names
            .iter()
            .any(|name| name.is_empty() || name.contains(&0))
        {
            return Err(format(format!("{path}: not a path of an archive")));
        }
        let (file, folders) = names.split_last().expect("split gives one name at least");
        let mut folder = &mut root;
        for name in folders {
            let node = folder
                .entry(*name)
                .or_insert_with(|| Node::Folder(BTreeMap::new()));
            folder = match node {
                Node::Folder(children) => children,
                Node::File(_) => return Err(format(format!("{path}: a file is in its way"))),
            };
        }
        if folder.insert(*file, Node::File(bytes)).is_some() {
            return Err(format(format!("{path}: a folder of that name is there")));
        }
    }

    let size = node_size(ROOT_BASE, ROOT_NAME) + table_size(&root);
    let table_size = u32::try_from(size).map_err(|_| format("the table is over 4 GB"))?;
    let origin = (HEADER_SIZE + u64::from(table_size)).div_ceil(DATA_ALIGN) * DATA_ALIGN;
    let mut writer = Writer {
        table: Vec::with_capacity(size),
        payloads: Vec::new(),
        pos: origin,
        nodes: 0,
    };
    let base = writer.node(ROOT_BASE, b"", ROOT_NAME, FOLDER | ARCHIVE | MOUNT_ROOT);
    writer.put(ROOT_NODE, &(base as u64).to_le_bytes());
    writer.put(MOUNT_TYPE, &MOUNT_TYPE_ARCHIVE.to_le_bytes());
    writer.put(base + PARENT, &0u64.to_le_bytes());
    let first = writer.children(&root);
    writer.put(ROOT_FIRST_CHILD, &(first as u64).to_le_bytes());
    debug_assert_eq!(writer.table.len(), size);

    let mut out = Vec::with_capacity(usize::try_from(writer.pos).unwrap_or(0));
    out.extend_from_slice(b"little-endian\0\0\0");
    out.extend_from_slice(&writer.nodes.to_le_bytes());
    out.extend_from_slice(&table_size.to_le_bytes());
    out.extend_from_slice(&writer.table);
    out.resize(origin as usize, 0);
    for payload in writer.payloads {
        out.extend_from_slice(payload);
    }
    Ok(out)
}
