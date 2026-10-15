// SPDX-License-Identifier: GPL-3.0-or-later
//! The server's game data: the files of the players' archive that `srp`
//! reads, in an archive of their own ([`crate::write`]) that names the
//! players' plan it was made for.
//!
//! [`extract`] takes them from 0.26g's `resources.db` with the plan of the
//! players' ZIP laid over it ([`crate::plan`]): every file under `gameplay/`,
//! and each project's `client_project` (srp only asks whether a project is
//! there, so it is kept empty). Each is checked against the full archive's
//! table first: its size, and the table's CRC-32 of its bytes
//! ([`crate::crc32`]). The table itself is the plan's, checked by sha256.
//!
//! [`PROVENANCE`] names the plan: its sha256 and its lines for
//! `game/resources.db` (with the installed file's size and the sha256 of
//! each range a region replaces, and of each blob). `srp` reads the plan of
//! the ZIP it hands out and refuses data made for another one ([`check`]):
//! the server cannot run with data other than the players install.
//!
//! The same inputs give the same bytes.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use updater_logic::resources::PLAN_NAME;
use updater_logic::sha256::{Sha256, to_hex};

use crate::plan::{RESOURCES_DB, open_installed, plan_text_of_zip};
use crate::{Archive, Error, crc32};

/// The files of `gameplay/` are read whole.
pub const GAMEPLAY: &str = "gameplay/";
/// A project is there when `projects/<project>/client_project` is.
pub const PROJECTS: &str = "projects/";
pub const PROJECT_MARKER: &str = "/client_project";
/// What the data was made from.
pub const PROVENANCE: &str = "srp/provenance";
const FORMAT: &str = "skhid server game data 1";

/// Whether the server reads `path` of the players' archive.
pub fn is_read(path: &str) -> bool {
    path.starts_with(GAMEPLAY) || (path.starts_with(PROJECTS) && path.ends_with(PROJECT_MARKER))
}

fn hex(digest: &[u8; 32]) -> String {
    String::from_utf8_lossy(&to_hex(digest)).into_owned()
}

/// What data made for the players' ZIP `zip` says it was made from: its
/// plan's sha256 and the plan's lines for `game/resources.db`; `plan none`
/// for no ZIP, or a ZIP without a plan.
pub fn provenance(zip: Option<&[u8]>) -> Result<String, Error> {
    let plan = match zip {
        Some(zip) => plan_text_of_zip(zip)?,
        None => None,
    };
    let mut text = format!("{FORMAT}\n");
    match plan {
        None => text.push_str("plan none\n"),
        Some(plan) => {
            let mut hasher = Sha256::new();
            hasher.update(&plan);
            text.push_str(&format!("plan {}\n", hex(&hasher.finish())));
            for line in String::from_utf8_lossy(&plan).lines() {
                let line = line.trim_end_matches('\r');
                let mut words = line.split(' ');
                if matches!(words.next(), Some("region" | "tail"))
                    && words.next() == Some(RESOURCES_DB)
                {
                    text.push_str(line);
                    text.push('\n');
                }
            }
        }
    }
    Ok(text)
}

/// What [`extract`] made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extracted {
    /// The files of `gameplay/`.
    pub gameplay: usize,
    /// The projects.
    pub projects: usize,
    pub provenance: String,
}

/// The server's files of `archive`, each checked against its table.
fn server_files<R: Read + Seek>(
    archive: &mut Archive<R>,
) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let read: Vec<(String, crate::Entry)> = archive
        .files_under("")
        .filter(|(path, _)| is_read(path))
        .map(|(path, entry)| (path.to_owned(), *entry))
        .collect();
    let mut files = BTreeMap::new();
    for (path, entry) in read {
        let bytes = archive.read(&path)?.expect("a listed file");
        if bytes.len() != entry.size as usize || crc32(&bytes) != entry.hash {
            return Err(crate::format(format!(
                "{path}: its bytes are not those its table says (size {}, CRC-32 {:08x})",
                entry.size, entry.hash
            )));
        }
        let kept = if path.starts_with(PROJECTS) {
            Vec::new()
        } else {
            bytes
        };
        files.insert(path, kept);
    }
    Ok(files)
}

/// The server's game data of 0.26g's `installed` archive with the plan of
/// the players' ZIP `zip` laid over it, the blobs in `blobs`; without a ZIP,
/// of the installed archive as it is.
pub fn extract(
    installed: &Path,
    zip: Option<&Path>,
    blobs: &Path,
) -> Result<(Vec<u8>, Extracted), Error> {
    let (mut files, zip_bytes) = match zip {
        Some(zip) => {
            let bytes = std::fs::read(zip)?;
            let (mut archive, _) = open_installed(installed, zip, blobs)?;
            (server_files(&mut archive)?, Some(bytes))
        }
        None => (
            server_files(&mut Archive::new(File::open(installed)?)?)?,
            None,
        ),
    };
    let provenance = provenance(zip_bytes.as_deref())?;
    let extracted = Extracted {
        gameplay: files
            .keys()
            .filter(|path| path.starts_with(GAMEPLAY))
            .count(),
        projects: files
            .keys()
            .filter(|path| path.starts_with(PROJECTS))
            .count(),
        provenance: provenance.clone(),
    };
    if extracted.gameplay == 0 {
        return Err(crate::format(format!(
            "{} has no {GAMEPLAY}",
            installed.display()
        )));
    }
    files.insert(PROVENANCE.to_owned(), provenance.into_bytes());
    Ok((crate::write::write(&files)?, extracted))
}

/// Why the server's game data is not for the players' ZIP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    /// What the data says it was made from ([`PROVENANCE`]).
    pub made_for: String,
    /// What the ZIP's plan is.
    pub zip_has: String,
}

/// The [`PROVENANCE`] of the server's game data `archive`, if it names the
/// plan of the players' ZIP `zip` (`None`: no ZIP): `Ok(Err(..))` if it names
/// another.
pub fn check<R: Read + Seek>(
    archive: &mut Archive<R>,
    zip: Option<&[u8]>,
) -> Result<Result<String, Mismatch>, Error> {
    let made_for = archive.read(PROVENANCE)?.ok_or_else(|| {
        crate::format(format!(
            "no {PROVENANCE}: not server game data (srp-game-data extract)"
        ))
    })?;
    let made_for = String::from_utf8_lossy(&made_for).into_owned();
    let zip_has = provenance(zip)?;
    Ok(if made_for == zip_has {
        Ok(made_for)
    } else {
        Err(Mismatch { made_for, zip_has })
    })
}

/// The first line naming the plan, of a [`PROVENANCE`] text.
pub fn plan_line(provenance: &str) -> &str {
    provenance
        .lines()
        .find(|line| line.starts_with("plan "))
        .unwrap_or("plan ?")
}

/// The plan file's name in a players' ZIP.
pub const ZIP_PLAN: &str = PLAN_NAME;

#[cfg(test)]
mod tests;
