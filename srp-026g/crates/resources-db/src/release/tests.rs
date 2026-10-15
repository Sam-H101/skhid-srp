// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;

/// A folder of its own for one test, removed when it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("srp-release-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn archive() -> Vec<u8> {
    let files: BTreeMap<String, Vec<u8>> = [
        ("flash_movies/menu.swf", vec![b'm'; 3000]),
        ("gameplay/item.options", b"an item\n".to_vec()),
    ]
    .into_iter()
    .map(|(path, bytes)| (path.to_owned(), bytes))
    .collect();
    crate::write::write(&files).unwrap()
}

/// `bytes` with its last 4 bytes chosen so that its CRC-32 is `crc` (a CRC
/// of messages of one length is linear in their bits).
fn with_crc(mut bytes: Vec<u8>, crc: u32) -> Vec<u8> {
    let n = bytes.len();
    bytes[n - 4..].fill(0);
    let zero = crc32(&vec![0; n]);
    let mut need = crc32(&bytes) ^ crc;
    let mut basis: Vec<(u32, u32)> = Vec::new();
    for bit in 0..32 {
        let mut probe = vec![0; n];
        probe[n - 4..].copy_from_slice(&(1u32 << bit).to_le_bytes());
        let (mut effect, mut chosen) = (crc32(&probe) ^ zero, 1u32 << bit);
        for &(b_effect, b_chosen) in &basis {
            if effect ^ b_effect < effect {
                effect ^= b_effect;
                chosen ^= b_chosen;
            }
        }
        if effect != 0 {
            basis.push((effect, chosen));
            basis.sort_by(|a, b| b.0.cmp(&a.0));
        }
    }
    let mut tail = 0;
    for &(effect, chosen) in &basis {
        if need ^ effect < need {
            need ^= effect;
            tail ^= chosen;
        }
    }
    assert_eq!(need, 0, "the CRC can be reached");
    bytes[n - 4..].copy_from_slice(&tail.to_le_bytes());
    bytes
}

fn menu_entry(bytes: &[u8]) -> crate::Entry {
    *Archive::new(std::io::Cursor::new(bytes.to_vec()))
        .unwrap()
        .entry("flash_movies/menu.swf")
        .unwrap()
}

#[test]
fn the_same_file_is_no_change() {
    let dir = Scratch::new("same");
    let original = dir.write("original.db", &archive());
    assert_eq!(release(&original, &original).unwrap(), Release::default());
}

#[test]
fn a_file_changed_in_place_with_its_crc_is_a_region_named_after_it() {
    let dir = Scratch::new("region");
    let base = archive();
    let entry = menu_entry(&base);
    let (at, size) = (entry.offset as usize, entry.size as usize);
    let mut changed = base.clone();
    let menu = with_crc(vec![b'n'; size], entry.hash);
    changed[at..at + size].copy_from_slice(&menu);
    let (original, final_db) = (
        dir.write("original.db", &base),
        dir.write("final.db", &changed),
    );
    let plan = release(&original, &final_db).unwrap();
    let sha = |bytes: &[u8]| {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex(&hasher.finish())
    };
    assert_eq!(
        plan.lines,
        [format!(
            "region game/resources.db {} {at} {size} {} {} menu",
            base.len(),
            sha(&base[at..at + size]),
            sha(&menu)
        )]
    );
    let blobs = dir.0.join("blobs");
    write_blobs(&plan, &original, &final_db, &blobs).unwrap();
    assert_eq!(std::fs::read(blobs.join(sha(&menu))).unwrap(), menu);
    assert_eq!(
        std::fs::read(blobs.join(sha(&base[at..at + size]))).unwrap(),
        &base[at..at + size]
    );
    // A second run keeps them; a blob with other bytes is refused.
    write_blobs(&plan, &original, &final_db, &blobs).unwrap();
    std::fs::write(blobs.join(sha(&menu)), b"other").unwrap();
    assert!(write_blobs(&plan, &original, &final_db, &blobs).is_err());
}

#[test]
fn a_file_changed_without_a_new_table_must_keep_its_crc() {
    let dir = Scratch::new("crc");
    let base = archive();
    let entry = menu_entry(&base);
    let mut changed = base.clone();
    changed[entry.offset as usize] ^= 1;
    let (original, final_db) = (
        dir.write("original.db", &base),
        dir.write("final.db", &changed),
    );
    let error = release(&original, &final_db).unwrap_err().to_string();
    assert!(error.contains("would refuse it"), "{error}");
}

#[test]
fn a_shorter_file_or_one_grown_without_a_new_table_is_refused() {
    let dir = Scratch::new("size");
    let base = archive();
    let original = dir.write("original.db", &base);
    let shorter = dir.write("shorter.db", &base[..base.len() - 1]);
    assert!(
        release(&original, &shorter)
            .unwrap_err()
            .to_string()
            .contains("shorter")
    );
    let mut longer = base.clone();
    longer.extend_from_slice(b"more");
    let longer = dir.write("longer.db", &longer);
    assert!(
        release(&original, &longer)
            .unwrap_err()
            .to_string()
            .contains("nothing would read")
    );
}

#[test]
fn put_writes_a_file_in_place_only_with_its_size_and_crc() {
    let dir = Scratch::new("put");
    let base = archive();
    let entry = menu_entry(&base);
    let db = dir.write("final.db", &base);
    let size = entry.size as usize;
    assert!(
        put(&db, "flash_movies/menu.swf", &vec![b'n'; size - 1])
            .unwrap_err()
            .to_string()
            .contains("same size")
    );
    assert!(
        put(&db, "flash_movies/menu.swf", &vec![b'n'; size])
            .unwrap_err()
            .to_string()
            .contains("would refuse")
    );
    assert!(put(&db, "flash_movies/none.swf", b"x").is_err());
    assert_eq!(
        std::fs::read(&db).unwrap(),
        base,
        "nothing written when refused"
    );
    let menu = with_crc(vec![b'n'; size], entry.hash);
    put(&db, "flash_movies/menu.swf", &menu).unwrap();
    let written = std::fs::read(&db).unwrap();
    assert_eq!(
        &written[entry.offset as usize..entry.offset as usize + size],
        &menu[..]
    );
    let plan = release(&dir.write("original.db", &base), &db).unwrap();
    assert_eq!(plan.lines.len(), 1);
    assert!(plan.lines[0].ends_with(" menu"));
}

#[test]
fn a_region_is_named_after_its_file_or_its_path() {
    let none = BTreeSet::new();
    assert_eq!(
        region_name("flash_movies/lobby_menu.swf", &none),
        "lobby_menu"
    );
    assert_eq!(region_name("a b/c+d.e", &none), "c_d");
    let taken: BTreeSet<String> = ["lobby_menu".to_owned()].into();
    assert_eq!(
        region_name("other/lobby_menu.swf", &taken),
        "other_lobby_menu.swf"
    );
    assert_eq!(
        region_name("x/resources_head.bin", &none),
        "x_resources_head.bin"
    );
}

/// The candidate release: `SRP_RELEASE_ORIGINAL` (0.26g's resources.db),
/// `SRP_RELEASE_FINAL` (the release's), `SRP_RELEASE_PLAN` (the plan its ZIP
/// carries): the plan read off the two files is that plan's lines.
#[test]
#[ignore = "needs 0.26g's resources.db and a release's; SRP_RELEASE_* name them"]
fn the_plan_of_a_real_release_is_its_zips() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
    let plan = release(
        Path::new(&var("SRP_RELEASE_ORIGINAL")),
        Path::new(&var("SRP_RELEASE_FINAL")),
    )
    .unwrap();
    let expected = std::fs::read_to_string(var("SRP_RELEASE_PLAN")).unwrap();
    let mut expected: Vec<&str> = expected
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| line.starts_with("region ") || line.starts_with("tail "))
        .collect();
    let mut lines: Vec<&str> = plan.lines.iter().map(String::as_str).collect();
    expected.sort_unstable();
    lines.sort_unstable();
    assert_eq!(lines, expected);
}
