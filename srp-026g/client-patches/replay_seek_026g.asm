; A replay's user seek empties the weapons' sound queues (build 2777,
; patch revision 11, client_patch.md section 3.24). One entry, at the
; start; patch_026g.py checks the hook's target.
;   nasm -f bin -DCAVE=0xA425A00 replay_seek_026g.asm -o replay_seek_026g.section.bin
BITS 32
ORG CAVE

; The replay's seek 0x1EE2E0 (esi=game, edi=target; only a replay has the
; reader it starts from), after set_time_in_ms(target) and only for a user
; seek (`test bl,bl` / `jnz`): replaces `lea eax,[edi-3E8h]` at 0x1EE3A3.
; Every client weapon of every place's three kits gets its delayed sounds
; (weapon+9FCh) and its fx history (weapon+828h) emptied as ~weapon 0x243790
; does: the old timeline's sounds and the re-simulation's burst are dropped.
seek_clear:
    pushad
    call .pc
.pc:
    pop ebp
    add ebp, 0xa0422c-.pc           ; the client weapon's item vtable, rebased
    mov eax, [esi+0x4d64]           ; the world
    push dword [eax+0x18654]        ; places end
    mov ebx, [eax+0x18650]          ; first place, 30h each
.place:
    cmp ebx, [esp]
    jae .done
    xor ecx, ecx                    ; kit 0..2
.kit:
    mov eax, [ebx+ecx*4]
    test eax, eax
    jz .next_kit
    mov eax, [eax+0x10c]            ; the player's inventory
    test eax, eax
    jz .next_kit
    xor edi, edi                    ; slot 0..22
.slot:
    mov esi, [eax+edi*4+0x10c]      ; an item: weapon+10h
    test esi, esi
    jz .next_slot
    cmp [esi], ebp
    jne .next_slot                  ; not a client weapon
    push eax
    push ecx
    lea edx, [esi+0x9f0]            ; &end (weapon+A00h)
    push edx
    mov eax, [esi+0x9ec]            ; begin (weapon+9FCh)
    call 0x245c50                   ; destroys the functors, plays nothing
    add esp, 4
    mov eax, [esi+0x9ec]
    mov [esi+0x9f0], eax            ; end = begin
    mov eax, [esi+0x81c]
    mov [esi+0x818], eax            ; fx history: write index = oldest
    pop ecx
    pop eax
.next_slot:
    inc edi
    cmp edi, 23
    jb .slot
.next_kit:
    inc ecx
    cmp ecx, 3
    jb .kit
    add ebx, 0x30
    jmp .place
.done:
    pop eax
    popad
    lea eax, [edi-0x3e8]            ; what the jump replaces
    jmp 0x1ee3a9
