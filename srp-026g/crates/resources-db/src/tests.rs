// SPDX-License-Identifier: GPL-3.0-or-later
use std::io::{Cursor, Read};

use super::*;

/// Five files packed by resource-porter's `resources-db pack` (the packer of
/// the archives the porter makes for players): `gameplay/first.txt`
/// ("first file\n"), `gameplay/items/item.options` and `same_bytes.options`
/// ("an item\n" both), `gameplay/items/big.bin` (5000 'x'), and an empty
/// `projects/level_01/day/client_project`.
const FIXTURE: &[u8] = include_bytes!("../tests/fixture.db");

fn open(bytes: &[u8]) -> Result<Archive<Cursor<Vec<u8>>>, Error> {
    Archive::new(Cursor::new(bytes.to_vec()))
}

/// Where the node named `name` has its `base_node` in `bytes`.
fn node_of(bytes: &[u8], name: &str) -> usize {
    let mut needle = name.as_bytes().to_vec();
    needle.push(0);
    let at = bytes
        .windows(needle.len())
        .position(|w| w == needle.as_slice())
        .expect("the fixture names it");
    at - NAME
}

fn table_offset(bytes: &[u8], name: &str) -> usize {
    node_of(bytes, name) - HEADER_SIZE as usize
}

#[test]
fn the_files_of_an_archive_are_listed_and_read() {
    let mut archive = open(FIXTURE).unwrap();
    assert_eq!(archive.len(), 5);
    let paths: Vec<&str> = archive.files_under("").map(|(path, _)| path).collect();
    assert_eq!(
        paths,
        [
            "gameplay/first.txt",
            "gameplay/items/big.bin",
            "gameplay/items/item.options",
            "gameplay/items/same_bytes.options",
            "projects/level_01/day/client_project",
        ]
    );
    assert_eq!(
        archive.read("gameplay/first.txt").unwrap().unwrap(),
        b"first file\n"
    );
    assert_eq!(
        archive.read("gameplay/items/big.bin").unwrap().unwrap(),
        vec![b'x'; 5000]
    );
    assert_eq!(
        archive
            .read("gameplay/items/same_bytes.options")
            .unwrap()
            .unwrap(),
        b"an item\n"
    );
    assert_eq!(
        archive
            .read("projects/level_01/day/client_project")
            .unwrap()
            .unwrap(),
        b""
    );
    assert_eq!(archive.read("gameplay/missing").unwrap(), None);
    assert_eq!(
        archive.read("gameplay").unwrap(),
        None,
        "a folder is no file"
    );
    assert_eq!(archive.entry("gameplay/items/big.bin").unwrap().size, 5000);
}

#[test]
fn files_under_a_prefix_are_those_of_its_folder_alone() {
    let archive = open(FIXTURE).unwrap();
    let items: Vec<&str> = archive
        .files_under("gameplay/items/")
        .map(|(path, _)| path)
        .collect();
    assert_eq!(
        items,
        [
            "gameplay/items/big.bin",
            "gameplay/items/item.options",
            "gameplay/items/same_bytes.options"
        ]
    );
    assert_eq!(archive.files_under("gameplay/x").count(), 0);
    assert_eq!(archive.empty_folders().count(), 0);
}

#[test]
fn a_file_that_is_not_an_archive_is_refused() {
    let mut bytes = FIXTURE.to_vec();
    bytes[..13].copy_from_slice(b"big-endian\0\0\0");
    assert!(matches!(open(&bytes), Err(Error::Format(_))));
    assert!(matches!(open(&FIXTURE[..10]), Err(Error::Io(_))));
}

#[test]
fn a_file_cut_inside_its_table_or_its_files_is_an_error() {
    let table_end =
        HEADER_SIZE as usize + u32::from_le_bytes(FIXTURE[20..24].try_into().unwrap()) as usize;
    let message = open(&FIXTURE[..table_end - 1]).unwrap_err().to_string();
    assert!(message.contains("ends inside its table"), "{message}");
    // The table is whole, the last file's bytes are not.
    let mut archive = open(&FIXTURE[..FIXTURE.len() - 1]).unwrap();
    let message = archive
        .read("gameplay/items/same_bytes.options")
        .unwrap_err()
        .to_string();
    assert!(message.contains("past the end"), "{message}");
}

#[test]
fn a_damaged_table_is_an_error_not_a_panic() {
    // A sibling pointer out of the table.
    let mut bytes = FIXTURE.to_vec();
    let node = node_of(&bytes, "first.txt");
    bytes[node + NEXT..node + NEXT + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(
        open(&bytes)
            .unwrap_err()
            .to_string()
            .contains("leaves the table")
    );

    // A node that is its own next sibling.
    let mut bytes = FIXTURE.to_vec();
    let node = node_of(&bytes, "first.txt");
    let own = (table_offset(&bytes, "first.txt") as u64).to_le_bytes();
    bytes[node + NEXT..node + NEXT + 8].copy_from_slice(&own);
    let message = open(&bytes).unwrap_err().to_string();
    assert!(
        message.contains("twice") || message.contains("loop"),
        "{message}"
    );

    // A name with no end.
    let mut bytes = FIXTURE.to_vec();
    let size = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let end = HEADER_SIZE as usize + size;
    for byte in &mut bytes[node_of(FIXTURE, "first.txt") + NAME..end] {
        if *byte == 0 {
            *byte = b'a';
        }
    }
    assert!(open(&bytes).is_err());

    // Every byte of the table changed, one at a time: never a panic.
    for at in HEADER_SIZE as usize..end {
        let mut bytes = FIXTURE.to_vec();
        bytes[at] ^= 0xFF;
        if let Ok(mut archive) = open(&bytes) {
            let paths: Vec<String> = archive.files_under("").map(|(p, _)| p.to_owned()).collect();
            for path in paths {
                let _ = archive.read(&path);
            }
        }
    }
}

#[test]
fn a_node_this_reader_does_not_read_is_refused_by_name() {
    for flag in [1u16 << 4, 1 << 6, 1 << 8, 1 << 11, 1 << 12] {
        let mut bytes = FIXTURE.to_vec();
        let node = node_of(&bytes, "item.options");
        let flags = u16::from_le_bytes(bytes[node + FLAGS..node + FLAGS + 2].try_into().unwrap());
        bytes[node + FLAGS..node + FLAGS + 2].copy_from_slice(&(flags | flag).to_le_bytes());
        let message = open(&bytes).unwrap_err().to_string();
        assert!(message.contains("gameplay/items/item.options"), "{message}");
        assert!(message.contains("not read"), "{message}");
    }
}

/// Turns the file node `name` into a hard link to the node `target` (a file's
/// or a folder's name): a link's class keeps the pointer 8 bytes before its
/// `base_node`, where a file's keeps its hash.
fn linked(name: &str, target: &str) -> Vec<u8> {
    let mut bytes = FIXTURE.to_vec();
    let node = node_of(&bytes, name);
    let flags = u16::from_le_bytes(bytes[node + FLAGS..node + FLAGS + 2].try_into().unwrap());
    bytes[node + FLAGS..node + FLAGS + 2].copy_from_slice(&(flags | HARD_LINK).to_le_bytes());
    let pointer = (table_offset(&bytes, target) as u64).to_le_bytes();
    bytes[node - LINK_BASE..node].copy_from_slice(&pointer);
    bytes
}

#[test]
fn a_hard_link_reads_as_the_file_it_names() {
    let mut archive = open(&linked("same_bytes.options", "big.bin")).unwrap();
    assert_eq!(
        archive.entry("gameplay/items/same_bytes.options"),
        archive.entry("gameplay/items/big.bin")
    );
    assert_eq!(
        archive
            .read("gameplay/items/same_bytes.options")
            .unwrap()
            .unwrap(),
        vec![b'x'; 5000]
    );
}

#[test]
fn a_hard_link_to_a_folder_holds_that_folders_files() {
    let mut archive = open(&linked("first.txt", "items")).unwrap();
    assert_eq!(archive.read("gameplay/first.txt").unwrap(), None);
    assert_eq!(archive.files_under("gameplay/first.txt/").count(), 3);
    assert_eq!(
        archive
            .read("gameplay/first.txt/item.options")
            .unwrap()
            .unwrap(),
        b"an item\n"
    );
    // A link to a link is not followed.
    let mut bytes = linked("first.txt", "item.options");
    let node = node_of(&bytes, "item.options");
    let flags = u16::from_le_bytes(bytes[node + FLAGS..node + FLAGS + 2].try_into().unwrap());
    bytes[node + FLAGS..node + FLAGS + 2].copy_from_slice(&(flags | HARD_LINK).to_le_bytes());
    assert!(
        open(&bytes)
            .unwrap_err()
            .to_string()
            .contains("links to a node")
    );
}

/// 0.26g's own archive: `SRP_TEST_RESOURCES_DB=<…/game/resources.db>`.
#[test]
#[ignore = "needs 0.26g's resources.db"]
fn the_archive_of_0_26g_reads() {
    let path = std::env::var("SRP_TEST_RESOURCES_DB").expect("SRP_TEST_RESOURCES_DB");
    let started = std::time::Instant::now();
    let mut archive = Archive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let opened = started.elapsed();
    assert_eq!(archive.len(), 79_933);
    // A hard link reads as the file it names.
    let link = archive
        .read("models/level/03/flora/pra_big_03.model/settings")
        .unwrap();
    assert!(link.is_some_and(|bytes| !bytes.is_empty()));
    assert_eq!(archive.empty_folders().count(), 0);
    // Through a link to a folder: that folder's files at the link's path.
    assert_eq!(
        archive
            .files_under("sounds/single/material/step/rabitz_sprint/")
            .count(),
        156
    );
    assert_eq!(archive.files_under("gameplay/").count(), 508);
    let dictionary = archive
        .read("gameplay/db_static_dictionaries")
        .unwrap()
        .unwrap();
    assert_eq!(dictionary.len(), 139_509);
    eprintln!("opened in {opened:?}");

    // What `write` copies: each node names the folder it is in (the start
    // of that folder's class: 936, the mount root's folder, for the root's
    // children), and the signature takes the 256 bytes after the table.
    let size = u32::from_le_bytes(table_header(&path)[20..24].try_into().unwrap()) as usize;
    let mut bytes = vec![0; HEADER_SIZE as usize + size + 256];
    std::fs::File::open(&path)
        .unwrap()
        .read_exact(&mut bytes)
        .unwrap();
    let nodes = parents(&bytes);
    assert_eq!(nodes.len(), 95_215);
    for (node, parent, folder) in nodes {
        assert_eq!(parent, folder, "the node at {node}");
    }
    assert!(bytes[HEADER_SIZE as usize + size..].iter().any(|&b| b != 0));
}

fn table_header(path: &str) -> [u8; 24] {
    let mut header = [0; 24];
    std::fs::File::open(path)
        .unwrap()
        .read_exact(&mut header)
        .unwrap();
    header
}

#[test]
fn the_crc_is_the_engines() {
    // The table of resource-porter's packer has it for every file.
    let mut archive = open(FIXTURE).unwrap();
    let paths: Vec<String> = archive.files_under("").map(|(p, _)| p.to_owned()).collect();
    for path in paths {
        let bytes = archive.read(&path).unwrap().unwrap();
        assert_eq!(archive.entry(&path).unwrap().hash, crc32(&bytes), "{path}");
    }
}

/// Every node reached from the root without a link in the table of the
/// archive `bytes`: its `base_node`, the folder its `m_parent` names (+32),
/// and the start of the class of the folder it is in.
pub(crate) fn parents(bytes: &[u8]) -> Vec<(usize, u64, u64)> {
    let size = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let table = &bytes[HEADER_SIZE as usize..HEADER_SIZE as usize + size];
    let u64_at = |at: usize| u64::from_le_bytes(table[at..at + 8].try_into().unwrap());
    let flags_at =
        |at: usize| u16::from_le_bytes(table[at + FLAGS..at + FLAGS + 2].try_into().unwrap());
    let mut found = vec![(ROOT_BASE, u64_at(ROOT_BASE + 32), 0)];
    let mut folders = vec![(u64_at(ROOT_FIRST_CHILD) as usize, ROOT_FIRST_CHILD)];
    while let Some((mut node, folder)) = folders.pop() {
        while node != 0 {
            found.push((node, u64_at(node + 32), folder as u64));
            let flags = flags_at(node);
            if flags & FOLDER != 0 && flags & HARD_LINK == 0 {
                folders.push((u64_at(node - FOLDER_BASE) as usize, node - FOLDER_BASE));
            }
            node = u64_at(node + NEXT) as usize;
        }
    }
    found
}
