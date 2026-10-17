; One upgrade (a camouflage, a decal) worn by several items at once: taking
; it off names the item it comes off (build 2777, patch 30,
; client_patch.md section 3.28). Four entries at fixed offsets;
; patch_026g.py checks the hooks' targets. At .srp+700h, after the patches'
; mount (which ends at +6C3h), before the pickup code (+800h).
;   nasm -f bin -DCAVE=0xA425700 shared_upgrades_026g.asm -o shared_upgrades_026g.section.bin
;
; The stock client takes an upgrade off "the item that holds it": the
; lobby's `46 02 <kit> <upgrade>` names no item, although the upgrade window
; that sends it calls drop_upgrade_item(<the item it shows>, <upgrade>)
; (0x29A9D0 reads the item for a log line only), and process_detach_upgrade
; (0x275220), which the server's `83 46 02 <upgrade>` and `83 46 01`'s
; detach_first run, takes it off the first item of the inventory whose
; upgrades hold it (find_if 0x277CF0). With the same upgrade on several
; items both are ambiguous. Here:
;   drop_upgrade   `46 02 <kit> <upgrade> <item>`: the window's item follows
;   detach_answer  `83 46 02 <upgrade> <item>`: that item's upgrade comes off
;   detach_first   `83 46 01`'s detach_first takes the upgrade off the item
;                  the answer puts it on (it was in another slot of it)
;   find_host      process_detach_upgrade looks for that item by its id
; process_detach_upgrade is given the item as a third argument by both its
; callers (each takes it off the stack again after the `ret 8`); an item id
; of 0 is the original's search. Nothing of the kind of upgrade is looked
; at: any upgrade in any slot. The code holds no address (relative calls
; and jumps only), so a rebase changes nothing.
BITS 32
ORG CAVE

new_packet              equ 0x37A0E0    ; eax = client+8, [esp+4] = id; `ret 4`
resize                  equ 0x07AB30    ; eax = buffer, [esp+4] = size; `ret 4`
enqueue                 equ 0x379640    ; [esp+4] = client+8, [esp+8] = packet; `ret 8`
process_detach_upgrade  equ 0x275220    ; [esp+4] = lobby client, [esp+8] = upgrade; `ret 8`
first_holder            equ 0x277CF0    ; cdecl (items, items end, &id): the original's find_if
DETACH_ANSWER_DONE      equ 0x275765    ; the 83 46 02 case's epilogue
ITEM_SIZE               equ 0xB4        ; an inventory item (lobby client+1EA978h: begin, end)
BUFFER                  equ 0x2BC       ; a packet's mutable buffer: +4 data, +0Ch size
HOST_ARG                equ 0x58        ; find_host's [esp+58h]: process_detach_upgrade's third argument

; drop_upgrade_item's handler 0x29A9D0, at 0x29AAD5: replaces `push edi /
; push ecx / call 0x2708C0` (the lobby client in eax, the kit in ecx, the
; upgrade in edi) with a call here and two nops. 0x2708C0 wrote `46 02
; <kit> <upgrade>`; this writes the same and then the call's first
; argument, the item, read from the Flash arguments as the handler reads
; them ([ebp+8] -> +8 -> +8; the upgrade is +20h). The handler then pops
; its registers and leaves: every register is kept but eax, ecx and edx.
drop_upgrade:
    mov edx, [ebp+8]
    mov edx, [edx+8]
    mov edx, [edx+8]                ; the item the window shows
    push ebx
    push esi
    push edi
    push ebp
    push edx                        ; [esp+8] after the packet's id is taken
    push edi                        ; [esp+4]
    push ecx                        ; [esp]
    lea edi, [eax+8]
    push 0x46
    mov eax, edi
    call new_packet
    mov esi, eax
    mov eax, [esi+BUFFER]
    mov ebp, [eax+0xC]              ; the size so far
    lea ecx, [ebp+13]
    push ecx
    call resize                     ; eax = the buffer still
    mov eax, [esi+BUFFER]
    mov eax, [eax+4]
    add eax, ebp
    mov byte [eax], 2
    pop ecx
    mov [eax+1], ecx                ; the kit
    pop ecx
    mov [eax+5], ecx                ; the upgrade
    pop ecx
    mov [eax+9], ecx                ; the item
    push esi
    push edi
    call enqueue
    pop ebp
    pop edi
    pop esi
    pop ebx
    ret

    times 0x60 - ($ - $$) db 0xCC

; process_succeeded_inventory_action's `83 46 02` case, at 0x275756:
; replaces `mov ecx,[eax]` / `push ecx` / `add eax,4` / `push ebp` /
; `mov [esi+4],eax` / `call process_detach_upgrade` (eax = the read
; position, esi = the reader, ebp = the lobby client) with a jump here and
; ten nops. The upgrade and then its item are read; the case's epilogue
; follows.
detach_answer:
    mov ecx, [eax]                  ; the upgrade
    mov edx, [eax+4]                ; the item it comes off
    add eax, 8
    mov [esi+4], eax
    push edx
    push ecx
    push ebp
    call process_detach_upgrade
    add esp, 4
    jmp DETACH_ANSWER_DONE

    times 0x80 - ($ - $$) db 0xCC

; The `83 46 01` case, at 0x275589: replaces `push edi / push ebp / call
; process_detach_upgrade` (ebx = the item, edi = the upgrade, ebp = the
; lobby client), which runs when detach_first is set, with a call here
; and two nops: the upgrade comes off the item it is put on.
detach_first:
    push ebx
    push edi
    push ebp
    call process_detach_upgrade
    add esp, 4
    ret

    times 0x90 - ($ - $$) db 0xCC

; process_detach_upgrade at 0x27534D: replaces `call 0x277CF0` (cdecl:
; the inventory's begin and end and the upgrade's id; returns the item or
; end), the search of the item to take the upgrade off. Here it is the
; item whose id the caller gave (end if there is none); with 0, the
; original's search. The function then takes the upgrade off that item
; if it holds it (0x2775B0), as before.
find_host:
    mov edx, [esp+HOST_ARG]
    test edx, edx
    jz .first
    mov eax, [esp+4]
    mov ecx, [esp+8]
.next:
    cmp eax, ecx
    je .done
    cmp [eax], edx
    je .done
    add eax, ITEM_SIZE
    jmp .next
.done:
    ret
.first:
    jmp first_holder
