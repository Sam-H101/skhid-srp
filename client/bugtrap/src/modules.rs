//! Repair only a missing module list from Wine DbgHelp. Use documented minidump
//! records and a snapshot of this process; never change kernel ptrace policy.
use core::{
    mem::{size_of, zeroed},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::*,
    System::{
        Diagnostics::{Debug::*, ToolHelp::*},
        Threading::GetCurrentProcess,
    },
};

type Result<T> = core::result::Result<T, u32>;
unsafe fn error() -> u32 {
    let e = GetLastError();
    if e == 0 { ERROR_INVALID_DATA } else { e }
}
unsafe fn seek(file: HANDLE, offset: u32) -> Result<()> {
    if SetFilePointerEx(file, offset as i64, null_mut(), FILE_BEGIN) == 0 {
        Err(error())
    } else {
        Ok(())
    }
}
unsafe fn position(file: HANDLE) -> Result<u32> {
    let mut p = 0;
    if SetFilePointerEx(file, 0, &mut p, FILE_CURRENT) == 0 {
        return Err(error());
    }
    u32::try_from(p).map_err(|_| ERROR_FILE_TOO_LARGE)
}
unsafe fn put(file: HANDLE, data: *const u8, len: u32) -> Result<()> {
    let mut written = 0;
    if WriteFile(file, data, len, &mut written, null_mut()) == 0 || written != len {
        Err(error())
    } else {
        Ok(())
    }
}
unsafe fn read<T: Copy>(file: HANDLE, offset: u32) -> Result<T> {
    seek(file, offset)?;
    let mut result: T = zeroed();
    let mut read = 0;
    if ReadFile(
        file,
        (&mut result as *mut T).cast(),
        size_of::<T>() as u32,
        &mut read,
        null_mut(),
    ) == 0
        || read as usize != size_of::<T>()
    {
        return Err(error());
    }
    Ok(result)
}
unsafe fn memory<T: Copy>(address: usize) -> Option<T> {
    let mut value: T = zeroed();
    let mut read = 0;
    if ReadProcessMemory(
        GetCurrentProcess(),
        address as *const _,
        (&mut value as *mut T).cast(),
        size_of::<T>(),
        &mut read,
    ) != 0
        && read == size_of::<T>()
    {
        Some(value)
    } else {
        None
    }
}

unsafe fn add_module(file: HANDLE, entry: &MODULEENTRY32W) -> Result<MINIDUMP_MODULE> {
    let mut module: MINIDUMP_MODULE = zeroed();
    module.BaseOfImage = entry.modBaseAddr as u64;
    module.SizeOfImage = entry.modBaseSize;
    module.ModuleNameRva = position(file)?;
    let n = entry
        .szExePath
        .iter()
        .position(|&x| x == 0)
        .unwrap_or(entry.szExePath.len());
    let bytes = (n * 2) as u32;
    put(file, (&bytes as *const u32).cast(), 4)?;
    put(file, entry.szExePath.as_ptr().cast(), bytes)?;
    put(file, (&0u16 as *const u16).cast(), 2)?;
    let base = entry.modBaseAddr as usize;
    if memory::<u16>(base) != Some(0x5a4d) {
        return Ok(module);
    }
    let Some(pe_offset) = memory::<u32>(base + 0x3c) else {
        return Ok(module);
    };
    if pe_offset
        > entry
            .modBaseSize
            .saturating_sub(size_of::<IMAGE_NT_HEADERS32>() as u32)
    {
        return Ok(module);
    }
    let Some(pe) = memory::<IMAGE_NT_HEADERS32>(base + pe_offset as usize) else {
        return Ok(module);
    };
    if pe.Signature != 0x4550 || pe.OptionalHeader.Magic != 0x10b {
        return Ok(module);
    }
    module.TimeDateStamp = pe.FileHeader.TimeDateStamp;
    module.CheckSum = pe.OptionalHeader.CheckSum;
    // Include the CodeView record so debuggers can locate the matching PDB.
    if pe.OptionalHeader.NumberOfRvaAndSizes > 6 {
        let dir = pe.OptionalHeader.DataDirectory[6];
        if dir
            .VirtualAddress
            .checked_add(dir.Size)
            .is_some_and(|end| end <= entry.modBaseSize)
        {
            for i in 0..(dir.Size as usize / size_of::<IMAGE_DEBUG_DIRECTORY>()).min(64) {
                let Some(debug) = memory::<IMAGE_DEBUG_DIRECTORY>(
                    base + dir.VirtualAddress as usize + i * size_of::<IMAGE_DEBUG_DIRECTORY>(),
                ) else {
                    break;
                };
                if debug.Type != 2
                    || debug.SizeOfData == 0
                    || debug.SizeOfData > 65536
                    || !debug
                        .AddressOfRawData
                        .checked_add(debug.SizeOfData)
                        .is_some_and(|end| end <= entry.modBaseSize)
                {
                    continue;
                }
                let mut cv = [0u8; 65536];
                let mut n = 0;
                if ReadProcessMemory(
                    GetCurrentProcess(),
                    (base + debug.AddressOfRawData as usize) as *const _,
                    cv.as_mut_ptr().cast(),
                    debug.SizeOfData as usize,
                    &mut n,
                ) != 0
                    && n == debug.SizeOfData as usize
                {
                    module.CvRecord.Rva = position(file)?;
                    module.CvRecord.DataSize = debug.SizeOfData;
                    put(file, cv.as_ptr(), debug.SizeOfData)?;
                    break;
                }
            }
        }
    }
    Ok(module)
}

pub unsafe fn ensure_modules(file: HANDLE) -> Result<()> {
    let header: MINIDUMP_HEADER = read(file, 0)?;
    if header.Signature != 0x504d444d || header.NumberOfStreams > 128 {
        return Err(ERROR_INVALID_DATA);
    }
    let mut location = None;
    for i in 0..header.NumberOfStreams {
        let offset = header
            .StreamDirectoryRva
            .checked_add(i * size_of::<MINIDUMP_DIRECTORY>() as u32)
            .ok_or(ERROR_INVALID_DATA)?;
        let dir: MINIDUMP_DIRECTORY = read(file, offset)?;
        if dir.StreamType == 4 {
            let count: u32 = read(file, dir.Location.Rva)?;
            if count > 0 {
                return Ok(());
            }
            location = Some(offset);
            break;
        }
    }
    let location = location.ok_or(ERROR_INVALID_DATA)?;
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, 0);
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(error());
    }
    let result = fill(file, snapshot, location);
    CloseHandle(snapshot);
    result
}
unsafe fn fill(file: HANDLE, snapshot: HANDLE, location: u32) -> Result<()> {
    let mut entry: MODULEENTRY32W = zeroed();
    entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
    let mut count = 0u32;
    if Module32FirstW(snapshot, &mut entry) == 0 {
        return Err(error());
    }
    loop {
        count += 1;
        if count > 4096 {
            return Err(ERROR_INVALID_DATA);
        }
        if Module32NextW(snapshot, &mut entry) == 0 {
            break;
        }
    }
    let mut end = 0;
    if SetFilePointerEx(file, 0, &mut end, FILE_END) == 0 {
        return Err(error());
    }
    let start = u32::try_from(end).map_err(|_| ERROR_FILE_TOO_LARGE)?;
    let size = 4 + count * size_of::<MINIDUMP_MODULE>() as u32;
    let mut next = start.checked_add(size).ok_or(ERROR_FILE_TOO_LARGE)?;
    seek(file, next)?;
    if SetEndOfFile(file) == 0 {
        return Err(error());
    }
    if Module32FirstW(snapshot, &mut entry) == 0 {
        return Err(error());
    }
    for i in 0..count {
        seek(file, next)?;
        let module = add_module(file, &entry)?;
        next = position(file)?;
        seek(file, start + 4 + i * size_of::<MINIDUMP_MODULE>() as u32)?;
        put(
            file,
            (&module as *const MINIDUMP_MODULE).cast(),
            size_of::<MINIDUMP_MODULE>() as u32,
        )?;
        if i + 1 < count && Module32NextW(snapshot, &mut entry) == 0 {
            return Err(error());
        }
    }
    seek(file, start)?;
    put(file, (&count as *const u32).cast(), 4)?;
    let dir = MINIDUMP_DIRECTORY {
        StreamType: 4,
        Location: MINIDUMP_LOCATION_DESCRIPTOR {
            DataSize: size,
            Rva: start,
        },
    };
    // Publish the new stream only after every module and name has been written.
    seek(file, location)?;
    put(
        file,
        (&dir as *const MINIDUMP_DIRECTORY).cast(),
        size_of::<MINIDUMP_DIRECTORY>() as u32,
    )
}
