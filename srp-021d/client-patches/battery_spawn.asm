; Mode 3 (match_time_vi_spawn) batteries for the 0.21d client.
; See docs/notes/battery_spawn.md. Assemble with
;   nasm -f bin battery_spawn.asm -o battery_spawn.bin
; and apply with patch.py, which takes the new tick, the cave, then the
; 9-byte hook, in that order, from the output.
;
; The original tick keeps the time of the next battery in +164, which the
; rule never serializes, and moves it on by spawn_time once a battery is
; placed. The client rolls back and re-simulates on nearly every remote
; input, so the restored state has the battery unplaced while +164 is
; already past it: no battery ever stays. The new tick derives the time
; from serialized fields only. The deserialize hook takes back batteries
; that a rolled-back simulation placed beyond the restored count.

bits 32

TICK            equ 0x1FC388    ; match_time_vi_spawn_rule::tick(u32, u32 t), ret 8
TICK_SIZE       equ 0xA7
HOOK            equ 0x1FC663    ; match_time_vi_spawn_rule::deserialize, after the
HOOK_BACK       equ 0x1FC66C    ;   rule's fields, before the items mask
CAVE            equ 0x6A1F90    ; the end of the zero padding after .text
CAVE_SIZE       equ 0x70

; The rule's fields, from BASE (rule + 0x110), so they take short offsets.
BASE            equ 0x110
RULE_ITEMS      equ 0x110 - BASE    ; victory_item_core* vector: begin, end (+114)
RULE_SPAWNERS   equ 0x120 - BASE    ; victory_item_spawner* vector, by item
RULE_RANDOM     equ 0x140 - BASE    ; LCG, serialized
RULE_START      equ 0x160 - BASE    ; m_match_start_time_ms, serialized
RULE_AVAILABLE  equ 0x168 - BASE    ; m_available_victory_items_count, serialized
RULE_SPAWN_TIME equ 0x16C - BASE    ; serialized
RULE_VARIATION  equ 0x170 - BASE    ; serialized

ITEM_OWNER      equ 0x8         ; the carrying player
ITEM_SPOTTED    equ 0x1B4       ; teams that see it on the HUD
ITEM_CONTAINER  equ 0x1B8
ITEM_INSERTED   equ 0x1D4
ITEM_INSERT     equ 0x90        ; vtable: insert(float3 const&, float)
ITEM_REMOVE     equ 0x94        ; remove(); nothing when not inserted
ITEM_MOVE       equ 0x98        ; set_transform(float3 const&, float)
SPAWNER_ANGLE   equ 0x10        ; the position is at +0

; The original, as [V 0x1FC388]: a re-roll at each t = start - variation +
; k*spawn_time draws the k-th battery's time from the LCG,
; t + hi(random * 2*variation). This keeps the re-roll and its draw and
; places battery a when the k-th draw is due and a < k, so the same
; batteries come at the same times, and a re-simulation places them again.
section tick start=0 vstart=TICK
tick:
    push    esi
    push    edi
    push    ebx
    lea     esi, [ecx + BASE]
    mov     edi, [esp + 0x14]               ; t
    mov     eax, [esi + RULE_ITEMS + 4]
    sub     eax, [esi + RULE_ITEMS]
    sar     eax, 2
    mov     ebx, [esi + RULE_AVAILABLE]
    cmp     ebx, eax
    jae     .done
    mov     eax, [esi + RULE_VARIATION]
    sub     eax, [esi + RULE_START]
    add     eax, edi
    xor     edx, edx
    div     dword [esi + RULE_SPAWN_TIME]   ; eax = re-rolls so far, edx = since the last
    mov     ecx, edx
    test    edx, edx
    jnz     .drawn
    mov     edx, [esi + RULE_RANDOM]
    imul    edx, edx, 0x8088405
    inc     edx
    mov     [esi + RULE_RANDOM], edx
.drawn:
    cmp     eax, ebx
    jbe     .done
    mov     eax, [esi + RULE_RANDOM]
    mov     edx, [esi + RULE_VARIATION]
    add     edx, edx
    mul     edx
    sub     edx, ecx
    add     edx, edi                        ; the last re-roll + its draw
    cmp     edi, edx
    jb      .done
    mov     eax, [esi + RULE_ITEMS]
    mov     ecx, [eax + ebx*4]
    mov     eax, [esi + RULE_SPAWNERS]
    mov     eax, [eax + ebx*4]
    push    ecx
    fld     dword [eax + SPAWNER_ANGLE]
    fstp    dword [esp]
    push    eax
    mov     edx, [ecx]
    cmp     byte [ecx + ITEM_INSERTED], 0
    jne     .move
    call    [edx + ITEM_INSERT]
    jmp     .placed
.move:
    call    [edx + ITEM_MOVE]
.placed:
    inc     dword [esi + RULE_AVAILABLE]
.done:
    pop     ebx
    pop     edi
    pop     esi
    ret     8
    times TICK_SIZE - ($ - $$) db 0xCC

; From HOOK: [ebp-4] = the rule, its available count just read; eax = the
; items mask, kept. Batteries from the count on go back to unplaced.
section cave start=TICK_SIZE vstart=CAVE
cave_deserialize:
    pushad
    mov     edi, [ebp - 4]
    add     edi, BASE
    mov     ebx, [edi + RULE_AVAILABLE]
.next:
    mov     eax, [edi + RULE_ITEMS + 4]
    sub     eax, [edi + RULE_ITEMS]
    sar     eax, 2
    cmp     ebx, eax
    jae     .done
    mov     eax, [edi + RULE_ITEMS]
    mov     esi, [eax + ebx*4]
    mov     ecx, esi
    mov     eax, [esi]
    call    [eax + ITEM_REMOVE]
    xor     eax, eax
    mov     [esi + ITEM_OWNER], eax
    mov     [esi + ITEM_SPOTTED], eax
    mov     [esi + ITEM_CONTAINER], eax
    inc     ebx
    jmp     .next
.done:
    popad
    mov     edi, [ebp - 4]                  ; the two instructions the hook replaced
    mov     esi, [edi + BASE + RULE_ITEMS]
    jmp     HOOK_BACK
    times CAVE_SIZE - ($ - $$) db 0

section hook start=TICK_SIZE+CAVE_SIZE vstart=HOOK
    jmp     cave_deserialize
    times 4 nop
