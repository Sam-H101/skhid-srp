use crate::text::Text;
use core::{
    ffi::c_void,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};
use windows_sys::Win32::{Foundation::*, System::Diagnostics::Debug::*};

macro_rules! number {
    ($static:ident, $get:ident, $set:ident, $default:expr) => {
        pub static $static: AtomicU32 = AtomicU32::new($default);
        #[unsafe(no_mangle)]
        pub extern "system" fn $get() -> u32 {
            $static.load(Ordering::Relaxed)
        }
        #[unsafe(no_mangle)]
        pub extern "system" fn $set(value: u32) {
            $static.store(value, Ordering::Relaxed);
        }
    };
}
number!(ACTIVITY, BT_GetActivityType, BT_SetActivityType, 1);
number!(EXIT_MODE, BT_GetExitMode, BT_SetExitMode, 0);
number!(FLAGS, BT_GetFlags, BT_SetFlags, 1);
number!(DUMP_TYPE, BT_GetDumpType, BT_SetDumpType, 1);
number!(REPORT_FORMAT, BT_GetReportFormat, BT_SetReportFormat, 1);
macro_rules! text {
    ($static:ident, $get:ident, $set:ident) => {
        pub static $static: Text = Text::new();
        #[unsafe(no_mangle)]
        pub extern "system" fn $get() -> *const u8 {
            $static.get()
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn $set(value: *const u8) {
            $static.set(value);
        }
    };
}
text!(APP_NAME, BT_GetAppName, BT_SetAppName);
text!(APP_VERSION, BT_GetAppVersion, BT_SetAppVersion);
text!(USER_MESSAGE, BT_GetUserMessage, BT_SetUserMessage);
text!(REPORT_PATH, BT_GetReportFilePath, BT_SetReportFilePath);
static INTRO1: Text = Text::new();
static INTRO2: Text = Text::new();
#[unsafe(no_mangle)]
pub extern "system" fn BT_GetDialogMessage(kind: u32) -> *const u8 {
    match kind {
        1 => INTRO1.get(),
        2 => INTRO2.get(),
        _ => c"".as_ptr().cast(),
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_SetDialogMessage(kind: u32, value: *const u8) {
    match kind {
        1 => INTRO1.set(value),
        2 => INTRO2.set(value),
        _ => SetLastError(ERROR_INVALID_PARAMETER),
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_SetUserMessageFromCode(code: u32) {
    let mut message = [0u8; 2048];
    if FormatMessageA(
        FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
        core::ptr::null(),
        code,
        0,
        message.as_mut_ptr(),
        message.len() as u32,
        core::ptr::null(),
    ) != 0
    {
        USER_MESSAGE.set(message.as_ptr());
    }
}

macro_rules! disabled_text {
    ($get:ident, $set:ident) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn $get() -> *const u8 {
            c"".as_ptr().cast()
        }
        #[unsafe(no_mangle)]
        pub extern "system" fn $set(_: *const u8) {}
    };
}
disabled_text!(BT_GetSupportURL, BT_SetSupportURL);
disabled_text!(BT_GetSupportEMail, BT_SetSupportEMail);
disabled_text!(BT_GetSupportHost, BT_SetSupportHost);
disabled_text!(BT_GetNotificationEMail, BT_SetNotificationEMail);
#[unsafe(no_mangle)]
pub extern "system" fn BT_GetSupportPort() -> i16 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn BT_SetSupportPort(_: i16) {}
#[unsafe(no_mangle)]
pub extern "system" fn BT_SetSupportServer(_: *const u8, _: i16) {}
#[unsafe(no_mangle)]
pub extern "system" fn BT_GetMailProfile() -> *const u8 {
    c"".as_ptr().cast()
}
#[unsafe(no_mangle)]
pub extern "system" fn BT_SetMailProfile(_: *const u8, _: *const u8) {}

pub static PRE: AtomicUsize = AtomicUsize::new(0);
pub static PRE_PARAM: AtomicUsize = AtomicUsize::new(0);
pub static POST: AtomicUsize = AtomicUsize::new(0);
pub static POST_PARAM: AtomicUsize = AtomicUsize::new(0);
type Handler = Option<unsafe extern "system" fn(isize)>;
macro_rules! handler {
    ($get:ident, $set:ident, $func:ident, $param:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn $get() -> Handler {
            core::mem::transmute($func.load(Ordering::Acquire))
        }
        #[unsafe(no_mangle)]
        pub extern "system" fn $set(f: Handler, p: isize) {
            $param.store(p as usize, Ordering::Relaxed);
            $func.store(f.map_or(0, |f| f as usize), Ordering::Release);
        }
    };
}
handler!(BT_GetPreErrHandler, BT_SetPreErrHandler, PRE, PRE_PARAM);
handler!(BT_GetPostErrHandler, BT_SetPostErrHandler, POST, POST_PARAM);
pub unsafe fn callback(func: &AtomicUsize, param: &AtomicUsize) {
    if let Some(f) = core::mem::transmute::<usize, Handler>(func.load(Ordering::Acquire)) {
        f(param.load(Ordering::Relaxed) as isize);
    }
}

pub unsafe fn unsupported() -> i32 {
    SetLastError(ERROR_NOT_SUPPORTED);
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_ReadVersionInfo(_: HMODULE) -> i32 {
    unsupported()
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_ExportRegistryKey(_: *const u8, _: *const u8) -> i32 {
    SetLastError(ERROR_NOT_SUPPORTED);
    ERROR_NOT_SUPPORTED as i32
}

// Attached paths remain in the game's own log directory; no archive is created.
// Accept calls without opening/copying arbitrary files in the crash handler.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_AddLogFile(_: *const u8) {
    SetLastError(ERROR_NOT_SUPPORTED);
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_AddRegFile(_: *const u8, _: *const u8) {
    SetLastError(ERROR_NOT_SUPPORTED);
}
#[unsafe(no_mangle)]
pub extern "system" fn BT_DeleteLogFile(_: *const u8) {}
#[unsafe(no_mangle)]
pub extern "system" fn BT_ClearLogFiles() {}
#[unsafe(no_mangle)]
pub extern "system" fn BT_GetLogFilesCount() -> u32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn BT_GetLogFileEntry(
    _: isize,
    _: i32,
    _: *mut u32,
    _: *mut u32,
    _: *mut c_void,
) -> u32 {
    ERROR_NO_MORE_ITEMS
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_OpenLogFile(_: *const u8, _: u32) -> isize {
    unsupported();
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_GetLogFileName(_: isize) -> *const u8 {
    unsupported();
    core::ptr::null()
}
macro_rules! log_get { ($($name:ident),*) => { $(#[unsafe(no_mangle)] pub unsafe extern "system" fn $name(_: isize) -> u32 { unsupported(); 0 })* }; }
macro_rules! log_set { ($($name:ident),*) => { $(#[unsafe(no_mangle)] pub unsafe extern "system" fn $name(_: isize, _: u32) -> i32 { unsupported() })* }; }
log_get!(
    BT_GetLogSizeInBytes,
    BT_GetLogSizeInEntries,
    BT_GetLogFlags,
    BT_GetLogLevel,
    BT_GetLogEchoMode
);
log_set!(
    BT_SetLogSizeInBytes,
    BT_SetLogSizeInEntries,
    BT_SetLogFlags,
    BT_SetLogLevel,
    BT_SetLogEchoMode
);
macro_rules! log_op { ($($name:ident),*) => { $(#[unsafe(no_mangle)] pub unsafe extern "system" fn $name(_: isize) -> i32 { unsupported() })* }; }
log_op!(BT_CloseLogFile, BT_FlushLogFile, BT_ClearLog);
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_AppLogEntry(_: isize, _: u32, _: *const u8) -> i32 {
    unsupported()
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_InsLogEntry(_: isize, _: u32, _: *const u8) -> i32 {
    unsupported()
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_AppLogEntryV(
    _: isize,
    _: u32,
    _: *const u8,
    _: *const c_void,
) -> i32 {
    unsupported()
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_InsLogEntryV(
    _: isize,
    _: u32,
    _: *const u8,
    _: *const c_void,
) -> i32 {
    unsupported()
}
// The cdecl caller owns the variable arguments. We never read them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn BT_AppLogEntryF(_: isize, _: u32, _: *const u8) -> i32 {
    unsupported()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn BT_InsLogEntryF(_: isize, _: u32, _: *const u8) -> i32 {
    unsupported()
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_SaveSnapshot(_: *const u8) -> i32 {
    crate::dump::capture(core::ptr::null_mut(), false) as i32
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_SaveSnapshotEx(p: *mut EXCEPTION_POINTERS, _: *const u8) -> i32 {
    crate::dump::capture(p, false) as i32
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_SendSnapshot() -> i32 {
    unsupported()
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_SendSnapshotEx(_: *mut EXCEPTION_POINTERS) -> i32 {
    unsupported()
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_MailSnapshot() -> i32 {
    unsupported()
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BT_MailSnapshotEx(_: *mut EXCEPTION_POINTERS) -> i32 {
    unsupported()
}
