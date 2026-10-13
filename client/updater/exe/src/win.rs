//! The few things the updater asks of Windows, each behind a small function
//! that returns the Windows error code when it fails.

use alloc::string::String;
use alloc::vec::Vec;
use core::ptr::{null, null_mut};

use updater_logic::sha256::Sha256;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, GENERIC_READ,
    GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE, SYSTEMTIME,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateDirectoryW, CreateFileW, DeleteFileW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FindClose,
    FindFirstFileW, FindNextFileW, FlushFileBuffers, GetDiskFreeSpaceExW, GetFileAttributesW,
    GetFileSizeEx, INVALID_FILE_ATTRIBUTES, MOVEFILE_REPLACE_EXISTING, MoveFileExW, OPEN_ALWAYS,
    OPEN_EXISTING, ReadFile, SetFilePointerEx, WIN32_FIND_DATAW, WriteFile,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetTickCount64};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, OpenMutexW, PROCESS_INFORMATION, STARTUPINFOW, Sleep,
};

/// A Windows error code (`GetLastError`).
pub type Code = u32;

/// `text` as Windows wants it: UTF-16 with a zero at its end.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

/// UTF-16 up to its first zero, as text.
pub fn from_wide(text: &[u16]) -> String {
    let end = text
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(text.len());
    char::decode_utf16(text[..end].iter().copied())
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

pub fn last_error() -> Code {
    unsafe { GetLastError() }
}

/// What Windows says of an error code, in the system's language, without
/// the line end: "Access is denied".
pub fn error_text(code: Code) -> String {
    let mut buffer = [0_u16; 512];
    let length = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            null(),
            code,
            0,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            null(),
        )
    } as usize;
    let text = from_wide(&buffer[..length.min(buffer.len())]);
    text.trim_end_matches(['\r', '\n', ' ', '.']).into()
}

/// The full path of this program.
pub fn module_path() -> Result<String, Code> {
    let mut buffer = alloc::vec![0_u16; 1024];
    loop {
        let length =
            unsafe { GetModuleFileNameW(null_mut(), buffer.as_mut_ptr(), buffer.len() as u32) }
                as usize;
        if length == 0 {
            return Err(last_error());
        }
        if length < buffer.len() {
            return Ok(from_wide(&buffer[..length]));
        }
        if buffer.len() >= 32768 {
            return Err(last_error());
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

/// The folder part of `path` (no `\` at its end), and what follows it.
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rfind(['\\', '/']) {
        Some(at) => (&path[..at], &path[at + 1..]),
        None => ("", path),
    }
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn open(path: &str, access: u32, share: u32, how: u32) -> Result<Handle, Code> {
    let handle = unsafe {
        CreateFileW(
            wide(path).as_ptr(),
            access,
            share,
            null(),
            how,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(last_error())
    } else {
        Ok(Handle(handle))
    }
}

/// Whether the error says that the file or its folder is not there.
pub fn is_missing(code: Code) -> bool {
    code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND
}

/// Opens a file for reading, whoever else has it open (a running program's
/// own file too).
fn open_read(path: &str) -> Result<Handle, Code> {
    open(
        path,
        GENERIC_READ,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        OPEN_EXISTING,
    )
}

fn read_some(file: &Handle, buffer: &mut [u8]) -> Result<usize, Code> {
    let mut read = 0_u32;
    let ok = unsafe {
        ReadFile(
            file.0,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut read,
            null_mut(),
        )
    };
    if ok == 0 {
        Err(last_error())
    } else {
        Ok(read as usize)
    }
}

/// A whole file, if it is at most `limit` bytes.
pub fn read_file(path: &str, limit: usize) -> Result<Vec<u8>, Code> {
    let file = open_read(path)?;
    let mut size = 0_i64;
    if unsafe { GetFileSizeEx(file.0, &mut size) } == 0 {
        return Err(last_error());
    }
    if size < 0 || size as u64 > limit as u64 {
        return Err(windows_sys::Win32::Foundation::ERROR_FILE_TOO_LARGE);
    }
    let mut data = alloc::vec![0_u8; size as usize];
    let mut filled = 0;
    while filled < data.len() {
        match read_some(&file, &mut data[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    data.truncate(filled);
    Ok(data)
}

/// A file's size and SHA-256, read in pieces.
pub fn hash_file(path: &str) -> Result<(u64, [u8; 32]), Code> {
    let file = open_read(path)?;
    let mut hash = Sha256::new();
    let mut buffer = alloc::vec![0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        match read_some(&file, &mut buffer)? {
            0 => return Ok((size, hash.finish())),
            read => {
                hash.update(&buffer[..read]);
                size += read as u64;
            }
        }
    }
}

fn write_all(file: &Handle, mut data: &[u8]) -> Result<(), Code> {
    while !data.is_empty() {
        let mut written = 0_u32;
        let piece = data.len().min(1 << 20) as u32;
        if unsafe { WriteFile(file.0, data.as_ptr(), piece, &mut written, null_mut()) } == 0 {
            return Err(last_error());
        }
        if written == 0 {
            return Err(windows_sys::Win32::Foundation::ERROR_WRITE_FAULT);
        }
        data = &data[written as usize..];
    }
    Ok(())
}

/// Writes a new file (over one of the same name) and has Windows put it on
/// the disk before returning.
pub fn write_file(path: &str, data: &[u8]) -> Result<(), Code> {
    let file = open(path, GENERIC_WRITE, 0, CREATE_ALWAYS)?;
    write_all(&file, data)?;
    if unsafe { FlushFileBuffers(file.0) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// Adds to the end of a file, making it if it is not there. A file longer
/// than `limit` is started anew first.
pub fn append_file(path: &str, data: &[u8], limit: u64) -> Result<(), Code> {
    let file = open(path, GENERIC_WRITE, FILE_SHARE_READ, OPEN_ALWAYS)?;
    let mut size = 0_i64;
    if unsafe { GetFileSizeEx(file.0, &mut size) } != 0 && size as u64 > limit {
        drop(file);
        return write_file(path, data);
    }
    // 2 = FILE_END.
    if unsafe { SetFilePointerEx(file.0, 0, null_mut(), 2) } == 0 {
        return Err(last_error());
    }
    write_all(&file, data)
}

pub fn exists(path: &str) -> bool {
    unsafe { GetFileAttributesW(wide(path).as_ptr()) != INVALID_FILE_ATTRIBUTES }
}

pub fn is_folder(path: &str) -> bool {
    let attributes = unsafe { GetFileAttributesW(wide(path).as_ptr()) };
    attributes != INVALID_FILE_ATTRIBUTES && attributes & FILE_ATTRIBUTE_DIRECTORY != 0
}

/// Deletes a file; one that is not there is no error.
pub fn delete(path: &str) -> Result<(), Code> {
    if unsafe { DeleteFileW(wide(path).as_ptr()) } != 0 {
        return Ok(());
    }
    match last_error() {
        code if is_missing(code) => Ok(()),
        code => Err(code),
    }
}

/// Renames a file within its disk, over `to` when `replace`.
pub fn rename(from: &str, to: &str, replace: bool) -> Result<(), Code> {
    let flags = if replace {
        MOVEFILE_REPLACE_EXISTING
    } else {
        0
    };
    if unsafe { MoveFileExW(wide(from).as_ptr(), wide(to).as_ptr(), flags) } == 0 {
        Err(last_error())
    } else {
        Ok(())
    }
}

/// Makes the folder and those above it that are missing.
pub fn create_folders(path: &str) -> Result<(), Code> {
    if is_folder(path) {
        return Ok(());
    }
    let (parent, _) = split_path(path);
    if !parent.is_empty() && !parent.ends_with(':') && parent != path {
        create_folders(parent)?;
    }
    if unsafe { CreateDirectoryW(wide(path).as_ptr(), null()) } == 0 {
        let code = last_error();
        if code != ERROR_ALREADY_EXISTS {
            return Err(code);
        }
    }
    Ok(())
}

/// The bytes this user may still write in `folder`'s disk.
pub fn free_space(folder: &str) -> Option<u64> {
    let mut free = 0_u64;
    let ok =
        unsafe { GetDiskFreeSpaceExW(wide(folder).as_ptr(), &mut free, null_mut(), null_mut()) };
    (ok != 0).then_some(free)
}

/// The names in `folder` that match `pattern` (`*.skhid-old*`), files only.
pub fn list(folder: &str, pattern: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut found: WIN32_FIND_DATAW = unsafe { core::mem::zeroed() };
    let query = alloc::format!("{folder}\\{pattern}");
    let handle = unsafe { FindFirstFileW(wide(&query).as_ptr(), &mut found) };
    if handle == INVALID_HANDLE_VALUE {
        return names;
    }
    loop {
        if found.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            names.push(from_wide(&found.cFileName));
        }
        if unsafe { FindNextFileW(handle, &mut found) } == 0 {
            break;
        }
    }
    unsafe {
        FindClose(handle);
    }
    names
}

pub fn sleep(milliseconds: u32) {
    unsafe { Sleep(milliseconds) }
}

/// Milliseconds since the system started.
pub fn ticks() -> u64 {
    unsafe { GetTickCount64() }
}

/// The local date and time, `2026-10-13 02:19:27`.
pub fn timestamp() -> String {
    let mut time: SYSTEMTIME = unsafe { core::mem::zeroed() };
    unsafe { GetLocalTime(&mut time) };
    alloc::format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond
    )
}

/// Whether a mutex of this name exists now (some process holds it open).
pub fn mutex_exists(name: &str) -> bool {
    // 0x0010_0000 = SYNCHRONIZE, the least right that opens it.
    let handle = unsafe { OpenMutexW(0x0010_0000, 0, wide(name).as_ptr()) };
    if handle.is_null() {
        return false;
    }
    unsafe {
        CloseHandle(handle);
    }
    true
}

/// Starts a program with `folder` as its working folder; does not wait.
pub fn start(application: &str, command: &str, folder: &str) -> Result<(), Code> {
    let mut startup: STARTUPINFOW = unsafe { core::mem::zeroed() };
    startup.cb = core::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { core::mem::zeroed() };
    let mut command = wide(command);
    let ok = unsafe {
        CreateProcessW(
            wide(application).as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            0,
            0,
            null(),
            wide(folder).as_ptr(),
            &startup,
            &mut process,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}
