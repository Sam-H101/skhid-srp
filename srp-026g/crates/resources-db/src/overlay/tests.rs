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
