use core::{
    fmt,
    ptr::null_mut,
    sync::atomic::{AtomicPtr, Ordering},
};
use windows_sys::Win32::{Foundation::*, System::Memory::*};

pub struct Wide<const N: usize> {
    pub buf: [u16; N],
    pub len: usize,
}
impl<const N: usize> Wide<N> {
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }
    pub fn wide(&mut self, s: &[u16]) -> fmt::Result {
        if self.len + s.len() >= N {
            return Err(fmt::Error);
        }
        self.buf[self.len..self.len + s.len()].copy_from_slice(s);
        self.len += s.len();
        self.buf[self.len] = 0;
        Ok(())
    }
    pub fn ptr(&self) -> *const u16 {
        self.buf.as_ptr()
    }
    pub fn truncate(&mut self, len: usize) {
        self.len = len;
        self.buf[len] = 0;
    }
}
impl<const N: usize> fmt::Write for Wide<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.encode_utf16() {
            self.wide(core::slice::from_ref(&c))?;
        }
        Ok(())
    }
}

// Immutable process-lifetime strings keep legacy getter pointers valid even
// after a later setter. Configuration is small; no allocator is used to dump.
pub struct Text(AtomicPtr<u8>);
impl Text {
    pub const fn new() -> Self {
        Self(AtomicPtr::new(null_mut()))
    }
    pub fn get(&self) -> *const u8 {
        let p = self.0.load(Ordering::Acquire);
        if p.is_null() { c"".as_ptr().cast() } else { p }
    }
    pub unsafe fn set(&self, value: *const u8) {
        let mut len = 0;
        if !value.is_null() {
            while len < 32767 && *value.add(len) != 0 {
                len += 1;
            }
        }
        let p = HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, len + 1).cast::<u8>();
        if p.is_null() {
            SetLastError(ERROR_NOT_ENOUGH_MEMORY);
            return;
        }
        if len != 0 {
            core::ptr::copy_nonoverlapping(value, p, len);
        }
        self.0.store(p, Ordering::Release);
    }
}
