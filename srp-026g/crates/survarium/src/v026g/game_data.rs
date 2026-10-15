//! The game's files srp takes its 0.26g data from: those of the
//! `resources.db` the players run, read once at start.
//!
//! `srp` reads the server's game data of the release (`SRP_GAME_DATA`):
//! the files of the players' archive it needs, made by `srp-game-data
//! extract` from 0.26g's `resources.db` with the plan of the players' ZIP
//! ([`resources_db::server`]), each checked against the players' file
//! table there. It names that plan, and `srp` refuses to start unless it is
//! the plan of the ZIP it hands out at `/download` (`SRP_DOWNLOAD_FILE`): the
//! server cannot run with data other than what the players install. So the
//! dictionary, every item's and map's config and the other `gameplay/`
//! files are the players' own, of the release they run: the server cannot
//! know an item, or a value of one, the players' game does not. A missing or
//! wrong file stops `srp` at start ([`init`]).
//!
//! The tables are derived from these files when first used, as the game
//! reads them: the dictionary ([`super::dictionary`]), each item's
//! parameters and the slots it covers ([`super::item_params`]), what it may
//! be modified with and the modifications ([`super::modifications`]), the
//! game modes ([`super::maps`]), drops, quests, skills, awards and the test
//! profiles. What the game's files do not say stays in code: the shop's
//! exceptions ([`super::prices::EXCEPTIONS`]), the respawn points of a
//! project ([`super::match_msgs::rules`]), the bundles.
//!
//! Without [`init`] (tests, tools), the data is that of
//! `resources/game-026g/players.db`: the server's game data of the release
//! the players run, as `srp-game-data extract` makes it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use binary_config_parser::tree::ConfigValue;
use resources_db::Archive;
use resources_db::server::{GAMEPLAY, PROJECT_MARKER, PROJECTS, check, plan_line};

/// The server's game data of the release (`srp-game-data extract`).
pub const GAME_DATA_ENV: &str = "SRP_GAME_DATA";
/// The players' ZIP (the web server's `/download`).
pub const DOWNLOAD_FILE_ENV: &str = "SRP_DOWNLOAD_FILE";

/// The server's game data of the release the players run
/// (`resources/game-026g/README.md`).
const TEST_ARCHIVE: &[u8] = include_bytes!("../../../../resources/game-026g/players.db");

/// Why the game's files cannot be had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameDataError(pub String);

impl fmt::Display for GameDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the game's files: {}", self.0)
    }
}

impl std::error::Error for GameDataError {}

fn error(message: impl Into<String>) -> GameDataError {
    GameDataError(message.into())
}

/// The game's files srp reads, and where they are from.
#[derive(Debug)]
pub struct GameData {
    /// Where they were read, for the log.
    pub origin: String,
    /// The plan of the players' ZIP they were made for
    /// ([`resources_db::server::PROVENANCE`]).
    pub provenance: String,
    /// Every file under `gameplay/`, by its path in the archive.
    files: BTreeMap<String, Vec<u8>>,
    /// The projects the archive has (`projects/<project>/client_project`).
    projects: BTreeSet<String>,
}

impl GameData {
    /// The `gameplay/` files and the projects of `archive`.
    pub fn from_archive<R: Read + Seek>(
        archive: &mut Archive<R>,
        origin: String,
        provenance: String,
    ) -> Result<Self, GameDataError> {
        let paths: Vec<String> = archive
            .files_under(GAMEPLAY)
            .map(|(path, _)| path.to_owned())
            .collect();
        let mut files = BTreeMap::new();
        for path in paths {
            let bytes = archive
                .read(&path)
                .map_err(|cause| error(format!("{path}: {cause}")))?
                .expect("a listed file");
            files.insert(path, bytes);
        }
        let projects = archive
            .files_under(PROJECTS)
            .filter_map(|(path, _)| {
                path.strip_prefix(PROJECTS)?
                    .strip_suffix(PROJECT_MARKER)
                    .map(str::to_owned)
            })
            .collect();
        Ok(Self {
            origin,
            provenance,
            files,
            projects,
        })
    }

    /// The data of tests and tools ([`TEST_ARCHIVE`]).
    pub fn test_archive() -> Self {
        let mut archive =
            Archive::new(Cursor::new(TEST_ARCHIVE)).expect("the test archive is an archive");
        let provenance = archive
            .read(resources_db::server::PROVENANCE)
            .expect("the test archive reads")
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        Self::from_archive(
            &mut archive,
            "resources/game-026g/players.db".into(),
            provenance,
        )
        .expect("the test archive reads")
    }

    /// The server's game data `data`, if it was made for the plan of the
    /// players' ZIP `zip` (none: no ZIP, so the data of no plan).
    pub fn open(data: &Path, zip: Option<&Path>) -> Result<Self, GameDataError> {
        let file = std::fs::File::open(data)
            .map_err(|cause| error(format!("{}: {cause}", data.display())))?;
        let mut archive =
            Archive::new(file).map_err(|cause| error(format!("{}: {cause}", data.display())))?;
        let zip_bytes = zip
            .map(|zip| {
                std::fs::read(zip).map_err(|cause| error(format!("{}: {cause}", zip.display())))
            })
            .transpose()?;
        let provenance = match check(&mut archive, zip_bytes.as_deref())
            .map_err(|cause| error(format!("{}: {cause}", data.display())))?
        {
            Ok(provenance) => provenance,
            Err(mismatch) => {
                let zip_name = zip.map_or("(no SRP_DOWNLOAD_FILE)".into(), |zip| {
                    zip.display().to_string()
                });
                return Err(error(format!(
                    "{} was made for {}, but the players' ZIP {zip_name} has {}: the server \
                     would not know what the players install. Make the server's data of that \
                     ZIP (make_client_026g.py writes it beside the ZIP it makes), or by hand: \
                     srp-game-data extract --db <0.26g resources.db> --zip {zip_name} \
                     --blobs <its resources/ folder> --out {}",
                    data.display(),
                    plan_line(&mismatch.made_for),
                    plan_line(&mismatch.zip_has),
                    data.display()
                )));
            }
        };
        let origin = match zip {
            Some(zip) => format!("{}, made for {}", data.display(), zip.display()),
            None => data.display().to_string(),
        };
        Self::from_archive(&mut archive, origin, provenance)
    }

    /// The bytes of the file at `path`, if the archive has it.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    /// Whether the archive has the project `project` (a game mode's
    /// `match_making_params.map_name`, e.g. `level_01/day`).
    pub fn has_project(&self, project: &str) -> bool {
        self.projects.contains(project)
    }

    /// How many `gameplay/` files there are.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

static SHARED: OnceLock<GameData> = OnceLock::new();

/// The game's files of this process: those [`init`] read, else those of
/// the test archive.
pub fn shared() -> &'static GameData {
    SHARED.get_or_init(GameData::test_archive)
}

/// The file at `path` of [`shared`]. Panics without it: [`init`] checks
/// every file the tables read first.
pub fn file(path: &str) -> &'static [u8] {
    shared()
        .file(path)
        .unwrap_or_else(|| panic!("{}: no {path}", shared().origin))
}

/// The game's files `srp` runs with: `SRP_GAME_DATA`, made for the plan
/// of `SRP_DOWNLOAD_FILE` (when that file is there).
pub fn from_env() -> Result<GameData, GameDataError> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let data = var(GAME_DATA_ENV).map(PathBuf::from).ok_or_else(|| {
        error(format!(
            "{GAME_DATA_ENV} is unset: it names the server's game data of the release the \
             players run (server-game-data.db, beside the players' ZIP)"
        ))
    })?;
    let zip = var(DOWNLOAD_FILE_ENV)
        .map(PathBuf::from)
        .filter(|zip| zip.is_file());
    GameData::open(&data, zip.as_deref())
}

/// Reads the game's files ([`from_env`]), makes them this process's, and
/// derives every table from them and checks it ([`super::tables::check`]).
/// `srp` calls it before anything else: whatever is wrong stops it here.
/// The files are the process's before the check, so that a table one
/// parser takes from another ([`super::item_params::all`] and the like) is
/// built from them and no other.
pub fn init() -> Result<&'static GameData, GameDataError> {
    let data = from_env()?;
    if SHARED.set(data).is_err() {
        return Err(error(
            "a table was used before the game's files were read; srp reads them first",
        ));
    }
    let data = shared();
    super::tables::check(data)?;
    log::info!(
        "game data: {} ({} gameplay files, {} projects; {})",
        data.origin,
        data.len(),
        data.projects.len(),
        plan_line(&data.provenance)
    );
    Ok(data)
}

/// A config of the game's files, parsed.
pub fn config(data: &GameData, path: &str) -> Result<Option<ConfigValue>, GameDataError> {
    data.file(path)
        .map(|bytes| {
            ConfigValue::parse(bytes).map_err(|cause| error(format!("{path}: {}", cause.0)))
        })
        .transpose()
}

#[cfg(test)]
mod tests;
