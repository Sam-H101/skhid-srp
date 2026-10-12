; Option B for the 0.21d client: upload the world snapshot to srp.
; See docs/notes/option_b.md. Assemble with
;   nasm -f bin snapshot_upload.asm -o snapshot_upload.bin
; and apply with patch.py, which takes the cave, then the two 6-byte hooks,
; in that order, from the output.

bits 32

SERIALIZE_ITEM  equ 0x1FDE3E    ; game_state_history_item::serialize(writer&, mask); eax = item
BUFFER_WRITE    equ 0x155EBE    ; buffer_writer::w(writer*, src, n)
ALLOCA_PROBE    equ 0x2B3F00    ; __alloca_probe_16; eax = size
HAS_HORIZON     equ 0x1DCBE6    ; pvp_match_core::is_there_event_horizon_item; eax = pvp core
HORIZON_ITEM    equ 0x1CAB13    ; game_world_core::event_horizon_history_item; eax = core
READY_HOOK      equ 0x1A27A8    ; network_client::on_match_ready, before the 0x42 send
REQUEST_HOOK    equ 0x1A16B1    ; on_match_packet_received, 0x59 without -debug_hash_mismatches
REQUEST_DONE    equ 0x1A1721    ; skips the rest of the message and returns

NC_PVP_CORE     equ 0x7A00      ; network_client -> pvp_match_core
NC_MATCH_CLIENT equ 0x7A08      ; network_client -> match_client (vtbl +0x18 new_packet, +0x1C enqueue)
NC_STATE        equ 0x7A1C      ; 2 = static info received
PVP_CORE        equ 0x138       ; pvp_match_core -> game_world_core
CORE_NEWEST     equ 0x2678      ; newest history item
CORE_MASKS      equ 0x5ED0      ; players-mask history; head at +8, node {+4 next, +8 mask, +0xC time}
ITEM_TIME       equ 0xB9938
PACKET_WRITER   equ 0x294

SNAPSHOT_MSG    equ 0x47        ; reliable, unordered; srp tells it from a 4-byte leave by length
CHUNK           equ 480         ; 12 header + 1 id + 8 chunk header + 480 = 501 <= 508
BUFFER_SIZE     equ 0x8000      ; the same stack buffer hash() serializes into

section cave start=0 vstart=0x6A1E0B align=1

; From READY_HOOK: ebx = network_client. Uploads the initial world that
; game_world_core::serialize() has just built, then sends 0x42 as before.
cave_ready:
    pushad
    mov     edi, ebx
    mov     eax, [edi + NC_PVP_CORE]
    mov     eax, [eax + PVP_CORE]
    mov     esi, [eax + CORE_NEWEST]
    test    esi, esi
    jz      .done
    call    upload
.done:
    popad
    lea     esi, [ebx + NC_MATCH_CLIENT]   ; the instruction the hook replaced
    jmp     READY_HOOK + 6

; From REQUEST_HOOK: edi = network_client. srp asked for a snapshot with 0x59;
; upload the event-horizon item, the one every client reports in 0x46.
cave_request:
    pushad
    cmp     dword [edi + NC_STATE], 2
    jne     .done
    mov     eax, [edi + NC_PVP_CORE]
    test    eax, eax
    jz      .done
    call    HAS_HORIZON
    test    al, al
    jz      .done
    mov     eax, [edi + NC_PVP_CORE]
    mov     eax, [eax + PVP_CORE]
    call    HORIZON_ITEM
    mov     esi, eax
    call    upload
.done:
    popad
    jmp     REQUEST_DONE

; esi = history item, edi = network_client. Sends the item as SNAPSHOT_MSG
; chunks: u32 item_time, u16 offset, u16 total, then up to CHUNK bytes.
%define item     ebp - 0x04
%define client   ebp - 0x08
%define buffer   ebp - 0x0C
%define total    ebp - 0x10
%define offset   ebp - 0x14
%define packet   ebp - 0x18
%define header   ebp - 0x20         ; u32 time, u16 offset, u16 total
%define mbuffer  ebp - 0x30         ; mutable_buffer {allocator, data, capacity, size}
%define writer   ebp - 0x48         ; buffer_writer {0, 0, 0, 0, u8 0xFF, mutable_buffer*}
upload:
    push    ebp
    mov     ebp, esp
    sub     esp, 0x50
    mov     [item], esi
    mov     [client], edi
    mov     eax, BUFFER_SIZE
    call    ALLOCA_PROBE
    mov     [buffer], esp

    xor     eax, eax
    mov     [mbuffer], eax
    mov     ecx, [buffer]
    mov     [mbuffer + 4], ecx
    mov     dword [mbuffer + 8], BUFFER_SIZE
    mov     [mbuffer + 0xC], eax
    mov     [writer], eax
    mov     [writer + 4], eax
    mov     [writer + 8], eax
    mov     [writer + 0xC], eax
    mov     byte [writer + 0x10], 0xFF
    lea     ecx, [mbuffer]
    mov     [writer + 0x14], ecx

    ; The players mask at the item's time, as hash() takes it (0 if none yet).
    mov     ecx, [esi + ITEM_TIME]
    mov     [header], ecx
    mov     eax, [edi + NC_PVP_CORE]
    mov     eax, [eax + PVP_CORE]
    mov     eax, [eax + CORE_MASKS + 8]
    xor     edx, edx
.find_mask:
    test    eax, eax
    jz      .serialize
    cmp     [eax + 0xC], ecx
    jbe     .found_mask
    mov     eax, [eax + 4]
    jmp     .find_mask
.found_mask:
    mov     edx, [eax + 8]
.serialize:
    push    edx
    lea     ecx, [writer]
    push    ecx
    mov     eax, [item]
    call    SERIALIZE_ITEM

    mov     eax, [mbuffer + 0xC]
    mov     [total], eax
    mov     [header + 6], ax
    and     dword [offset], 0
.next_chunk:
    mov     eax, [offset]
    cmp     eax, [total]
    jae     .sent
    mov     [header + 4], ax

    mov     eax, [client]
    mov     ecx, [eax + NC_MATCH_CLIENT]
    mov     eax, [ecx]
    push    SNAPSHOT_MSG
    call    [eax + 0x18]
    mov     [packet], eax

    push    8
    lea     ecx, [header]
    push    ecx
    add     eax, PACKET_WRITER
    push    eax
    call    BUFFER_WRITE

    mov     eax, [total]
    sub     eax, [offset]
    cmp     eax, CHUNK
    jbe     .write_chunk
    mov     eax, CHUNK
.write_chunk:
    push    eax
    mov     ecx, [buffer]
    add     ecx, [offset]
    push    ecx
    mov     ecx, [packet]
    add     ecx, PACKET_WRITER
    push    ecx
    call    BUFFER_WRITE

    mov     eax, [client]
    mov     ecx, [eax + NC_MATCH_CLIENT]
    mov     eax, [ecx]
    push    dword [packet]
    call    [eax + 0x1C]

    add     dword [offset], CHUNK
    jmp     .next_chunk
.sent:
    mov     esp, ebp
    pop     ebp
    ret

section ready_hook follows=cave vstart=READY_HOOK align=1
    jmp     cave_ready
    nop

section request_hook follows=ready_hook vstart=REQUEST_HOOK align=1
    jmp     cave_request
    nop
