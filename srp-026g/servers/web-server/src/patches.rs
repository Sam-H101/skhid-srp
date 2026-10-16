//! `GET /patches` (the 0.26g page only): the patches of `resources.db` the
//! players' updater keeps in the game folder (`client/updater/README.md`,
//! "Patches"), as JSON:
//!
//! ```text
//! {"patch_14":{"sha256":"<64 hex>","url":"http://203.0.113.5/patches/resources.db.patch_14"},...}
//! ```
//!
//! They are the patches the server's game data was made of
//! (`survarium::v026g::game_data`, `Provenance::offered`), in the game's
//! order, so the players run the archives the server runs on; `{}` when it
//! was made of `resources.db` alone. It is public, as the download is: the
//! updater asks it with no token. A 404 would keep PLAY off for everyone,
//! so it is answered whenever the 0.26g page is.

use crate::{ContentType, Response};

/// A patch the updater is told to fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferedPatch {
    /// `patch_<n>`: the game's file `resources.db.patch_<n>`.
    pub name: String,
    /// The sha256 of the file, lowercase hex.
    pub sha256: String,
    /// Where the updater downloads it.
    pub url: String,
}

/// `text` as a JSON string. srp-game-data only writes names, checksums
/// and URLs that need no escape (the updater takes none), but a string is
/// never written unescaped.
fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The list, in the order given.
pub(crate) fn json(patches: &[OfferedPatch]) -> String {
    let entries: Vec<String> = patches
        .iter()
        .map(|patch| {
            format!(
                "{}:{{\"sha256\":{},\"url\":{}}}",
                string(&patch.name),
                string(&patch.sha256),
                string(&patch.url)
            )
        })
        .collect();
    format!("{{{}}}", entries.join(","))
}

pub(crate) fn response(patches: &[OfferedPatch]) -> Response {
    Response {
        status: "200 OK",
        content_type: ContentType::Json,
        body: json(patches),
    }
}
