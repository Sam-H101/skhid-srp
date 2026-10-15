//! First installation of the original game, before applying the Skhid ZIP.
use crate::{
    Session,
    http::{self, HttpError},
    install::Layout,
    log::log,
    win,
};
use alloc::{format, string::String, vec::Vec};
use updater_logic::{
    dbpatch::FileId,
    game::{self, Pin},
    sha256::to_hex,
    pipe::{ToLauncher, state, step},
    url,
};

pub fn present(layout: &Layout) -> bool {
    ["resources.db", "binaries\\x86\\survarium.exe"]
        .iter()
        .all(|name| {
            win::file_size(&format!("{}\\{name}", layout.game_dir)).is_ok_and(|size| size > 0)
        })
}

fn extractor_path(layout: &Layout) -> String {
    format!("{}\\{}", layout.exe_dir, game::EXTRACTOR_NAME)
}

/// The extractor beside the updater is the pinned file. It comes with the
/// Skhid archive; when it is not there or not that file, the Skhid files
/// are installed first, which brings it.
pub fn extractor_ready(layout: &Layout) -> bool {
    win::hash_file(&extractor_path(layout))
        .is_ok_and(|(size, hash)| game::EXTRACTOR.matches(size, hash))
}

/// Removes what an attempt that was cut short left in the game folder (the
/// PC switched off, the updater killed while the extractor ran): extraction
/// folders, each with up to 6.3 GB in it, and, once the game is there, the
/// 2.9 GB download. The caller holds the installation lock, so no other
/// updater is extracting here.
pub fn sweep(layout: &Layout) {
    for name in win::list_folders(&layout.game_dir, STAGE_PATTERN) {
        let stage = format!("{}\\{name}", layout.game_dir);
        clean_stage(&stage);
        if win::exists(&stage) {
            log!(
                "left over and not removed (in use, or it holds files that are not the game's): {stage}"
            );
        } else {
            log!("removed a left-over extraction folder: {stage}");
        }
    }
    let archive = archive_path(layout);
    if present(layout) && win::exists(&archive) && win::delete(&archive).is_ok() {
        log!("removed the left-over game download: {archive}");
    }
    let output = output_path(layout);
    if win::exists(&output) {
        log!("what the extractor printed in an attempt that was cut short:");
        log_output(&output);
    }
}

/// Where the extractor's messages go while it runs, beside the log.
fn output_path(layout: &Layout) -> String {
    format!("{}\\updater_skhid.7zr.txt", layout.exe_dir)
}

/// The extractor prints some twenty lines for the five files; the end of
/// more than this is kept, where the errors are.
const OUTPUT_LIMIT: usize = 16 * 1024;

/// Copies what the extractor printed into the log, line by line, and removes
/// its file. Without this the log has the extractor's exit number only, and
/// not which file it could not write or why.
fn log_output(path: &str) {
    match win::read_tail(path, OUTPUT_LIMIT) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
                log!("7zr: {line}");
            }
        }
        Err(code) => log!(
            "what the extractor printed cannot be read: {}",
            win::error_text(code)
        ),
    }
    let _ = win::delete(path);
}

/// `win::staging_dir` makes `.skhid-extract-<ticks>-<n>`.
const STAGE_PATTERN: &str = ".skhid-extract-*";

fn archive_path(layout: &Layout) -> String {
    format!("{}\\.skhid-game.7z", layout.game_dir)
}

fn io_error(action: &str, code: win::Code) -> Option<String> {
    if code == windows_sys::Win32::Foundation::ERROR_CANCELLED {
        return None;
    }
    Some(format!("Cannot {action}: {}.", win::error_text(code)))
}

/// A mirror override supplies only a URL, never a hash or a way to skip it.
fn address(layout: &Layout, name: &str, default: &str) -> Result<String, Option<String>> {
    let path = format!("{}\\{name}", layout.exe_dir);
    match win::read_file(&path, 4096) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(|s| s.trim().into())
            .map_err(|_| Some(format!("{name} must contain a UTF-8 HTTPS download URL."))),
        Err(code) if win::is_missing(code) => Ok(default.into()),
        Err(code) => Err(io_error(&format!("read {name}"), code)),
    }
}

fn check(
    path: &str,
    pin: Pin,
    session: &mut Session,
) -> Result<Option<win::LockedFile>, Option<String>> {
    match win::file_size(path) {
        Ok(size) if size != pin.size => return Ok(None),
        Err(code) if win::is_missing(code) => return Ok(None),
        Err(code) => return Err(io_error("read the downloaded file size", code)),
        _ => {}
    }
    let file = match win::LockedFile::open(path) {
        Ok(file) => file,
        Err(code) if win::is_missing(code) => return Ok(None),
        Err(code) => return Err(io_error("open the downloaded file", code)),
    };
    session.begin_progress();
    session.state(state::CHECKING);
    let (size, hash) = file
        .verify(&mut |done| session.progress(done, pin.size))
        .map_err(|code| io_error("verify the downloaded file", code))?;
    Ok(pin.matches(size, hash).then_some(file))
}

/// Streams `url` into a new file at `path`, flushed: at most `limit` bytes.
/// A failed transfer is deleted. `what` names it for the player.
fn download_to(
    session: &mut Session,
    url: &url::Url,
    path: &str,
    limit: u64,
    what: &str,
) -> Result<(), Option<String>> {
    let result = (|| {
        session.begin_progress();
        session.state(state::DOWNLOADING);
        let output =
            win::DownloadFile::create(path).map_err(|code| io_error("save the download", code))?;
        http::get_stream(url, "", limit,
            &mut |chunk| output.write(chunk).map_err(|code| HttpError::Network("write download", code)),
            &mut |done, total| session.progress(done, total.unwrap_or(limit)),
        ).map_err(|error| match error {
            HttpError::Stopped => None,
            _ => Some(format!("The {what} download failed. {} No downloaded file was installed. Retry or use another direct HTTPS mirror.", crate::text::http(url, "", &error))),
        })?;
        output
            .flush()
            .map_err(|code| io_error("flush the download to disk", code))
    })();
    if result.is_err() {
        let _ = win::delete(path);
    }
    result
}

/// Returns a read lock only after hashing the bytes actually on disk. The lock
/// stays alive until the extractor exits, preventing replacement after checking.
fn fetch(
    session: &mut Session,
    address: &str,
    path: &str,
    pin: Pin,
) -> Result<win::LockedFile, Option<String>> {
    let url = url::download(address)
        .ok_or_else(|| Some("The game download URL is invalid. Use a direct HTTPS URL.".into()))?;
    if let Some(file) = check(path, pin, session)? {
        log!("reusing verified download: {path}");
        return Ok(file);
    }
    download_to(session, &url, path, pin.size, "game")?;
    let checked = check(path, pin, session).and_then(|file| file.ok_or_else(|| Some(format!(
        "Download rejected: size or SHA-256 mismatch. Expected {} bytes and SHA-256 {}. Nothing from this download was extracted or run.", pin.size, pin.sha256))));
    if checked.is_err() {
        let _ = win::delete(path);
    }
    checked
}

/// Where a pristine resources.db comes from when the one here is not the
/// original and cannot be put back: the address in this file beside the
/// updater, else [`game::DB_URL`] (none yet: then the game's own pinned
/// archive, as for a first installation).
const DB_URL_FILE: &str = "updater_skhid.db-url";

/// Puts a pristine resources.db in the place of one that is missing or not
/// `original` (the stock file): from the address of [`DB_URL_FILE`] (the
/// file itself, plain or zstd-compressed, checked against `original` once
/// unpacked and only then moved into place), else from the pinned game
/// archive. `Err(None)`: told to stop.
pub fn reinstall_db(
    session: &mut Session,
    layout: &Layout,
    original: FileId,
) -> Result<(), Option<String>> {
    let db = format!("{}\\resources.db", layout.game_dir);
    let address = address(layout, DB_URL_FILE, game::DB_URL)?;
    if address.is_empty() {
        log!("resources.db: no {DB_URL_FILE}: the original game's archive is downloaded for it");
        win::delete(&db).map_err(|code| io_error("remove game\\resources.db", code))?;
        return ensure(session, layout).map(|_| ());
    }
    let url = url::download(&address)
        .ok_or_else(|| Some(format!("{DB_URL_FILE} must hold a direct HTTPS download URL.")))?;
    // The download (never more than the file unpacked) and the file.
    let needed = 2 * original.size + (256 << 20);
    let free = win::free_space(&layout.game_dir)
        .ok_or_else(|| Some("Cannot check free disk space.".into()))?;
    if free < needed {
        return Err(Some(format!(
            "it needs {} MB of free disk space; this disk has {} MB.",
            needed >> 20,
            free >> 20
        )));
    }
    log!("resources.db: downloading a pristine copy from {address}");
    let download = format!("{}\\.skhid-resources.db.download", layout.game_dir);
    let staged = format!("{db}.skhid-new");
    // zstd may add a little to what does not compress.
    let limit = original.size + original.size / 128 + (1 << 20);
    download_to(session, &url, &download, limit, "resources.db")?;
    session.begin_progress();
    session.state(state::INSTALLING);
    session.send(ToLauncher::InstallInfo {
        step: step::EXTRACT,
        text: "resources.db",
    });
    session.named_file = true;
    let unpacked = crate::unzstd::unpack(&download, &staged, &mut |done| {
        session.progress(done, original.size)
    });
    let _ = win::delete(&download);
    let result = match unpacked {
        Ok(file) if file == original => win::rename(&staged, &db, true)
            .map_err(|code| io_error("put resources.db in its place", code)),
        Ok(file) => Err(Some(format!(
            "the file downloaded from {address} is not the original resources.db ({} bytes, SHA-256 {})",
            file.size,
            core::str::from_utf8(&to_hex(&file.sha256)).unwrap_or("")
        ))),
        Err(None) => Err(None),
        Err(Some(why)) => Err(Some(format!("the download cannot be unpacked ({why})"))),
    };
    if result.is_err() {
        let _ = win::delete(&staged);
    } else {
        log!("resources.db: the pristine copy is in place, checked");
    }
    result
}

fn extracted(stage: &str, name: &str) -> String {
    format!("{stage}\\Survarium\\game\\{}", name.replace('/', "\\"))
}

fn clean_stage(stage: &str) {
    for (name, _) in game::FILES {
        let _ = win::delete(&extracted(stage, name));
    }
    for dir in [
        "resources\\sources\\splash_screens",
        "resources\\sources",
        "resources\\ssl",
        "resources",
        "binaries\\x86",
        "binaries",
        "",
    ] {
        win::remove_dir(&format!("{stage}\\Survarium\\game\\{dir}"));
    }
    win::remove_dir(&format!("{stage}\\Survarium"));
    win::remove_dir(stage);
}

/// Installs only missing original files. Every extraction and check finishes
/// before promotion; failed promotions undo only files added by this attempt.
pub fn ensure(session: &mut Session, layout: &Layout) -> Result<bool, Option<String>> {
    if present(layout) {
        return Ok(false);
    }
    log!("original game files missing; downloading the pinned 0.26g archive");
    // The extractor first: without it 2.9 GB would be downloaded for nothing.
    // It is a file of the Skhid archive and is never downloaded on its own;
    // it stays locked from this check until it has run.
    let extractor_path = extractor_path(layout);
    let Some(extractor) = check(&extractor_path, game::EXTRACTOR, session)? else {
        return Err(Some(format!(
            "The original game cannot be unpacked: {} beside the updater is missing or is not the file this updater runs (7-Zip's 7zr.exe, {} bytes, SHA-256 {}). It comes with the Skhid archive: unpack that archive again. Nothing was downloaded or run.",
            game::EXTRACTOR_NAME,
            game::EXTRACTOR.size,
            game::EXTRACTOR.sha256
        )));
    };
    let archive_path = archive_path(layout);
    // Archive + unpacked files + room for the Skhid overlay; an archive that
    // is here already (kept after an extraction that failed) needs no room
    // again. This works on a 32-bit updater too: file sizes and progress are
    // u64 throughout.
    let downloaded = win::file_size(&archive_path).is_ok_and(|size| size == game::ARCHIVE.size);
    let needed = if downloaded { 0 } else { game::ARCHIVE.size }
        + game::FILES.iter().map(|(_, size)| size).sum::<u64>()
        + (256 << 20);
    let free = win::free_space(&layout.game_dir)
        .ok_or_else(|| Some("Cannot check free disk space for the game installation.".into()))?;
    if free < needed {
        return Err(Some(format!(
            "The game installation needs {needed} bytes of free space; this disk has {free}."
        )));
    }
    let game_url = address(layout, "updater_skhid.game-url", game::GAME_URL)?;
    let archive = fetch(session, &game_url, &archive_path, game::ARCHIVE)?;
    let stage = win::staging_dir(&layout.game_dir)
        .map_err(|code| io_error("create an extraction folder", code))?;
    let result = (|| {
        session.state(state::INSTALLING);
        session.send(ToLauncher::InstallInfo {
            step: step::EXTRACT,
            text: "Survarium 0.26g",
        });
        // -sccUTF-8: what it prints is UTF-8, so a folder name in any
        // alphabet reads right in the log.
        let mut command = format!(
            "\"{extractor_path}\" x \"{archive_path}\" -o\"{stage}\" -y -bd -bb0 -sccUTF-8"
        );
        for (name, _) in game::FILES {
            command.push_str(&format!(" \"Survarium/game/{name}\""));
        }
        let output_path = output_path(layout);
        let code = win::run_wait(
            &extractor_path,
            &command,
            &layout.game_dir,
            &output_path,
            &mut || session.pump(),
        );
        log_output(&output_path);
        let code = code.map_err(|code| io_error("extract the original game", code))?;
        log!("the extractor ended with exit {code}");
        if code != 0 {
            return Err(Some(format!(
                "The game extractor failed (exit {code}). Nothing was installed. What it \
                 printed is in updater_skhid.log beside the updater."
            )));
        }
        for (name, expected) in game::FILES {
            if win::file_size(&extracted(&stage, name)).ok() != Some(*expected) {
                return Err(Some(format!(
                    "The extracted game file {name} is missing or has the wrong size. Nothing was installed."
                )));
            }
        }
        // Do not silently accept an empty file or a directory in a required
        // destination, and do not overwrite a user's conflicting path.
        for (name, _) in game::FILES {
            let target = format!("{}\\{}", layout.game_dir, name.replace('/', "\\"));
            if win::exists(&target) && !win::file_size(&target).is_ok_and(|size| size > 0) {
                return Err(Some(format!(
                    "Cannot install {name}: its destination is empty, unreadable or a directory. Move it aside and retry."
                )));
            }
        }
        let original = check(
            &extracted(&stage, "binaries/x86/survarium.exe"),
            game::ORIGINAL_EXE,
            session,
        )?;
        if original.is_none() {
            return Err(Some("The extracted survarium.exe failed its pinned SHA-256 check. Nothing was installed.".into()));
        }
        drop(original);
        if !session.pump() {
            return Err(None);
        }
        let mut added: Vec<String> = Vec::new();
        for (name, _) in game::FILES {
            let target = format!("{}\\{}", layout.game_dir, name.replace('/', "\\"));
            // Preserve existing files, including the Skhid certificate and DLL.
            if win::exists(&target) {
                continue;
            }
            let result = win::create_folders(win::split_path(&target).0)
                .and_then(|()| win::rename(&extracted(&stage, name), &target, false));
            if let Err(code) = result {
                for path in added.iter().rev() {
                    let _ = win::delete(path);
                }
                return Err(io_error("install the original game files", code));
            }
            added.push(target);
        }
        log!("installed original game: {} files added", added.len());
        Ok(true)
    })();
    clean_stage(&stage);
    drop(extractor);
    drop(archive);
    // Retain the verified download on extraction failure for a retry; it is
    // always rehashed next time. Successful installs need no archive cache.
    // The extractor is a file of the Skhid archive and stays.
    if result.is_ok() {
        let _ = win::delete(&archive_path);
    }
    result
}
