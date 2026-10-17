; Patch 31: a kit chosen before the match starts is played at once (build
; 2777, client_patch.md section 3.29; the rules: ../docs/notes/
; kit-change-before-start.md). Four entries at fixed offsets (KIT_CHANGE_*
; in patch_026g.py, which checks each hook's target). The profile_changed
; callback's hiding of the kits after a pick (0x233F28) is nops
; (KIT_PICK): the kits stay shown, and the hud's cursor where it is, for
; another pick, before the start and on the death screen alike.
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
%define SHOWN_STATE 0x1F1C60       ; network client: the time rule's state the camera was last set for
%define IN_PROCESS  3

; The respawn rule's tick (0x316820; ecx = the rule, [esp+8] the tick's
; time; thiscall, three arguments, `ret 0Ch`) begins here, at every tick of
; every game and every replay of a tick. It runs in the tick list 0, before
; the places' ticks (the late join's binds give the places the keys
; (place+1) << 16, the rules have the core's own counter, from 0), which take
; the kit choices of this tick (0x321790): it acts on the next kit as the
; tick before left it. The time rule ticks after both (list 1) and may make
; the state 3 at this very tick: so a state of 1 or 2 here means play has
; not begun before this tick. Each place is tried (try_place); then the
; three instructions the jump replaced.
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
; them. A place whose next kit is another than the kit in use, whose player
; is alive and in the world (so not due to spawn: a respawn time is set only
; for a dead player, and a player entering is not in the world yet), and
; which has no statistics yet (no shot, no hit, no kill, no event of any
; kind and no damage taken from a player in this match), is respawned in the
; next kit where it stands, by the rule's own two calls as its tick makes
; them for a place it respawns (0x316999, 0x3169B9): the player out of the
; world (vtable+24h, 1), then the spawn (rule vtable+5Ch) with the place, a
; respawn point made of the player's feet, its yaw and what is left of its
; invulnerability, and the time. No death is counted and no respawn time
; set.
try_place:
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
times 0xD8-($-$$) db 0xcc

; The client's view of a spawn into another kit (0x2678E0, slot +60h of the
; client's respawn rule; esi = the network client, eax = the game world),
; after it has made the new kit's player the local one: `push 1` / `call
; 0x1F8200` (the player camera, first person, the input's first person
; mode) at 0x26794D. As the time rule's change does (0x27B1C8, called from
; its tick 0x267AA0 for a tick first played), the camera and the input are
; the player's in play (state 3) and the warmup's otherwise, by the state
; the network client keeps for them (+1F1C60h), which only a first play of
; a tick changes: a rollback that plays a spawn again after play began
; leaves the player in play.
kit_view:
    cmp dword [esi+SHOWN_STATE], IN_PROCESS
    jne .warmup
    push 1
    call 0x1f8200
    ret
.warmup:
    mov edx, eax
    jmp 0x1f8190                    ; switch_to_warmup_camera(edx)
times 0xF0-($-$$) db 0xcc

; game_world_ui (esi; ebp = the local player) when the local player has
; just become alive: at 0x23B7DA `call 0x23A1A0` / `push 0` / ... /
; `call 0x2383B0` (the call that shows or hides the kits, 0x23B7F5). The
; original hides them; while the camera is the warmup's (the state the
; network client set it for is not 3) they are shown, as on the death screen.
ui_alive:
    call 0x23a1a0                   ; with al and edx as the original set them
    call kits_until_play
    jmp 0x23b7fc
times 0xFF-($-$$) db 0xcc

; At every update (0x23B7FC, `lea eax,[ebp+0D0h]`): a live local player has
; the kits shown before play and hidden in play (0x2383B0 returns at once
; when they are as asked). eax, ecx and edx are not read after it before
; they are written.
ui_start:
    cmp byte [ebp+ALIVE], 0
    je .done
    call kits_until_play
.done:
    lea eax, [ebp+0xd0]
    ret

; esi = the hud: the kits shown when the state the network client of the
; hud's game set the camera for is not 3, hidden when it is (0x2383B0 with
; eax = the hud and that flag in the low byte, which alone it reads).
kits_until_play:
    mov eax, [esi+0x18]             ; the game world
    mov eax, [eax+0x9c]             ; the game
    mov eax, [eax+0x53ac]           ; the network client (0x23B74F)
    cmp dword [eax+SHOWN_STATE], IN_PROCESS
    setne al
    push eax
    mov eax, esi
    call 0x2383b0
    ret
