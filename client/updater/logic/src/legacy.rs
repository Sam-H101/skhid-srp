//! What the updaters before the database patches changed in the game's own
//! `resources.db`, read back so it can be undone once: the regions they
//! wrote in place (the menus) and the tails they appended, as their record
//! `updater_skhid.resources.state` (and the journal of a write cut short,
//! the same lines) says. `client/updater/README.md`, "Putting resources.db
//! back to stock". This goes once every player's db is stock again.
//!
//! ```text
//! skhid-resources-state 1        (2 with a tail)
//! region <path> <file size> <offset> <size> <original sha256> <target sha256> <name>
//! tail <path> <file size> <new size> <tail sha256> <name> <region>
//! file <sha256> <size> <path>
//! end
//! ```
//!
//! Only what names `game/resources.db` is kept; `file` lines are skipped
//! (no release placed a whole file).

use alloc::string::String;
use alloc::vec::Vec;

use crate::manifest::{lines, number, word};
use crate::sha256::from_hex;

pub const RECORD_MAGIC: &[u8] = b"skhid-resources-state";
const DB: &str = "game/resources.db";

/// Bytes of the db written in place: `[offset, offset + size)` held
/// `original` as shipped.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Region {
    pub offset: u64,
    pub size: u64,
    pub original: [u8; 32],
    pub name: String,
}

/// What was recorded: the regions, and the size the db had before a tail
/// was appended to it.
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct Record {
    pub regions: Vec<Region>,
    pub shipped_size: Option<u64>,
}

impl Record {
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty() && self.shipped_size.is_none()
    }

    /// Adds what `other` has and this has not.
    pub fn merge(&mut self, other: Record) {
        for region in other.regions {
            if !self.regions.contains(&region) {
                self.regions.push(region);
            }
        }
        self.shipped_size = self.shipped_size.or(other.shipped_size);
    }
}

fn values<const N: usize>(text: &[u8]) -> Option<[&[u8]; N]> {
    let mut out = [&text[..0]; N];
    let mut parts = text.split(|&byte| byte == b' ');
    for slot in &mut out {
        *slot = parts.next().filter(|part| !part.is_empty())?;
    }
    parts.next().is_none().then_some(out)
}

fn region(text: &[u8]) -> Option<Option<Region>> {
    let [path, _, offset, size, original, _, name] = values::<7>(text)?;
    let region = Region {
        offset: number(offset)?,
        size: number(size).filter(|&size| size > 0)?,
        original: from_hex(original)?,
        name: String::from_utf8_lossy(name).into(),
    };
    region.offset.checked_add(region.size)?;
    Some(path.eq_ignore_ascii_case(DB.as_bytes()).then_some(region))
}

fn tail(text: &[u8]) -> Option<Option<u64>> {
    let [path, shipped, ..] = values::<6>(text)?;
    let shipped = number(shipped)?;
    Some(path.eq_ignore_ascii_case(DB.as_bytes()).then_some(shipped))
}

/// The record or a journal. `None` for anything that is not a whole one.
pub fn parse_record(text: &[u8]) -> Option<Record> {
    let mut lines = lines(text);
    let (magic, version) = word(lines.next()?);
    if magic != RECORD_MAGIC || !matches!(number(version), Some(1 | 2)) {
        return None;
    }
    let mut record = Record::default();
    for line in lines {
        match word(line) {
            (b"region", text) => record.regions.extend(region(text)?),
            (b"tail", text) => {
                if let Some(shipped) = tail(text)? {
                    record.shipped_size = Some(shipped);
                }
            }
            (b"end", _) => return Some(record),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::{digest, to_hex};
    use alloc::format;

    fn hex(name: &[u8]) -> String {
        String::from_utf8(to_hex(&digest(name)).to_vec()).unwrap()
    }

    /// What the live players' updaters recorded: the three menus.
    fn live() -> String {
        format!(
            "skhid-resources-state 1\r\n\
             region game/resources.db 6285533906 17970448 1194596 {} {} main_menu\r\n\
             region game/resources.db 6285533906 19165044 38985769 {} {} lobby_menu\r\n\
             region game/resources.db 6285533906 58150813 3508744 {} {} login_menu\r\n\
             end\r\n",
            hex(b"main"),
            hex(b"main live"),
            hex(b"lobby"),
            hex(b"lobby live"),
            hex(b"login"),
            hex(b"login live")
        )
    }

    #[test]
    fn the_live_record_is_read() {
        let record = parse_record(live().as_bytes()).unwrap();
        assert_eq!(record.regions.len(), 3);
        assert_eq!(
            record.regions[1],
            Region {
                offset: 19_165_044,
                size: 38_985_769,
                original: digest(b"lobby"),
                name: "lobby_menu".into()
            }
        );
        assert_eq!(record.shipped_size, None);
        let text = live();
        for cut in 0..text.len() - 4 {
            assert_eq!(parse_record(&text.as_bytes()[..cut]), None, "cut at {cut}");
        }
    }

    #[test]
    fn a_tail_and_other_files_are_told_apart() {
        let text = live()
            .replace("skhid-resources-state 1", "skhid-resources-state 2")
            .replace(
                "end\r\n",
                &format!(
                    "tail game/resources.db 6285533906 6328475738 {} new_items resources_head\r\n\
                 region game/other.db 10 0 5 {} {} other\r\n\
                 file {} 5 game/maps/x.db\r\nend\r\n",
                    hex(b"tail"),
                    hex(b"a"),
                    hex(b"b"),
                    hex(b"c")
                ),
            );
        let mut record = parse_record(text.as_bytes()).unwrap();
        assert_eq!(record.regions.len(), 3);
        assert_eq!(record.shipped_size, Some(6_285_533_906));
        // A journal adds what it has.
        let journal = parse_record(live().as_bytes()).unwrap();
        record.merge(journal);
        assert_eq!(record.regions.len(), 3);
        assert!(parse_record(b"skhid-resources-state 3\r\nend\r\n").is_none());
        assert!(parse_record(b"skhid-resources 1\r\nend\r\n").is_none());
        assert!(Record::default().is_empty());
    }
}
