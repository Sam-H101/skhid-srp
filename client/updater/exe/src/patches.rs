//! The patch step: after the overlay and the resource step, at every run,
//! the game folder's `resources.db.patch_<n>` files are made the server's
//! list (`GET <site>/patches`, `updater_logic::patches`): a listed patch
//! that is missing or damaged is downloaded from the address the list gives
//! it (plain HTTP or HTTPS, any host: the list's SHA-256 decides),
//! a patch file the list has not is deleted, and every patch file is hashed.
//! `client/updater/README.md`, "Patches".
//!
//! Required: PLAY stays off ([`Outcome::Blocked`]) until every listed patch
//! is there with its checksum and no other patch file is; the next start
//! tries again. A download goes to `<file>.skhid-part`, is flushed, read
//! back and checked, and only then renamed to its name: a patch file is
//! never half written in place.
//!
//! Beside the updater: `updater_skhid.patches`, the server's last list, and
//! `updater_skhid.patches.status`, `ready` or `blocked <why>`. `ready`
//! (play_skhid.bat) and a start with the server down check the folder
//! against that list.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use updater_logic::patches::{
    FILE_PREFIX, List, ListError, Local, MAX_LIST, Patch, Problem, check, overlay_patch,
    parse_list, todo, write_list,
};
use updater_logic::pipe::{ToLauncher, state, step};
use updater_logic::sha256::to_hex;
use updater_logic::url::Url;
use windows_sys::Win32::Foundation::{
    ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION,
};

use crate::Session;
use crate::http::{self, HttpError};
use crate::install::Layout;
use crate::log::log;
use crate::text;
use crate::win::{self, Code, File};

/// Beside the updater: the server's last list, as it was read.
const LIST_FILE: &str = "updater_skhid.patches";
/// `ready` once the folder was checked against that list, else `blocked <why>`.
const STATUS_FILE: &str = "updater_skhid.patches.status";
/// A patch while it is downloaded, beside its place. It is a
/// `resources.db.patch_*` the game does not take, so one left over is
/// deleted as any other.
const PART: &str = ".skhid-part";
/// Room left on the disk after a patch is downloaded.
const DISK_MARGIN: u64 = 64 << 20;
/// The largest patch taken: a bound only, the disk decides first.
const MAX_PATCH: u64 = 1 << 40;
/// How often the list, and each patch, is asked for in one run.
const LIST_TRIES: u32 = 3;
const FETCH_TRIES: u32 = 2;
const RETRY_PAUSE_MS: u32 = 3000;

/// How the step ended.
pub enum Outcome {
    /// Every listed patch is in place: the game may be played.
    Ready,
    /// The launcher asked to stop.
    Stopped,
    /// PLAY stays off, with these words.
    Blocked(String),
}

fn hex(sha256: &[u8; 32]) -> String {
    String::from_utf8_lossy(&to_hex(sha256)).into()
}

fn place(layout: &Layout, file_name: &str) -> String {
    format!("{}\\{file_name}", layout.game_dir)
}

/// The patch files in the game folder, by their names.
fn files(layout: &Layout) -> Vec<String> {
    win::list(&layout.game_dir, &format!("{FILE_PREFIX}*"))
}

fn look(layout: &Layout, patch: &Patch) -> Local {
    let path = place(layout, &patch.file_name());
    match win::hash_file(&path) {
        Ok((_, sha256)) => Local::Found(sha256),
        Err(code) if win::is_missing(code) => Local::Missing,
        Err(code) => {
            log!("patches: {path} cannot be read ({})", win::error_text(code));
            Local::Unreadable
        }
    }
}

fn problem(problem: &Problem) -> String {
    match problem {
        Problem::Extra(name) => format!("{name} is not in the list"),
        Problem::Missing(number) => format!("{FILE_PREFIX}{number} is missing"),
        Problem::Damaged(number) => {
            format!("{FILE_PREFIX}{number} does not have the list's checksum")
        }
    }
}

/// The folder checked against the list kept beside the updater, after a
/// run that ended `ready`: `Err` says what is not in place, for the log.
fn kept_in_place(layout: &Layout) -> Result<(), String> {
    let status = win::read_file(&format!("{}\\{STATUS_FILE}", layout.exe_dir), 4096)
        .map_err(|code| format!("{STATUS_FILE}: {}", win::error_text(code)))?;
    if status.trim_ascii() != b"ready" {
        return Err(format!(
            "the last run did not finish: {}",
            String::from_utf8_lossy(status.trim_ascii())
        ));
    }
    let bytes = win::read_file(&format!("{}\\{LIST_FILE}", layout.exe_dir), MAX_LIST)
        .map_err(|code| format!("{LIST_FILE}: {}", win::error_text(code)))?;
    let list = parse_list(&bytes).map_err(|error| format!("{LIST_FILE}: {error:?}"))?;
    check(&list, &files(layout), &mut |patch| look(layout, patch)).map_err(|p| problem(&p))
}

/// `updater_skhid.exe ready`, from play_skhid.bat and play_skhid.sh: reads
/// only. `Err` (the words for the player) unless the last run ended with
/// every patch in place and the folder still is so.
pub fn ready(layout: &Layout) -> Result<(), String> {
    match kept_in_place(layout) {
        Ok(()) => {
            log!("ready: the patches are in place");
            Ok(())
        }
        Err(why) => {
            log!("ready: NOT READY: patches: {why}: the game is not started");
            Err(text::patches_not_ready())
        }
    }
}

/// Why the step stopped short.
enum Fail {
    Stopped,
    /// PLAY stays off: what the player is told, what the log and the status
    /// file say.
    Blocked(String, String),
}

/// The words for a file that cannot be deleted or replaced.
fn file_fail(path: &str, code: Code, what: &str) -> Fail {
    let why = format!("{path} cannot be {what} ({})", win::error_text(code));
    if code == ERROR_SHARING_VIOLATION || code == ERROR_LOCK_VIOLATION {
        Fail::Blocked(text::patches_game_running(), why)
    } else {
        Fail::Blocked(text::patches_failed(), why)
    }
}

struct Run<'a> {
    session: &'a mut Session,
    layout: &'a Layout,
    server: &'a Url,
}

impl Run<'_> {
    /// The server's list, asked for a few times.
    fn list(&mut self) -> Result<(List, Vec<u8>), Fail> {
        let mut last = Fail::Stopped;
        for attempt in 1..=LIST_TRIES {
            let error = match http::get(self.server, "/patches", MAX_LIST, &mut |_, _| true) {
                Ok(body) => {
                    return match parse_list(&body) {
                        Ok(list) => Ok((list, body)),
                        Err(error) => Err(Fail::Blocked(
                            text::patches_bad_list(self.server, &list_error(&error)),
                            format!("the list is not one ({error:?})"),
                        )),
                    };
                }
                Err(HttpError::Status(404)) => {
                    return Err(Fail::Blocked(
                        text::patches_not_offered(self.server),
                        "the server has no /patches".into(),
                    ));
                }
                Err(HttpError::TooLarge) => {
                    return Err(Fail::Blocked(
                        text::patches_bad_list(self.server, "too large"),
                        "the list is too large".into(),
                    ));
                }
                Err(error) => error,
            };
            let message = text::http(self.server, "/patches", &error);
            log!("patches: the list, try {attempt} of {LIST_TRIES}: {message}");
            last = Fail::Blocked(
                text::patches_unreachable(&message),
                format!("the list could not be fetched: {message}"),
            );
            if attempt < LIST_TRIES && !self.session.pause(RETRY_PAUSE_MS) {
                return Err(Fail::Stopped);
            }
        }
        Err(last)
    }

    fn delete(&self, name: &str) -> Result<(), Fail> {
        let path = place(self.layout, name);
        match win::delete(&path) {
            Ok(()) => {
                log!("patches: deleted {name}");
                Ok(())
            }
            Err(code) => Err(file_fail(&path, code, "deleted")),
        }
    }

    /// Downloads one patch into its place.
    fn fetch(&mut self, patch: &Patch) -> Result<(), Fail> {
        let name = patch.file_name();
        let target = place(self.layout, &name);
        let part = format!("{target}{PART}");
        let mut last = Fail::Stopped;
        for attempt in 1..=FETCH_TRIES {
            self.session.state(state::DOWNLOADING);
            self.session.begin_progress();
            let result = self.download(&part, patch);
            match result {
                Ok(()) => {}
                Err(Fail::Stopped) => {
                    let _ = win::delete(&part);
                    return Err(Fail::Stopped);
                }
                Err(fail) => {
                    let _ = win::delete(&part);
                    if let Fail::Blocked(_, why) = &fail {
                        log!("patches: {name}: try {attempt} of {FETCH_TRIES}: {why}");
                    }
                    let disk = matches!(&fail, Fail::Blocked(_, why) if why.starts_with("disk:"));
                    last = fail;
                    if disk {
                        break;
                    }
                    if attempt < FETCH_TRIES && !self.session.pause(RETRY_PAUSE_MS) {
                        return Err(Fail::Stopped);
                    }
                    continue;
                }
            }
            // Checked: into its place.
            if let Err(code) = win::rename(&part, &target, true) {
                let _ = win::delete(&part);
                return Err(file_fail(&target, code, "replaced"));
            }
            self.session.state(state::INSTALLING);
            self.session.send(ToLauncher::InstallInfo {
                step: step::EXTRACT,
                text: &name,
            });
            self.session.named_file = true;
            log!(
                "patches: {name}: downloaded {}, checksum right ({})",
                patch.address,
                hex(&patch.sha256)
            );
            return Ok(());
        }
        Err(last)
    }

    /// One try: the file at the patch's address into `part`, flushed, read
    /// back and checked.
    fn download(&mut self, part: &str, patch: &Patch) -> Result<(), Fail> {
        let output = File::create(part).map_err(|code| file_fail(part, code, "written"))?;
        let folder = self.layout.game_dir.clone();
        let mut no_room: Option<(u64, u64)> = None;
        let mut asked = false;
        let session = &mut *self.session;
        let result = http::get_stream(
            &patch.url,
            "",
            MAX_PATCH,
            &mut |chunk| {
                output
                    .write(chunk)
                    .map_err(|code| HttpError::Network("write the patch", code))
            },
            &mut |done, total| {
                // Before the first byte: is there room for what the server
                // says it sends?
                if !asked {
                    asked = true;
                    let needed = total.unwrap_or(0).saturating_add(DISK_MARGIN);
                    if let Some(free) = win::free_space(&folder)
                        && free < needed
                    {
                        no_room = Some((needed, free));
                        return false;
                    }
                }
                session.progress(done, total.unwrap_or(0))
            },
        );
        if let Some((needed, free)) = no_room {
            drop(output);
            return Err(Fail::Blocked(
                text::resources_disk_full(&folder, needed, free),
                format!("disk: {free} bytes free in {folder}, {needed} needed"),
            ));
        }
        match result {
            Ok(()) => {}
            Err(HttpError::Stopped) => return Err(Fail::Stopped),
            Err(HttpError::Network(_, code))
                if code == ERROR_DISK_FULL || code == ERROR_HANDLE_DISK_FULL =>
            {
                let free = win::free_space(&folder).unwrap_or(0);
                return Err(Fail::Blocked(
                    text::resources_disk_full(&folder, DISK_MARGIN, free),
                    format!("disk: the disk is full ({free} bytes free in {folder})"),
                ));
            }
            Err(error) => {
                let message = text::http(&patch.url, &patch.url.prefix, &error);
                return Err(Fail::Blocked(
                    text::patches_download(&patch.file_name(), &message),
                    message,
                ));
            }
        }
        output
            .flush()
            .map_err(|code| file_fail(part, code, "written"))?;
        drop(output);
        match win::hash_file(part) {
            Ok((_, sha256)) if sha256 == patch.sha256 => Ok(()),
            Ok((size, sha256)) => Err(Fail::Blocked(
                text::patches_wrong(&patch.url, &patch.file_name()),
                format!(
                    "{} ({size} bytes) has SHA-256 {}, the list says {}",
                    patch.address,
                    hex(&sha256),
                    hex(&patch.sha256)
                ),
            )),
            Err(code) => Err(file_fail(part, code, "read back")),
        }
    }
}

fn list_error(error: &ListError) -> String {
    match error {
        ListError::TooLarge => "too large".into(),
        ListError::Syntax(at) => format!("not JSON at byte {at}"),
        ListError::BadName(name) => format!("bad name {}", name.escape_debug()),
        ListError::BadEntry(name) => format!("bad entry {}", name.escape_debug()),
        ListError::BadHash(name) => format!("bad checksum of {}", name.escape_debug()),
        ListError::BadUrl(name) => format!("bad address of {}", name.escape_debug()),
        ListError::Duplicate(name) => format!("{} twice", name.escape_debug()),
    }
}

/// Writes `data` beside, then renames it over `path`.
fn save(path: &str, data: &[u8]) -> Result<(), Code> {
    if win::read_file(path, data.len() + 1).ok().as_deref() == Some(data) {
        return Ok(());
    }
    let staged = format!("{path}.skhid-new");
    let result = win::write_file(&staged, data).and_then(|()| win::rename(&staged, path, true));
    if result.is_err() {
        let _ = win::delete(&staged);
    }
    result
}

/// The server could not be asked (`reason`): the game may be played when the
/// folder is as the list kept says, after a run that ended `ready`.
fn offline(layout: &Layout, reason: &str) -> Outcome {
    match kept_in_place(layout) {
        Ok(()) => {
            log!("patches: the server could not be asked; the patches kept are in place");
            Outcome::Ready
        }
        Err(why) => {
            let message = text::patches_offline(reason);
            log!("patches: BLOCKED: {why}. The player is told: {message}");
            Outcome::Blocked(message)
        }
    }
}

/// The patch step. `server`: `Err(why)` when the server could not be asked
/// this run: the folder is then checked against the list kept from the last
/// run.
/// `overlay`: the files the overlay installed (the ZIP's names).
pub fn run(
    session: &mut Session,
    layout: &Layout,
    server: Result<&Url, &str>,
    overlay: &[&str],
) -> Outcome {
    let status_path = format!("{}\\{STATUS_FILE}", layout.exe_dir);
    let server = match server {
        Ok(server) => server,
        Err(reason) => return offline(layout, reason),
    };
    let outcome = step(session, layout, server, overlay);
    let status = match &outcome {
        Err(Fail::Stopped) => {
            log!("patches: stopped; the rest next time");
            let _ = win::write_file(&status_path, b"blocked stopped\r\n");
            return Outcome::Stopped;
        }
        Err(Fail::Blocked(message, why)) => {
            log!("patches: BLOCKED: {why}. The player is told: {message}");
            format!("blocked {why}\r\n")
        }
        Ok(()) => "ready\r\n".into(),
    };
    if let Err(code) = save(&status_path, status.as_bytes()) {
        log!(
            "patches: {STATUS_FILE} cannot be written ({})",
            win::error_text(code)
        );
    }
    match outcome {
        Ok(()) => Outcome::Ready,
        Err(Fail::Blocked(message, _)) => Outcome::Blocked(message),
        Err(Fail::Stopped) => Outcome::Stopped,
    }
}

fn step(
    session: &mut Session,
    layout: &Layout,
    server: &Url,
    overlay: &[&str],
) -> Result<(), Fail> {
    if let Some(name) = overlay_patch(overlay) {
        return Err(Fail::Blocked(
            text::patches_failed(),
            format!("the overlay has {name}: patches come from the list alone"),
        ));
    }
    let mut run = Run {
        session,
        layout,
        server,
    };
    let (list, _) = run.list()?;
    // The list is kept before the folder is changed, and the status says
    // `blocked` until the run is through: `ready` checks against the newest.
    let list_path = format!("{}\\{LIST_FILE}", layout.exe_dir);
    let status_path = format!("{}\\{STATUS_FILE}", layout.exe_dir);
    let _ = save(&status_path, b"blocked checking\r\n");
    if let Err(code) = save(&list_path, &write_list(&list)) {
        return Err(Fail::Blocked(
            text::patches_failed(),
            format!("{LIST_FILE} cannot be written ({})", win::error_text(code)),
        ));
    }
    let found = files(layout);
    let todo = todo(&list, &found, &mut |patch| look(layout, patch));
    log!(
        "patches: the list has {} patch(es){}; {} to download, {} to delete",
        list.patches.len(),
        if list.patches.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                list.patches
                    .iter()
                    .map(Patch::name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        todo.fetch.len(),
        todo.delete.len()
    );
    for name in &todo.delete {
        run.delete(name)?;
    }
    for fetch in &todo.fetch {
        if !run.session.pump() {
            return Err(Fail::Stopped);
        }
        if fetch.damaged {
            log!(
                "patches: {} does not have the list's checksum: downloaded again",
                fetch.patch.file_name()
            );
            run.delete(&fetch.patch.file_name())?;
        }
        run.fetch(&fetch.patch)?;
    }
    // Every patch file hashed once more, as `ready` does.
    check(&list, &files(layout), &mut |patch| look(layout, patch)).map_err(|problem| {
        Fail::Blocked(
            text::patches_failed(),
            format!("after the step: {}", self::problem(&problem)),
        )
    })?;
    log!(
        "patches: {} patch(es) in place, checked",
        list.patches.len()
    );
    Ok(())
}
