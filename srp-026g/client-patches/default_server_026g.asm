; 0.26g build 2777: use an embedded login address only when neither
; -server nor -replay supplied one. Enter from the missing-server error
; at 0x1EB25B, after the original option checks and string initialization.
;
; Appended after the 420-byte snapshot-upload cave. patch_026g.py appends
; the validated, NUL-terminated host:port immediately after this code.
; No absolute address operands: the string uses call/pop, the return is rel32.
;
; nasm -f bin -DCAVE=0xA425000 default_server_026g.asm -o default_server_026g.section.bin
; nasm -f bin -DCAVE=0x325A60 default_server_026g.asm -o default_server_026g.hash-mismatch.bin

bits 32
%ifndef CAVE
%define CAVE 0xA425000
%endif
org CAVE + 420

    push ecx
    push esi
    push edi
    call .here
.here:
    pop esi
    add esi, address - .here
    mov edi, [ebp - 0x494]       ; beginning of the initialized 512-byte string
    cld
.copy:
    lodsb
    stosb
    test al, al
    jnz .copy
    dec edi
    mov [ebp - 0x490], edi      ; end pointer excludes the terminating NUL
    pop edi
    pop esi
    pop ecx
    jmp 0x1EAE90                ; original host:port validation and connection
address:
