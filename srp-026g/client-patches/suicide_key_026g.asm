; Patch 34: the commit_suicide key (K) kills one's own player (build 2777).
;   nasm -f bin -DCAVE=0xA425980 suicide_key_026g.asm -o suicide_key_026g.section.bin
; client_patch.md section 3.32.
;
; commit_suicide is game action 1Fh (action table entry 0xA74D20, default key
; kK). A key bound to it already reaches the player input handler's event
; buffer (0x23FEAC), but the input builder 0x241250, which turns the buffered
; actions into the input's action mask (handler+33Ch), never asks for 1Fh,
; and bit 24 is the one bit it never sets. The match simulation still reads
; it: base_player::process_quick_slots (0x2D8BC1, `test dword [edi+4],
; 1000000h`) hits the player's own head bone "brain" through hit_impl
; (0x2D9AE0 -> 0x2D9B90), the attacker being the player. It is the relayed
; input every game applies to every player, so every game does the same.
BITS 32
ORG CAVE

%define ACTION_STATE 0x242460      ; (handler, action, &state) -> al, `ret 0Ch`
%define COMMIT_SUICIDE 0x1F
%define ACTIONS_MASK 0x33C         ; the handler's action mask of the input
%define SUICIDE_BIT 0x1000000      ; bit 24

; 0x242196 `call 0x242460` (spot, the builder's last action; ebx = the
; handler, pushed with action 22h and &state, the caller's [esp+44h]) ->
; `call suicide`. Action 1Fh is asked the same way first; on the key going
; down (state 0, as spot) bit 24 is set. Then the original call goes on with
; the same arguments and return address. When spot is in the buffer it
; writes the state again; when it is not (al = 0) the builder reads no state
; (0x24219D `jz 0x2421B0`), so nothing after it sees this one.
suicide:
    push dword [esp+0x0C]          ; &state
    push COMMIT_SUICIDE
    push ebx
    call ACTION_STATE
    test al, al
    jz .spot
    mov eax, [esp+0x0C]
    cmp dword [eax], 0
    jne .spot
    or dword [ebx+ACTIONS_MASK], SUICIDE_BIT
.spot:
    jmp ACTION_STATE
