//! What a program without the Rust standard library and without a C
//! runtime must bring itself: the heap, the panic handler and the helper
//! routines the compiler calls on 32-bit x86 (as `client/bugtrap` does).

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;

use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapAlloc, HeapFree, HeapReAlloc};

/// The process heap. It aligns to 8 bytes; a larger alignment is not asked
/// for anywhere here and is refused.
struct Heap;

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() > 8 {
            return core::ptr::null_mut();
        }
        unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)).cast() }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _: Layout) {
        unsafe {
            HeapFree(GetProcessHeap(), 0, pointer.cast::<c_void>());
        }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if layout.align() > 8 {
            return core::ptr::null_mut();
        }
        unsafe { HeapReAlloc(GetProcessHeap(), 0, pointer.cast::<c_void>(), size.max(1)).cast() }
    }
}

#[global_allocator]
static HEAP: Heap = Heap;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    crate::fatal_panic(info)
}

// x86 MSVC stack probe ABI: EAX holds the frame size. Touch each page so
// Windows commits guard pages before moving ESP into the newly reserved frame.
core::arch::global_asm!(
    ".text",
    ".globl __chkstk",
    "__chkstk:",
    "push ecx",
    "lea ecx, [esp + 8]",
    "2:",
    "cmp eax, 4096",
    "jb 3f",
    "sub ecx, 4096",
    "test dword ptr [ecx], eax",
    "sub eax, 4096",
    "jmp 2b",
    "3:",
    "sub ecx, eax",
    "test dword ptr [ecx], eax",
    "mov eax, esp",
    "mov esp, ecx",
    "mov ecx, [eax]",
    "mov eax, [eax + 4]",
    "push eax",
    "ret",
);
#[unsafe(no_mangle)]
static _fltused: i32 = 0;

/// 64-bit division without a division instruction on u64 (which would call
/// the helpers below again).
fn div_rem(mut n: u64, d: u64) -> (u64, u64) {
    let (mut q, mut r) = (0_u64, 0_u64);
    for _ in 0..64 {
        let carry = r >> 63;
        r = (r << 1) | (n >> 63);
        n <<= 1;
        q <<= 1;
        if carry != 0 || r >= d {
            r = r.wrapping_sub(d);
            q |= 1;
        }
    }
    (q, r)
}

// MSVC's 64-bit division helpers use callee stack cleanup, unlike ordinary C.
unsafe extern "stdcall" fn divide(n: u64, d: u64) -> u64 {
    div_rem(n, d).0
}
unsafe extern "stdcall" fn remainder(n: u64, d: u64) -> u64 {
    div_rem(n, d).1
}
core::arch::global_asm!(
    ".globl __aulldiv", "__aulldiv:", "jmp {divide}",
    ".globl __aullrem", "__aullrem:", "jmp {remainder}",
    divide = sym divide, remainder = sym remainder,
);
