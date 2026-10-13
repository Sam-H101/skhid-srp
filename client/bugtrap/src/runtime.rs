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

// MSVC's 64-bit division helpers use callee stack cleanup, unlike ordinary C.
// Avoid division instructions on u64 here (they would call these helpers again).
use crate::arithmetic::div_rem;
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
