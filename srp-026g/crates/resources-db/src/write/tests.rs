// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{BTreeMap, BTreeSet};
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

fn stored(bytes: &[u8], share: Option<u64>) -> Stored {
    Stored {
        size: bytes.len() as u32,
        hash: crc32(bytes),
        share,
    }
}

#[test]
fn write_to_gives_what_write_gives() {
    let files = files(&[
        ("a.txt", b"1"),
        ("gameplay/items/x.options", &[3; 5000]),
        ("gameplay/z", b""),
    ]);
    let table = files
        .iter()
        .map(|(path, bytes)| (path.clone(), stored(bytes, None)))
        .collect();
    let mut out = Vec::new();
    let size = write_to(&mut out, &table, &BTreeSet::new(), |path| {
        Ok(files[path].clone())
    })
    .unwrap();
    assert_eq!(out, write(&files).unwrap());
    assert_eq!(size, out.len() as u64);
}

#[test]
fn write_to_keeps_empty_folders_and_writes_shared_bytes_once() {
    let texture = vec![9; 4000];
    let table = BTreeMap::from([
        ("models/a/texture".to_owned(), stored(&texture, Some(77))),
        ("models/b/texture".to_owned(), stored(&texture, Some(77))),
        ("models/c/other".to_owned(), stored(b"other", None)),
    ]);
    let empty = BTreeSet::from([
        "collision/empty".to_owned(),
        // Not empty after all: written once, with its file.
        "models/c".to_owned(),
    ]);
    let mut asked = Vec::new();
    let mut out = Vec::new();
    write_to(&mut out, &table, &empty, |path| {
        asked.push(path.to_owned());
        Ok(if path.ends_with("texture") {
            texture.clone()
        } else {
            b"other".to_vec()
        })
    })
    .unwrap();
    assert_eq!(asked, ["models/a/texture", "models/c/other"]);
    let mut archive = Archive::new(Cursor::new(out.clone())).unwrap();
    assert_eq!(
        archive.empty_folders().collect::<Vec<_>>(),
        [b"collision/empty".as_slice()]
    );
    let (a, b) = (
        *archive.entry("models/a/texture").unwrap(),
        *archive.entry("models/b/texture").unwrap(),
    );
    assert_eq!(a, b, "one copy of the shared bytes");
    assert_eq!(archive.read("models/b/texture").unwrap().unwrap(), texture);
    assert_eq!(archive.read("models/c/other").unwrap().unwrap(), b"other");
    assert_eq!(out.len() as u64, a.offset + 4000 + 5);
}

#[test]
fn write_to_refuses_bytes_unlike_their_table_and_shares_that_differ() {
    let table = BTreeMap::from([("a".to_owned(), stored(b"one", None))]);
    let wrong = |bytes: &'static [u8]| {
        let mut out = Vec::new();
        write_to(&mut out, &table, &BTreeSet::new(), |_| Ok(bytes.to_vec())).is_err()
    };
    assert!(!wrong(b"one"));
    assert!(wrong(b"two"), "the same size, another checksum");
    assert!(wrong(b"three"), "another size");

    let clash = BTreeMap::from([
        ("a".to_owned(), stored(b"one", Some(1))),
        ("b".to_owned(), stored(b"other", Some(1))),
    ]);
    let mut out = Vec::new();
    assert!(write_to(&mut out, &clash, &BTreeSet::new(), |_| Ok(Vec::new())).is_err());
    let file_and_folder = BTreeSet::from(["a/b".to_owned()]);
    assert!(write_to(&mut out, &table, &file_and_folder, |_| Ok(b"one".to_vec())).is_err());
}

/// The table of the archive `bytes` for [`write_to`], each file's share key
/// where its bytes are, as resource-porter writes an archive read again.
fn shared_by_offset(bytes: &[u8]) -> BTreeMap<String, Stored> {
    let archive = Archive::new(Cursor::new(bytes.to_vec())).unwrap();
    archive
        .files_under("")
        .map(|(path, entry)| {
            let stored = Stored {
                size: entry.size,
                hash: entry.hash,
                share: Some(entry.offset),
            };
            (path.to_owned(), stored)
        })
        .collect()
}

#[test]
fn an_empty_file_shares_no_bytes_though_its_offset_is_the_next_files() {
    let files = files(&[
        ("a/empty", b""),
        ("a/next", b"next file"),
        ("b/empty", b""),
        ("b/last", b"last"),
        ("c/empty_at_the_end", b""),
    ]);
    let bytes = write(&files).unwrap();
    let table = shared_by_offset(&bytes);
    assert_eq!(
        table["a/empty"].share, table["a/next"].share,
        "an empty file is where the next one's bytes start"
    );
    // Written again with those keys: the same files, the same bytes.
    let mut again = Vec::new();
    write_to(&mut again, &table, &BTreeSet::new(), |path| {
        Ok(files[path].clone())
    })
    .unwrap();
    assert_eq!(read_all(&again), files);
    assert_eq!(again, bytes);
    // An empty file of any key, before or after a file of that key.
    let key = |share| Stored {
        share: Some(share),
        ..stored(b"", None)
    };
    let table = BTreeMap::from([
        ("a".to_owned(), key(7)),
        ("b".to_owned(), stored(b"bytes", Some(7))),
        ("c".to_owned(), key(7)),
        ("d".to_owned(), stored(b"bytes", Some(7))),
    ]);
    let mut out = Vec::new();
    write_to(&mut out, &table, &BTreeSet::new(), |path| {
        Ok(if table[path].size == 0 {
            Vec::new()
        } else {
            b"bytes".to_vec()
        })
    })
    .unwrap();
    let read = read_all(&out);
    assert_eq!(read["a"], b"");
    assert_eq!(read["d"], b"bytes");
}

#[test]
fn a_path_longer_than_the_game_takes_is_not_written() {
    let at_most = format!("gameplay/{}", "x".repeat(crate::MAX_PATH - 9));
    assert!(write(&files(&[(&at_most, b"1")])).is_ok());
    let longer = format!("{at_most}y");
    let error = write(&files(&[(&longer, b"1")])).unwrap_err();
    assert!(matches!(error, Error::Write(_)), "{error:?}");
    assert!(
        error.to_string().contains("longer than the game takes"),
        "{error}"
    );
    let folder = BTreeSet::from([longer]);
    let mut out = Vec::new();
    assert!(write_to(&mut out, &BTreeMap::new(), &folder, |_| Ok(Vec::new())).is_err());
}

#[test]
fn what_cannot_be_written_is_a_write_error() {
    for bad in ["", "a//b", "a\0b"] {
        let error = write(&BTreeMap::from([(bad.to_owned(), Vec::new())])).unwrap_err();
        assert!(matches!(error, Error::Write(_)), "{bad:?}: {error:?}");
        assert!(
            error.to_string().starts_with("cannot write the archive: "),
            "{error}"
        );
    }
    let table = BTreeMap::from([("a".to_owned(), stored(b"one", None))]);
    let mut out = Vec::new();
    let error = write_to(&mut out, &table, &BTreeSet::new(), |_| Ok(b"two".to_vec())).unwrap_err();
    assert!(matches!(error, Error::Write(_)), "{error:?}");
}
