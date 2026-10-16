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
    for short in [&FIXTURE[..0], &FIXTURE[..10], &FIXTURE[..23]] {
        let message = open(short).unwrap_err().to_string();
        assert!(
            message.contains("shorter than its 24-byte header"),
            "{message}"
        );
    }
}

#[test]
fn sizes_past_the_end_of_the_file_are_refused_before_anything_is_allocated() {
    // A table of 4 GB in a file of a few kB.
    let mut bytes = FIXTURE.to_vec();
    bytes[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
    let message = open(&bytes).unwrap_err().to_string();
    assert!(message.contains("ends inside its table"), "{message}");
    // A file of 4 GB, and one whose end is past the end of u64.
    for (size, offset) in [(u32::MAX, None), (1, Some(u64::MAX))] {
        let mut bytes = FIXTURE.to_vec();
        let start = node_of(&bytes, "big.bin") - FILE_BASE;
        bytes[start..start + 4].copy_from_slice(&size.to_le_bytes());
        if let Some(offset) = offset {
            bytes[start + 8..start + 16].copy_from_slice(&offset.to_le_bytes());
        }
        let mut archive = open(&bytes).unwrap();
        let message = archive
            .read("gameplay/items/big.bin")
            .unwrap_err()
            .to_string();
        assert!(message.contains("past the end"), "{message}");
    }
}

#[test]
fn a_name_that_is_not_utf8_is_refused() {
    let mut bytes = FIXTURE.to_vec();
    let at = node_of(&bytes, "first.txt") + NAME;
    bytes[at] = 0xFF;
    let message = open(&bytes).unwrap_err().to_string();
    assert!(
        message.contains("gameplay/\u{FFFD}irst.txt: a name that is not UTF-8"),
        "{message}"
    );
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
    link(FIXTURE.to_vec(), name, target)
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

/// `bytes` with the file node `name` turned into a hard link to the node
/// `target` (each named by its own name, in the archive `bytes`).
fn link(mut bytes: Vec<u8>, name: &str, target: &str) -> Vec<u8> {
    let node = node_of(&bytes, name);
    let flags = u16::from_le_bytes(bytes[node + FLAGS..node + FLAGS + 2].try_into().unwrap());
    bytes[node + FLAGS..node + FLAGS + 2].copy_from_slice(&(flags | HARD_LINK).to_le_bytes());
    let pointer = (table_offset(&bytes, target) as u64).to_le_bytes();
    bytes[node - LINK_BASE..node].copy_from_slice(&pointer);
    bytes
}

#[test]
fn a_link_to_a_folder_it_is_in_is_refused_when_it_is_met() {
    // Many nodes elsewhere, so the table's node budget is large: a folder
    // that holds a link to itself must be refused at once, not walked
    // until the budget is spent (its paths a byte longer each time).
    let mut files: BTreeMap<String, Vec<u8>> = (0..5000)
        .map(|i| (format!("other/file_{i:04}"), Vec::new()))
        .collect();
    files.insert("loop/self_link".into(), Vec::new());
    files.insert("up/down/up_link".into(), Vec::new());
    let bytes = write::write(&files).unwrap();
    for (name, target) in [("self_link", "loop"), ("up_link", "up")] {
        let started = std::time::Instant::now();
        let message = open(&link(bytes.clone(), name, target))
            .unwrap_err()
            .to_string();
        assert!(message.contains("links to a folder it is in"), "{message}");
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
    // Two links that lead to one another: the second is in the first.
    let mut fixture = link(FIXTURE.to_vec(), "first.txt", "items");
    fixture = link(fixture, "item.options", "gameplay");
    let message = open(&fixture).unwrap_err().to_string();
    assert!(message.contains("links to a folder it is in"), "{message}");
    // A link to a folder beside it is no loop.
    let mut archive = open(&link(bytes, "self_link", "down")).unwrap();
    assert!(archive.read("loop/self_link/up_link").unwrap().is_some());
}

#[test]
fn a_path_through_links_longer_than_the_game_takes_is_refused() {
    // Each path the table has is short enough; the path of `inner`'s file
    // through the link is not.
    let long = "l".repeat(200);
    let files = BTreeMap::from([
        (format!("outer/{long}"), Vec::new()),
        (format!("inner/{}", "f".repeat(60)), Vec::new()),
    ]);
    let bytes = link(write::write(&files).unwrap(), &long, "inner");
    let message = open(&bytes).unwrap_err().to_string();
    assert!(
        message.contains("267 bytes, longer than the game takes (249)"),
        "{message}"
    );
    // At MAX_PATH it is taken.
    let files = BTreeMap::from([
        (format!("outer/{long}"), Vec::new()),
        (format!("inner/{}", "f".repeat(MAX_PATH - 207)), Vec::new()),
    ]);
    let archive = open(&link(write::write(&files).unwrap(), &long, "inner")).unwrap();
    let longest = archive
        .files_under("outer/")
        .map(|(path, _)| path.len())
        .max();
    assert_eq!(longest, Some(MAX_PATH));
}

/// The CRC bit by bit, as the table is made.
fn crc32_bitwise(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc.reverse_bits()
}

#[test]
fn the_crc_by_its_table_is_the_crc_bit_by_bit() {
    let mut data = Vec::new();
    let mut state: u32 = 1;
    for length in 0..600 {
        assert_eq!(crc32(&data), crc32_bitwise(&data), "{length} bytes");
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        data.push((state >> 16) as u8);
    }
    assert_eq!(crc32(b""), 0);
    assert_eq!(crc32(b"123456789"), crc32_bitwise(b"123456789"));
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
