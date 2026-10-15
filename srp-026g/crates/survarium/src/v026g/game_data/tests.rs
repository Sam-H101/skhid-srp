use super::*;

#[test]
fn the_test_archive_is_the_releases_gameplay_files() {
    let data = shared();
    assert_eq!(data.origin, "resources/game-026g/players.db");
    // 0.26g's 508, and the configs of the Duty exoskeleton and the Bandit
    // balaclava.
    assert_eq!(data.len(), 510);
    assert!(
        data.file("gameplay/items/armour/back/duty_exo_026.options")
            .is_some()
    );
    assert!(
        data.file("gameplay/items/armour/mask/bandit_026.options")
            .is_some()
    );
    assert!(data.file("gameplay/missing").is_none());
    // The projects are there as markers; level_08 at night is not.
    assert!(data.has_project("level_01/day_2"));
    assert!(!data.has_project("level_08/night_2"));
    // It names the plan of the players' ZIP it was made of.
    assert!(plan_line(&data.provenance).starts_with("plan "));
    assert_ne!(plan_line(&data.provenance), "plan none");
    assert!(super::super::tables::check(data).is_ok());
}

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("srp-game-data-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn srp_needs_the_servers_data_of_the_players_zip() {
    // SAFETY: no other test of this crate reads these variables.
    unsafe {
        std::env::remove_var(GAME_DATA_ENV);
    }
    let message = from_env().unwrap_err().to_string();
    assert!(message.contains(GAME_DATA_ENV), "{message}");

    let folder = Folder::new("provenance");
    let data = folder.0.join("server-game-data.db");
    std::fs::write(&data, TEST_ARCHIVE).unwrap();
    // The test data was made for a release's ZIP: with no ZIP, or a ZIP
    // without a plan, srp does not start, and says how to make the data.
    let message = GameData::open(&data, None).unwrap_err().to_string();
    assert!(message.contains("plan none"), "{message}");
    assert!(message.contains("srp-game-data extract"), "{message}");
    let zip = folder.0.join("players.zip");
    std::fs::write(&zip, plain_zip()).unwrap();
    let message = GameData::open(&data, Some(&zip)).unwrap_err().to_string();
    assert!(message.contains(&zip.display().to_string()), "{message}");
    // Not server data at all.
    std::fs::write(&data, b"little-endian").unwrap();
    assert!(GameData::open(&data, None).is_err());
}

/// A ZIP of one stored file, without a plan.
fn plain_zip() -> Vec<u8> {
    let name = b"README.txt";
    let data = b"hello";
    let crc = crc32_ieee(data);
    let mut zip = Vec::new();
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
    zip.extend_from_slice(name);
    zip.extend_from_slice(data);
    let start = zip.len() as u32;
    let mut directory = b"PK\x01\x02\x14\x00".to_vec();
    fields(&mut directory);
    directory.extend_from_slice(&[0; 10]);
    directory.extend_from_slice(&0u32.to_le_bytes());
    directory.extend_from_slice(name);
    zip.extend_from_slice(&directory);
    zip.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
    zip.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    zip.extend_from_slice(&start.to_le_bytes());
    zip.extend_from_slice(&[0, 0]);
    zip
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name}")))
}

/// 0.26g's own archive (`SRP_TEST_RESOURCES_DB`) with no plan: its
/// `gameplay/` files are the test archive's but the dictionary and the
/// release's two configs, and every table derived from it is the test
/// archive's without the release's two items.
#[test]
#[ignore = "needs 0.26g's resources.db"]
fn the_installed_archive_of_0_26g_is_the_test_data_without_the_release() {
    use super::super::dictionary::{self, Dictionary};
    use super::super::{item_params, maps, modifications};
    let installed = env_path("SRP_TEST_RESOURCES_DB");
    let (bytes, _) = resources_db::server::extract(&installed, None, Path::new("")).unwrap();
    let mut archive = Archive::new(Cursor::new(bytes)).unwrap();
    let stock = GameData::from_archive(&mut archive, "0.26g".into(), String::new()).unwrap();
    let release = shared();
    super::super::tables::check(&stock).unwrap();
    let released = [
        dictionary::PATH,
        "gameplay/items/armour/back/duty_exo_026.options",
        "gameplay/items/armour/mask/bandit_026.options",
    ];
    assert_eq!(stock.len() + 2, release.len());
    for (path, bytes) in &release.files {
        if !released.contains(&path.as_str()) {
            assert_eq!(stock.file(path), Some(bytes.as_slice()), "{path}");
        }
    }
    assert_eq!(stock.projects, release.projects);
    let new = |id: u16| id == 1805 || id == 1810;
    let stock_dictionary = Dictionary::parse(stock.file(dictionary::PATH).unwrap()).unwrap();
    let mut items = Dictionary::shared().items.clone();
    items.retain(|item| !new(item.dict_id));
    assert_eq!(stock_dictionary.items, items);
    let mut params = item_params::all().to_vec();
    params.retain(|row| !new(row.dict_id));
    assert_eq!(
        item_params::derive(&stock, &stock_dictionary).unwrap(),
        params
    );
    let mut mods = modifications::all_item_mods().to_vec();
    mods.retain(|row| !new(row.dict_id));
    assert_eq!(
        modifications::derive_item_mods(&stock, &stock_dictionary).unwrap(),
        mods
    );
    assert_eq!(
        maps::derive(&stock, &stock_dictionary).unwrap(),
        maps::all()
    );
}

/// The candidate release: `srp-game-data extract` of 0.26g's archive
/// (`SRP_TEST_RESOURCES_DB`) with the players' ZIP (`SRP_TEST_PLAYERS_ZIP`)
/// and its blobs (`SRP_TEST_RESOURCE_BLOBS`) is the test data, byte for
/// byte, and srp takes it with that ZIP.
#[test]
#[ignore = "needs 0.26g's resources.db and the players' ZIP with its blobs"]
fn the_server_data_of_the_release_is_the_test_data() {
    let zip = env_path("SRP_TEST_PLAYERS_ZIP");
    let (bytes, extracted) = resources_db::server::extract(
        &env_path("SRP_TEST_RESOURCES_DB"),
        Some(&zip),
        &env_path("SRP_TEST_RESOURCE_BLOBS"),
    )
    .unwrap();
    eprintln!("{} bytes: {extracted:?}", bytes.len());
    assert!(bytes == TEST_ARCHIVE, "the test data is not the release's server data");
    let folder = Folder::new("release");
    let data = folder.0.join("server-game-data.db");
    std::fs::write(&data, &bytes).unwrap();
    let opened = GameData::open(&data, Some(&zip)).unwrap();
    super::super::tables::check(&opened).unwrap();
    assert_eq!(opened.files, shared().files);
    assert_eq!(opened.projects, shared().projects);
}
