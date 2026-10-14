//! The resource step: after the overlay, at every run, the bytes of the
//! game's own files that the plan names (`updater_skhid.resources`, a file
//! of the players' ZIP; `updater_logic::resources`) are made what it says,
//! and those an older plan changed and this one no longer names are given
//! their original bytes back. `client/updater/README.md`, "Resources".
//!
//! Best effort: whatever fails here is logged and tried again at the next
//! run. It never fails the update, never keeps PLAY off, and never touches
//! the overlay's files or its state.
//!
//! For one region, in order:
//!
//! 1. Its bytes are hashed (the file may be open in the game meanwhile).
//!    The target: done. Else:
//! 2. The target's blob is fetched from `<site>/resources/<sha256>` into
//!    `updater_skhid.cache\` (streamed to the disk, its size and SHA-256
//!    checked; one that is there and checks is used again).
//! 3. The file is opened by this program alone (share mode 0). A game that
//!    runs holds it: no region is changed this run.
//! 4. A region that holds the original, with no checked copy of it kept,
//!    is copied into `updater_skhid.backup\<original sha256>` first:
//!    written, flushed, read back and checked.
//! 5. It is recorded (`updater_skhid.resources.state`) and journalled
//!    (`updater_skhid.resources.journal`), each flushed to the disk, before
//!    its first byte is written: a region is never changed unless there is
//!    a note of how to put it back.
//! 6. The blob is written at the region's offset, flushed, and the region
//!    read back and checked. The journal goes.
//! 7. If the write or the check fails, the original is written back (from
//!    the copy kept, else from the server's blob of it) and checked.
//!
//! A region of the record the plan no longer has gets its original back the
//! same way, before the plan's own regions are written.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use updater_logic::manifest::Installed;
use updater_logic::pipe::{ToLauncher, state, step};
use updater_logic::resources::{
    FileLook, FileUndo, Look, PLAN_NAME, Plan, PlanError, Region, Todo, Undo, WholeFile, blocked,
    file_done, file_undo, files_leaving, leaving, names_overlay_file, parse_plan, parse_record,
    todo, undo, write_record,
};
use updater_logic::sha256::{Sha256, to_hex};
use updater_logic::url::Url;
use windows_sys::Win32::Foundation::{ERROR_HANDLE_EOF, ERROR_SHARING_VIOLATION};

use crate::Session;
use crate::http::{self, HttpError};
use crate::install::Layout;
use crate::log::log;
use crate::win::{self, Code, File};

/// Beside the updater.
const PLAN_FILE: &str = "updater_skhid.resources";
const RECORD_FILE: &str = "updater_skhid.resources.state";
const JOURNAL_FILE: &str = "updater_skhid.resources.journal";
const CACHE_DIR: &str = "updater_skhid.cache";
const BACKUP_DIR: &str = "updater_skhid.backup";
/// A blob or a copy while it is written, before it is checked.
const PART: &str = ".part";
/// How often a blob is asked for in one run.
const FETCH_TRIES: u32 = 2;
const RETRY_PAUSE_MS: u32 = 3000;
/// Bytes read or written at a time.
const PIECE: usize = 64 * 1024;

fn hex(sha256: &[u8; 32]) -> String {
    String::from_utf8_lossy(&to_hex(sha256)).into()
}

/// The SHA-256 of `size` bytes of `file` from `offset`.
fn hash_range(file: &File, offset: u64, size: u64) -> Result<[u8; 32], Code> {
    file.seek(offset)?;
    let mut hash = Sha256::new();
    let mut buffer = alloc::vec![0_u8; PIECE];
    let mut left = size;
    while left > 0 {
        let want = (left.min(PIECE as u64)) as usize;
        let read = file.read(&mut buffer[..want])?;
        if read == 0 {
            return Err(ERROR_HANDLE_EOF);
        }
        hash.update(&buffer[..read]);
        left -= read as u64;
    }
    Ok(hash.finish())
}

/// Copies `size` bytes of `from` (from `from_at`) to `to` (at `to_at`), and
/// returns the SHA-256 of what was read. `progress(done)` after each piece.
fn copy(
    from: &File,
    from_at: u64,
    to: &File,
    to_at: u64,
    size: u64,
    progress: &mut dyn FnMut(u64),
) -> Result<[u8; 32], Code> {
    from.seek(from_at)?;
    to.seek(to_at)?;
    let mut hash = Sha256::new();
    let mut buffer = alloc::vec![0_u8; PIECE];
    let mut done = 0_u64;
    while done < size {
        let want = ((size - done).min(PIECE as u64)) as usize;
        let read = from.read(&mut buffer[..want])?;
        if read == 0 {
            return Err(ERROR_HANDLE_EOF);
        }
        hash.update(&buffer[..read]);
        to.write(&buffer[..read])?;
        done += read as u64;
        progress(done);
    }
    Ok(hash.finish())
}

/// A file of `size` bytes with this SHA-256, held so that no one changes it
/// until it is dropped.
fn checked(path: &str, size: u64, sha256: &[u8; 32]) -> Option<File> {
    let file = File::locked(path).ok()?;
    (file.size().ok()? == size && hash_range(&file, 0, size).ok()? == *sha256).then_some(file)
}

/// What the bytes of `region` are now. The file may be open in the game.
fn look(place: &str, region: &Region) -> Look {
    let file = match File::shared(place) {
        Ok(file) => file,
        Err(code) if win::is_missing(code) => return Look::Missing,
        Err(code) => {
            log!(
                "resources: {place} cannot be read ({})",
                win::error_text(code)
            );
            return Look::Unreadable;
        }
    };
    match file.size() {
        Ok(size) if size != region.file_size => return Look::OtherSize(size),
        Ok(_) => {}
        Err(_) => return Look::Unreadable,
    }
    let started = win::ticks();
    match hash_range(&file, region.offset, region.size) {
        Ok(hash) => {
            log!(
                "resources: {} hashed in {} ms",
                region.name,
                win::ticks().saturating_sub(started)
            );
            Look::Bytes(hash)
        }
        Err(code) => {
            log!(
                "resources: {place} cannot be read ({})",
                win::error_text(code)
            );
            Look::Unreadable
        }
    }
}

fn file_look(place: &str) -> FileLook {
    match win::hash_file(place) {
        Ok((size, hash)) => FileLook::Found(size, hash),
        Err(code) if win::is_missing(code) => FileLook::Missing,
        Err(_) => FileLook::Unreadable,
    }
}

/// Why a region was not written.
enum Fail {
    /// The launcher asked to stop.
    Stopped,
    /// Another program holds the file.
    GameRunning,
    /// Said in the log already.
    Other,
}

/// Where the launcher's window is in this step.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Phase {
    Quiet,
    Downloading,
    Installing,
}

struct Step<'a> {
    session: &'a mut Session,
    layout: &'a Layout,
    /// Where blobs are fetched; `None` when the server could not be asked
    /// this run (only what is kept here is used then).
    server: Option<&'a Url>,
    record: Plan,
    /// The record as it is on the disk.
    saved: Plan,
    phase: Phase,
    /// A file is held by another program: no region is touched any more.
    game_running: bool,
    stopped: bool,
}

impl Step<'_> {
    fn beside(&self, name: &str) -> String {
        format!("{}\\{name}", self.layout.exe_dir)
    }

    fn cached(&self, sha256: &[u8; 32]) -> String {
        format!("{}\\{CACHE_DIR}\\{}", self.layout.exe_dir, hex(sha256))
    }

    fn backup(&self, region: &Region) -> String {
        format!(
            "{}\\{BACKUP_DIR}\\{}",
            self.layout.exe_dir,
            hex(&region.original)
        )
    }

    /// Tells the launcher what is done now, when that changes.
    fn enter(&mut self, phase: Phase) {
        if self.phase != phase {
            self.phase = phase;
            self.session.state(match phase {
                Phase::Downloading => state::DOWNLOADING,
                _ => state::INSTALLING,
            });
        }
    }

    /// The launcher's line: "Extracting file <name>".
    fn installing(&mut self, name: &str) {
        self.enter(Phase::Installing);
        let short = name.rsplit('/').next().unwrap_or(name);
        self.session.send(ToLauncher::InstallInfo {
            step: step::EXTRACT,
            text: short,
        });
        self.session.named_file = true;
        self.session.begin_progress();
    }

    /// Writes the record, as the overlay's state is written: beside, then
    /// renamed over. No record when there is nothing in it.
    fn save(&mut self) -> bool {
        if self.record == self.saved {
            return true;
        }
        let path = self.beside(RECORD_FILE);
        let result = if self.record.is_empty() {
            win::delete(&path)
        } else {
            let staged = format!("{path}.skhid-new");
            let result = win::write_file(&staged, &write_record(&self.record))
                .and_then(|()| win::rename(&staged, &path, true));
            if result.is_err() {
                let _ = win::delete(&staged);
            }
            result
        };
        match result {
            Ok(()) => {
                self.saved = self.record.clone();
                true
            }
            Err(code) => {
                log!(
                    "resources: {RECORD_FILE} cannot be written ({})",
                    win::error_text(code)
                );
                false
            }
        }
    }

    /// Notes that `region` is being written, on the disk, before the write.
    fn journal(&self, region: &Region) -> bool {
        let text = write_record(&Plan {
            regions: alloc::vec![region.clone()],
            files: Vec::new(),
        });
        match win::write_file(&self.beside(JOURNAL_FILE), &text) {
            Ok(()) => true,
            Err(code) => {
                log!(
                    "resources: {JOURNAL_FILE} cannot be written ({})",
                    win::error_text(code)
                );
                false
            }
        }
    }

    fn end_journal(&self) {
        let _ = win::delete(&self.beside(JOURNAL_FILE));
    }

    /// The blob with this SHA-256 and size, in the cache: the one there if
    /// it checks, else fetched from the server.
    fn fetch(&mut self, sha256: &[u8; 32], size: u64, what: &str) -> Result<String, Fail> {
        let path = self.cached(sha256);
        if checked(&path, size, sha256).is_some() {
            log!("resources: {what}: using the blob kept in {CACHE_DIR}");
            return Ok(path);
        }
        let Some(server) = self.server else {
            log!(
                "resources: {what}: its blob is not here and the server could not be asked: next time"
            );
            return Err(Fail::Other);
        };
        let folder = self.beside(CACHE_DIR);
        if let Err(code) = win::create_folders(&folder) {
            log!(
                "resources: {folder} cannot be made ({})",
                win::error_text(code)
            );
            return Err(Fail::Other);
        }
        let name = hex(sha256);
        let address = format!("/resources/{name}");
        let part = format!("{path}{PART}");
        for attempt in 1..=FETCH_TRIES {
            self.enter(Phase::Downloading);
            self.session.begin_progress();
            let output = match File::create(&part) {
                Ok(output) => output,
                Err(code) => {
                    log!(
                        "resources: {part} cannot be written ({})",
                        win::error_text(code)
                    );
                    return Err(Fail::Other);
                }
            };
            let session = &mut *self.session;
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
                if checked(&part, size, sha256).is_some() {
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
                    log!("resources: {what}: fetched {address} ({size} bytes, checksum right)");
                    return Ok(path);
                }
                Err(message) => {
                    let _ = win::delete(&part);
                    if self.session.stopped {
                        self.stopped = true;
                        return Err(Fail::Stopped);
                    }
                    log!("resources: {what}: fetch {attempt} of {FETCH_TRIES} failed: {message}");
                    if attempt < FETCH_TRIES && !self.session.pause(RETRY_PAUSE_MS) {
                        self.stopped = true;
                        return Err(Fail::Stopped);
                    }
                }
            }
        }
        Err(Fail::Other)
    }

    /// Opens a file of a region by this program alone.
    fn open(&mut self, place: &str) -> Result<File, Fail> {
        match File::exclusive(place) {
            Ok(file) => Ok(file),
            // The game opens resources.db at its start (share mode read and
            // write) and keeps it open until it exits.
            Err(ERROR_SHARING_VIOLATION) => {
                log!(
                    "resources: the game is running ({place} is open in another program): \
                     no region is changed now, next time"
                );
                self.game_running = true;
                Err(Fail::GameRunning)
            }
            Err(code) => {
                log!(
                    "resources: {place} cannot be opened to write ({})",
                    win::error_text(code)
                );
                Err(Fail::Other)
            }
        }
    }

    /// A checked copy of the region's original is kept.
    fn backup_kept(&self, region: &Region) -> bool {
        checked(&self.backup(region), region.size, &region.original).is_some()
    }

    /// Copies the region (the original) out of `file` into the backup
    /// folder: written, flushed, read back and checked.
    fn make_backup(&self, file: &File, region: &Region) -> bool {
        let folder = self.beside(BACKUP_DIR);
        let path = self.backup(region);
        let part = format!("{path}{PART}");
        let result = win::create_folders(&folder)
            .map_err(|code| win::error_text(code))
            .and_then(|()| File::create(&part).map_err(|code| win::error_text(code)))
            .and_then(|copy_file| {
                let copied = copy(file, region.offset, &copy_file, 0, region.size, &mut |_| {})
                    .map_err(|code| win::error_text(code))?;
                if copied != region.original {
                    return Err("the region changed while it was copied".into());
                }
                copy_file.flush().map_err(|code| win::error_text(code))?;
                match hash_range(&copy_file, 0, region.size) {
                    Ok(hash) if hash == region.original => Ok(()),
                    Ok(_) => Err("the copy read back with other bytes".into()),
                    Err(code) => Err(win::error_text(code)),
                }
            })
            .and_then(|()| win::rename(&part, &path, true).map_err(|code| win::error_text(code)));
        match result {
            Ok(()) => {
                log!(
                    "resources: {}: its original is kept in {BACKUP_DIR}",
                    region.name
                );
                true
            }
            Err(message) => {
                let _ = win::delete(&part);
                log!(
                    "resources: {}: no copy of its original could be kept ({message}): not written",
                    region.name
                );
                false
            }
        }
    }

    /// Writes the blob at `path` (`sha256`) into the region, flushes, and
    /// reads it back. The blob is checked again as it is held.
    fn write(
        &mut self,
        file: &File,
        region: &Region,
        path: &str,
        sha256: &[u8; 32],
    ) -> Result<(), String> {
        let Some(blob) = checked(path, region.size, sha256) else {
            return Err(format!("{path} is not the blob it was"));
        };
        let size = region.size;
        let session = &mut *self.session;
        let copied = copy(&blob, 0, file, region.offset, size, &mut |done| {
            // The launcher may say stop: a region is finished all the same.
            let _ = session.progress(done, size);
        })
        .map_err(|code| format!("write: {}", win::error_text(code)))?;
        if copied != *sha256 {
            return Err("the blob changed while it was written".into());
        }
        file.flush()
            .map_err(|code| format!("flush: {}", win::error_text(code)))?;
        match hash_range(file, region.offset, size) {
            Ok(hash) if hash == *sha256 => Ok(()),
            Ok(_) => Err("the region read back with other bytes".into()),
            Err(code) => Err(format!("read back: {}", win::error_text(code))),
        }
    }

    /// Puts the region's original back: from the copy kept, else from the
    /// server's blob of it. `true` when the region is the original again.
    fn put_back(&mut self, file: &File, region: &Region) -> bool {
        let kept = self.backup(region);
        let (source, from_server) = if checked(&kept, region.size, &region.original).is_some() {
            (kept, false)
        } else {
            match self.fetch(&region.original, region.size, &region.name) {
                Ok(path) => (path, true),
                Err(_) => {
                    log!(
                        "resources: {}: no original to put back now: next time",
                        region.name
                    );
                    return false;
                }
            }
        };
        let name = region.name.clone();
        self.installing(&name);
        let result = self.write(file, region, &source, &region.original);
        if from_server {
            let _ = win::delete(&source);
        }
        match result {
            Ok(()) => {
                log!(
                    "resources: {}: its original is back ({})",
                    region.name,
                    if from_server {
                        "from the server"
                    } else {
                        "from the copy kept"
                    }
                );
                self.end_journal();
                true
            }
            Err(message) => {
                log!(
                    "resources: {}: its original could not be put back ({message}): next time",
                    region.name
                );
                false
            }
        }
    }

    /// A region of the record the plan no longer has: its original back.
    fn remove(&mut self, region: &Region) {
        let place = self.layout.place(&region.path);
        match undo(region, look(&place, region)) {
            Undo::Forget => {
                log!(
                    "resources: {} is no longer planned and holds no change",
                    region.name
                );
                self.record.forget_region(region);
            }
            Undo::Wait => {}
            Undo::Restore => {
                // As for a write: nothing is fetched for a file the game
                // holds, and the file is not held through a download.
                match self.open(&place) {
                    Ok(probe) => drop(probe),
                    Err(_) => return,
                }
                if !self.backup_kept(region)
                    && self
                        .fetch(&region.original, region.size, &region.name)
                        .is_err()
                {
                    log!(
                        "resources: {}: no original to put back now: next time",
                        region.name
                    );
                    return;
                }
                let Ok(file) = self.open(&place) else {
                    return;
                };
                if !self.journal(region) {
                    return;
                }
                log!(
                    "resources: {} is no longer planned: its original is put back",
                    region.name
                );
                if self.put_back(&file, region) {
                    self.record.forget_region(region);
                }
            }
        }
        self.save();
    }

    /// A region of the plan: its target.
    fn apply(&mut self, region: &Region, cut_short: bool) {
        let place = self.layout.place(&region.path);
        let found = look(&place, region);
        let kept = matches!(found, Look::Bytes(hash) if hash == region.original)
            && self.backup_kept(region);
        match todo(region, found, kept) {
            Todo::Skip => match found {
                Look::Missing => log!("resources: {}: {place} is not there: skipped", region.name),
                Look::OtherSize(size) => log!(
                    "resources: {}: {place} has {size} bytes, not {} (another build of the game): skipped",
                    region.name,
                    region.file_size
                ),
                _ => log!("resources: {}: skipped, next time", region.name),
            },
            Todo::Done => {
                if !self.record.regions.contains(region) {
                    if self.record.record_region(region) {
                        log!("resources: {} holds its target", region.name);
                    } else {
                        log!(
                            "resources: {} holds its target, but an older region over its bytes is still recorded",
                            region.name
                        );
                    }
                }
            }
            Todo::Write { backup, damaged } => {
                if blocked(region, &self.record) {
                    log!(
                        "resources: {} waits: an older region over its bytes is not put back yet",
                        region.name
                    );
                    return;
                }
                let earlier = self
                    .record
                    .regions
                    .iter()
                    .any(|other| other.same_place(region) && other.target != region.target);
                if earlier {
                    log!(
                        "resources: {}: the plan has a new target for it",
                        region.name
                    );
                } else if damaged {
                    log!(
                        "resources: {}: holds neither its original nor its target{}: the target is written over it",
                        region.name,
                        if cut_short {
                            " (a write of it was cut short)"
                        } else {
                            ""
                        }
                    );
                }
                // A game that runs holds the file: nothing is fetched for it
                // now. (It is not held through the download: a game started
                // meanwhile must be able to open it.)
                match self.open(&place) {
                    Ok(probe) => drop(probe),
                    Err(_) => return,
                }
                let Ok(blob) = self.fetch(&region.target, region.size, &region.name) else {
                    return;
                };
                let Ok(file) = self.open(&place) else {
                    return;
                };
                if backup && !self.make_backup(&file, region) {
                    return;
                }
                // Recorded and journalled before the first byte is written.
                if !self.record.record_region(region) || !self.save() || !self.journal(region) {
                    log!("resources: {}: not written", region.name);
                    return;
                }
                self.installing(&region.name);
                match self.write(&file, region, &blob, &region.target) {
                    Ok(()) => {
                        log!("resources: {}: written and checked", region.name);
                        self.end_journal();
                        let _ = win::delete(&blob);
                    }
                    Err(message) => {
                        log!(
                            "resources: {}: the write failed ({message}): the original is put back",
                            region.name
                        );
                        if self.put_back(&file, region) {
                            self.record.forget_region(region);
                        }
                    }
                }
            }
        }
        self.save();
    }

    /// A file of the record the plan no longer has.
    fn remove_file(&mut self, file: &WholeFile) {
        let place = self.layout.place(&file.path);
        match file_undo(file, file_look(&place)) {
            FileUndo::Delete => match win::delete(&place) {
                Ok(()) => {
                    log!("resources: removed {} (no longer planned)", file.path);
                    self.record.forget_file(&file.path);
                }
                Err(code) => log!(
                    "resources: {} cannot be removed ({}): next time",
                    file.path,
                    win::error_text(code)
                ),
            },
            FileUndo::Forget => {
                log!(
                    "resources: {} is no longer planned and is not the file placed: left as it is",
                    file.path
                );
                self.record.forget_file(&file.path);
            }
            FileUndo::Wait => {}
        }
        self.save();
    }

    /// A file of the plan: fetched by its checksum, renamed into its place.
    fn place_file(&mut self, file: &WholeFile) {
        let place = self.layout.place(&file.path);
        if file_done(file, file_look(&place)) {
            if !self.record.files.contains(file) {
                self.record.record_file(file);
                self.save();
            }
            return;
        }
        let Ok(blob) = self.fetch(&file.sha256, file.size, &file.path) else {
            return;
        };
        self.record.record_file(file);
        if !self.save() {
            log!("resources: {}: not placed", file.path);
            return;
        }
        self.installing(&file.path);
        let result = win::create_folders(win::split_path(&place).0)
            .and_then(|()| win::rename(&blob, &place, true));
        match result {
            Ok(()) => log!("resources: placed {}", file.path),
            Err(code) => log!(
                "resources: {} cannot be placed ({}): next time",
                file.path,
                win::error_text(code)
            ),
        }
    }

    /// Removes from the cache what no line of the plan needs any more, and
    /// from the backup folder the originals of regions neither planned nor
    /// recorded, and what a run that was cut short left half written.
    fn sweep(&self, plan: &Plan) {
        let wanted_blobs: Vec<String> = plan
            .regions
            .iter()
            .map(|region| hex(&region.target))
            .chain(plan.files.iter().map(|file| hex(&file.sha256)))
            .collect();
        let wanted_backups: Vec<String> = plan
            .regions
            .iter()
            .chain(self.record.regions.iter())
            .map(|region| hex(&region.original))
            .collect();
        for (folder, wanted) in [(CACHE_DIR, &wanted_blobs), (BACKUP_DIR, &wanted_backups)] {
            let folder = self.beside(folder);
            if !win::is_folder(&folder) {
                continue;
            }
            for name in win::list(&folder, "*") {
                if !wanted.iter().any(|keep| keep.eq_ignore_ascii_case(&name)) {
                    match win::delete(&format!("{folder}\\{name}")) {
                        Ok(()) => log!("resources: removed {name} from {folder}"),
                        Err(code) => log!(
                            "resources: {folder}\\{name} cannot be removed ({})",
                            win::error_text(code)
                        ),
                    }
                }
            }
            // Only when it is empty.
            win::remove_dir(&folder);
        }
    }
}

/// The resource step. `server`: `None` when the server could not be asked
/// this run. Reports to the launcher only when it changes something.
pub fn run(
    session: &mut Session,
    layout: &Layout,
    server: Option<&Url>,
    installed: Option<&Installed>,
) {
    let plan_path = format!("{}\\{PLAN_FILE}", layout.exe_dir);
    let overlay: Vec<&str> = installed
        .iter()
        .flat_map(|installed| installed.files.iter().map(|file| file.name.as_str()))
        .collect();
    let plan = match win::read_file(&plan_path, 1 << 20) {
        Ok(bytes) => match parse_plan(&bytes) {
            Ok(plan) => plan,
            Err(PlanError::NewerFormat) => {
                log!(
                    "resources: {PLAN_FILE} is of a newer format than this updater reads: nothing is changed"
                );
                return;
            }
            Err(error) => {
                log!("resources: {PLAN_FILE} cannot be used ({error:?}): nothing is changed");
                return;
            }
        },
        // The overlay has none: whatever an older plan changed goes back.
        Err(code)
            if win::is_missing(code)
                && !overlay
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(PLAN_NAME)) =>
        {
            Plan::default()
        }
        // The overlay has one and it is gone or unreadable: nothing is put
        // back until the overlay has brought it again.
        Err(code) => {
            log!(
                "resources: {PLAN_FILE} cannot be read ({}): nothing is changed",
                win::error_text(code)
            );
            return;
        }
    };
    if let Some(name) = names_overlay_file(&plan, &overlay) {
        log!("resources: {PLAN_FILE} names {name}, a file of the overlay: nothing is changed");
        return;
    }
    let record_path = format!("{}\\{RECORD_FILE}", layout.exe_dir);
    let record = match win::read_file(&record_path, 1 << 20) {
        Ok(bytes) => parse_record(&bytes).unwrap_or_else(|| {
            log!("resources: {RECORD_FILE} is damaged: started anew");
            Plan::default()
        }),
        Err(_) => Plan::default(),
    };
    let journal_path = format!("{}\\{JOURNAL_FILE}", layout.exe_dir);
    let journal = win::read_file(&journal_path, 1 << 20)
        .ok()
        .map(|bytes| parse_record(&bytes).unwrap_or_default());
    let nothing_kept = !win::is_folder(&format!("{}\\{CACHE_DIR}", layout.exe_dir))
        && !win::is_folder(&format!("{}\\{BACKUP_DIR}", layout.exe_dir));
    if plan.is_empty() && record.is_empty() && journal.is_none() && nothing_kept {
        return;
    }
    log!(
        "resources: the plan has {} region(s) and {} file(s); {} region(s) and {} file(s) recorded",
        plan.regions.len(),
        plan.files.len(),
        record.regions.len(),
        record.files.len()
    );
    let mut step = Step {
        session,
        layout,
        server,
        saved: record.clone(),
        record,
        phase: Phase::Quiet,
        game_running: false,
        stopped: false,
    };
    // A write that was cut short: its region is recorded (it was before the
    // write began), and what it holds is told by its checksum below.
    let mut cut_short: Vec<Region> = Vec::new();
    if let Some(journal) = journal {
        for region in &journal.regions {
            log!("resources: a write of {} was cut short", region.name);
            if !step.record.record_region(region) {
                log!(
                    "resources: {} is over the bytes of another recorded region",
                    region.name
                );
            }
            cut_short.push(region.clone());
        }
        if step.save() {
            step.end_journal();
        }
    }

    for region in leaving(&plan, &step.record) {
        if step.game_running || step.stopped || !step.session.pump() {
            break;
        }
        step.remove(&region);
    }
    for region in &plan.regions {
        if step.game_running || step.stopped || !step.session.pump() {
            break;
        }
        let cut = cut_short.iter().any(|other| other.same_place(region));
        step.apply(region, cut);
    }
    for file in files_leaving(&plan, &step.record) {
        if step.stopped {
            break;
        }
        step.remove_file(&file);
    }
    for file in &plan.files {
        if step.stopped || !step.session.pump() {
            break;
        }
        step.place_file(file);
    }
    step.save();
    if step.stopped {
        log!("resources: stopped; the rest next time");
        return;
    }
    step.sweep(&plan);
}
