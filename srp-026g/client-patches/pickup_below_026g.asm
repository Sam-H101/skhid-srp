; Patch 23: a battery under the floor one stands on can be picked up (build
; 2777, base_player::detect_usable_object 0x2D84C0). The entries are at
; fixed offsets; patch_026g.py checks each hook's target.
;   nasm -f bin -DCAVE=0xA425800 pickup_below_026g.asm -o pickup_below_026g.section.bin
BITS 32
ORG CAVE

; The scan casts one ray from the eye along the view (the collision world's
; ray_query, vtable+4Ch: every hit, nearest first) and walks the hits: a
; usable object (group 80h) is the result if its can_use (vtable+30h) says
; yes; at any other hit it stops once the groups seen so far hold both
; walkable (2) and hittable (8), so a battery behind a wall or under a floor
; is never seen. Its frame after `push ebx/esi/edi` (esp = base): [esp+0Ch]
; the groups so far, [esp+10h] the result, and [esp+40h], new (the frame
; grows by 8 at 0x2D84C9), set once such a hit was crossed. [ebp+8] = the
; player: +43Ch its position (the feet), +47Ch the ray's start (the eye).
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
; the player's feet and at most 1 m from them across; its point becomes the
; feet, which the use's range check (0x300830: the eye to the point, within
; the native distance, every tick of the use) keeps in reach. Anything else
; is dropped as the original drops a usable object it cannot use.
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
    xorps xmm2, xmm2
    comiss xmm0, xmm2
    jbe .reject                    ; not below (or NaN)
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
