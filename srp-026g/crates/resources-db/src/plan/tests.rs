// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::Path;

use updater_logic::resources::parse_plan;
use updater_logic::sha256::digest;

use super::*;

/// `../tests/fixture.db` packed again with `gameplay/items/new.options`
/// ("a new item\n") added: the archive a release makes of it.
pub(crate) const RELEASE: &[u8] = include_bytes!("../../tests/release.db");
pub(crate) const INSTALLED: &[u8] = include_bytes!("../../tests/fixture.db");

pub(crate) struct Folder(pub(crate) PathBuf);

impl Folder {
    pub(crate) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "srp-resources-db-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("blobs")).unwrap();
        Self(dir)
    }

    pub(crate) fn installed(&self, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("resources.db");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    pub(crate) fn blobs(&self) -> PathBuf {
        self.0.join("blobs")
    }

    /// Puts `bytes` among the blobs, by their sha256; returns it.
    pub(crate) fn blob(&self, bytes: &[u8]) -> String {
        let sha = hex(&digest(bytes));
        std::fs::write(self.blobs().join(&sha), bytes).unwrap();
        sha
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The plan that makes `RELEASE` of `INSTALLED`: its first
/// `INSTALLED.len()` bytes as a region, the rest as the tail.
pub(crate) fn release_plan(folder: &Folder) -> String {
    let size = INSTALLED.len();
    let original = hex(&digest(INSTALLED));
    let head = folder.blob(&RELEASE[..size]);
    let tail = folder.blob(&RELEASE[size..]);
    format!(
        "skhid-resources 2\r\n\
         region game/resources.db {size} 0 {size} {original} {head} resources_head\r\n\
         tail game/resources.db {size} {} {tail} resources_tail resources_head\r\n\
         end\r\n",
        RELEASE.len()
    )
}

fn open_with(installed: &Path, plan: &str, blobs: &Path) -> Result<(Patched, Applied), Error> {
    Patched::open(installed, &parse_plan(plan.as_bytes()).unwrap(), blobs)
}

#[test]
fn the_plan_gives_the_archive_the_players_have() {
    let folder = Folder::new("release");
    let installed = folder.installed(INSTALLED);
    let (mut patched, applied) =
        open_with(&installed, &release_plan(&folder), &folder.blobs()).unwrap();
    assert_eq!(applied.regions, ["resources_head"]);
    assert_eq!(applied.tail.as_deref(), Some("resources_tail"));
    assert_eq!(applied.size, RELEASE.len() as u64);
    let mut bytes = Vec::new();
    patched.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, RELEASE);

    let (patched, _) = open_with(&installed, &release_plan(&folder), &folder.blobs()).unwrap();
    let mut archive = Archive::new(patched).unwrap();
    assert_eq!(
        archive.read("gameplay/items/new.options").unwrap().unwrap(),
        b"a new item\n"
    );
    assert_eq!(
        archive.read("gameplay/first.txt").unwrap().unwrap(),
        b"first file\n"
    );
    // Nothing was written.
    assert_eq!(std::fs::read(&installed).unwrap(), INSTALLED);
}

#[test]
fn an_empty_plan_is_the_installed_archive() {
    let folder = Folder::new("empty");
    let installed = folder.installed(INSTALLED);
    let (mut patched, applied) =
        open_with(&installed, "skhid-resources 1\r\nend\r\n", &folder.blobs()).unwrap();
    assert_eq!(
        applied,
        Applied {
            size: INSTALLED.len() as u64,
            ..Applied::default()
        }
    );
    let mut bytes = Vec::new();
    patched.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, INSTALLED);
    // Read from anywhere.
    patched.seek(SeekFrom::Start(9000)).unwrap();
    let mut rest = Vec::new();
    patched.read_to_end(&mut rest).unwrap();
    assert_eq!(rest, &INSTALLED[9000..]);
}

#[test]
fn regions_of_another_file_are_left_alone() {
    let folder = Folder::new("other");
    let installed = folder.installed(INSTALLED);
    let other = format!(
        "skhid-resources 1\r\nregion game/other.db 10 0 2 {} {} other\r\nend\r\n",
        hex(&digest(b"ab")),
        hex(&digest(b"cd"))
    );
    let (_, applied) = open_with(&installed, &other, &folder.blobs()).unwrap();
    assert!(applied.regions.is_empty());
}

#[test]
fn whatever_is_not_the_plans_is_refused() {
    let folder = Folder::new("refused");
    let plan = release_plan(&folder);
    let message = |installed: &[u8], plan: &str| {
        let path = folder.installed(installed);
        open_with(&path, plan, &folder.blobs())
            .map(|_| ())
            .unwrap_err()
            .to_string()
    };
    // Another installed build: another size, other bytes.
    assert!(message(&INSTALLED[..9000], &plan).contains("9000"));
    let mut other = INSTALLED.to_vec();
    other[100] ^= 1;
    assert!(message(&other, &plan).contains("another build"));
    // A missing blob, a blob of other bytes, of another size.
    let head = hex(&digest(&RELEASE[..INSTALLED.len()]));
    let tail = hex(&digest(&RELEASE[INSTALLED.len()..]));
    std::fs::remove_file(folder.blobs().join(&tail)).unwrap();
    assert!(message(INSTALLED, &plan).contains("no blob"));
    std::fs::write(folder.blobs().join(&tail), b"a new item!").unwrap();
    assert!(message(INSTALLED, &plan).contains("not the plan's"));
    std::fs::write(folder.blobs().join(&tail), b"x").unwrap();
    assert!(message(INSTALLED, &plan).contains("is 1 bytes"));
    std::fs::remove_file(folder.blobs().join(&head)).unwrap();
    assert!(message(INSTALLED, &plan).contains("resources_head: no blob"));
}

/// A ZIP of stored entries.
pub(crate) fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in files {
        let crc = updater_logic::inflate::crc32(0, data);
        let at = zip.len() as u32;
        let fields = |zip: &mut Vec<u8>| {
            zip.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            zip.extend_from_slice(&crc.to_le_bytes());
            zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
            zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
            zip.extend_from_slice(&[0, 0]);
        };
        zip.extend_from_slice(b"PK\x03\x04");
        fields(&mut zip);
        zip.extend_from_slice(name.as_bytes());
        zip.extend_from_slice(data);
        directory.extend_from_slice(b"PK\x01\x02\x14\x00");
        fields(&mut directory);
        // Comment length, disk, internal and external attributes.
        directory.extend_from_slice(&[0; 10]);
        directory.extend_from_slice(&at.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let start = zip.len() as u32;
    zip.extend_from_slice(&directory);
    zip.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    zip.extend_from_slice(&(files.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(files.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    zip.extend_from_slice(&start.to_le_bytes());
    zip.extend_from_slice(&[0, 0]);
    zip
}

#[test]
fn the_plan_is_read_from_the_players_zip() {
    let folder = Folder::new("zip");
    let installed = folder.installed(INSTALLED);
    let plan = release_plan(&folder);
    let zip = folder.0.join("players.zip");
    std::fs::write(
        &zip,
        zip_of(&[("README.txt", b"hello"), (PLAN_NAME, plan.as_bytes())]),
    )
    .unwrap();
    let (mut archive, applied) = open_installed(&installed, &zip, &folder.blobs()).unwrap();
    assert_eq!(applied.tail.as_deref(), Some("resources_tail"));
    assert!(
        archive
            .read("gameplay/items/new.options")
            .unwrap()
            .is_some()
    );

    // A ZIP without a plan: the installed archive as it is.
    std::fs::write(&zip, zip_of(&[("README.txt", b"hello")])).unwrap();
    let (mut archive, applied) = open_installed(&installed, &zip, &folder.blobs()).unwrap();
    assert_eq!(applied.size, INSTALLED.len() as u64);
    assert_eq!(archive.read("gameplay/items/new.options").unwrap(), None);

    // A plan the updater would not read.
    std::fs::write(&zip, zip_of(&[(PLAN_NAME, b"skhid-resources 1\r\nregion")])).unwrap();
    let message = open_installed(&installed, &zip, &folder.blobs())
        .unwrap_err()
        .to_string();
    assert!(message.contains(PLAN_NAME), "{message}");
    std::fs::write(&zip, b"not a zip").unwrap();
    assert!(open_installed(&installed, &zip, &folder.blobs()).is_err());
}

/// 0.26g's archive with a players' ZIP and its blobs:
/// `SRP_TEST_RESOURCES_DB`, `SRP_TEST_PLAYERS_ZIP`, `SRP_TEST_RESOURCE_BLOBS`.
#[test]
#[ignore = "needs 0.26g's resources.db and a players' ZIP with its blobs"]
fn a_release_of_0_26g_opens() {
    let var = |name: &str| PathBuf::from(std::env::var(name).expect(name));
    let started = std::time::Instant::now();
    let (archive, applied) = open_installed(
        &var("SRP_TEST_RESOURCES_DB"),
        &var("SRP_TEST_PLAYERS_ZIP"),
        &var("SRP_TEST_RESOURCE_BLOBS"),
    )
    .unwrap();
    eprintln!(
        "{applied:?}: {} files, opened and checked in {:?}",
        archive.len(),
        started.elapsed()
    );
}
