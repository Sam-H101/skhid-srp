; Patch 31: the mouse turns the warmup camera while the kits are shown
; before the start (build 2777, client_patch.md section 3.29; the rules:
; ../docs/notes/kit-change-before-start.md). One entry, called from
; WARMUP_LOOK_HOOK in patch_026g.py, which checks the site.
;   nasm -f bin -DCAVE=0xA425D40 warmup_look_026g.asm -o warmup_look_026g.section.bin
;
; The game world's input handler (game world+1F8h, priority 7, vtable
; 0x9FC588) gets a mouse move before the player's input handler (game
; world+4D58h, priority 10): the input world calls the handlers in their
; priority's order and stops at the first that answers 1 (0x3B9F10). Its
; on_mouse_move (0x25D0E0, esi = the handler) puts the hud's cursor where
; the mouse is (game world vtable+30h) and answers whether the hud's cursor
; is shown (hud+2DCh; the hud is at game world+1FCh, the handler's +4), at
; 0x25D118: `cmp byte [esi+2E0h],0` / ... / `setne al`. The hud shows its
; cursor with the kits (0x2383B0 -> 0x23A930), and patch 31 shows the kits
; to a live player before the start: the cursor took every move, and the
; warmup camera, which turns by the moves the player's handler adds up
; (0x240200, its warmup mode 0x242200), stood still.
;
; This answers 0 (ZF set, `setne al` gives 0) while the player's handler is
; in the warmup mode (+358h = 3, which only the warmup camera sets,
; 0x1F8190: before play) and the kits are shown (hud+2DFh): the cursor moves
; and the move goes on to the player's handler, which turns the warmup
; camera (the wheel its distance) as without the kits. Otherwise the
; original's answer. The cursor's clicks are the hud's as before (the
; handler's on_mouse_key_action is not changed). Nothing a match computes:
; the warmup mode gives the player's input no turn (0x242200 empties it).
BITS 32
ORG CAVE

%define CURSOR      0x2E0          ; handler: the hud's cursor shown (hud+2DCh)
%define KITS        0x2E3          ; handler: the kits shown (hud+2DFh, 0x2383B0)
%define INPUT       0x4B60         ; handler: the player's input handler (game world+4D58h)
%define MODE        0x358          ; player's input handler: its mode
%define WARMUP      3              ; the warmup mode (0x1F8190)

warmup_look:
    cmp byte [esi+CURSOR], 0
    je .done                        ; no cursor: ZF, the move goes on (the original)
    mov eax, [esi+INPUT]
    test eax, eax
    jz .cursor
    cmp dword [eax+MODE], WARMUP
    jne .cursor
    cmp byte [esi+KITS], 0
    je .cursor
    xor eax, eax                    ; ZF: the move goes on to the warmup camera too
.done:
    ret
.cursor:
    cmp byte [esi+CURSOR], 0        ; not ZF: the cursor takes the move (the original)
    ret
