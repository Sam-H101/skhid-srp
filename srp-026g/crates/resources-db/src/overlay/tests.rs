// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::BTreeMap;
use std::io::Cursor;

use super::*;

pub(crate) fn archive_of(list: &[(&str, &str)]) -> Vec<u8> {
    let files: BTreeMap<String, Vec<u8>> = list
        .iter()
        .map(|(path, bytes)| ((*path).to_owned(), bytes.as_bytes().to_vec()))
        .collect();
    crate::write::write(&files).unwrap()
}

fn overlay(archives: &[Vec<u8>]) -> Result<Overlay<Cursor<Vec<u8>>>, Error> {
    let names: Vec<String> = (0..archives.len())
        .map(|i| format!("archive {i}"))
        .collect();
    let opened = archives
        .iter()
        .map(|bytes| Archive::new(Cursor::new(bytes.clone())).unwrap())
        .collect();
    Overlay::new(opened, &names)
}

fn read(overlay: &mut Overlay<Cursor<Vec<u8>>>, path: &str) -> Option<Vec<u8>> {
    overlay.read(path).unwrap()
}

#[test]
fn a_later_file_replaces_an_earlier_one_and_new_files_are_added() {
    let mut seen = overlay(&[
        archive_of(&[("gameplay/a", "db a"), ("gameplay/b", "db b")]),
        archive_of(&[("gameplay/a", "patch a"), ("gameplay/c", "patch c")]),
    ])
    .unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(read(&mut seen, "gameplay/a").unwrap(), b"patch a");
    assert_eq!(read(&mut seen, "gameplay/b").unwrap(), b"db b");
    assert_eq!(read(&mut seen, "gameplay/c").unwrap(), b"patch c");
    assert_eq!(read(&mut seen, "gameplay/d"), None);
    assert_eq!(seen.entry("gameplay/a").unwrap().0, 1);
    assert_eq!(seen.entry("gameplay/b").unwrap().0, 0);
    let listed: Vec<(&str, usize)> = seen
        .files_under("gameplay/")
        .map(|(path, index, _)| (path, index))
        .collect();
    assert_eq!(
        listed,
        [("gameplay/a", 1), ("gameplay/b", 0), ("gameplay/c", 1)]
    );
}

#[test]
fn folders_merge_and_the_last_archive_of_a_path_wins() {
    let mut seen = overlay(&[
        archive_of(&[("gameplay/items/x", "0"), ("gameplay/items/y", "0")]),
        archive_of(&[("gameplay/items/y", "1"), ("gameplay/items/new/z", "1")]),
        archive_of(&[("gameplay/items/y", "2"), ("gameplay/other", "2")]),
    ])
    .unwrap();
    let all: Vec<(&str, usize)> = seen
        .files_under("")
        .map(|(path, index, _)| (path, index))
        .collect();
    assert_eq!(
        all,
        [
            ("gameplay/items/new/z", 1),
            ("gameplay/items/x", 0),
            ("gameplay/items/y", 2),
            ("gameplay/other", 2),
        ]
    );
    assert_eq!(read(&mut seen, "gameplay/items/y").unwrap(), b"2");
    // One archive alone is itself.
    let alone = overlay(&[archive_of(&[("a/b", "1")])]).unwrap();
    assert_eq!(alone.len(), 1);
}

#[test]
fn a_file_over_a_folder_or_a_folder_over_a_file_is_refused() {
    let folder = archive_of(&[("gameplay/items/x", "0")]);
    let file = archive_of(&[("gameplay/items", "1")]);
    for (archives, path) in [
        ([folder.clone(), file.clone()], "gameplay/items"),
        ([file, folder], "gameplay/items/x"),
    ] {
        let message = overlay(&archives).unwrap_err().to_string();
        assert!(message.contains("archive 1 has the file"), "{message}");
        assert!(message.contains(path), "{message}");
        assert!(message.contains("archive 0"), "{message}");
    }
    // A name that only begins as a folder's does not clash.
    let seen = overlay(&[
        archive_of(&[("gameplay/items/x", "0")]),
        archive_of(&[("gameplay/items.x", "1"), ("gameplay/item", "1")]),
    ])
    .unwrap();
    assert_eq!(seen.len(), 3);
}

/// An archive of `files` and the empty folders `empty`.
fn with_empty(files: &[(&str, &str)], empty: &[&str]) -> Vec<u8> {
    let table = files
        .iter()
        .map(|(path, bytes)| {
            let stored = crate::write::Stored {
                size: bytes.len() as u32,
                hash: crate::crc32(bytes.as_bytes()),
                share: None,
            };
            ((*path).to_owned(), stored)
        })
        .collect();
    let empty = empty.iter().map(|path| (*path).to_owned()).collect();
    let mut out = Vec::new();
    crate::write::write_to(&mut out, &table, &empty, |path| {
        let (_, bytes) = files.iter().find(|(name, _)| *name == path).unwrap();
        Ok(bytes.as_bytes().to_vec())
    })
    .unwrap();
    out
}

#[test]
fn a_file_over_an_empty_folder_or_an_empty_folder_over_a_file_is_refused() {
    let empty = with_empty(&[("a/x", "0")], &["a/empty", "b/c/empty"]);
    for (archives, path) in [
        (
            [empty.clone(), archive_of(&[("a/empty", "1")])],
            "the file a/empty",
        ),
        ([empty.clone(), archive_of(&[("b/c", "1")])], "the file b/c"),
        (
            [archive_of(&[("a/empty", "1")]), empty.clone()],
            "the empty folder a/empty",
        ),
        (
            [archive_of(&[("b", "1")]), empty.clone()],
            "the empty folder b/c/empty",
        ),
    ] {
        let message = overlay(&archives).unwrap_err().to_string();
        assert!(
            message.contains(&format!("archive 1 has {path}")),
            "{message}"
        );
        assert!(
            message.contains("a file and a folder at one path"),
            "{message}"
        );
    }
}

#[test]
fn the_empty_folders_seen_are_those_no_archive_fills() {
    let seen = overlay(&[
        with_empty(&[("a/x", "0")], &["a/empty", "b/filled", "c/d", "e"]),
        with_empty(&[("b/filled/f", "1")], &["c/d/deeper", "z"]),
    ])
    .unwrap();
    let empty: Vec<&str> = seen.empty_folders().collect();
    assert_eq!(empty, ["a/empty", "c/d/deeper", "e", "z"]);
    // One archive's own.
    let alone = overlay(&[with_empty(&[("a/x", "0")], &["q"])]).unwrap();
    assert_eq!(alone.empty_folders().collect::<Vec<_>>(), ["q"]);
}
