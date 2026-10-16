// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::PathBuf;

use super::*;
use crate::write::SIGNATURE_SIZE;
use crate::{HEADER_SIZE, ROOT_FIRST_CHILD};

/// A folder of its own under the system's temporary one, emptied first and
/// removed after.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("resources-db-pack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self, path: &str, bytes: &[u8]) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn table_end(bytes: &[u8]) -> usize {
    HEADER_SIZE as usize + u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize
}

#[test]
fn a_folder_packs_at_its_paths_and_reads_back_the_same_twice() {
    let folder = Folder::new("paths");
    folder.file("flash_movies/main_menu.swf", &[1; 5000]);
    folder.file("flash_movies/login_menu.swf", b"login");
    folder.file("gameplay/items/armour/mask/new.options", b"mask");
    folder.file("gameplay/items/empty.options", b"");
    folder.file("top", b"t");
    let (bytes, files) = pack(&folder.0).unwrap();
    let paths: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(
        paths,
        [
            "flash_movies/login_menu.swf",
            "flash_movies/main_menu.swf",
            "gameplay/items/armour/mask/new.options",
            "gameplay/items/empty.options",
            "top",
        ]
    );
    assert_eq!(pack(&folder.0).unwrap().0, bytes, "the same bytes twice");
    let mut archive = Archive::new(Cursor::new(bytes.clone())).unwrap();
    assert_eq!(archive.len(), 5);
    assert_eq!(
        archive.read("flash_movies/main_menu.swf").unwrap().unwrap(),
        [1; 5000]
    );
    assert_eq!(archive.read("top").unwrap().unwrap(), b"t");
    // As the game's own archives: the zeroed signature after the table,
    // the files from the next 2048-byte boundary after it.
    let end = table_end(&bytes);
    let first = archive
        .files_under("")
        .map(|(_, entry)| entry.offset)
        .min()
        .unwrap();
    assert_eq!(first, (end as u64 + SIGNATURE_SIZE).div_ceil(2048) * 2048);
    assert!(bytes[end..first as usize].iter().all(|&b| b == 0));
    // And the root's children name the mount root's folder.
    let nodes = crate::tests::parents(&bytes);
    for &(node, parent, folder) in &nodes[1..] {
        assert_eq!(parent, folder, "the node at {node}");
    }
    assert!(
        nodes
            .iter()
            .any(|&(_, parent, _)| parent == ROOT_FIRST_CHILD as u64)
    );
}

#[test]
fn what_an_archive_cannot_say_is_refused() {
    let none = Folder::new("none");
    assert!(pack(&none.0).unwrap_err().to_string().contains("no file"));
    assert!(pack(&none.0.join("missing")).is_err());

    let empty = Folder::new("empty-folder");
    empty.file("a/file", b"1");
    std::fs::create_dir_all(empty.0.join("a/nothing")).unwrap();
    let message = pack(&empty.0).unwrap_err().to_string();
    assert!(message.contains("an empty folder"), "{message}");

    let file = Folder::new("not-a-folder");
    file.file("x", b"1");
    assert!(
        pack(&file.0.join("x"))
            .unwrap_err()
            .to_string()
            .contains("not a folder")
    );

    let link = Folder::new("link");
    link.file("a", b"1");
    std::os::unix::fs::symlink(link.0.join("a"), link.0.join("b")).unwrap();
    let message = pack(&link.0).unwrap_err().to_string();
    assert!(message.contains("not a plain file"), "{message}");

    for hidden in [".git/config", "gameplay/.x.options.swp", ".DS_Store"] {
        let folder = Folder::new("hidden");
        folder.file("gameplay/x.options", b"1");
        folder.file(hidden, b"1");
        let message = pack(&folder.0).unwrap_err().to_string();
        assert!(
            message.contains("a hidden file or folder"),
            "{hidden}: {message}"
        );
    }

    // 6 + 1 + 242 bytes are taken, one more is not.
    let long = Folder::new("long");
    let name = "n".repeat(242);
    long.file(&format!("folder/{name}"), b"1");
    assert!(pack(&long.0).is_ok());
    long.file(&format!("folder/{name}x"), b"1");
    let message = pack(&long.0).unwrap_err().to_string();
    assert!(
        message.contains("250 bytes, longer than the game takes (249)"),
        "{message}"
    );

    let backslash = Folder::new("backslash");
    backslash.file("a\\b", b"1");
    assert!(
        pack(&backslash.0)
            .unwrap_err()
            .to_string()
            .contains("UTF-8")
    );
}

#[test]
fn a_check_against_other_files_fails() {
    let folder = Folder::new("check");
    folder.file("a", b"1");
    folder.file("b/c", b"2");
    let (bytes, files) = pack(&folder.0).unwrap();
    let mut other = files.clone();
    other.insert("a".into(), b"9".to_vec());
    assert!(check(&bytes, &other).is_err());
    let mut fewer = files.clone();
    fewer.remove("a");
    assert!(check(&bytes, &fewer).is_err());
    let mut renamed = fewer;
    renamed.insert("z".into(), b"1".to_vec());
    assert!(check(&bytes, &renamed).is_err());
    assert!(check(&bytes, &files).is_ok());
}
