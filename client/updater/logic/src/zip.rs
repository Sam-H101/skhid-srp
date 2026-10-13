//! Reads the players' ZIP from memory: the entries of its central directory,
//! each checked before a byte is written anywhere.
//!
//! Only what `make_client_026g.py` writes is understood: one disk, no ZIP64,
//! no encryption, stored or deflated entries. A name is taken only if it is a
//! plain relative path of ordinary characters, so an archive can never write
//! outside the game folder.

use alloc::vec::Vec;

use crate::inflate::{InflateError, crc32, inflate};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ZipError {
    /// No end-of-central-directory record: not a ZIP, or cut short.
    NotZip,
    /// A record points outside the file or has a wrong signature.
    Damaged,
    /// ZIP64, several disks, encryption or a compression method other than
    /// stored and deflated.
    Unsupported,
    /// A name that is not a plain relative path (see [`safe_name`]).
    BadName,
    /// The same name twice (also when only the case differs).
    Duplicate,
    /// More entries or more bytes than an update may have.
    TooLarge,
    /// An entry's bytes do not give its recorded size and CRC-32.
    BadData,
}

/// The most entries an archive may have.
pub const MAX_ENTRIES: usize = 512;
/// The most one entry may unpack to.
pub const MAX_ENTRY_SIZE: u32 = 256 << 20;
/// The most all entries together may unpack to.
pub const MAX_TOTAL_SIZE: u64 = 1 << 30;

/// One file of the archive (folders are left out).
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    /// The path inside the archive, `/` between its parts.
    pub name: &'a str,
    /// The size once unpacked.
    pub size: u32,
    crc: u32,
    deflated: bool,
    packed: &'a [u8],
}

impl Entry<'_> {
    /// The file's bytes, unpacked and checked against its size and CRC-32.
    pub fn read(&self) -> Result<Vec<u8>, ZipError> {
        let data = if self.deflated {
            match inflate(self.packed, self.size as usize) {
                Ok(data) => data,
                Err(InflateError::Truncated | InflateError::Invalid | InflateError::TooLarge) => {
                    return Err(ZipError::BadData);
                }
            }
        } else {
            self.packed.to_vec()
        };
        if data.len() != self.size as usize || crc32(0, &data) != self.crc {
            return Err(ZipError::BadData);
        }
        Ok(data)
    }
}

fn u16_at(data: &[u8], at: usize) -> Option<usize> {
    let bytes = data.get(at..at + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]) as usize)
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Names Windows keeps for devices, whatever the extension.
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Whether `name` is a path this updater writes: relative, `/` between
/// parts, each part of ASCII letters, digits, `.`, `_` and `-` only, not
/// `.` or `..`, not ending in a dot, and no Windows device name. (No
/// spaces, colons or backslashes: no drive, no stream, no way up.)
pub fn safe_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 200 {
        return false;
    }
    name.split('/').all(|part| {
        let stem = part.split('.').next().unwrap_or("");
        !part.is_empty()
            && part != "."
            && part != ".."
            && !part.ends_with('.')
            && part.len() <= 100
            && part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
            && !RESERVED
                .iter()
                .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    })
}

/// The files of the archive `data`, in the order of its directory.
pub fn entries(data: &[u8]) -> Result<Vec<Entry<'_>>, ZipError> {
    // The end record is the last thing in the file, before a comment of at
    // most 65535 bytes.
    const END: [u8; 4] = [b'P', b'K', 5, 6];
    if data.len() < 22 {
        return Err(ZipError::NotZip);
    }
    let lowest = data.len().saturating_sub(22 + 65535);
    let end = (lowest..=data.len().saturating_sub(22))
        .rev()
        .find(|&at| data.get(at..at + 4) == Some(&END[..]))
        .ok_or(ZipError::NotZip)?;
    let disk = u16_at(data, end + 4).ok_or(ZipError::Damaged)?;
    let directory_disk = u16_at(data, end + 6).ok_or(ZipError::Damaged)?;
    let here = u16_at(data, end + 8).ok_or(ZipError::Damaged)?;
    let count = u16_at(data, end + 10).ok_or(ZipError::Damaged)?;
    let size = u32_at(data, end + 12).ok_or(ZipError::Damaged)?;
    let offset = u32_at(data, end + 16).ok_or(ZipError::Damaged)?;
    if disk != 0 || directory_disk != 0 || here != count {
        return Err(ZipError::Unsupported);
    }
    if count == 0xFFFF || size == u32::MAX || offset == u32::MAX {
        return Err(ZipError::Unsupported);
    }
    if count > MAX_ENTRIES {
        return Err(ZipError::TooLarge);
    }
    let mut at = offset as usize;
    if at.checked_add(size as usize).is_none_or(|stop| stop > end) {
        return Err(ZipError::Damaged);
    }
    let mut found: Vec<Entry> = Vec::new();
    let mut total = 0_u64;
    for _ in 0..count {
        if u32_at(data, at) != Some(0x0201_4B50) {
            return Err(ZipError::Damaged);
        }
        let flags = u16_at(data, at + 8).ok_or(ZipError::Damaged)?;
        let method = u16_at(data, at + 10).ok_or(ZipError::Damaged)?;
        let crc = u32_at(data, at + 16).ok_or(ZipError::Damaged)?;
        let packed_size = u32_at(data, at + 20).ok_or(ZipError::Damaged)?;
        let size = u32_at(data, at + 24).ok_or(ZipError::Damaged)?;
        let name_length = u16_at(data, at + 28).ok_or(ZipError::Damaged)?;
        let extra_length = u16_at(data, at + 30).ok_or(ZipError::Damaged)?;
        let comment_length = u16_at(data, at + 32).ok_or(ZipError::Damaged)?;
        let local = u32_at(data, at + 42).ok_or(ZipError::Damaged)?;
        let name = data
            .get(at + 46..at + 46 + name_length)
            .ok_or(ZipError::Damaged)?;
        at += 46 + name_length + extra_length + comment_length;
        if at > end {
            return Err(ZipError::Damaged);
        }
        // Bit 0: encrypted. Bit 6: strong encryption. Bit 13: hidden names.
        if flags & 0x2041 != 0 {
            return Err(ZipError::Unsupported);
        }
        if packed_size == u32::MAX || size == u32::MAX || local == u32::MAX {
            return Err(ZipError::Unsupported);
        }
        let name = core::str::from_utf8(name).map_err(|_| ZipError::BadName)?;
        // A folder: nothing to write (folders are made for the files).
        if let Some(folder) = name.strip_suffix('/') {
            if !safe_name(folder) {
                return Err(ZipError::BadName);
            }
            if size != 0 {
                return Err(ZipError::Damaged);
            }
            continue;
        }
        if !safe_name(name) {
            return Err(ZipError::BadName);
        }
        let deflated = match method {
            0 => false,
            8 => true,
            _ => return Err(ZipError::Unsupported),
        };
        if !deflated && packed_size != size {
            return Err(ZipError::Damaged);
        }
        if size > MAX_ENTRY_SIZE {
            return Err(ZipError::TooLarge);
        }
        total += u64::from(size);
        if total > MAX_TOTAL_SIZE {
            return Err(ZipError::TooLarge);
        }
        if found
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(name))
        {
            return Err(ZipError::Duplicate);
        }
        // The local header repeats the name; its own lengths say where the
        // bytes start.
        let local = local as usize;
        if u32_at(data, local) != Some(0x0403_4B50) {
            return Err(ZipError::Damaged);
        }
        let local_name = u16_at(data, local + 26).ok_or(ZipError::Damaged)?;
        let local_extra = u16_at(data, local + 28).ok_or(ZipError::Damaged)?;
        if data.get(local + 30..local + 30 + local_name) != Some(name.as_bytes()) {
            return Err(ZipError::Damaged);
        }
        let start = local + 30 + local_name + local_extra;
        let packed = start
            .checked_add(packed_size as usize)
            .filter(|&stop| stop <= offset as usize)
            .and_then(|stop| data.get(start..stop))
            .ok_or(ZipError::Damaged)?;
        found.push(Entry {
            name,
            size,
            crc,
            deflated,
            packed,
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tests/data/bundle.zip`: made by `tests/data/make_fixtures.py` with
    /// Python's zipfile, the way `make_client_026g.py` writes the download.
    const BUNDLE: &[u8] = include_bytes!("../tests/data/bundle.zip");

    #[test]
    fn the_bundle_reads_back() {
        let entries = entries(BUNDLE).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name).collect();
        assert_eq!(
            names,
            [
                "game/binaries/x86/survarium_skhid.exe",
                "game/binaries/x86/play_skhid.bat",
                "game/resources/ssl/survarium_login_server.crt",
                "README.txt",
                "empty.txt",
            ]
        );
        let exe = entries[0].read().unwrap();
        assert_eq!(exe.len(), 70_000);
        assert!(
            exe.iter()
                .enumerate()
                .all(|(i, &byte)| byte == (i * i % 251) as u8)
        );
        assert_eq!(
            entries[1].read().unwrap(),
            b"@echo off\r\nstart survarium_skhid.exe\r\n"
        );
        assert_eq!(entries[3].read().unwrap(), b"stored, not deflated");
        assert_eq!(entries[4].read().unwrap(), b"");
    }

    #[test]
    fn a_cut_archive_is_refused() {
        for cut in [0, 1, 21, 22, BUNDLE.len() / 2, BUNDLE.len() - 1] {
            assert!(entries(&BUNDLE[..cut]).is_err(), "cut at {cut}");
        }
        assert_eq!(
            entries(b"not a zip at all, but long enough to look").err(),
            Some(ZipError::NotZip)
        );
    }

    #[test]
    fn changed_bytes_never_pass_as_good_data() {
        let good: Vec<Vec<u8>> = entries(BUNDLE)
            .unwrap()
            .iter()
            .map(|entry| entry.read().unwrap())
            .collect();
        let mut data = BUNDLE.to_vec();
        // Every byte changed in turn: either the archive is refused, an
        // entry fails its check, or what is read is still a file's right
        // bytes (the change was in a field this reader does not use, like a
        // date, or in a name).
        for at in 0..data.len() {
            data[at] ^= 0x55;
            if let Ok(found) = entries(&data) {
                for entry in &found {
                    if let Ok(bytes) = entry.read() {
                        assert!(
                            good.contains(&bytes),
                            "byte {at}: {} read as other data",
                            entry.name
                        );
                    }
                }
            }
            data[at] ^= 0x55;
        }
    }

    #[test]
    fn only_plain_relative_names_are_safe() {
        for good in [
            "a",
            "README.txt",
            "game/binaries/x86/survarium_skhid.exe",
            "a-b_c.d/e.f",
        ] {
            assert!(safe_name(good), "{good}");
        }
        for bad in [
            "", "/a", "a/", "a//b", "../a", "a/../b", "a/./b", ".", "..", "a\\b", "c:/a", "c:a",
            "a b", "a:b", "nul", "NUL.txt", "game/con", "com1.exe", "a.", "a/b.", "\u{444}",
            "a\0b", "a\nb", "~/a", "%TEMP%/a", "a*", "a?",
        ] {
            assert!(!safe_name(bad), "{bad:?}");
        }
        assert!(!safe_name(&"a/".repeat(101)));
    }

    #[test]
    fn hostile_archives_are_refused() {
        for (file, error) in [
            ("evil-parent.zip", ZipError::BadName),
            ("evil-absolute.zip", ZipError::BadName),
            ("evil-backslash.zip", ZipError::BadName),
            ("evil-duplicate.zip", ZipError::Duplicate),
            ("evil-encrypted-flag.zip", ZipError::Unsupported),
        ] {
            let data =
                std::fs::read(format!("{}/tests/data/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap();
            assert_eq!(entries(&data).err(), Some(error), "{file}");
        }
        // A size that lies: the entry is listed, its bytes are refused.
        let data = std::fs::read(format!(
            "{}/tests/data/evil-size.zip",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let found = entries(&data).unwrap();
        assert_eq!(found[0].read().err(), Some(ZipError::BadData));
    }
}
