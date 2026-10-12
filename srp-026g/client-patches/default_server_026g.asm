; 0.26g build 2777: use an embedded login address only when neither
; -server nor -replay supplied one. Enter from the missing-server error
; at 0x1EB25B, after the original option checks. Initialize the parsed
; server key, then repeat the original server-option read and startup path.
;
; Appended after the 420-byte snapshot-upload cave. patch_026g.py appends
; the validated, NUL-terminated host:port immediately after this code.
; The string and key use call/pop-relative addressing; the return is rel32.
;
; nasm -f bin -DCAVE=0xA425000 default_server_026g.asm -o default_server_026g.section.bin
; nasm -f bin -DCAVE=0x325A60 default_server_026g.asm -o default_server_026g.hash-mismatch.bin

bits 32
%ifndef CAVE
%define CAVE 0xA425000
%endif
org CAVE + 420

    push ecx
    push edx
    push esi
    push edi
    call .here
.here:
    pop esi
    lea edx, [esi + 0xA9C550 - .here] ; parsed command_line::key for server
    add esi, address - .here
    mov edi, [edx]              ; key string: start/end/capacity at +0/+4/+8
    cld
.copy:
    lodsb
    stosb
    test al, al
    jnz .copy
    dec edi
    mov [edx + 4], edi         ; end excludes NUL
    mov dword [edx + 0x224], 4 ; type_string, not type_unset
    pop edi
    pop esi
    pop edx
    pop ecx
    jmp 0x1EAE47               ; reread the key through the original code
address:
