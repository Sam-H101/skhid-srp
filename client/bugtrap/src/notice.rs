// A notice at the game's start when Windows does not let the game write to
// its folder in Documents (settings, log). The game itself says nothing:
// before client patch revision 4 it stopped two seconds after its start, and
// since then it runs on with settings that are not saved.
//
// It is asked from here because this DLL runs inside the game: Windows'
// Controlled folder access judges each program by itself, so only the game's
// own try says whether the game is let through.
use crate::{
    notice_text::{is_game, language, notice},
    text::Wide,
};
use core::{
    fmt::Write,
    ptr::{null, null_mut},
    sync::atomic::{AtomicBool, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::*,
    System::{
        Environment::GetCommandLineW, LibraryLoader::GetModuleFileNameW,
        Threading::GetCurrentProcessId,
    },
    UI::{Shell::*, WindowsAndMessaging::*},
};

static DONE: AtomicBool = AtomicBool::new(false);

unsafe fn length(text: *const u16, limit: usize) -> usize {
    let mut n = 0;
    while n < limit && *text.add(n) != 0 {
        n += 1;
    }
    n
}

/// 0 when a file can be made in `folder` (made first when it is missing, as
/// the game does), else the Windows error of the step that was refused.
unsafe fn refused(folder: &Wide<1024>) -> u32 {
    if CreateDirectoryW(folder.ptr(), null()) == 0 {
        let error = GetLastError();
        if error != ERROR_ALREADY_EXISTS {
            return error.max(1);
        }
    }
    let mut file = Wide::<1100>::new();
    let _ = file.wide(&folder.buf[..folder.len]);
    if write!(file, "\\skhid-write-test-{}.tmp", GetCurrentProcessId()).is_err() {
        return 0;
    }
    // A new file of our own name, gone when the handle closes.
    let handle = CreateFileW(
        file.ptr(),
        GENERIC_WRITE,
        0,
        null(),
        CREATE_NEW,
        FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE,
        null_mut(),
    );
    if handle == INVALID_HANDLE_VALUE {
        let error = GetLastError();
        return if error == ERROR_FILE_EXISTS {
            0
        } else {
            error.max(1)
        };
    }
    CloseHandle(handle);
    0
}

/// Once per process, from the game's own set-up call (never from DllMain:
/// a window cannot be shown under the loader's lock).
pub unsafe fn check_once() {
    if DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    let mut exe = Wide::<1024>::new();
    let n = GetModuleFileNameW(null_mut(), exe.buf.as_mut_ptr(), exe.buf.len() as u32) as usize;
    if n == 0 || n >= exe.buf.len() {
        return;
    }
    exe.len = n;
    if !is_game(&exe.buf[..n]) {
        return;
    }
    // The game's user folder (0.26g, 0x18BD0): SHGetFolderPath(CSIDL_PERSONAL),
    // a backslash and the application's name. Nothing moves it.
    let mut folder = Wide::<1024>::new();
    if SHGetFolderPathW(
        null_mut(),
        CSIDL_PERSONAL as i32,
        null_mut(),
        0,
        folder.buf.as_mut_ptr(),
    ) != 0
    {
        return;
    }
    folder.len = length(folder.buf.as_ptr(), MAX_PATH as usize);
    if folder.len == 0 || write!(folder, "\\Survarium").is_err() {
        return;
    }
    let error = refused(&folder);
    if error == 0 {
        return;
    }
    let line = GetCommandLineW();
    let line = core::slice::from_raw_parts(line, length(line, 32767));
    let text = notice(language(line));
    let mut message = Wide::<4096>::new();
    let _ = write!(message, "{}", text.before_folder);
    let _ = message.wide(&folder.buf[..folder.len]);
    let _ = write!(message, "{}", text.before_exe);
    let _ = message.wide(&exe.buf[..exe.len]);
    let _ = write!(
        message,
        "{}\r\n\r\n({} {})",
        text.after_exe, text.error, error
    );
    let mut title = Wide::<128>::new();
    let _ = write!(title, "{}", text.title);
    MessageBoxW(
        null_mut(),
        message.ptr(),
        title.ptr(),
        MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST,
    );
}
