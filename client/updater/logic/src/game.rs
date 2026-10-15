//! Release pins: changing a mirror never changes which bytes are accepted.
use crate::sha256::from_hex;

pub const GAME_URL: &str = "https://drive.usercontent.google.com/download?id=1LTK2rzJPELLXxtTavZwdzr247NFwT_GK&export=download&confirm=t";
/// The extractor is a file of the Skhid archive, beside the updater. It is
/// run only when it is the pinned file, as the downloaded one would be.
pub const EXTRACTOR_NAME: &str = "7zr.exe";

#[derive(Clone, Copy)]
pub struct Pin {
    pub size: u64,
    pub sha256: &'static str,
}

impl Pin {
    pub fn matches(self, size: u64, hash: [u8; 32]) -> bool {
        size == self.size && from_hex(self.sha256.as_bytes()) == Some(hash)
    }
}

// Downloaded from the user's Drive mirror; its SHA-1 independently matches
// archive.org's bb30b71fab155b3435917690fd2be87984546163 (build 2777).
pub const ARCHIVE: Pin = Pin {
    size: 2_906_544_103,
    sha256: "7435558eeb17098fc9337c4ba9470402cae4c9be974ef5d9bc544f19b122a028",
};
// The official standalone 7zr.exe of 7-Zip 26.04 (www.7-zip.org/a/7zr.exe,
// fetched over HTTPS on 2026-10-13); the copy in ../../extractor.
pub const EXTRACTOR: Pin = Pin {
    size: 602_624,
    sha256: "256feca8e274e5da655e2a284fabafd9f554365eb164862089dacd4e8276d282",
};
pub const ORIGINAL_EXE: Pin = Pin {
    size: 12_894_840,
    sha256: "ac33d9937015c2299bf62cc6b127cb308b48a3475b30fff2ddd65479ae3bf153",
};

/// The stock resources.db of build 2777, the input of the first database
/// patch (`dbpatch`; make_client_026g.py writes it as the list's
/// `original`).
pub const RESOURCES_DB: Pin = Pin {
    size: 6_285_533_906,
    sha256: "c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c",
};
/// Where a pristine resources.db (plain or zstd-compressed) is downloaded
/// from when a player's is not the original and cannot be put back;
/// `updater_skhid.db-url` beside the updater overrides it. Empty: none is
/// hosted yet, and the game's own archive ([`ARCHIVE`]) is downloaded for it.
pub const DB_URL: &str = "";

/// Only the files needed to run the game, relative to `Survarium/game`.
/// Old vendor installers, services and updaters are never extracted or run.
pub const FILES: &[(&str, u64)] = &[
    ("binaries/x86/survarium.exe", 12_894_840),
    ("binaries/x86/bugtrap.dll", 278_648),
    ("resources/ssl/survarium_login_server.crt", 757),
    (
        "resources/sources/splash_screens/development_engine.bmp",
        536_056,
    ),
    ("resources.db", 6_285_533_906),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256::digest;

    #[test]
    fn size_and_sha256_are_both_mandatory() {
        let pin = Pin {
            size: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        assert!(pin.matches(3, digest(b"abc")));
        assert!(!pin.matches(3, digest(b"abd"))); // same-length corruption
        assert!(!pin.matches(2, digest(b"abc"))); // truncated transfer
        assert!(!pin.matches(4, digest(b"abc"))); // excess bytes
        assert!(!Pin { sha256: "", ..pin }.matches(3, digest(b"abc")));
        assert!(
            !Pin {
                sha256: "not a checksum",
                ..pin
            }
            .matches(3, digest(b"abc"))
        );
    }

    #[test]
    fn release_pins_and_paths_are_well_formed() {
        assert!(FILES.contains(&("resources.db", RESOURCES_DB.size)));
        for pin in [ARCHIVE, EXTRACTOR, ORIGINAL_EXE, RESOURCES_DB] {
            assert!(pin.size > 0);
            assert!(from_hex(pin.sha256.as_bytes()).is_some());
        }
        for (path, size) in FILES {
            assert!(crate::zip::safe_name(path));
            assert!(*size > 0);
        }
        assert!(crate::url::download(GAME_URL).unwrap().secure);
        assert!(crate::zip::safe_name(EXTRACTOR_NAME));
    }
}
