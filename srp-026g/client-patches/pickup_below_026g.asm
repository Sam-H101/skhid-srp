; Patch 23: a battery under the floor one stands on can be picked up (build
; 2777, base_player::detect_usable_object 0x2D84C0). The entries are at
; fixed offsets; patch_026g.py checks each hook's target.
;   nasm -f bin -DCAVE=0xA425800 pickup_below_026g.asm -o pickup_below_026g.section.bin
BITS 32
ORG CAVE

; The scan's frame after `push ebx/esi/edi` (esp = base): [esp+0Ch] the
; hits' groups so far, [esp+10h] the result, [esp+40h] new (the frame grows
; by 8 at 0x2D84C9): set once a wall or floor was crossed. [ebp+8] = the
; player: +43Ch the feet, +47Ch the eye. client_patch.md section 3.23.
%define CROSSED 0x40
%define DISTANCE 0xA710D8          ; usable_objects_detection_distance, 2.0

; 0x2D84E9 `fld dword [DISTANCE]` -> `call reach; nop`: the ray goes twice
; as far. What it finds beyond the native distance is held to the battery
; rule of `candidate`.
reach:
    mov byte [esp+4+CROSSED], 0
    fld dword [DISTANCE]
    fadd st0, st0
    ret
times 32-($-$$) db 0xcc

; 0x2D861C `and al,0Ah; cmp al,0Ah; jz 0x2D866D` -> jmp: a hit that is no
; usable object. Where the original stops, the scan goes on and remembers
; it; the walkable and hittable bits are cleared so the next such hit is
; seen as one too.
solid:
    and al, 0x0A
    cmp al, 0x0A
    jne .next
    mov byte [esp+CROSSED], 1
    and word [esp+0x0C], 0xFFF5
.next:
    jmp 0x2D8622
times 64-($-$$) db 0xcc

; 0x2D85BE `mov eax,[ebp+8]; mov edx,[esi]; mov edx,[edx+30h]` -> jmp:
; esi = the usable object a hit belongs to, edi = the hit (+0 its point).
; Within the native distance and before any crossed hit: the original. Else
; only a battery (its can_use is victory_item_core's) whose centre is below
; the player's feet (or at most 10 cm above them: a battery half in the
; floor) and at most 1 m from them across; its point becomes the
; feet, which the use's range check (0x300830) keeps in reach. Anything
; else is skipped, past the original's keeping of the first unusable object
; for the prompt (0x2D85DC): it is not to be seen at all.
candidate:
    mov eax, [ebp+8]
    movss xmm0, [edi]
    subss xmm0, [eax+0x47C]
    mulss xmm0, xmm0
    movss xmm1, [edi+4]
    subss xmm1, [eax+0x480]
    mulss xmm1, xmm1
    addss xmm0, xmm1
    movss xmm1, [edi+8]
    subss xmm1, [eax+0x484]
    mulss xmm1, xmm1
    addss xmm0, xmm1
    movss xmm1, [DISTANCE]
    mulss xmm1, xmm1
    comiss xmm0, xmm1
    ja .special
    cmp byte [esp+CROSSED], 0
    jne .special
.original:
    mov eax, [ebp+8]
    mov edx, [esi]
    mov edx, [edx+0x30]
    jmp 0x2D85C6
.special:
    mov edx, [esi]
    cmp dword [edx+0x30], 0x300710 ; victory_item_core::can_use
    jne .reject
    sub esp, 0x40
    push esp
    mov ecx, esi
    call [edx+0x44]                ; get_transform: a battery's own (0x300A20)
    mov eax, [ebp+8]
    movss xmm0, [eax+0x440]
    subss xmm0, [esp+0x34]         ; below = feet.y - centre.y
    movss xmm1, [esp+0x30]
    subss xmm1, [eax+0x43C]
    mulss xmm1, xmm1
    movss xmm2, [esp+0x38]
    subss xmm2, [eax+0x444]
    mulss xmm2, xmm2
    addss xmm1, xmm2               ; across^2
    add esp, 0x40
    comiss xmm0, [above]
    jbe .reject                    ; more than 10 cm above (or NaN)
    movss xmm2, [across]
    comiss xmm2, xmm1
    jb .reject                     ; too far across (or NaN)
    mov ecx, [eax+0x43C]
    mov [edi], ecx
    mov ecx, [eax+0x440]
    mov [edi+4], ecx
    mov ecx, [eax+0x444]
    mov [edi+8], ecx
    jmp .original
.reject:
    and dword [esp+0x0C], 0xFF7F
    jmp 0x2D85FC
times 320-($-$$) db 0xcc

across: dd 1.0                     ; (1 m)^2
above:  dd -0.1                    ; below the feet, at least -10 cm
