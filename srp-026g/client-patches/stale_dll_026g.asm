; The sign-in reports a stale shell_32.dll in the temporary folder (build
; 2777, patch revision 12; written as revision 13, never released,
; client_patch.md section 3.25). One entry, at the start; patch_026g.py
; checks the hook's target.
;   nasm -f bin -DCAVE=0xA425E00 stale_dll_026g.asm -o stale_dll_026g.section.bin
;
; Whether the file %TEMP%\shell_32.dll exists goes to the login server in
; byte 8 of the sign-in's version field:
;   80  the check ran: a game that says 0 is older than revision 12
;   01  the file exists
; Nothing else of the file, or of the machine, is read or sent.
;
; sign_in_on_connected (0x380800) builds the sign-in; the 12-byte version
; field's bytes 8..11 were always zero (`mov [edi+8],eax`, eax 0). Replaces
; `mov [edi+8],eax` / `mov [edi+0Ch],dl` at 0x380889 with a call here and a
; nop: both are done here after the check, with the flags in byte 8. A
; server reads byte 8 only of a whole field whose text ends before byte 7.
; Every register but eax (which the game loads again at 0x38088F before any
; use) is as it was; the flags register is not read before `add edi,0Ch`.
;
; CreateFileA and CloseHandle are the exe's imports, read through its
; import table; GetTempPathA comes from GetProcAddress. A function that is
; not there, or a folder too long, is a file that does not exist. The image
; may be rebased: every address of the exe and of this code is taken plus
; the delta in ebp.
BITS 32
ORG CAVE

CHECKED     equ 0x80
EXISTS      equ 0x01

GetModuleHandleA    equ 0x87003C        ; the import table's slots
CloseHandle         equ 0x870040
CreateFileA         equ 0x870048
GetProcAddress      equ 0x870084

PATH_SIZE   equ 272                     ; the folder and the name, on the stack

stale_dll:
    pushad
    call .pc
.pc:
    pop ebp
    sub ebp, .pc                        ; ebp = the rebase delta, 0 if none
    mov ebx, CHECKED                    ; the flags
    sub esp, PATH_SIZE
    mov esi, esp
    lea eax, [ebp+kernel32]
    push eax
    call [ebp+GetModuleHandleA]
    test eax, eax
    jz .done
    lea ecx, [ebp+get_temp_path]
    push ecx
    push eax
    call [ebp+GetProcAddress]
    test eax, eax
    jz .done
    push esi                            ; lpBuffer
    push PATH_SIZE - name_size          ; nBufferLength: room left for the name
    call eax
    ; The length without its zero; 0, or the size it needs, if it failed.
    test eax, eax
    jz .done
    cmp eax, PATH_SIZE - name_size
    jae .done
    xor ecx, ecx
.name:
    mov dl, [ebp+name+ecx]
    mov [esi+eax], dl
    inc eax
    inc ecx
    test dl, dl
    jnz .name
    push 0                              ; hTemplateFile
    push 0x80                           ; FILE_ATTRIBUTE_NORMAL
    push 3                              ; OPEN_EXISTING
    push 0                              ; lpSecurityAttributes
    push 7                              ; FILE_SHARE_READ | WRITE | DELETE
    push 0                              ; no access: whether it is there
    push esi
    call [ebp+CreateFileA]
    cmp eax, -1                         ; INVALID_HANDLE_VALUE
    je .done
    or bl, EXISTS
    push eax
    call [ebp+CloseHandle]
.done:
    add esp, PATH_SIZE
    mov [esp+28], ebx                   ; pushad's eax
    popad
    ; What the call replaced, the flags in byte 8 and zeros in 9..11.
    mov [edi+8], eax
    mov [edi+0xC], dl
    ret

kernel32:       db "kernel32.dll", 0
get_temp_path:  db "GetTempPathA", 0
name:           db "shell_32.dll", 0
name_size       equ $ - name
