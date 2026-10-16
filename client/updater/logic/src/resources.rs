//! The one-time restore of the game's `resources.db` (`client/updater/README.md`,
//! "Restoring resources.db"): the record an earlier updater kept of the
//! regions of the db it rewrote in place, and what is to be done with each
//! to give it its stock bytes back.
//!
//! Content now comes as the db's patches only (`crate::patches`). Before
//! them the players' ZIP had a plan, and the updater wrote the plan's entries
//! over the db in place (the settings menu's, the lobby's and the login
//! screen's Flash movies: `main_menu`, `lobby_menu`, `login_menu`). It
//! recorded each region before its first byte was written, in
//! `updater_skhid.resources.state` beside it:
//!
//! ```text
//! skhid-resources-state 1
//! region <path> <file size> <offset> <size> <original sha256> <target sha256> <name>
//! end
//! ```
//!
//! and in `updater_skhid.resources.journal` the one region being written.
//! `<original>` is the SHA-256 of the region's stock bytes, `<file size>`
//! the size of the db of that build. Every recorded region now gets its
//! original back; then the record is gone and nothing more is done, ever.
//!
//! A line of another word is skipped (`tail` and `file` lines were never in
//! a players' plan); a record of a format above 2, one cut short (no `end`)
//! or with a region line that cannot be read is no record (nothing is
//! changed). Only regions of `game/resources.db` are taken.

use alloc::string::String;
use alloc::vec::Vec;

use crate::manifest::{lines, number, push_number, word};
use crate::sha256::{from_hex, to_hex};

const RECORD_MAGIC: &[u8] = b"skhid-resources-state";
/// The one file a region was ever written in.
pub const DB_PATH: &str = "game/resources.db";
/// The most regions a record may have.
const MAX_REGIONS: usize = 256;
/// The longest region name.
const MAX_LABEL: usize = 64;

/// Bytes of the db that an earlier updater changed from `original` (its
/// stock bytes) to `target`.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Region {
    pub path: String,
    pub file_size: u64,
    pub offset: u64,
    pub size: u64,
    pub original: [u8; 32],
    pub target: [u8; 32],
    pub name: String,
}

impl Region {
    /// The same bytes of the same file of the same build, the same original
    /// in them: the same region, whatever its target was.
    pub fn same_place(&self, other: &Region) -> bool {
        self.path.eq_ignore_ascii_case(&other.path)
            && self.file_size == other.file_size
            && self.offset == other.offset
            && self.size == other.size
            && self.original == other.original
    }
}

/// Exactly `N` values, one space between each.
fn fields<const N: usize>(values: &[u8]) -> Option<[&[u8]; N]> {
    let mut out = [&values[..0]; N];
    let mut parts = values.split(|&byte| byte == b' ');
    for slot in &mut out {
        *slot = parts.next().filter(|part| !part.is_empty())?;
    }
    parts.next().is_none().then_some(out)
}

/// A region's name: what the log and the launcher's line say.
fn label(text: &[u8]) -> Option<String> {
    let ok = !text.is_empty()
        && text.len() <= MAX_LABEL
        && text
            .iter()
            .all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'));
    ok.then(|| String::from_utf8_lossy(text).into())
}

fn region(values: &[u8]) -> Option<Region> {
    let [path, file_size, offset, size, original, target, name] = fields::<7>(values)?;
    let path = core::str::from_utf8(path).ok()?;
    if !path.eq_ignore_ascii_case(DB_PATH) {
        return None;
    }
    let region = Region {
        path: DB_PATH.into(),
        file_size: number(file_size)?,
        offset: number(offset)?,
        size: number(size)?,
        original: from_hex(original)?,
        target: from_hex(target)?,
        name: label(name)?,
    };
    let fits = region.size > 0
        && region
            .offset
            .checked_add(region.size)
            .is_some_and(|end| end <= region.file_size);
    (fits && region.original != region.target).then_some(region)
}

/// Reads the record (or the journal): its regions. `None` for anything
/// that is not a whole, well-formed one.
pub fn parse_record(text: &[u8]) -> Option<Vec<Region>> {
    let mut lines = lines(text);
    let (name, version) = word(lines.next()?);
    if name != RECORD_MAGIC || !matches!(number(version), Some(1 | 2)) {
        return None;
    }
    let mut regions: Vec<Region> = Vec::new();
    for line in lines {
        match word(line) {
            (b"region", values) => {
                let region = region(values)?;
                if !regions.iter().any(|other| other.same_place(&region)) {
                    regions.push(region);
                }
                if regions.len() > MAX_REGIONS {
                    return None;
                }
            }
            (b"end", _) => return Some(regions),
            _ => {}
        }
    }
    None
}

/// The record's text, as the earlier updaters wrote it.
pub fn write_record(regions: &[Region]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(RECORD_MAGIC);
    out.extend_from_slice(b" 1\r\n");
    for region in regions {
        out.extend_from_slice(b"region ");
        out.extend_from_slice(region.path.as_bytes());
        for value in [region.file_size, region.offset, region.size] {
            out.push(b' ');
            push_number(&mut out, value);
        }
        for hash in [&region.original, &region.target] {
            out.push(b' ');
            out.extend_from_slice(&to_hex(hash));
        }
        out.push(b' ');
        out.extend_from_slice(region.name.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"end\r\n");
    out
}

/// What a look at a region's bytes found.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Look {
    /// The file is not there.
    Missing,
    /// The file has another size than in the region's build of the game.
    OtherSize(u64),
    /// The file cannot be read now.
    Unreadable,
    /// The SHA-256 of the region's bytes.
    Bytes([u8; 32]),
}

/// What to do with a recorded region.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Undo {
    /// Nothing to put back (the stock bytes are there, or the file is gone
    /// or of another build): the record of it goes.
    Forget,
    /// The stock bytes are to be put back.
    Restore,
    /// The file cannot be read now: the record stays for the next run.
    Wait,
}

pub fn undo(region: &Region, look: Look) -> Undo {
    match look {
        Look::Missing | Look::OtherSize(_) => Undo::Forget,
        Look::Unreadable => Undo::Wait,
        Look::Bytes(hash) if hash == region.original => Undo::Forget,
        Look::Bytes(_) => Undo::Restore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::digest;

    fn hex(data: &[u8]) -> String {
        String::from_utf8(to_hex(&digest(data)).to_vec()).unwrap()
    }

    /// The live plan's three regions of build 2777's resources.db, with
    /// made-up targets.
    fn record() -> Vec<Region> {
        [
            ("main_menu", 17_970_448, 1_194_596),
            ("lobby_menu", 19_165_044, 38_985_769),
            ("login_menu", 58_150_813, 3_508_744),
        ]
        .into_iter()
        .map(|(name, offset, size)| Region {
            path: DB_PATH.into(),
            file_size: 6_285_533_906,
            offset,
            size,
            original: digest(alloc::format!("{name} original").as_bytes()),
            target: digest(alloc::format!("{name} target").as_bytes()),
            name: name.into(),
        })
        .collect()
    }

    #[test]
    fn the_record_the_earlier_updaters_wrote_is_read() {
        let regions = record();
        let text = write_record(&regions);
        assert_eq!(parse_record(&text), Some(regions.clone()));
        // The line the earlier updaters wrote, byte for byte.
        let line = alloc::format!(
            "region game/resources.db 6285533906 19165044 38985769 {} {} lobby_menu\r\n",
            hex(b"lobby_menu original"),
            hex(b"lobby_menu target")
        );
        assert!(String::from_utf8(text.clone()).unwrap().contains(&line));
        assert!(text.starts_with(b"skhid-resources-state 1\r\n"));
        // LF only, upper-case hex, other words (a tail, a whole file),
        // format 2, and lines after the end are all fine.
        let other = String::from_utf8(text)
            .unwrap()
            .replace("\r\n", "\n")
            .replace("skhid-resources-state 1", "skhid-resources-state 2")
            .replace(
                &hex(b"login_menu target"),
                &hex(b"login_menu target").to_ascii_uppercase(),
            )
            .replace(
                "end\n",
                "tail game/resources.db 1 2 aa new_items file_table\nfile aa 1 game/x.db\nend\nregion not read\n",
            );
        assert_eq!(parse_record(other.as_bytes()), Some(regions));
        assert_eq!(
            parse_record(b"skhid-resources-state 1\nend\n"),
            Some(Vec::new())
        );
    }

    #[test]
    fn a_record_that_cannot_be_read_is_none() {
        let text = write_record(&record());
        for cut in 0..text.len() - 4 {
            assert_eq!(parse_record(&text[..cut]), None, "cut at {cut}");
        }
        let text = String::from_utf8(text).unwrap();
        for (from, to) in [
            ("skhid-resources-state 1", "skhid-resources-state 3"),
            ("skhid-resources-state 1", "skhid-resources 1"),
            // Only the db, and a plain number, checksum and name.
            (
                "game/resources.db 6285533906 19165044",
                "../resources.db 6285533906 19165044",
            ),
            (
                "game/resources.db 6285533906 19165044",
                "game/other.db 6285533906 19165044",
            ),
            (" 6285533906 19165044", " 6285533906 -19165044"),
            (" 38985769 ", " 0 "),
            (" lobby_menu", " lobby menu"),
            (&hex(b"lobby_menu original"), "zz"),
            // Past the file's end, or no change at all.
            (" 6285533906 19165044", " 58150812 19165044"),
            (&hex(b"lobby_menu target"), &hex(b"lobby_menu original")),
        ] {
            assert!(text.contains(from), "{from}");
            assert_eq!(
                parse_record(text.replacen(from, to, 1).as_bytes()),
                None,
                "{from} -> {to}"
            );
        }
        // The upper-case path of a Windows hand is the db too.
        let upper = text.replace("game/resources.db", "GAME/Resources.db");
        assert_eq!(parse_record(upper.as_bytes()), Some(record()));
    }

    #[test]
    fn a_region_twice_is_one() {
        let mut regions = record();
        let mut newer = regions[1].clone();
        newer.target = digest(b"a newer target");
        regions.push(newer);
        assert_eq!(parse_record(&write_record(&regions)), Some(record()));
        let mut build = regions[0].clone();
        build.file_size += 1;
        assert!(!build.same_place(&regions[0]));
    }

    #[test]
    fn what_a_region_holds_decides_what_is_done() {
        let region = &record()[1];
        assert_eq!(undo(region, Look::Bytes(region.original)), Undo::Forget);
        assert_eq!(undo(region, Look::Bytes(region.target)), Undo::Restore);
        assert_eq!(undo(region, Look::Bytes(digest(b"x"))), Undo::Restore);
        assert_eq!(undo(region, Look::Missing), Undo::Forget);
        assert_eq!(undo(region, Look::OtherSize(1)), Undo::Forget);
        assert_eq!(undo(region, Look::Unreadable), Undo::Wait);
    }
}
