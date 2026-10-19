; Patch 31: the end of a drag of the warmup camera, and the camera's limits,
; the lobby's (build 2777, client_patch.md section 3.29; with
; warmup_look_026g.asm). Two entries at fixed offsets (WARMUP_LIMITS_* in
; patch_026g.py, which checks them).
;   nasm -f bin -DCAVE=0xA425A98 warmup_limits_026g.asm -o warmup_limits_026g.section.bin
BITS 32
ORG CAVE

%define WORLD       -0x1F8         ; the game world's input handler: the game world
%define PITCH       0x350          ; player's input handler: the warmup camera's pitch (yaw +34Ch)
%define DISTANCE    0x354          ; and its distance (1.0 from the constructor, 0x23FDE7)
%define CLICK       8
%define PITCH_MAX   0x3F860A92     ; 1.0471976f: +-60 degrees, the lobby camera's (0x9C56C0, 0xA2DB08)
%define NEAREST     0x3FC00000     ; 1.5f, the lobby camera's nearest (0xA7AFA4)
%define FARTHEST    0x40933333     ; 4.6f, its farthest (0xA2DB0C)

; warmup_look's no button held (esi = the game world's input handler, ebx =
; its state; keeps them and edi): a drag (moves above CLICK) ends with the
; hud's pointer, and the system's cursor, where it began (game world
; vtable+3Ch, as the lobby camera puts the cursor back, 0x283DE8); the
; moves are 0 again.
drag_end:
    mov ecx, [ebx]
    and dword [ebx], 0
    cmp ecx, CLICK
    jbe .done
    push dword [ebx+8]
    push dword [ebx+4]
    lea ecx, [esi+WORLD]
    mov eax, [ecx]
    call [eax+0x3c]
.done:
    ret
times 0x20-($-$$) db 0xcc

; process_warmup_mode (0x242200, esi = the player's input handler) stores the
; distance (0x242219: it adds the wheel of the frame, +348h, with no limit)
; and then the new pitch (`movss [esi+350h],xmm0` at 0x24225E, with no
; limit): that store calls this. The pitch is kept within +-60 degrees and
; the distance within 1.5 and 4.6, as the lobby camera keeps its own
; (0x283EF0). Both as integers: a float's bits order as signed integers
; for a positive and any negative one is below; a not-a-number is past the
; limit. eax and edx are written after it before they are read.
warmup_limits:
    movd eax, xmm0
    mov edx, eax
    and edx, 0x7fffffff
    cmp edx, PITCH_MAX
    jbe .pitch
    and eax, 0x80000000
    or eax, PITCH_MAX
.pitch:
    mov [esi+PITCH], eax
    mov eax, [esi+DISTANCE]
    mov edx, NEAREST
    cmp eax, edx
    cmovl eax, edx
    mov edx, FARTHEST
    cmp eax, edx
    cmovg eax, edx
    mov [esi+DISTANCE], eax
    ret
