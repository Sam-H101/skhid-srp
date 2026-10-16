; Patch 30: a kit chosen before the match starts is played at once (build
; 2777, client_patch.md section 3.28). Three entries at fixed offsets
; (KIT_CHANGE_* in patch_026g.py, which checks each hook's target).
;   nasm -f bin -DCAVE=0xA425EC0 kit_change_026g.asm -o kit_change_026g.section.bin
BITS 32
ORG CAVE

%define PLACES      0x18458        ; core: 20 places of 30h: 3 players, +0Ch kit in use, +0Dh next kit
%define STATISTICS  0x18FA8        ; core: the statistics handler, its shared statistics at +8
%define TIME_STATE  0x18FE4        ; core: the time rule's state: 1 waiting, 2 countdown, 3 in process
%define RECORD      0x1F8          ; a place's statistics: events[28][3] u16, then received damage[20][3] f32
%define CLEAN       (0x198 / 4)    ; the dwords of the two
%define FEET        0x43C          ; player: the feet, float3
%define YAW         0x490          ; player: m_rotation_y
%define ALIVE       0x33C          ; player: m_is_alive; +33Fh: in the world's tick lists
%define CLIENT_CORE 0x1F8          ; game_world_core_client: its core

; The respawn rule's tick (0x316820; ecx = the rule, [esp+8] the tick's
; time; thiscall, three arguments, `ret 0Ch`) begins here, at every tick of
; every game and every replay of a tick, before the rule spawns the places
; whose respawn time is now. While the match waits or counts down (state 1
; or 2), each place is tried (try_place). Then the three instructions the
; jump replaced.
kit_change:
    pushad
    mov ebx, ecx                    ; the rule
    mov ebp, [esp+32+8]             ; the tick's time
    mov esi, [ebx+0x134]            ; the core
    mov eax, [esi+TIME_STATE]
    dec eax
    shr eax, 1
    jnz .done                       ; not waiting, not counting down
    xor edi, edi                    ; the place
.place:
    call try_place
    inc edi
    cmp edi, 20
    jb .place
.done:
    popad
    push ebp
    mov ebp, esp
    sub esp, 0x34
    jmp 0x316826

.no:
    ret
; ebx = the rule, esi = the core, edi = the place, ebp = the time; keeps
; them. A place whose next kit (the player's 0xDE, set by the place's tick
; 0x321790) is not the kit in use, whose player is alive, in the world and
; not due to spawn at this tick (the tick spawns it in the next kit
; itself), and which has no statistics yet (no shot, no hit, no kill, no
; event of any kind and no damage taken from a player in this match), is
; respawned in the next kit where it stands, by the rule's own two calls as
; its tick makes them for a place it respawns (0x316999, 0x3169B9): the
; player out of the world (vtable+24h, 1), then the spawn (rule
; vtable+5Ch) with the place, a respawn point made of the player's feet,
; its yaw and what is left of its spawn protection, and the time. No death
; is counted and no respawn time set.
try_place:
    cmp [ebx+edi*4+0x138], ebp
    je kit_change.no
    imul eax, edi, 0x30
    add eax, [esi+PLACES]
    movzx ecx, byte [eax+0xd]       ; the next kit
    movzx edx, byte [eax+0xc]       ; the kit in use
    cmp ecx, edx
    je kit_change.no
    cmp ecx, 3
    jae kit_change.no
    cmp dword [eax+ecx*4], 0
    je kit_change.no                ; no player for that kit
    mov eax, [eax+edx*4]            ; the player in use
    mov ecx, [eax+ALIVE]
    test cl, cl
    jz kit_change.no                ; dead
    shr ecx, 24
    jz kit_change.no                ; not in the world
    mov edx, [esi+STATISTICS]
    test edx, edx
    jz kit_change.no
    imul ecx, edi, RECORD
    lea edx, [edx+ecx+8]
    push CLEAN
    pop ecx
.clean:
    cmp dword [edx+ecx*4-4], 0
    jne kit_change.no
    loop .clean
    push eax                        ; the player
    lea ecx, [eax+0x108]            ; its damage model, as the spawn finds it (0x316E78)
    mov edx, [ecx]
    call [edx+8]
    mov eax, [eax]
    mov eax, [eax+0x32c]            ; invulnerable until; -1: not (0x2E5550)
    inc eax
    jz .unprotected
    dec eax
    sub eax, ebp
    ja .protected
.unprotected:
    xor eax, eax
.protected:
    pop ecx                         ; the player
    push eax                        ; the point +20h: the protection
    push eax
    push eax
    push dword [ecx+YAW]            ; +14h
    push dword [ecx+FEET+8]         ; +08h: the position
    push dword [ecx+FEET+4]
    push dword [ecx+FEET]
    push eax
    push eax
    mov eax, [ecx]
    push 1
    call [eax+0x24]                 ; out of the world (0x2DA790)
    mov eax, esp
    push ebp                        ; the time
    push eax                        ; the point
    push edi                        ; the place
    mov ecx, ebx
    mov eax, [ebx]
    call [eax+0x5c]                 ; the spawn (0x316DA0)
    add esp, 0x24
    ret
times 0xE0-($-$$) db 0xcc

; game_world_ui (esi; ebp = the local player) when the local player has
; just become alive: at 0x23B7DA `call 0x23A1A0` / `push 0`, then the call
; that shows or hides the kits (0x23B7F5: `mov eax,esi` / `call 0x2383B0`).
; The original hides them; while the match waits or counts down they are
; shown, as on the death screen.
ui_alive:
    call 0x23a1a0                   ; with al and edx as the original set them
    call match_state
    xor ecx, ecx
    dec eax
    shr eax, 1
    setz cl
    push ecx
    jmp 0x23b7f5
times 0x100-($-$$) db 0xcc

; At every update after that (0x23B7FC, `lea eax,[ebp+0D0h]`): kits shown
; to a live player once the match is in process are hidden. eax, ecx and
; edx are not read after it before they are written.
ui_start:
    cmp byte [esi+0x2df], 0         ; the kits shown
    je .done
    cmp byte [ebp+ALIVE], 0
    je .done
    call match_state
    cmp eax, 3
    jb .done
    push 0
    mov eax, esi
    call 0x2383b0
.done:
    lea eax, [ebp+0xd0]
    ret

; eax = the time rule's state in the hud's game (whose world 0x2383B0's
; 0x1F8120 reads as it is).
match_state:
    mov eax, [esi+0x18]             ; the game
    mov eax, [eax+0x4d4c]           ; its world (game_world_core_client)
    mov eax, [eax+CLIENT_CORE+TIME_STATE]
    ret
