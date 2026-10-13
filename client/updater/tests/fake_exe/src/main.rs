//! A stand-in for the game and for the launcher in the updater's tests.
//! It writes `<its own path>.started.txt` with its command line and its
//! working folder (UTF-16, a line each) and ends. It carries the mark the
//! launcher finds the game's version by.
#![no_std]
#![no_main]

use core::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL, WriteFile,
};
use windows_sys::Win32::System::Environment::{GetCommandLineW, GetCurrentDirectoryW};
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::System::Threading::ExitProcess;

/// What `get_local_version` of the launcher looks for in the game exe: the
/// 32 bytes of the mark, then the version.
#[used]
#[unsafe(no_mangle)]
static VERSION_MARK: [u8; 48] = *b"survarium_version_header_mark!!\x000.26g0-fake\0\0\0\0\0";

// The heap, the panic handler and the compiler's helpers: the updater's own.
#[path = "../../../exe/src/rt.rs"]
mod rt;

pub fn fatal_panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { ExitProcess(3) }
}

unsafe fn length(text: *const u16) -> usize {
    let mut length = 0;
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    length
}

unsafe fn write(file: *mut core::ffi::c_void, text: *const u16, units: usize) {
    let mut written = 0;
    unsafe {
        WriteFile(
            file,
            text.cast(),
            (units * 2) as u32,
            &mut written,
            null_mut(),
        )
    };
}

/// # Safety
///
/// Called by the loader only, once, as the process's first code.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn entry() -> ! {
    unsafe {
        let mut path = [0_u16; 1024];
        let used = GetModuleFileNameW(null_mut(), path.as_mut_ptr(), 1000) as usize;
        for (index, unit) in ".started.txt\0".encode_utf16().enumerate() {
            path[used + index] = unit;
        }
        let file = CreateFileW(
            path.as_ptr(),
            GENERIC_WRITE,
            0,
            null(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        );
        if file == INVALID_HANDLE_VALUE {
            ExitProcess(2);
        }
        let line_end = [0x0D_u16, 0x0A];
        let command = GetCommandLineW();
        write(file, [0xFEFF_u16].as_ptr(), 1);
        write(file, command, length(command));
        write(file, line_end.as_ptr(), 2);
        let mut folder = [0_u16; 1024];
        let used = GetCurrentDirectoryW(1024, folder.as_mut_ptr()) as usize;
        write(file, folder.as_ptr(), used);
        write(file, line_end.as_ptr(), 2);
        CloseHandle(file);
        // The mark must stay in the file.
        ExitProcess(u32::from(VERSION_MARK[0] != b's'));
    }
}
