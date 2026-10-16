//! The quick check of the patch files (`client/updater/README.md`,
//! "Patches"): what the updater found when it last hashed each one, so that
//! a start does not read gigabytes again to learn that nothing changed.
//!
//! After a patch file is downloaded or hashed whole, its SHA-256 is kept
//! with what Windows says of that file: its size, its last-write time and
//! its file ID (the volume's serial number and the 64-bit index of
//! `GetFileInformationByHandle`), and when it was hashed. At a later start
//! a file whose three values are still those, whose SHA-256 kept is the
//! list's, and which was hashed less than [`RECHECK_AFTER`] ago, counts as
//! checked without being read ([`Record::still`]). Anything else is hashed:
//! another size, time or ID (written to, replaced, copied back), another
//! SHA-256 in the list, no record or one that cannot be read, a hash older
//! than a week (or "later" than now: a clock set back). `updater_skhid.exe
//! check` hashes every file whatever the record says.
//!
//! The record, `updater_skhid.patches.checked` beside the updater:
//!
//! ```text
//! skhid-patches-checked 1
//! file resources.db.patch_12 <sha256> <size> <written> <volume> <index> <hashed>
//! end
//! ```
//!
//! `<written>` and `<hashed>` are Windows times (100 ns since 1601), all
//! numbers decimal. A record without its `end`, of another first line, or
//! with a line that cannot be read is no record: every file is hashed.

use alloc::string::String;
use alloc::vec::Vec;

use crate::patches::{FILE_PREFIX, file_number};
use crate::sha256::{from_hex, to_hex};

const HEADER: &str = "skhid-patches-checked 1";
const END: &str = "end";

/// One second in Windows time (100 ns).
pub const SECOND: u64 = 10_000_000;
/// How long a file's hash stands: a week. After it the file is hashed
/// again, unchanged or not.
pub const RECHECK_AFTER: u64 = 7 * 24 * 3600 * SECOND;

/// What Windows says of a file: whether it is still the file that was
/// hashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub size: u64,
    /// The last-write time (100 ns since 1601).
    pub written: u64,
    /// The volume's serial number ...
    pub volume: u32,
    /// ... and the file's index on it: together the file ID.
    pub index: u64,
}

impl Identity {
    /// Whether these values can tell the file: a file system that gives no
    /// index (0) or no time cannot, and its files are always hashed.
    pub fn usable(&self) -> bool {
        self.index != 0 && self.written != 0
    }
}

/// A patch file as it was when it was hashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checked {
    pub number: u8,
    pub sha256: [u8; 32],
    pub identity: Identity,
    /// When it was hashed (100 ns since 1601).
    pub hashed: u64,
}

/// The record: one entry each patch number at most.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    pub files: Vec<Checked>,
}

impl Record {
    pub fn get(&self, number: u8) -> Option<&Checked> {
        self.files.iter().find(|file| file.number == number)
    }

    /// Whether the file of patch `number` counts as checked now, with
    /// `sha256` (the list's) and no hashing: it is `identity` as when it was
    /// hashed, with that SHA-256 then, less than [`RECHECK_AFTER`] ago.
    pub fn still(&self, number: u8, sha256: &[u8; 32], identity: &Identity, now: u64) -> bool {
        self.get(number).is_some_and(|file| {
            identity.usable()
                && file.identity == *identity
                && file.sha256 == *sha256
                && file.hashed <= now
                && now - file.hashed < RECHECK_AFTER
        })
    }

    /// Notes a file just hashed (or downloaded and hashed): `identity` is
    /// what Windows said of it before and after the read.
    pub fn hashed(&mut self, number: u8, sha256: [u8; 32], identity: Identity, now: u64) {
        self.forget(number);
        if identity.usable() {
            self.files.push(Checked {
                number,
                sha256,
                identity,
                hashed: now,
            });
            self.files.sort_by_key(|file| file.number);
        }
    }

    pub fn forget(&mut self, number: u8) {
        self.files.retain(|file| file.number != number);
    }

    /// Only the entries of these patch numbers (the list's) stay.
    pub fn keep_only(&mut self, numbers: &[u8]) {
        self.files.retain(|file| numbers.contains(&file.number));
    }
}

/// Reads the record. Anything but a whole record is none (an empty one).
pub fn parse_record(bytes: &[u8]) -> Record {
    read(bytes).unwrap_or_default()
}

fn read(bytes: &[u8]) -> Option<Record> {
    let text = core::str::from_utf8(bytes).ok()?;
    let mut lines = text.lines().map(|line| line.trim_end_matches('\r'));
    if lines.next()? != HEADER {
        return None;
    }
    let mut record = Record::default();
    for line in lines {
        if line == END {
            return Some(record);
        }
        let mut words = line.split(' ');
        let (Some("file"), Some(name), Some(sha256), Some(size), Some(written)) = (
            words.next(),
            words.next(),
            words.next(),
            words.next(),
            words.next(),
        ) else {
            return None;
        };
        let (Some(volume), Some(index), Some(hashed), None) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            return None;
        };
        let number = file_number(name).filter(|_| name.starts_with(FILE_PREFIX))?;
        if record.get(number).is_some() {
            return None;
        }
        record.files.push(Checked {
            number,
            sha256: from_hex(sha256.as_bytes())?,
            identity: Identity {
                size: decimal(size)?,
                written: decimal(written)?,
                volume: u32::try_from(decimal(volume)?).ok()?,
                index: decimal(index)?,
            },
            hashed: decimal(hashed)?,
        });
    }
    None
}

fn decimal(text: &str) -> Option<u64> {
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The record as it is kept.
pub fn write_record(record: &Record) -> Vec<u8> {
    let mut out = String::from(HEADER);
    out.push_str("\r\n");
    for file in &record.files {
        let id = &file.identity;
        out.push_str(&alloc::format!(
            "file {FILE_PREFIX}{} {} {} {} {} {} {}\r\n",
            file.number,
            String::from_utf8_lossy(&to_hex(&file.sha256)),
            id.size,
            id.written,
            id.volume,
            id.index,
            file.hashed
        ));
    }
    out.push_str(END);
    out.push_str("\r\n");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::digest;

    const NOW: u64 = 134_050_000_000_000_000; // 2025-10-11 in Windows time
    const DAY: u64 = 24 * 3600 * SECOND;

    fn identity(index: u64) -> Identity {
        Identity {
            size: 86_199_749,
            written: NOW - 3 * DAY,
            volume: 0x1234_abcd,
            index,
        }
    }

    fn record() -> Record {
        let mut record = Record::default();
        record.hashed(14, digest(b"14"), identity(7), NOW - DAY);
        record.hashed(12, digest(b"12"), identity(5), NOW - 2 * DAY);
        record
    }

    #[test]
    fn a_file_as_it_was_hashed_counts_as_checked() {
        let record = record();
        assert!(record.still(12, &digest(b"12"), &identity(5), NOW));
        assert!(record.still(14, &digest(b"14"), &identity(7), NOW));
        // In order, one entry a number.
        assert_eq!(
            record.files.iter().map(|f| f.number).collect::<Vec<_>>(),
            [12, 14]
        );
    }

    #[test]
    fn a_changed_file_is_hashed_again() {
        let record = record();
        let sha = digest(b"12");
        let base = identity(5);
        for changed in [
            Identity {
                size: base.size + 1,
                ..base
            },
            Identity {
                written: base.written + 1,
                ..base
            },
            Identity {
                volume: base.volume + 1,
                ..base
            },
            // Another file of the name: copied back, replaced.
            Identity { index: 6, ..base },
        ] {
            assert!(!record.still(12, &sha, &changed, NOW), "{changed:?}");
        }
        // The list has another SHA-256 for it.
        assert!(!record.still(12, &digest(b"12 new"), &base, NOW));
        // No entry for it.
        assert!(!record.still(13, &sha, &base, NOW));
    }

    #[test]
    fn a_hash_stands_for_a_week() {
        let mut record = Record::default();
        let sha = digest(b"x");
        let id = identity(9);
        record.hashed(3, sha, id, NOW);
        assert!(record.still(3, &sha, &id, NOW));
        assert!(record.still(3, &sha, &id, NOW + RECHECK_AFTER - 1));
        assert!(!record.still(3, &sha, &id, NOW + RECHECK_AFTER));
        assert!(!record.still(3, &sha, &id, NOW + 30 * DAY));
        // A clock set back: the hash is "in the future", so hashed again.
        assert!(!record.still(3, &sha, &id, NOW - 1));
        // Hashed again: it stands from then.
        record.hashed(3, sha, id, NOW + 8 * DAY);
        assert!(record.still(3, &sha, &id, NOW + 8 * DAY + 1));
        assert_eq!(record.files.len(), 1);
    }

    #[test]
    fn a_file_system_without_file_ids_is_always_hashed() {
        let mut record = Record::default();
        let sha = digest(b"x");
        let none = identity(0);
        record.hashed(3, sha, none, NOW);
        assert!(record.files.is_empty());
        assert!(!record.still(3, &sha, &none, NOW));
        let untimed = Identity {
            written: 0,
            ..identity(4)
        };
        record.hashed(3, sha, untimed, NOW);
        assert!(!record.still(3, &sha, &untimed, NOW));
    }

    #[test]
    fn a_record_reads_back() {
        let mut record = record();
        let text = write_record(&record);
        assert_eq!(parse_record(&text), record);
        let text = String::from_utf8(text).unwrap();
        assert!(text.starts_with("skhid-patches-checked 1\r\nfile resources.db.patch_12 "));
        assert!(text.ends_with("\r\nend\r\n"));
        record.forget(12);
        assert_eq!(record.files.len(), 1);
        record.keep_only(&[12]);
        assert!(record.files.is_empty());
        assert_eq!(parse_record(&write_record(&record)), record);
        // LF alone is read too.
        let lf = text.replace("\r\n", "\n");
        assert_eq!(parse_record(lf.as_bytes()), self::record());
    }

    #[test]
    fn a_record_that_cannot_be_read_is_none() {
        let good = String::from_utf8(write_record(&record())).unwrap();
        let line = good.lines().nth(1).unwrap().to_owned();
        for bad in [
            String::new(),
            "skhid-patches-checked 2\r\nend\r\n".into(),
            good.replace("\r\nend\r\n", "\r\n"),
            good.replace(&line, &alloc::format!("{line} 5")),
            good.replace(&line, &line.replacen("file ", "region ", 1)),
            good.replace(&line, &line.replacen("patch_12", "patch_012", 1)),
            good.replace(
                &line,
                &line.replacen("resources.db.patch_12", "RESOURCES.DB.PATCH_12", 1),
            ),
            good.replace(&line, &line.replacen(" 86199749 ", " -1 ", 1)),
            good.replace(&line, &line.replacen(" 86199749 ", " 8619974x ", 1)),
            good.replace(&line, &line.replacen(" 86199749 ", "  86199749 ", 1)),
            good.replace(&line, &line.replacen(" 305441741 ", " 4294967296 ", 1)),
            good.replace("end", &alloc::format!("{line}\r\nend")),
        ] {
            assert_eq!(parse_record(bad.as_bytes()), Record::default(), "{bad:?}");
        }
        assert_eq!(parse_record(&[0xff, 0xfe]), Record::default());
        // A line after `end` does not matter.
        assert_eq!(
            parse_record(alloc::format!("{good}junk\r\n").as_bytes()),
            record()
        );
    }
}
