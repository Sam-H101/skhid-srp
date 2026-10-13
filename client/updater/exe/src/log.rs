//! The updater's log: `updater_skhid.log` beside it, one line per step, so a
//! player can send it when something went wrong. It holds no account name
//! and no password; it does hold the folder the game is in.

use alloc::string::String;
use core::cell::UnsafeCell;

use crate::win;

/// The log starts anew once it is longer than this.
const LIMIT: u64 = 256 * 1024;

/// The log file's path. Written once at start, before anything else runs;
/// the program has one thread.
struct Path(UnsafeCell<Option<String>>);
unsafe impl Sync for Path {}
static PATH: Path = Path(UnsafeCell::new(None));

pub fn start(path: String) {
    unsafe { *PATH.0.get() = Some(path) };
}

/// Adds a line with the time before it. A log that cannot be written is no
/// reason to stop: the line is dropped.
pub fn line(text: &str) {
    let Some(path) = (unsafe { &*PATH.0.get() }) else {
        return;
    };
    let line = alloc::format!("{} {}\r\n", win::timestamp(), text);
    let _ = win::append_file(path, line.as_bytes(), LIMIT);
}

macro_rules! log {
    ($($argument:tt)*) => {
        $crate::log::line(&alloc::format!($($argument)*))
    };
}
pub(crate) use log;
