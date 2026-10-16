//! The patches of the game's `resources.db` (`client/updater/README.md`,
//! "Patches"): the server's list of them, and what is to be done with the
//! files in the game folder.
//!
//! The game (client patch revision 14, patch 27) mounts `resources.db` and
//! then every `resources.db.patch_<n>` beside it, n = 1 to 255 in order, a
//! missing number skipped. A later patch's file wins over the same file in
//! the db and in the patches before it. n is the patch revision that
//! brought the patch, so most numbers are missing.
//!
//! The update server says which patches there are, `GET <site>/patches`
//! (over the update server's own address, HTTPS), and where each one is:
//!
//! ```text
//! {"patch_14": {"sha256": "<64 hex>", "url": "http://203.0.113.5/patch_14"},
//!  "patch_17": {"sha256": "<64 hex>", "url": "https://files.example/patch_17"}}
//! ```
//!
//! a JSON object and nothing else: every key `patch_<n>`, n from 1 to 255
//! written without a leading zero (the game's own rule: `patch_014` is not
//! a patch), no key twice; every value an object of exactly two strings,
//! `sha256` (64 hex characters) and `url` (`http://` or `https://`, any
//! host, a bare IP address too, and a path: [`crate::url::patch`]). Plain
//! HTTP is taken for the files: each is checked against its `sha256`, which
//! came over HTTPS. Whitespace between the parts is allowed; an escape in a
//! string is not (no name, checksum or address needs one). Anything else is
//! not a list: the step then changes nothing and PLAY stays off. `{}` is a
//! list: no patch.
//!
//! The file of `patch_<n>` is `GET <url>`, and goes to
//! `game/resources.db.patch_<n>`. What is done with the folder:
//!
//! - every listed patch whose file is missing, or whose SHA-256 is not the
//!   list's, is downloaded (the damaged file deleted first);
//! - every other `resources.db.patch_*` there is deleted (a patch the list
//!   no longer has: a rollback; a name the game does not take; a download
//!   left over);
//! - the game may be played when every listed patch is there with its
//!   checksum and no other `resources.db.patch_*` is ([`check`]).

use alloc::string::String;
use alloc::vec::Vec;

use crate::sha256::{from_hex, to_hex};
use crate::url::{self, Url};

/// The longest list taken from the server: 255 entries with addresses of
/// up to 1024 bytes ([`url::patch`]).
pub const MAX_LIST: usize = 512 * 1024;
/// What every patch file's name starts with, in the game folder.
pub const FILE_PREFIX: &str = "resources.db.patch_";
/// What a patch is named in the list (`patch_14`).
const NAME_PREFIX: &str = "patch_";

/// One patch of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    /// 1 to 255.
    pub number: u8,
    pub sha256: [u8; 32],
    /// Where its file is, as the list says it ...
    pub address: String,
    /// ... and read ([`url::patch`]).
    pub url: Url,
}

impl Patch {
    /// Its name in the list and on the server: `patch_14`.
    pub fn name(&self) -> String {
        alloc::format!("{NAME_PREFIX}{}", self.number)
    }

    /// Its file in the game folder: `resources.db.patch_14`.
    pub fn file_name(&self) -> String {
        alloc::format!("{FILE_PREFIX}{}", self.number)
    }
}

/// The server's list: its patches, in the order of their numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct List {
    pub patches: Vec<Patch>,
}

impl List {
    pub fn get(&self, number: u8) -> Option<&Patch> {
        self.patches.iter().find(|patch| patch.number == number)
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ListError {
    /// Longer than [`MAX_LIST`].
    TooLarge,
    /// Not a JSON object of strings (at this byte).
    Syntax(usize),
    /// A key that is not `patch_<1..255>`.
    BadName(String),
    /// A value that is not an object of `sha256` and `url` alone (the key's).
    BadEntry(String),
    /// A `sha256` that is not 64 hex characters (the key's).
    BadHash(String),
    /// A `url` that is not an `http://` or `https://` address of a file
    /// (the key's).
    BadUrl(String),
    /// A key twice.
    Duplicate(String),
}

/// `patch_<n>` with n from 1 to 255, no leading zero, no sign: n.
pub fn parse_name(name: &str) -> Option<u8> {
    number(name.strip_prefix(NAME_PREFIX)?)
}

/// The patch number of a file in the game folder: `resources.db.patch_<n>`
/// as the game takes it (Windows' names are of either case; n as in
/// [`parse_name`]). `None` for any other name.
pub fn file_number(file_name: &str) -> Option<u8> {
    let prefix = file_name.get(..FILE_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(FILE_PREFIX) {
        return None;
    }
    number(&file_name[FILE_PREFIX.len()..])
}

/// Whether the name is one of the patch files' (`resources.db.patch_*`, of
/// either case), a patch the game takes or not.
pub fn is_patch_file(file_name: &str) -> bool {
    file_name
        .get(..FILE_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(FILE_PREFIX))
}

fn number(digits: &str) -> Option<u8> {
    if digits.is_empty()
        || digits.len() > 3
        || digits.starts_with('0')
        || !digits.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let value: u32 = digits.parse().ok()?;
    u8::try_from(value).ok().filter(|&n| n >= 1)
}

/// Reads the server's list (or the copy kept of it).
pub fn parse_list(bytes: &[u8]) -> Result<List, ListError> {
    if bytes.len() > MAX_LIST {
        return Err(ListError::TooLarge);
    }
    let mut reader = Reader { bytes, at: 0 };
    reader.space();
    reader.expect(b'{')?;
    let mut list = List::default();
    reader.space();
    if !reader.take(b'}') {
        loop {
            reader.space();
            let key = reader.string()?;
            reader.space();
            reader.expect(b':')?;
            reader.space();
            let name = core::str::from_utf8(key).map_err(|_| ListError::Syntax(reader.at))?;
            let number = parse_name(name).ok_or_else(|| ListError::BadName(name.into()))?;
            let entry = reader.entry().map_err(|error| match error {
                ListError::BadEntry(_) => ListError::BadEntry(name.into()),
                other => other,
            })?;
            let (Some(sha256), Some(address)) = (entry.0, entry.1) else {
                return Err(ListError::BadEntry(name.into()));
            };
            let sha256 = from_hex(sha256).ok_or_else(|| ListError::BadHash(name.into()))?;
            let address = core::str::from_utf8(address)
                .ok()
                .filter(|address| address.len() <= 1024)
                .ok_or_else(|| ListError::BadUrl(name.into()))?;
            let url = url::patch(address).ok_or_else(|| ListError::BadUrl(name.into()))?;
            if list.get(number).is_some() {
                return Err(ListError::Duplicate(name.into()));
            }
            list.patches.push(Patch {
                number,
                sha256,
                address: address.into(),
                url,
            });
            reader.space();
            if reader.take(b'}') {
                break;
            }
            reader.expect(b',')?;
        }
    }
    reader.space();
    if reader.at != bytes.len() {
        return Err(ListError::Syntax(reader.at));
    }
    list.patches.sort_by_key(|patch| patch.number);
    Ok(list)
}

/// The list as it is kept beside the updater: one line, in the order of
/// the numbers.
pub fn write_list(list: &List) -> Vec<u8> {
    let mut out = Vec::from(&b"{"[..]);
    for (i, patch) in list.patches.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(b", ");
        }
        out.push(b'"');
        out.extend_from_slice(patch.name().as_bytes());
        out.extend_from_slice(b"\": {\"sha256\": \"");
        out.extend_from_slice(&to_hex(&patch.sha256));
        out.extend_from_slice(b"\", \"url\": \"");
        out.extend_from_slice(patch.address.as_bytes());
        out.extend_from_slice(b"\"}");
    }
    out.extend_from_slice(b"}\n");
    out
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn space(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.at += 1;
        }
    }

    fn take(&mut self, c: u8) -> bool {
        if self.bytes.get(self.at) == Some(&c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), ListError> {
        if self.take(c) {
            Ok(())
        } else {
            Err(ListError::Syntax(self.at))
        }
    }

    /// A list's value: `{"sha256": "...", "url": "..."}`, in either order,
    /// nothing else. `BadEntry` (named by the caller) for another value.
    #[allow(clippy::type_complexity)]
    fn entry(&mut self) -> Result<(Option<&'a [u8]>, Option<&'a [u8]>), ListError> {
        let bad = || ListError::BadEntry(String::new());
        if !self.take(b'{') {
            return Err(bad());
        }
        let (mut sha256, mut address) = (None, None);
        self.space();
        if self.take(b'}') {
            return Err(bad());
        }
        loop {
            self.space();
            if self.bytes.get(self.at) != Some(&b'"') {
                return Err(bad());
            }
            let key = self.string()?;
            self.space();
            self.expect(b':')?;
            self.space();
            if self.bytes.get(self.at) != Some(&b'"') {
                return Err(bad());
            }
            let value = self.string()?;
            let slot = match key {
                b"sha256" => &mut sha256,
                b"url" => &mut address,
                _ => return Err(bad()),
            };
            if slot.replace(value).is_some() {
                return Err(bad());
            }
            self.space();
            if self.take(b'}') {
                return Ok((sha256, address));
            }
            self.expect(b',')?;
        }
    }

    /// A string without escapes or control characters: its bytes.
    fn string(&mut self) -> Result<&'a [u8], ListError> {
        self.expect(b'"')?;
        let start = self.at;
        loop {
            match self.bytes.get(self.at) {
                Some(b'"') => break,
                Some(b'\\') | None => return Err(ListError::Syntax(self.at)),
                Some(&c) if c < 0x20 => return Err(ListError::Syntax(self.at)),
                Some(_) => self.at += 1,
            }
        }
        let text = &self.bytes[start..self.at];
        self.at += 1;
        Ok(text)
    }
}

/// What a listed patch's file is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Local {
    Missing,
    /// There, with this SHA-256.
    Found([u8; 32]),
    /// There, but it cannot be read.
    Unreadable,
}

/// A patch to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetch {
    pub patch: Patch,
    /// A file is there with other bytes (or cannot be read): it is deleted
    /// first.
    pub damaged: bool,
}

/// What is to be done with the game folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Todo {
    /// The `resources.db.patch_*` files there that are not a listed patch,
    /// by their names as found.
    pub delete: Vec<String>,
    /// The listed patches that are missing or damaged, in order.
    pub fetch: Vec<Fetch>,
}

impl Todo {
    pub fn is_empty(&self) -> bool {
        self.delete.is_empty() && self.fetch.is_empty()
    }
}

/// The files of the game folder that are not a listed patch: of `files`
/// (the names found there), each `resources.db.patch_*` whose number the
/// list has not, or that is no patch the game takes.
pub fn extra(list: &List, files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|name| is_patch_file(name))
        .filter(|name| file_number(name).is_none_or(|number| list.get(number).is_none()))
        .cloned()
        .collect()
}

/// What to do: `files` are the names found in the game folder, `look`
/// hashes a listed patch's file.
pub fn todo(list: &List, files: &[String], look: &mut dyn FnMut(&Patch) -> Local) -> Todo {
    let fetch = list
        .patches
        .iter()
        .filter_map(|patch| match look(patch) {
            Local::Found(sha256) if sha256 == patch.sha256 => None,
            Local::Missing => Some(Fetch {
                patch: patch.clone(),
                damaged: false,
            }),
            Local::Found(_) | Local::Unreadable => Some(Fetch {
                patch: patch.clone(),
                damaged: true,
            }),
        })
        .collect();
    Todo {
        delete: extra(list, files),
        fetch,
    }
}

/// Why the game folder is not what the list says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A `resources.db.patch_*` the list has not.
    Extra(String),
    Missing(u8),
    /// There with other bytes, or unreadable.
    Damaged(u8),
}

/// `Ok` when the game may be played with these files: every listed patch
/// there with its checksum, no other `resources.db.patch_*`. Else the
/// first thing found.
pub fn check(
    list: &List,
    files: &[String],
    look: &mut dyn FnMut(&Patch) -> Local,
) -> Result<(), Problem> {
    if let Some(name) = extra(list, files).into_iter().next() {
        return Err(Problem::Extra(name));
    }
    for patch in &list.patches {
        match look(patch) {
            Local::Found(sha256) if sha256 == patch.sha256 => {}
            Local::Missing => return Err(Problem::Missing(patch.number)),
            Local::Found(_) | Local::Unreadable => return Err(Problem::Damaged(patch.number)),
        }
    }
    Ok(())
}

/// A file of the overlay (the ZIP's names) that is a patch file: the
/// patches come from the list alone.
pub fn overlay_patch<'a>(overlay: &[&'a str]) -> Option<&'a str> {
    overlay.iter().copied().find(|name| {
        name.get(..5)
            .is_some_and(|game| game.eq_ignore_ascii_case("game/"))
            && is_patch_file(&name[5..])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::digest;

    fn hex(data: &[u8]) -> String {
        String::from_utf8(to_hex(&digest(data)).to_vec()).unwrap()
    }

    fn address(number: u8) -> String {
        alloc::format!("http://203.0.113.5/patch_{number}")
    }

    fn patch(number: u8, data: &[u8]) -> Patch {
        let address = address(number);
        Patch {
            number,
            sha256: digest(data),
            url: url::patch(&address).unwrap(),
            address,
        }
    }

    fn entry(name: &str, sha256: &str, url: &str) -> String {
        alloc::format!("\"{name}\": {{\"sha256\": \"{sha256}\", \"url\": \"{url}\"}}")
    }

    /// A list of one entry.
    fn one(name: &str, sha256: &str, url: &str) -> String {
        alloc::format!("{{{}}}", entry(name, sha256, url))
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| String::from(*name)).collect()
    }

    #[test]
    fn a_list_is_read_in_the_order_of_its_numbers() {
        let text = alloc::format!(
            " {{\r\n  \"patch_17\" : {{ \"url\" : \"https://files.example.org:8443/db/patch_17\" ,\n\t\"sha256\":\"{}\" }},\n\t\"patch_14\":{{\"sha256\":\"{}\",\"url\":\"http://203.0.113.5/patch_14\"}} }}\n",
            hex(b"17"),
            hex(b"14").to_ascii_uppercase()
        );
        let list = parse_list(text.as_bytes()).unwrap();
        assert_eq!(list.patches.len(), 2);
        assert_eq!(list.patches[0], patch(14, b"14"));
        let p17 = &list.patches[1];
        assert_eq!((p17.number, p17.sha256), (17, digest(b"17")));
        assert_eq!(p17.address, "https://files.example.org:8443/db/patch_17");
        assert_eq!(
            (
                p17.url.secure,
                p17.url.host.as_str(),
                p17.url.port,
                p17.url.prefix.as_str()
            ),
            (true, "files.example.org", 8443, "/db/patch_17")
        );
        assert!(!list.patches[0].url.secure);
        // Kept as it was read.
        assert_eq!(parse_list(&write_list(&list)), Ok(list.clone()));
        assert_eq!(
            write_list(&list),
            alloc::format!(
                "{{{}, {}}}\n",
                entry("patch_14", &hex(b"14"), "http://203.0.113.5/patch_14"),
                entry(
                    "patch_17",
                    &hex(b"17"),
                    "https://files.example.org:8443/db/patch_17"
                )
            )
            .into_bytes()
        );
    }

    #[test]
    fn an_empty_object_is_a_list_of_no_patch() {
        assert_eq!(parse_list(b"{}"), Ok(List::default()));
        assert_eq!(parse_list(b" { \n } \r\n"), Ok(List::default()));
        assert_eq!(write_list(&List::default()), b"{}\n");
    }

    #[test]
    fn the_whole_range_of_numbers_is_taken() {
        let mut text = String::from("{");
        let long = alloc::format!("https://files.example.org/{}", "a".repeat(990));
        assert_eq!(long.len(), 1016);
        for n in 1..=255_u32 {
            if n > 1 {
                text.push(',');
            }
            text += &entry(&alloc::format!("patch_{n}"), &hex(&n.to_le_bytes()), &long);
        }
        text.push('}');
        let list = parse_list(text.as_bytes()).unwrap();
        assert_eq!(list.patches.len(), 255);
        assert_eq!(list.patches[254].number, 255);
        assert!(write_list(&list).len() <= MAX_LIST);
    }

    #[test]
    fn names_are_patch_and_a_number_from_1_to_255() {
        assert_eq!(parse_name("patch_1"), Some(1));
        assert_eq!(parse_name("patch_14"), Some(14));
        assert_eq!(parse_name("patch_255"), Some(255));
        for bad in [
            "patch_0",
            "patch_256",
            "patch_014",
            "patch_01",
            "patch_",
            "patch_+1",
            "patch_-1",
            "patch_1 ",
            " patch_1",
            "patch_1a",
            "patch_1000",
            "Patch_1",
            "patch1",
            "patch_14/../x",
            "../patch_14",
            "resources.db.patch_14",
            "patch_99999999999999999999",
        ] {
            assert_eq!(parse_name(bad), None, "{bad}");
            let text = one(bad, &hex(b"x"), &address(1));
            assert_eq!(
                parse_list(text.as_bytes()),
                Err(ListError::BadName(bad.into())),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_name_with_a_path_is_no_name() {
        assert_eq!(parse_name("patch_14\\x"), None);
        // A backslash is no part of a list's string at all.
        let text = one("patch_14\\\\x", &hex(b"x"), &address(14));
        assert!(matches!(
            parse_list(text.as_bytes()),
            Err(ListError::Syntax(_))
        ));
    }

    #[test]
    fn hashes_are_64_hex_characters() {
        for bad in [
            String::new(),
            hex(b"x")[..63].into(),
            hex(b"x") + "0",
            hex(b"x").replace(|c: char| c.is_ascii_digit(), "g"),
            alloc::format!(" {}", &hex(b"x")[1..]),
        ] {
            let text = one("patch_3", &bad, &address(3));
            assert_eq!(
                parse_list(text.as_bytes()),
                Err(ListError::BadHash("patch_3".into())),
                "{bad}"
            );
        }
    }

    #[test]
    fn addresses_are_http_or_https_and_nothing_else() {
        for good in [
            "http://203.0.113.5/patch_14",
            "http://203.0.113.5:8080/files/patch_14",
            "https://files.example.org/patch_14",
            "http://localhost/patch_14",
        ] {
            assert!(
                parse_list(one("patch_14", &hex(b"x"), good).as_bytes()).is_ok(),
                "{good}"
            );
        }
        let long = alloc::format!("https://files.example.org/{}", "a".repeat(1000));
        for bad in [
            "",
            "ftp://203.0.113.5/patch_14",
            "file:///C:/Windows/patch_14",
            "file://server/share/patch_14",
            "smb://server/patch_14",
            "data:text/plain,x",
            "//203.0.113.5/patch_14",
            "/patches/patch_14",
            "patch_14",
            "http://203.0.113.5",
            "http://203.0.113.5/",
            "http://user:pw@203.0.113.5/patch_14",
            "http://203.0.113.5/patch_14#x",
            "http://203.0.113.5/a b",
            "http://203.0.113.5:0/patch_14",
            "http://[2001:db8::1]/patch_14",
            long.as_str(),
        ] {
            assert_eq!(
                parse_list(one("patch_14", &hex(b"x"), bad).as_bytes()),
                Err(ListError::BadUrl("patch_14".into())),
                "{bad}"
            );
        }
    }

    /// The URLs srp-game-data takes, and no other, are taken in a list
    /// (`tests/data/patch_urls.txt`).
    #[test]
    fn a_list_takes_the_patch_urls_srp_game_data_takes() {
        for (url, taken) in crate::url::tests::shared_patch_urls() {
            let list = one("patch_14", &hex(b"x"), &url);
            assert_eq!(parse_list(list.as_bytes()).is_ok(), taken, "{url:?}");
        }
    }

    #[test]
    fn an_entry_is_sha256_and_url_alone() {
        let h = hex(b"x");
        let a = address(2);
        for bad in [
            alloc::format!("\"{h}\""),
            "5".into(),
            "null".into(),
            alloc::format!("[\"{h}\", \"{a}\"]"),
            "{}".into(),
            alloc::format!("{{\"sha256\": \"{h}\"}}"),
            alloc::format!("{{\"url\": \"{a}\"}}"),
            alloc::format!("{{\"sha256\": \"{h}\", \"url\": \"{a}\", \"size\": \"5\"}}"),
            alloc::format!("{{\"sha256\": \"{h}\", \"sha256\": \"{h}\", \"url\": \"{a}\"}}"),
            alloc::format!("{{\"sha256\": \"{h}\", \"url\": \"{a}\", \"url\": \"{a}\"}}"),
            alloc::format!("{{\"SHA256\": \"{h}\", \"url\": \"{a}\"}}"),
            alloc::format!("{{\"sha256\": 5, \"url\": \"{a}\"}}"),
            alloc::format!("{{\"sha256\": \"{h}\", \"url\": null}}"),
            alloc::format!("{{\"sha256\": \"{h}\", \"url\": {{\"href\": \"{a}\"}}}}"),
            alloc::format!("{{\"sha256\": \"{h}\", \"url\": \"{a}\",}}"),
        ] {
            let text = alloc::format!("{{\"patch_2\": {bad}}}");
            assert_eq!(
                parse_list(text.as_bytes()),
                Err(ListError::BadEntry("patch_2".into())),
                "{bad}"
            );
        }
    }

    #[test]
    fn anything_but_such_an_object_is_not_a_list() {
        let e = entry("patch_1", &hex(b"x"), &address(1));
        let e2 = entry("patch_2", &hex(b"x"), &address(2));
        for bad in [
            String::new(),
            " ".into(),
            "[]".into(),
            "null".into(),
            "{".into(),
            "}".into(),
            "{,}".into(),
            alloc::format!("{{{e}"),
            alloc::format!("{{{e},}}"),
            alloc::format!("{{{e} {e2}}}"),
            alloc::format!("{{{e}}} {{}}"),
            alloc::format!("{{{e}}}x"),
            alloc::format!("{{{}}}", e.replacen(':', "", 1)),
            alloc::format!("{{{}}}", e.replacen("\"patch_1\"", "patch_1", 1)),
            alloc::format!("{{{}}}", e.replace('"', "'")),
            alloc::format!("{{{}}}", e.replacen(", ", " ", 1)),
            alloc::format!("{{{}}}", e.replacen("sha256", "sha\\u0032", 1)),
            alloc::format!("{{{}}}", e.replacen("/patch_1", "\\/patch_1", 1)),
            alloc::format!("{{{}}}", e.replacen("/patch_1", "/patch\n_1", 1)),
            alloc::format!("\u{feff}{{{e}}}"),
            "<html>Sign in to the Wi-Fi</html>".into(),
        ] {
            assert!(
                matches!(parse_list(bad.as_bytes()), Err(ListError::Syntax(_))),
                "{bad:?}: {:?}",
                parse_list(bad.as_bytes())
            );
        }
        assert!(matches!(
            parse_list(&[b'{', 0xff, b'}']),
            Err(ListError::Syntax(_))
        ));
        assert_eq!(
            parse_list(&alloc::vec![b' '; MAX_LIST + 1]),
            Err(ListError::TooLarge)
        );
    }

    #[test]
    fn a_name_twice_is_not_a_list() {
        let text = alloc::format!(
            "{{{}, {}}}",
            entry("patch_2", &hex(b"a"), &address(2)),
            entry("patch_2", &hex(b"a"), &address(2))
        );
        assert_eq!(
            parse_list(text.as_bytes()),
            Err(ListError::Duplicate("patch_2".into()))
        );
    }

    #[test]
    fn file_names_are_the_games_own() {
        assert_eq!(file_number("resources.db.patch_14"), Some(14));
        assert_eq!(file_number("RESOURCES.DB.Patch_255"), Some(255));
        for bad in [
            "resources.db.patch_0",
            "resources.db.patch_014",
            "resources.db.patch_256",
            "resources.db.patch_",
            "resources.db.patch_1.db",
            "resources.db.patch_14.skhid-part",
            "resources.db",
            "resources.db.patch",
            "xresources.db.patch_1",
        ] {
            assert_eq!(file_number(bad), None, "{bad}");
        }
        assert!(is_patch_file("resources.db.patch_014"));
        assert!(is_patch_file("Resources.DB.patch_14.skhid-part"));
        assert!(!is_patch_file("resources.db"));
        assert!(!is_patch_file("resources.db.patch"));
        assert_eq!(patch(14, b"").name(), "patch_14");
        assert_eq!(patch(14, b"").file_name(), "resources.db.patch_14");
    }

    /// A game folder: name -> contents.
    fn folder(files: &[(&str, &[u8])]) -> (Vec<String>, impl FnMut(&Patch) -> Local) {
        let found: Vec<(String, [u8; 32])> = files
            .iter()
            .map(|(name, data)| (String::from(*name), digest(data)))
            .collect();
        let names = found.iter().map(|(name, _)| name.clone()).collect();
        let look = move |patch: &Patch| {
            found
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&patch.file_name()))
                .map_or(Local::Missing, |(_, sha256)| Local::Found(*sha256))
        };
        (names, look)
    }

    fn list(patches: &[(u8, &[u8])]) -> List {
        List {
            patches: patches.iter().map(|(n, data)| patch(*n, data)).collect(),
        }
    }

    #[test]
    fn a_fresh_folder_downloads_every_patch_gaps_and_all() {
        let list = list(&[(1, b"one"), (14, b"fourteen"), (255, b"last")]);
        let (files, mut look) = folder(&[("resources.db", b"db")]);
        let todo = todo(&list, &files, &mut look);
        assert_eq!(todo.delete, Vec::<String>::new());
        assert_eq!(
            todo.fetch
                .iter()
                .map(|f| (f.patch.number, f.damaged))
                .collect::<Vec<_>>(),
            [(1, false), (14, false), (255, false)]
        );
        assert_eq!(check(&list, &files, &mut look), Err(Problem::Missing(1)));
    }

    #[test]
    fn a_folder_that_has_them_all_has_nothing_to_do() {
        let list = list(&[(14, b"fourteen"), (17, b"seventeen")]);
        let (files, mut look) = folder(&[
            ("resources.db", b"db"),
            ("resources.db.patch_14", b"fourteen"),
            ("RESOURCES.DB.PATCH_17", b"seventeen"),
        ]);
        assert!(todo(&list, &files, &mut look).is_empty());
        assert_eq!(check(&list, &files, &mut look), Ok(()));
        // No list entry, no file: also nothing.
        let (files, mut look) = folder(&[("resources.db", b"db")]);
        assert!(todo(&List::default(), &files, &mut look).is_empty());
        assert_eq!(check(&List::default(), &files, &mut look), Ok(()));
    }

    #[test]
    fn one_missing_is_downloaded_alone() {
        let list = list(&[(14, b"fourteen"), (17, b"seventeen"), (20, b"twenty")]);
        let (files, mut look) = folder(&[
            ("resources.db.patch_14", b"fourteen"),
            ("resources.db.patch_20", b"twenty"),
        ]);
        let todo = todo(&list, &files, &mut look);
        assert_eq!(todo.delete, Vec::<String>::new());
        assert_eq!(
            todo.fetch,
            [Fetch {
                patch: patch(17, b"seventeen"),
                damaged: false
            }]
        );
        assert_eq!(check(&list, &files, &mut look), Err(Problem::Missing(17)));
    }

    #[test]
    fn a_damaged_one_is_downloaded_again() {
        let list = list(&[(14, b"fourteen"), (17, b"seventeen")]);
        let (files, mut look) = folder(&[
            ("resources.db.patch_14", b"fourteen, but damaged"),
            ("resources.db.patch_17", b"seventeen"),
        ]);
        let todo = todo(&list, &files, &mut look);
        assert_eq!(todo.delete, Vec::<String>::new());
        assert_eq!(
            todo.fetch,
            [Fetch {
                patch: patch(14, b"fourteen"),
                damaged: true
            }]
        );
        assert_eq!(check(&list, &files, &mut look), Err(Problem::Damaged(14)));
        // One that cannot be read counts as damaged.
        let mut unreadable = |_: &Patch| Local::Unreadable;
        assert!(
            super::todo(&list, &files, &mut unreadable)
                .fetch
                .iter()
                .all(|f| f.damaged)
        );
        assert_eq!(
            check(&list, &files, &mut unreadable),
            Err(Problem::Damaged(14))
        );
    }

    #[test]
    fn what_the_list_has_not_is_deleted() {
        let list = list(&[(14, b"fourteen")]);
        let (files, mut look) = folder(&[
            ("resources.db", b"db"),
            ("resources.db.patch_14", b"fourteen"),
            // A patch the list no longer has: a rollback.
            ("resources.db.patch_15", b"fifteen"),
            // Names the game does not take, and a download left over.
            ("resources.db.patch_014", b"x"),
            ("resources.db.patch_0", b"x"),
            ("Resources.db.Patch_300", b"x"),
            ("resources.db.patch_14.skhid-part", b"four"),
            // Not a patch file at all: left alone.
            ("resources.db.skhid-new", b"x"),
            ("readme.txt", b"x"),
        ]);
        let todo = todo(&list, &files, &mut look);
        assert_eq!(
            todo.delete,
            names(&[
                "resources.db.patch_15",
                "resources.db.patch_014",
                "resources.db.patch_0",
                "Resources.db.Patch_300",
                "resources.db.patch_14.skhid-part",
            ])
        );
        assert_eq!(todo.fetch, []);
        assert_eq!(
            check(&list, &files, &mut look),
            Err(Problem::Extra("resources.db.patch_15".into()))
        );
        // An empty list: every patch goes.
        let (files, mut look) = folder(&[("resources.db.patch_14", b"fourteen")]);
        assert_eq!(
            super::todo(&List::default(), &files, &mut look).delete,
            names(&["resources.db.patch_14"])
        );
    }

    #[test]
    fn the_overlay_may_not_bring_a_patch_file() {
        assert_eq!(
            overlay_patch(&["game/binaries/x86/survarium_skhid.exe", "README.txt"]),
            None
        );
        assert_eq!(
            overlay_patch(&["README.txt", "game/resources.db.patch_14"]),
            Some("game/resources.db.patch_14")
        );
        assert_eq!(
            overlay_patch(&["GAME/Resources.db.patch_x"]),
            Some("GAME/Resources.db.patch_x")
        );
        assert_eq!(overlay_patch(&["game/binaries/resources.db.patch_1"]), None);
    }
}
