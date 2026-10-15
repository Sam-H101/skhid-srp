//! The game's database: after the overlay, at every start the game may be
//! played after, `game\resources_skhid.db` is made the release's
//! (`updater_logic::dbpatch`; `client/updater/README.md`, "The game's
//! database"). The game's own `resources.db` is only read.
//!
//! - It is checked at the revision of the state kept (its size and the
//!   SHA-256 of its first and last 16 MB): the patches newer than that are
//!   applied to it in place, and the whole file is hashed.
//! - It is missing, or nothing says what it is: an unknown one is hashed
//!   whole and taken at the revision it is; anything else is built anew,
//!   `resources.db` copied (and hashed as it is read: it must be the list's
//!   original), every patch applied, the whole file hashed, then renamed
//!   into its place.
//! - A result of another SHA-256 than the release's is built anew from the
//!   original once; then PLAY stays off.
//! - A `resources.db` that is missing or not the original is downloaded
//!   again (`game::reinstall_db`).
//!
//! Before that, once: what the updaters before this one wrote into
//! `resources.db` in place (the menus) is put back from their copies or the
//! server's blobs, the whole file checked against the original, and their
//! files removed.
//!
//! Anything not done keeps PLAY off with words that say what to do
//! ([`Outcome::Blocked`]); the next start repairs it.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use updater_logic::dbpatch::{
    self, FileId, Look, MANIFEST_NAME, Manifest, Op, Patch, State, Todo, parse_manifest,
    parse_state, probe_ranges, write_state,
};
use updater_logic::legacy::{self, Record};
use updater_logic::manifest::Installed;
use updater_logic::pipe::{ToLauncher, state, step};
use updater_logic::sha256::{Sha256, to_hex};
use updater_logic::url::Url;
use windows_sys::Win32::Foundation::{ERROR_HANDLE_EOF, ERROR_SHARING_VIOLATION};

use crate::Session;
use crate::http::{self, HttpError};
use crate::install::Layout;
use crate::log::log;
use crate::unzstd::Zstd;
use crate::win::{self, Code, File};

/// Beside the updater.
const LIST_FILE: &str = "updater_skhid.patches";
const STATE_FILE: &str = "updater_skhid.patches.state";
/// What the last run found: `ready` or `blocked <why>`, for support.
const STATUS_FILE: &str = "updater_skhid.patches.status";
const CACHE_DIR: &str = "updater_skhid.cache";
/// What the updaters before this one kept beside them.
const LEGACY_FILES: [&str; 4] = [
    "updater_skhid.resources.state",
    "updater_skhid.resources.journal",
    "updater_skhid.resources.status",
    "updater_skhid.resources",
];
const LEGACY_BACKUP_DIR: &str = "updater_skhid.backup";
/// `resources_skhid.db` while it is built; one left by a run that was cut
/// short is removed.
const BUILDING: &str = ".skhid-build";
/// Room left on the disk after a build.
const DISK_MARGIN: u64 = 64 << 20;
const PART: &str = ".part";
const FETCH_TRIES: u32 = 2;
const RETRY_PAUSE_MS: u32 = 3000;
/// Bytes read or written at a time.
const PIECE: usize = 1 << 20;

fn hex(sha256: &[u8; 32]) -> String {
    String::from_utf8_lossy(&to_hex(sha256)).into()
}

/// The SHA-256 of `size` bytes of `file` from `offset`.
fn hash_range(file: &File, offset: u64, size: u64) -> Result<[u8; 32], Code> {
    let mut hash = Sha256::new();
    hash_into(&mut hash, file, offset, size, &mut |_| true)?;
    Ok(hash.finish())
}

fn hash_into(
    hash: &mut Sha256,
    file: &File,
    offset: u64,
    size: u64,
    progress: &mut dyn FnMut(u64) -> bool,
) -> Result<(), Code> {
    file.seek(offset)?;
    let mut buffer = alloc::vec![0_u8; PIECE];
    let mut done = 0;
    while done < size {
        let want = ((size - done).min(PIECE as u64)) as usize;
        let read = file.read(&mut buffer[..want])?;
        if read == 0 {
            return Err(ERROR_HANDLE_EOF);
        }
        hash.update(&buffer[..read]);
        done += read as u64;
        if !progress(done) {
            return Err(windows_sys::Win32::Foundation::ERROR_CANCELLED);
        }
    }
    Ok(())
}

/// The SHA-256 of what the quick look reads.
fn probe(file: &File, size: u64) -> Result<[u8; 32], Code> {
    let mut hash = Sha256::new();
    for (offset, length) in probe_ranges(size) {
        hash_into(&mut hash, file, offset, length, &mut |_| true)?;
    }
    Ok(hash.finish())
}

/// A file of this size and SHA-256, held so that no one changes it until it
/// is dropped.
fn checked(path: &str, file: &FileId) -> Option<File> {
    let held = File::locked(path).ok()?;
    (held.size().ok()? == file.size && hash_range(&held, 0, file.size).ok()? == file.sha256)
        .then_some(held)
}

/// The quick look at `resources_skhid.db`.
fn look(path: &str) -> Look {
    let file = match File::shared(path) {
        Ok(file) => file,
        Err(code) if win::is_missing(code) => return Look::Missing,
        Err(_) => return Look::Unreadable,
    };
    match file.size().and_then(|size| Ok((size, probe(&file, size)?))) {
        Ok((size, probe)) => Look::Found { size, probe },
        Err(_) => Look::Unreadable,
    }
}

fn read_list(layout: &Layout) -> Result<Option<Manifest>, String> {
    let path = format!("{}\\{LIST_FILE}", layout.exe_dir);
    match win::read_file(&path, 4 << 20) {
        Ok(bytes) => parse_manifest(&bytes)
            .map(Some)
            .map_err(|error| format!("{LIST_FILE} cannot be used ({error:?})")),
        Err(code) if win::is_missing(code) => Ok(None),
        Err(code) => Err(format!(
            "{LIST_FILE} cannot be read ({})",
            win::error_text(code)
        )),
    }
}

fn read_state(layout: &Layout) -> Option<State> {
    let bytes = win::read_file(&format!("{}\\{STATE_FILE}", layout.exe_dir), 4096).ok()?;
    parse_state(&bytes)
}

/// Why the database is not ready: the first that was found decides what
/// the player is told.
#[derive(Clone)]
enum Problem {
    GameRunning,
    DiskFull {
        folder: String,
        needed: u64,
        free: u64,
    },
    Offline,
    /// The original is missing or not the original, and could not be
    /// downloaded again.
    Original(String),
}

/// How the step ended.
pub enum Outcome {
    Ready,
    Stopped,
    /// PLAY stays off, with these words.
    Blocked(String),
}

/// Why something was not done (said in the log already).
enum Fail {
    Stopped,
    Other,
}

struct Step<'a> {
    session: &'a mut Session,
    layout: &'a Layout,
    /// `None` when the server could not be asked this run.
    server: Option<&'a Url>,
    problem: Option<Problem>,
    /// `resources.db` was downloaded again this run (once at most).
    reinstalled: bool,
}

impl Step<'_> {
    fn beside(&self, name: &str) -> String {
        format!("{}\\{name}", self.layout.exe_dir)
    }

    fn original_path(&self) -> String {
        self.layout.place(dbpatch::ORIGINAL_PATH)
    }

    fn skhid_path(&self) -> String {
        self.layout.place(dbpatch::SKHID_PATH)
    }

    fn cached(&self, sha256: &[u8; 32]) -> String {
        format!("{}\\{CACHE_DIR}\\{}", self.layout.exe_dir, hex(sha256))
    }

    fn trouble(&mut self, problem: Problem) {
        if self.problem.is_none() {
            self.problem = Some(problem);
        }
    }

    /// The launcher's line: "Extracting file <name>", with the bar.
    fn installing(&mut self, name: &str) {
        self.session.state(state::INSTALLING);
        self.session.send(ToLauncher::InstallInfo {
            step: step::EXTRACT,
            text: name,
        });
        self.session.named_file = true;
        self.session.begin_progress();
    }

    /// Opens a file by this program alone; a game that runs holds it.
    fn open(&mut self, path: &str) -> Result<File, Fail> {
        match File::exclusive(path) {
            Ok(file) => Ok(file),
            Err(ERROR_SHARING_VIOLATION) => {
                log!("db: the game is running ({path} is open in another program)");
                self.trouble(Problem::GameRunning);
                Err(Fail::Other)
            }
            Err(code) => {
                log!(
                    "db: {path} cannot be opened to write ({})",
                    win::error_text(code)
                );
                Err(Fail::Other)
            }
        }
    }

    /// The whole file hashed, with the bar. `Err(None)`: told to stop.
    fn hash_whole(&mut self, path: &str, what: &str) -> Result<FileId, Option<Code>> {
        let file = File::shared(path).map_err(Some)?;
        let size = file.size().map_err(Some)?;
        self.session.state(state::CHECKING);
        self.session.begin_progress();
        let started = win::ticks();
        let mut hash = Sha256::new();
        let session = &mut *self.session;
        hash_into(&mut hash, &file, 0, size, &mut |done| {
            session.progress(done, size)
        })
        .map_err(|code| {
            (code != windows_sys::Win32::Foundation::ERROR_CANCELLED).then_some(code)
        })?;
        log!(
            "db: {what} hashed whole in {} ms ({size} bytes)",
            win::ticks().saturating_sub(started)
        );
        Ok(FileId {
            sha256: hash.finish(),
            size,
        })
    }

    /// The blob, in the cache: the one there if it checks, else fetched.
    fn fetch(&mut self, blob: &FileId, what: &str) -> Result<String, Fail> {
        let path = self.cached(&blob.sha256);
        if checked(&path, blob).is_some() {
            log!("db: {what}: using the blob kept in {CACHE_DIR}");
            return Ok(path);
        }
        let Some(server) = self.server else {
            log!("db: {what}: its blob is not here and the server could not be asked");
            self.trouble(Problem::Offline);
            return Err(Fail::Other);
        };
        let folder = self.beside(CACHE_DIR);
        if let Err(code) = win::create_folders(&folder) {
            log!("db: {folder} cannot be made ({})", win::error_text(code));
            return Err(Fail::Other);
        }
        let address = format!("/resources/{}", hex(&blob.sha256));
        let part = format!("{path}{PART}");
        for attempt in 1..=FETCH_TRIES {
            self.session.state(state::DOWNLOADING);
            self.session.begin_progress();
            let output = match File::create(&part) {
                Ok(output) => output,
                Err(code) => {
                    log!("db: {part} cannot be written ({})", win::error_text(code));
                    return Err(Fail::Other);
                }
            };
            let session = &mut *self.session;
            let size = blob.size;
            let result = http::get_stream(
                server,
                &address,
                size,
                &mut |chunk| {
                    output
                        .write(chunk)
                        .map_err(|code| HttpError::Network("write the blob", code))
                },
                &mut |done, _| session.progress(done, size),
            )
            .map_err(|error| crate::text::http(server, &address, &error))
            .and_then(|()| {
                output.flush().map_err(|code| {
                    format!("the blob cannot be written ({})", win::error_text(code))
                })
            });
            drop(output);
            let result = result.and_then(|()| {
                if checked(&part, blob).is_some() {
                    win::rename(&part, &path, true).map_err(|code| {
                        format!("the blob cannot be renamed ({})", win::error_text(code))
                    })
                } else {
                    Err(format!(
                        "the server's blob is not {size} bytes with that SHA-256"
                    ))
                }
            });
            match result {
                Ok(()) => {
                    log!("db: {what}: fetched {address} ({size} bytes, checksum right)");
                    return Ok(path);
                }
                Err(message) => {
                    let _ = win::delete(&part);
                    if self.session.stopped {
                        return Err(Fail::Stopped);
                    }
                    log!("db: {what}: fetch {attempt} of {FETCH_TRIES} failed: {message}");
                    if attempt < FETCH_TRIES && !self.session.pause(RETRY_PAUSE_MS) {
                        return Err(Fail::Stopped);
                    }
                }
            }
        }
        Err(Fail::Other)
    }

    /// Every blob of `patches`, fetched before the long work starts.
    fn fetch_all(&mut self, patches: &[Patch]) -> Result<Vec<String>, Fail> {
        let mut paths = Vec::new();
        for patch in patches {
            paths.push(self.fetch(&patch.blob, &format!("patch {}", patch.revision))?);
        }
        Ok(paths)
    }

    /// Room for `grow` more bytes on the game's disk and for the blobs not
    /// in the cache yet.
    fn room(&mut self, grow: u64, patches: &[Patch]) -> bool {
        let blobs: u64 = patches
            .iter()
            .filter(|patch| {
                win::file_size(&self.cached(&patch.blob.sha256)).ok() != Some(patch.blob.size)
            })
            .map(|patch| patch.blob.size)
            .sum();
        let needed = grow + blobs + DISK_MARGIN;
        let folder = self.layout.game_dir.clone();
        match win::free_space(&folder) {
            Some(free) if free < needed => {
                log!("db: {free} bytes free in {folder}, {needed} needed");
                self.trouble(Problem::DiskFull {
                    folder,
                    needed,
                    free,
                });
                false
            }
            _ => true,
        }
    }

    /// Applies `patch` to `file` (its input, `patch.from`), from the
    /// decompressed blob at `blob`. Flushed; not hashed (the caller hashes
    /// the whole file once its last patch is in).
    fn apply(&mut self, file: &File, patch: &Patch, blob: &str) -> Result<(), String> {
        let Some(held) = checked(blob, &patch.blob) else {
            return Err(format!("{blob} is not the blob it was"));
        };
        let mut data = Zstd::open(held)?;
        let total = patch.data_size();
        let mut done = 0_u64;
        let mut end = patch.from.size;
        let mut buffer = alloc::vec![0_u8; PIECE];
        let session = &mut *self.session;
        for op in &patch.ops {
            let at = match *op {
                Op::Write { offset, .. } => offset,
                Op::Append { length } => {
                    end += length;
                    end - length
                }
            };
            file.seek(at)
                .map_err(|code| format!("seek: {}", win::error_text(code)))?;
            let mut left = op.length();
            while left > 0 {
                let piece = (left.min(PIECE as u64)) as usize;
                data.read_exact(&mut buffer[..piece])?;
                file.write(&buffer[..piece])
                    .map_err(|code| format!("write: {}", win::error_text(code)))?;
                left -= piece as u64;
                done += piece as u64;
                // A patch is finished once begun: a stop waits for its end.
                let _ = session.progress(done, total);
            }
        }
        if data.read(&mut buffer[..1])? != 0 {
            return Err("the blob holds more than the patch's writes and appends".into());
        }
        file.flush()
            .map_err(|code| format!("flush: {}", win::error_text(code)))?;
        match file.size() {
            Ok(size) if size == patch.to.size => Ok(()),
            Ok(size) => Err(format!("the file has {size} bytes, not {}", patch.to.size)),
            Err(code) => Err(format!("size: {}", win::error_text(code))),
        }
    }

    /// Applies `patches` in order to the file at `path`, hashes it whole and
    /// compares with `target`. `Ok(false)`: another SHA-256.
    fn apply_all(
        &mut self,
        path: &str,
        patches: &[Patch],
        blobs: &[String],
        target: &FileId,
    ) -> Result<bool, Fail> {
        {
            let file = self.open(path)?;
            for (patch, blob) in patches.iter().zip(blobs) {
                self.installing("resources_skhid.db");
                if let Err(message) = self.apply(&file, patch, blob) {
                    log!(
                        "db: patch {} could not be applied: {message}",
                        patch.revision
                    );
                    return Ok(false);
                }
                log!("db: patch {} applied", patch.revision);
            }
        }
        match self.hash_whole(path, "resources_skhid.db") {
            Ok(file) if file == *target => Ok(true),
            Ok(file) => {
                log!(
                    "db: the result is {} bytes with SHA-256 {}, not the release's {} bytes with {}",
                    file.size,
                    hex(&file.sha256),
                    target.size,
                    hex(&target.sha256)
                );
                Ok(false)
            }
            Err(None) => Err(Fail::Stopped),
            Err(Some(code)) => {
                log!("db: {path} cannot be read back ({})", win::error_text(code));
                Ok(false)
            }
        }
    }

    /// Writes the state of the database checked whole at `revision`.
    fn keep_state(&self, revision: u64, file: FileId) {
        let path = self.skhid_path();
        let probe = File::shared(&path).and_then(|held| probe(&held, file.size));
        let result = probe.and_then(|probe| {
            let state = State {
                revision,
                file,
                probe,
            };
            let target = self.beside(STATE_FILE);
            let staged = format!("{target}.skhid-new");
            win::write_file(&staged, &write_state(&state))
                .and_then(|()| win::rename(&staged, &target, true))
        });
        if let Err(code) = result {
            log!(
                "db: {STATE_FILE} cannot be written ({}): it is hashed whole next time",
                win::error_text(code)
            );
        }
    }

    fn forget_state(&self) -> bool {
        win::delete(&self.beside(STATE_FILE)).is_ok()
    }

    /// `resources.db` is not there or not `original`: downloaded again,
    /// once a run.
    fn reinstall(&mut self, original: FileId) -> Result<(), Fail> {
        if self.reinstalled {
            self.trouble(Problem::Original(
                "the downloaded copy is not the original either".into(),
            ));
            return Err(Fail::Other);
        }
        self.reinstalled = true;
        // The game of an older exe holds it.
        if win::exists(&self.original_path()) {
            drop(self.open(&self.original_path())?);
        }
        match crate::game::reinstall_db(self.session, self.layout, original) {
            Ok(()) => Ok(()),
            Err(None) => Err(Fail::Stopped),
            Err(Some(why)) => {
                log!("db: resources.db could not be downloaded again: {why}");
                self.trouble(Problem::Original(why));
                Err(Fail::Other)
            }
        }
    }

    /// The original, copied to `to` and hashed as it is read. `Ok(true)`:
    /// it is `original`.
    fn copy_original(&mut self, to: &str, original: &FileId) -> Result<bool, Fail> {
        let from = self.original_path();
        let source = match File::shared(&from) {
            Ok(file) => file,
            Err(code) if win::is_missing(code) => {
                log!("db: {from} is not there");
                return Ok(false);
            }
            Err(code) => {
                log!("db: {from} cannot be read ({})", win::error_text(code));
                return Ok(false);
            }
        };
        let size = match source.size() {
            Ok(size) if size == original.size => size,
            Ok(size) => {
                log!(
                    "db: {from} has {size} bytes, not {} as the original",
                    original.size
                );
                return Ok(false);
            }
            Err(_) => return Ok(false),
        };
        let output = match File::create(to) {
            Ok(file) => file,
            Err(code) => {
                log!("db: {to} cannot be made ({})", win::error_text(code));
                return Err(Fail::Other);
            }
        };
        self.installing("resources_skhid.db");
        let started = win::ticks();
        let mut hash = Sha256::new();
        let mut buffer = alloc::vec![0_u8; PIECE];
        let mut done = 0_u64;
        while done < size {
            let read = match source.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    log!("db: {from} cannot be read whole");
                    return Ok(false);
                }
                Ok(read) => read,
            };
            hash.update(&buffer[..read]);
            if let Err(code) = output.write(&buffer[..read]) {
                log!("db: {to} cannot be written ({})", win::error_text(code));
                return Err(Fail::Other);
            }
            done += read as u64;
            if !self.session.progress(done, size) {
                return Err(Fail::Stopped);
            }
        }
        if let Err(code) = output.flush() {
            log!("db: {to} cannot be written ({})", win::error_text(code));
            return Err(Fail::Other);
        }
        let sha256 = hash.finish();
        log!(
            "db: resources.db copied in {} ms",
            win::ticks().saturating_sub(started)
        );
        if sha256 != original.sha256 {
            log!("db: {from} is not the original (SHA-256 {})", hex(&sha256));
            return Ok(false);
        }
        Ok(true)
    }

    /// `resources_skhid.db` built anew: the original copied, every patch
    /// applied, the whole file checked, renamed into its place. Twice at
    /// most, once more when the first result is not the release's.
    fn build(&mut self, manifest: &Manifest, legacy: &mut bool) -> Result<(), Fail> {
        let (revision, target) = manifest.target();
        let path = self.skhid_path();
        let building = format!("{path}{BUILDING}");
        // What is there is of no use; a game that holds it is told.
        if win::exists(&path) {
            drop(self.open(&path)?);
            let _ = self.forget_state();
            if let Err(code) = win::delete(&path) {
                log!("db: {path} cannot be removed ({})", win::error_text(code));
                return Err(Fail::Other);
            }
        }
        if !self.room(target.size, &manifest.patches) {
            return Err(Fail::Other);
        }
        let blobs = self.fetch_all(&manifest.patches)?;
        for attempt in 1..=2 {
            log!("db: building resources_skhid.db (revision {revision}), attempt {attempt}");
            if !self.copy_original(&building, &manifest.original)? {
                let _ = win::delete(&building);
                self.reinstall(manifest.original)?;
                if !self.copy_original(&building, &manifest.original)? {
                    let _ = win::delete(&building);
                    self.trouble(Problem::Original(
                        "the downloaded copy is not the original either".into(),
                    ));
                    return Err(Fail::Other);
                }
            }
            if *legacy {
                // Checked whole as it was copied.
                self.finish_legacy();
                *legacy = false;
            }
            if self.apply_all(&building, &manifest.patches, &blobs, &target)? {
                return match win::rename(&building, &path, true) {
                    Ok(()) => {
                        log!("db: resources_skhid.db built, revision {revision}, checked whole");
                        self.keep_state(revision, target);
                        Ok(())
                    }
                    Err(code) => {
                        log!(
                            "db: {path} cannot be put in place ({})",
                            win::error_text(code)
                        );
                        let _ = win::delete(&building);
                        Err(Fail::Other)
                    }
                };
            }
            let _ = win::delete(&building);
        }
        Err(Fail::Other)
    }

    /// The patches after `revision` applied in place. `Ok(false)`: the
    /// result is not the release's (the caller builds anew).
    fn update(&mut self, manifest: &Manifest, revision: u64) -> Result<bool, Fail> {
        let patches = manifest.after(revision);
        let (to, target) = manifest.target();
        let path = self.skhid_path();
        drop(self.open(&path)?);
        let grow = target
            .size
            .saturating_sub(manifest.at(revision).map_or(0, |file| file.size));
        if !self.room(grow, patches) {
            return Err(Fail::Other);
        }
        let blobs = self.fetch_all(patches)?;
        log!("db: resources_skhid.db from revision {revision} to {to}");
        // Changed from here on: an interruption leaves no state, and the
        // next start hashes it.
        if !self.forget_state() {
            log!("db: {STATE_FILE} cannot be removed");
            return Err(Fail::Other);
        }
        if self.apply_all(&path, patches, &blobs, &target)? {
            log!("db: resources_skhid.db is at revision {to}, checked whole");
            self.keep_state(to, target);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// The files of the updaters before this one are there.
    fn legacy_pending(&self) -> bool {
        LEGACY_FILES[..2]
            .iter()
            .any(|name| win::exists(&self.beside(name)))
            || win::is_folder(&self.beside(LEGACY_BACKUP_DIR))
    }

    /// Puts back the bytes of `resources.db` that the updaters before this
    /// one wrote in place, and cuts off what they appended. Not checked here:
    /// the whole file is checked against the original after.
    fn restore_legacy(&mut self) {
        let mut record = Record::default();
        for name in &LEGACY_FILES[..2] {
            if let Ok(bytes) = win::read_file(&self.beside(name), 1 << 20) {
                match legacy::parse_record(&bytes) {
                    Some(found) => record.merge(found),
                    None => log!("db: {name} cannot be read: the whole file decides"),
                }
            }
        }
        if record.is_empty() {
            return;
        }
        let path = self.original_path();
        if !win::exists(&path) {
            return;
        }
        log!(
            "db: resources.db was changed in place by an earlier updater ({} region(s)): put back",
            record.regions.len()
        );
        let Ok(file) = self.open(&path) else {
            return;
        };
        for region in &record.regions {
            if hash_range(&file, region.offset, region.size).ok() == Some(region.original) {
                continue;
            }
            let original = FileId {
                sha256: region.original,
                size: region.size,
            };
            let kept = format!(
                "{}\\{LEGACY_BACKUP_DIR}\\{}",
                self.layout.exe_dir,
                hex(&region.original)
            );
            let source = if checked(&kept, &original).is_some() {
                kept
            } else {
                match self.fetch(&original, &region.name) {
                    Ok(path) => path,
                    Err(_) => {
                        log!("db: {}: no copy of its original", region.name);
                        continue;
                    }
                }
            };
            let Some(blob) = checked(&source, &original) else {
                continue;
            };
            let result = (|| {
                let mut buffer = alloc::vec![0_u8; PIECE];
                blob.seek(0)?;
                file.seek(region.offset)?;
                let mut left = region.size;
                while left > 0 {
                    let read = blob.read(&mut buffer[..(left.min(PIECE as u64)) as usize])?;
                    if read == 0 {
                        return Err(ERROR_HANDLE_EOF);
                    }
                    file.write(&buffer[..read])?;
                    left -= read as u64;
                }
                file.flush()
            })();
            match result.and_then(|()| hash_range(&file, region.offset, region.size)) {
                Ok(hash) if hash == region.original => {
                    log!("db: {}: its original is back in resources.db", region.name);
                }
                _ => log!("db: {}: its original could not be put back", region.name),
            }
        }
        if let Some(shipped) = record.shipped_size
            && file.size().is_ok_and(|size| size > shipped)
        {
            match file.set_len(shipped).and_then(|()| file.flush()) {
                Ok(()) => log!("db: what an earlier updater appended to resources.db is cut off"),
                Err(code) => log!(
                    "db: resources.db cannot be cut back ({})",
                    win::error_text(code)
                ),
            }
        }
    }

    /// `resources.db` is checked stock: the earlier updaters' files go.
    fn finish_legacy(&self) {
        for name in LEGACY_FILES {
            let _ = win::delete(&self.beside(name));
        }
        let folder = self.beside(LEGACY_BACKUP_DIR);
        for name in win::list(&folder, "*") {
            let _ = win::delete(&format!("{folder}\\{name}"));
        }
        win::remove_dir(&folder);
        log!("db: resources.db is the original again; the earlier updater's copies are removed");
    }

    /// The original checked whole, downloaded again if need be.
    fn verify_original(&mut self, original: FileId) -> Result<(), Fail> {
        let path = self.original_path();
        match self.hash_whole(&path, "resources.db") {
            Ok(file) if file == original => return Ok(()),
            Ok(_) => log!("db: resources.db is not the original"),
            Err(None) => return Err(Fail::Stopped),
            Err(Some(code)) => log!(
                "db: resources.db cannot be read ({})",
                win::error_text(code)
            ),
        }
        self.reinstall(original)?;
        match self.hash_whole(&path, "resources.db") {
            Ok(file) if file == original => Ok(()),
            Err(None) => Err(Fail::Stopped),
            _ => {
                self.trouble(Problem::Original(
                    "the downloaded copy is not the original either".into(),
                ));
                Err(Fail::Other)
            }
        }
    }

    /// Removes from the cache what no patch after `revision` needs.
    fn sweep(&self, manifest: &Manifest, revision: u64) {
        let folder = self.beside(CACHE_DIR);
        if !win::is_folder(&folder) {
            return;
        }
        let wanted: Vec<String> = manifest
            .after(revision)
            .iter()
            .map(|patch| hex(&patch.blob.sha256))
            .collect();
        for name in win::list(&folder, "*") {
            if !wanted.iter().any(|keep| keep.eq_ignore_ascii_case(&name)) {
                let _ = win::delete(&format!("{folder}\\{name}"));
            }
        }
        win::remove_dir(&folder);
    }
}

/// The revision the database is at, checked; `None` when it is not one of
/// the list's.
fn todo(step: &mut Step, manifest: &Manifest) -> Result<Todo, Fail> {
    let path = step.skhid_path();
    let state = read_state(step.layout);
    let found = dbpatch::todo(manifest, state.as_ref(), look(&path));
    if found != Todo::Hash {
        return Ok(found);
    }
    log!("db: nothing says what resources_skhid.db is: it is hashed whole");
    match step.hash_whole(&path, "resources_skhid.db") {
        Ok(file) => match manifest.revision_of(&file) {
            Some(revision) => {
                log!("db: resources_skhid.db is at revision {revision}");
                step.keep_state(revision, file);
                Ok(if revision == manifest.target().0 {
                    Todo::Done
                } else {
                    Todo::From(revision)
                })
            }
            None => {
                log!("db: resources_skhid.db is no revision of the list: it is built anew");
                Ok(Todo::Build)
            }
        },
        Err(None) => Err(Fail::Stopped),
        Err(Some(_)) => Ok(Todo::Build),
    }
}

/// `updater_skhid.exe ready`, from play_skhid.bat and play_skhid.sh before
/// they start the game without the launcher: reads only. `Err` (the words
/// for the player) when the list beside it cannot be read, or
/// `resources_skhid.db` is not checked at its last revision.
pub fn ready(layout: &Layout) -> Result<(), String> {
    let manifest = match read_list(layout) {
        Ok(Some(manifest)) => manifest,
        Ok(None) => return Ok(()),
        Err(why) => {
            log!("ready: NOT READY: {why}");
            return Err(crate::text::db_not_ready());
        }
    };
    let path = layout.place(dbpatch::SKHID_PATH);
    match dbpatch::todo(&manifest, read_state(layout).as_ref(), look(&path)) {
        Todo::Done => {
            log!("ready: resources_skhid.db is the release's");
            Ok(())
        }
        other => {
            log!("ready: NOT READY: resources_skhid.db: {other:?}");
            Err(crate::text::db_not_ready())
        }
    }
}

/// The step. `server`: `None` when the server could not be asked this run.
pub fn run(
    session: &mut Session,
    layout: &Layout,
    server: Option<&Url>,
    installed: Option<&Installed>,
) -> Outcome {
    let status = format!("{}\\{STATUS_FILE}", layout.exe_dir);
    let in_overlay = installed.is_some_and(|installed| {
        installed
            .files
            .iter()
            .any(|file| file.name.eq_ignore_ascii_case(MANIFEST_NAME))
    });
    let blocked = |why: &str, message: String| {
        log!("db: BLOCKED: {why}. The player is told: {message}");
        let _ = win::write_file(&status, format!("blocked {why}\r\n").as_bytes());
        Outcome::Blocked(message)
    };
    let manifest = match read_list(layout) {
        Ok(Some(manifest)) => Some(manifest),
        Ok(None) if in_overlay => {
            return blocked(
                &format!("{LIST_FILE} of the overlay is gone"),
                crate::text::db_failed(),
            );
        }
        Ok(None) => None,
        Err(why) => return blocked(&why, crate::text::db_failed()),
    };
    let mut step = Step {
        session,
        layout,
        server,
        problem: None,
        reinstalled: false,
    };
    let building = format!("{}{BUILDING}", step.skhid_path());
    // What a run that was cut short left, each up to 6.3 GB: a build, a
    // download of the original and its unpacked copy (game::reinstall_db).
    for left in [
        building,
        format!("{}\\.skhid-resources.db.download", layout.game_dir),
        format!("{}.skhid-new", step.original_path()),
    ] {
        if win::exists(&left) && win::delete(&left).is_ok() {
            log!("db: removed {left}, left by a run that was cut short");
        }
    }
    let mut legacy = step.legacy_pending();
    if legacy {
        step.restore_legacy();
    }
    let Some(manifest) = manifest else {
        // An archive without the list (made without the launcher's files):
        // no resources_skhid.db; what earlier updaters left stays until a
        // list says what the original is.
        return Outcome::Ready;
    };
    let (revision, target) = manifest.target();
    log!(
        "db: the list has {} patch(es), the release's resources_skhid.db is revision {revision} ({} bytes)",
        manifest.patches.len(),
        target.size
    );
    let result = (|| {
        if step.problem.is_some() {
            return Err(Fail::Other);
        }
        let todo = todo(&mut step, &manifest)?;
        if legacy && todo != Todo::Build {
            step.verify_original(manifest.original)?;
            step.finish_legacy();
            legacy = false;
        }
        match todo {
            Todo::Done => Ok(()),
            Todo::From(from) => match step.update(&manifest, from)? {
                true => Ok(()),
                false => {
                    log!(
                        "db: the patched resources_skhid.db is not the release's: it is built anew"
                    );
                    step.build(&manifest, &mut legacy)
                }
            },
            Todo::Build | Todo::Hash => step.build(&manifest, &mut legacy),
        }
    })();
    match result {
        Ok(()) => {
            step.sweep(&manifest, revision);
            let _ = win::write_file(&status, b"ready\r\n");
            Outcome::Ready
        }
        Err(Fail::Stopped) => {
            log!("db: stopped; the rest next time");
            Outcome::Stopped
        }
        Err(Fail::Other) => {
            let (why, message) = match step.problem.clone() {
                Some(Problem::GameRunning) => {
                    ("the game is running", crate::text::db_game_running())
                }
                Some(Problem::DiskFull {
                    folder,
                    needed,
                    free,
                }) => (
                    "not enough disk space",
                    crate::text::db_disk_full(&folder, needed, free),
                ),
                Some(Problem::Offline) => (
                    "a blob is not here and the server could not be asked",
                    crate::text::db_offline(),
                ),
                Some(Problem::Original(why)) => (
                    "resources.db is not the original",
                    crate::text::db_original(&why),
                ),
                None => ("see above", crate::text::db_failed()),
            };
            blocked(why, message)
        }
    }
}
