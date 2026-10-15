// SPDX-License-Identifier: GPL-3.0-or-later
use std::io::Cursor;
use std::path::PathBuf;

use super::*;
use crate::plan::tests::{Folder, INSTALLED, release_plan, zip_of};

fn open(bytes: &[u8]) -> Archive<Cursor<Vec<u8>>> {
    Archive::new(Cursor::new(bytes.to_vec())).unwrap()
}

fn read_all(archive: &mut Archive<Cursor<Vec<u8>>>) -> BTreeMap<String, Vec<u8>> {
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

#[test]
fn an_archive_written_reads_back_the_same_and_is_made_the_same_twice() {
    let files: BTreeMap<String, Vec<u8>> = [
        ("gameplay/a.options", b"first".to_vec()),
        ("gameplay/items/long_name_of_an_item.options", vec![7; 3000]),
        ("gameplay/items/b", Vec::new()),
        ("projects/level_01/day/client_project", Vec::new()),
        ("srp/provenance", b"text\n".to_vec()),
    ]
    .into_iter()
    .map(|(path, bytes)| (path.to_owned(), bytes))
    .collect();
    let bytes = crate::write::write(&files).unwrap();
    assert_eq!(bytes, crate::write::write(&files).unwrap());
    let mut archive = open(&bytes);
    assert_eq!(read_all(&mut archive), files);
    for (path, content) in &files {
        let entry = archive.entry(path).unwrap();
        assert_eq!(entry.hash, crate::crc32(content), "{path}");
        assert!(
            entry.offset >= 2048,
            "the files' bytes come after the table"
        );
    }
    for bad in ["", "a//b", "gameplay/", "/a"] {
        let files = BTreeMap::from([(bad.to_owned(), Vec::new())]);
        assert!(crate::write::write(&files).is_err(), "{bad:?}");
    }
    let clash = BTreeMap::from([
        ("a/b".to_owned(), Vec::new()),
        ("a/b/c".to_owned(), Vec::new()),
    ]);
    assert!(crate::write::write(&clash).is_err());
}

#[test]
fn the_crc_is_the_engines() {
    // The tables of resource-porter's packer: the fixture's files.
    let mut archive = open(INSTALLED);
    for (path, bytes) in read_all(&mut archive) {
        assert_eq!(
            archive.entry(&path).unwrap().hash,
            crate::crc32(&bytes),
            "{path}"
        );
    }
}

#[test]
fn the_installed_archive_alone_gives_its_files_and_no_plan() {
    let folder = Folder::new("server-installed");
    let installed = folder.installed(INSTALLED);
    let (bytes, extracted) = extract(&installed, None, &folder.blobs()).unwrap();
    assert_eq!((extracted.gameplay, extracted.projects), (4, 1));
    assert_eq!(extracted.provenance, format!("{FORMAT}\nplan none\n"));
    let mut data = open(&bytes);
    let files = read_all(&mut data);
    assert_eq!(files["gameplay/first.txt"], b"first file\n");
    assert_eq!(files["projects/level_01/day/client_project"], b"");
    assert_eq!(files[PROVENANCE], extracted.provenance.as_bytes());
    assert_eq!(files.len(), 6);
    // No ZIP: data of no plan is the server's.
    assert!(check(&mut data, None).unwrap().is_ok());
}

#[test]
fn a_release_gives_its_files_and_names_its_plan() {
    let folder = Folder::new("server-release");
    let installed = folder.installed(INSTALLED);
    let plan = release_plan(&folder);
    let zip_bytes = zip_of(&[("README.txt", b"hello"), (PLAN_NAME, plan.as_bytes())]);
    let zip = folder.0.join("players.zip");
    std::fs::write(&zip, &zip_bytes).unwrap();
    let (bytes, extracted) = extract(&installed, Some(&zip), &folder.blobs()).unwrap();
    assert_eq!(
        bytes,
        extract(&installed, Some(&zip), &folder.blobs()).unwrap().0
    );
    assert_eq!(extracted.gameplay, 5);
    let lines: Vec<&str> = extracted.provenance.lines().collect();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_eq!(lines[0], FORMAT);
    let mut hasher = Sha256::new();
    hasher.update(plan.as_bytes());
    assert_eq!(lines[1], format!("plan {}", hex(&hasher.finish())));
    assert!(
        lines[2].starts_with("region game/resources.db ") && lines[2].ends_with(" resources_head")
    );
    assert!(
        lines[3].starts_with("tail game/resources.db ") && lines[3].ends_with(" resources_head")
    );
    let mut data = open(&bytes);
    assert_eq!(
        data.read("gameplay/items/new.options").unwrap().unwrap(),
        b"a new item\n"
    );

    // The server's ZIP decides: the same plan, another, none.
    assert!(check(&mut data, Some(&zip_bytes)).unwrap().is_ok());
    let other = zip_of(&[(PLAN_NAME, b"skhid-resources 1\r\nend\r\n")]);
    let mismatch = check(&mut data, Some(&other)).unwrap().unwrap_err();
    assert_eq!(mismatch.made_for, extracted.provenance);
    assert_ne!(plan_line(&mismatch.zip_has), plan_line(&mismatch.made_for));
    assert!(
        check(&mut data, Some(&zip_of(&[("README.txt", b"hello")])))
            .unwrap()
            .is_err()
    );
    assert!(check(&mut data, None).unwrap().is_err());
    // An archive that is not server data.
    assert!(check(&mut open(INSTALLED), None).is_err());
}

#[test]
fn a_file_unlike_its_table_is_refused() {
    let folder = Folder::new("server-damaged");
    let mut damaged = INSTALLED.to_vec();
    let at = damaged
        .windows(11)
        .position(|w| w == b"first file\n")
        .unwrap();
    damaged[at] = b'F';
    let installed = folder.installed(&damaged);
    let message = extract(&installed, None, &folder.blobs())
        .unwrap_err()
        .to_string();
    assert!(message.contains("gameplay/first.txt"), "{message}");
}

/// 0.26g's archive with a players' ZIP and its blobs, as in
/// `plan::tests::a_release_of_0_26g_opens`.
#[test]
#[ignore = "needs 0.26g's resources.db and a players' ZIP with its blobs"]
fn the_server_data_of_a_release_of_0_26g() {
    let var = |name: &str| PathBuf::from(std::env::var(name).expect(name));
    let zip = var("SRP_TEST_PLAYERS_ZIP");
    let (bytes, extracted) = extract(
        &var("SRP_TEST_RESOURCES_DB"),
        Some(&zip),
        &var("SRP_TEST_RESOURCE_BLOBS"),
    )
    .unwrap();
    eprintln!("{} bytes, {extracted:?}", bytes.len());
    assert!(extracted.gameplay > 500 && extracted.projects > 70);
    let zip = std::fs::read(zip).unwrap();
    assert!(check(&mut open(&bytes), Some(&zip)).unwrap().is_ok());
}
