; The values the 0.26g launcher (build 2777) read from the installer's
; registry key, worked out from where the launcher itself is.
;
;   nasm -f bin -DCAVE=0x49A000 launcher_values_026g.asm -o launcher_values_026g.bin
;
; The launcher reads five strings of HKLM\Software\Vostok Games\Survarium
; through registry_key::get_string (0x14027): `file to execute`, `arguments`
; and `working folder` when PLAY is pressed, `file to execute` again to read
; the game's version out of it, `update server` before it starts the updater
; and `launcher` when it restarts itself. Only the game's installer wrote
; them. A game that was unpacked has none, and the launcher then shows
; "Error" and never enables PLAY.
;
; patch_launcher_026g.py makes the first seven bytes of get_string a jump to
; `hook` below. For those five names the hook fills the caller's buffer
; itself; every other name goes on to the original code and the registry.
;
;   file to execute   <the launcher's folder>\<exe_name>
;   working folder    <the launcher's folder>
;   launcher          the launcher's own path
;   arguments         <arguments>
;   update server     <server>
;
; exe_name, arguments and server are the three zero-filled slots at the end,
; ASCII with a zero at the end, filled in by patch_launcher_026g.py.
;
; registry_key::get_string, as its callers use it (launcher-026g.md, 2 and 6):
;   edi          the buffer, UTF-16
;   esi          its size in characters
;   [esp+4]      the open key (not used here)
;   [esp+8]      the value's name, UTF-16
;   returns al = 1 when the buffer holds the string, 0 when not; `ret 8`.
;
; The code never names an address of its own: it finds where it was loaded
; (call/pop) and adds that to the two image addresses it uses, so it also
; works when Windows or Wine loads the launcher at another base.

bits 32
org CAVE

GET_STRING_BACK         equ 0x1402E     ; get_string after its first 7 bytes
IAT_GetModuleFileNameW  equ 0x3B0BC

EXE_NAME_SIZE           equ 32
ARGUMENTS_SIZE          equ 160
SERVER_SIZE             equ 64

%define NAME(first, third) (((first) << 8) | (third))

hook:
        push    ebx
        call    .here
.here:  pop     ebx
        sub     ebx, .here              ; ebx = where it is loaded - CAVE's image
        mov     eax, [esp+12]           ; the value's name
        movzx   ecx, word [eax]
        movzx   edx, word [eax+4]
        shl     ecx, 8
        or      ecx, edx                ; its first and third letters
        cmp     ecx, NAME('f', 'l')     ; "file to execute" ("first install version" is f, r)
        je      .exe
        cmp     ecx, NAME('w', 'r')     ; "working folder"
        je      .folder
        cmp     ecx, NAME('l', 'u')     ; "launcher"
        je      .self
        cmp     ecx, NAME('a', 'g')     ; "arguments"
        je      .arguments
        cmp     ecx, NAME('u', 'd')     ; "update server" ("UninstallString" has a capital U)
        je      .server

        ; Any other name: the instructions the jump replaced, then the rest
        ; of get_string. (Its `push ecx` only makes room for a local.)
        pop     ebx
        push    ebp
        mov     ebp, esp
        push    ecx
        lea     eax, [esi+esi]
        jmp     GET_STRING_BACK

.exe:   push    1
        jmp     .path
.folder:
        push    2
        jmp     .path
.self:  push    0
.path:  push    esi                     ; GetModuleFileNameW(NULL, buffer, size)
        push    edi
        push    0
        call    [ebx + IAT_GetModuleFileNameW]
        pop     edx                     ; 1, 2 or 0, pushed above
        test    eax, eax
        jz      .fail
        cmp     eax, esi                ; the size itself: the path was cut
        jae     .fail
        test    edx, edx
        jz      .ok                     ; "launcher": the whole path
        lea     ecx, [edi+eax*2]
.back:  sub     ecx, 2                  ; back to the last backslash
        cmp     ecx, edi
        jb      .fail
        cmp     word [ecx], 0x5C
        jne     .back
        cmp     edx, 2
        jne     .name
        mov     word [ecx], 0           ; "working folder": cut there
        jmp     .ok
.name:  lea     eax, [ecx+2]            ; "file to execute": the name goes after it
        mov     edx, eax
        sub     edx, edi
        shr     edx, 1                  ; characters before the name
        add     edx, EXE_NAME_SIZE
        cmp     edx, esi
        ja      .fail
        lea     edx, [ebx + exe_name]
        jmp     .copy

.arguments:
        lea     edx, [ebx + arguments]
        mov     ecx, ARGUMENTS_SIZE
        jmp     .whole
.server:
        lea     edx, [ebx + server]
        mov     ecx, SERVER_SIZE
.whole: cmp     esi, ecx                ; room for the whole slot
        jb      .fail
        mov     eax, edi

.copy:  movzx   ecx, byte [edx]         ; ASCII at edx to UTF-16 at eax, zero included
        mov     [eax], cx
        inc     edx
        add     eax, 2
        test    ecx, ecx
        jnz     .copy
.ok:    mov     al, 1
        jmp     .done
.fail:  xor     al, al
.done:  pop     ebx
        ret     8

        align   16, db 0xCC
exe_name:
        times EXE_NAME_SIZE db 0
arguments:
        times ARGUMENTS_SIZE db 0
server:
        times SERVER_SIZE db 0
