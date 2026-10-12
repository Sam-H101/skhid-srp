// Read access to the game's own `resources.db`, so combat uses the shipped
// gameplay data rather than copies of it.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use binary_config_parser::{ConfigError, ConfigValue};
use resources_db::{Archive, FileEntry, NodeKind};

/// Where the game keeps it, relative to srp's usual working directory.
pub const DEFAULT_PATH: &str = "../Survarium/resources.db";
pub const PATH_VARIABLE: &str = "SRP_RESOURCES_DB";

#[derive(Debug)]
pub struct ResourceError(pub String);

impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ResourceError {}

impl From<ConfigError> for ResourceError {
    fn from(error: ConfigError) -> Self {
        Self(error.to_string())
    }
}

pub struct Resources {
    archive: Archive,
    files: HashMap<String, FileEntry>,
}

impl Resources {
    /// `SRP_RESOURCES_DB`, or the game's copy beside srp.
    pub fn configured_path() -> PathBuf {
        std::env::var_os(PATH_VARIABLE)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_PATH))
    }

    pub fn open(path: &Path) -> Result<Self, ResourceError> {
        let archive = Archive::open(path)
            .map_err(|error| ResourceError(format!("{}: {error}", path.display())))?;
        // The unpacker indexes slices directly; a damaged file must not take the
        // server down, so treat a panic while listing as a load error.
        let entries = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| archive.list()))
            .map_err(|_| ResourceError(format!("{}: unreadable file index", path.display())))?;
        let files = entries
            .into_iter()
            .filter(|entry| matches!(entry.kind, NodeKind::File { .. }))
            .map(|entry| (normalize(&String::from_utf8_lossy(&entry.path)), entry))
            .collect();
        Ok(Self { archive, files })
    }

    pub fn exists(&self, path: &str) -> bool {
        self.files.contains_key(&normalize(path))
    }

    /// File bytes by engine path; a leading `resources/` is optional.
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, ResourceError> {
        let entry = self
            .files
            .get(&normalize(path))
            .ok_or_else(|| ResourceError(format!("missing resource `{path}`")))?;
        self.archive
            .read_file(entry)
            .map_err(|error| ResourceError(format!("{path}: {error}")))
    }

    pub fn config(&mut self, path: &str) -> Result<ConfigValue, ResourceError> {
        let bytes = self.read(path)?;
        ConfigValue::parse(&bytes).map_err(|error| ResourceError(format!("{path}: {error}")))
    }
}

/// Engine paths as the archive stores them: no `resources/` prefix, forward
/// slashes, lower case.
fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    path.strip_prefix("resources/").unwrap_or(&path).to_owned()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// The game's data when it is installed beside srp; tests that need it
    /// skip (with a note) otherwise.
    pub fn game_resources() -> Option<Resources> {
        let path = Resources::configured_path();
        let path = if path.is_relative() {
            // Tests run from the crate directory.
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(path)
        } else {
            path
        };
        match Resources::open(&path) {
            Ok(resources) => Some(resources),
            Err(error) => {
                eprintln!("skipping: game data not available ({error})");
                None
            }
        }
    }

    #[test]
    fn normalizes_engine_paths() {
        assert_eq!(
            normalize("resources/gameplay\\Weapons/ak_74u.options"),
            "gameplay/weapons/ak_74u.options"
        );
    }

    #[test]
    fn reads_a_shipped_config() {
        let Some(mut resources) = game_resources() else {
            return;
        };
        let weapon = resources.config("gameplay/weapons/ak_74u.options").unwrap();
        let parameters = weapon.req("parameters").unwrap();
        assert_eq!(parameters.f32_at("bullet_damage").unwrap(), 0.4);
        assert_eq!(parameters.i32_at("magazine_capacity").unwrap(), 30);
    }
}
