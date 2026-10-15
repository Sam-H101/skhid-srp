// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::BTreeMap;
use std::io::Cursor;

use super::*;
use crate::Archive;

fn files(list: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
    list.iter()
        .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
        .collect()
}

fn read_all(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = Archive::new(Cursor::new(bytes.to_vec())).unwrap();
    let paths: Vec<String> = archive
        .files_under("")
        .map(|(path, _)| path.to_owned())
        .collect();
    paths
        .into_iter()
        .map(|path| {
            let bytes = archive.read(&path).unwrap().unwrap();
            (path, bytes)
        })
        .collect()
}

fn table_end(bytes: &[u8]) -> usize {
    HEADER_SIZE as usize + u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize
}

#[test]
fn an_archive_written_reads_back_the_same_and_is_made_the_same_twice() {
    let files = files(&[
        ("gameplay/a.options", b"first"),
        ("gameplay/items/long_name_of_an_item.options", &[7; 3000]),
        ("gameplay/items/b", b""),
        ("projects/level_01/day/client_project", b""),
        ("srp/provenance", b"text\n"),
    ]);
    let bytes = write(&files).unwrap();
    assert_eq!(bytes, write(&files).unwrap());
    assert_eq!(read_all(&bytes), files);
    let archive = Archive::new(Cursor::new(bytes.clone())).unwrap();
    for (path, content) in &files {
        assert_eq!(archive.entry(path).unwrap().hash, crc32(content), "{path}");
    }
    for bad in ["", "a//b", "gameplay/", "/a", "a\0b"] {
        let files = BTreeMap::from([(bad.to_owned(), Vec::new())]);
        assert!(write(&files).is_err(), "{bad:?}");
    }
    let clash = BTreeMap::from([
        ("a/b".to_owned(), Vec::new()),
        ("a/b/c".to_owned(), Vec::new()),
    ]);
    assert!(write(&clash).is_err());
}

#[test]
fn every_node_names_the_folder_it_is_in() {
    let bytes = write(&files(&[
        ("a.txt", b"1"),
        ("gameplay/items/armour/mask/x.options", b"2"),
        ("gameplay/items/armour/y.options", b"3"),
        ("gameplay/z", b"4"),
        ("projects/p/client_project", b""),
    ]))
    .unwrap();
    let nodes = crate::tests::parents(&bytes);
    // The root, 4 files at depth 0..4, the folders gameplay, items, armour,
    // mask, projects, p, and a third file of gameplay/.
    assert_eq!(nodes.len(), 1 + 5 + 6);
    let (root, root_parent, _) = nodes[0];
    assert_eq!(
        (root, root_parent),
        (ROOT_BASE, 0),
        "the mount root is in none"
    );
    for &(node, parent, folder) in &nodes[1..] {
        assert_eq!(parent, folder, "the node at {node}");
        assert_ne!(parent, 0);
    }
    // The root's children name the mount root's folder, at 936, as in
    // 0.26g's archive.
    let at_root = nodes[1..]
        .iter()
        .filter(|&&(_, parent, _)| parent == ROOT_FIRST_CHILD as u64)
        .count();
    assert_eq!(at_root, 3, "a.txt, gameplay, projects");
}

#[test]
fn the_signature_has_room_after_the_table_and_the_files_come_after_it() {
    // Tables that end anywhere in a 2048-byte block: names of every length.
    for length in 1..120 {
        let name = "n".repeat(length);
        let bytes = write(&files(&[(&format!("gameplay/{name}"), b"payload")])).unwrap();
        let end = table_end(&bytes);
        let first = Archive::new(Cursor::new(bytes.clone()))
            .unwrap()
            .files_under("")
            .map(|(_, entry)| entry.offset)
            .min()
            .unwrap();
        assert_eq!(first, (end as u64 + SIGNATURE_SIZE).div_ceil(2048) * 2048);
        assert!(bytes[end..first as usize].iter().all(|&b| b == 0));
    }
    // A table of about 2048 - 24 - 256 bytes or more pushes the files to
    // the next block.
    let mut long = Vec::new();
    for i in 0..20 {
        long.push((format!("gameplay/file_{i:02}"), b"x".to_vec()));
    }
    let bytes = write(&long.into_iter().collect()).unwrap();
    let end = table_end(&bytes) as u64;
    assert!(end < 4096 && end + SIGNATURE_SIZE > 2048, "{end}");
    assert_eq!(bytes[4096], b'x');
}
