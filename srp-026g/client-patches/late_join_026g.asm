; Experimental late join, build 2777. Requires the growing-roster dialect.
bits 32
%ifndef CAVE
%error CAVE is required
%endif
org CAVE

; eax=core, [esp+4]=slot, ret 4. Used for initial and appended slots.
; Separate ID ranges keep world IDs independent of the initial roster size.
bind_slot:
    jmp bind_slot_reserved

times 96-($-$$) db 0xcc
; Construct both banks of twenty histories; preserve the live roster count.
history_capacity:
    mov esi, 20
    lea ecx, [ebp+0xe3e4]
    jmp 0x2d390e

times 128-($-$$) db 0xcc
; D8's stock debug flag is 00 or FF. Normalize it before advertising the
; experimental dialect, so debug clients send 81 and ordinary clients 80.
connect_capability:
    and bl, 1
    or bl, 0x80
    mov [eax+edi], bl
    mov eax, [esp+0x1c]
    jmp 0x27a244

times 192-($-$$) db 0xcc
; cdecl(bytes, length, uninitialized_profile_storage, dictionary) -> bool.
; Validation runs first: a malformed descriptor never touches the output.
; Construct in its final location: items have pointers into their own storage.
; Called by the background-loader handler and final metadata installation.
decode_profile:
    push ebp
    mov ebp, esp
    push ebx
    push esi
    push edi
    sub esp, 12
    push dword [ebp+20]
    push dword [ebp+12]
    push dword [ebp+8]
    call C_CODE
    add esp, 12
    test eax, eax
    jz .done
    mov ecx, [ebp+16]
    call 0x2f7c50
    mov eax, [ebp+8]
    mov [esp], eax
    mov [esp+4], eax
    mov eax, [ebp+12]
    mov [esp+8], eax
    mov esi, esp
    mov edi, [ebp+16]
    push dword [ebp+20]
    call 0x1f1070
    mov eax, 1
.done:
    add esp, 12
    pop edi
    pop esi
    pop ebx
    pop ebp
    ret

times 320-($-$$) db 0xcc
; cdecl(context, game_world, spatial_tree, dictionary) -> query id.
; Requests only usable kits. The resource engine copies the stack variants;
; their profile pointer refers to the runtime's retained context storage.
query_players:
    push ebp
    mov ebp, esp
    push ebx
    push esi
    push edi
    sub esp, 0x120
    xor eax, eax
    mov edi, esp
    mov ecx, 0x48
    rep stosd
    mov esi, [ebp+8]
    call .pc
.pc:
    pop ebx
    sub ebx, .pc                    ; executable relocation delta
    call 0x205ba0                   ; native user-data type id
    mov [ebp-0x11c], eax
.kit:
    mov ecx, [ebp-0x118]
    bt [esi+44], ecx                ; lj_context.kit_mask
    jnc .next
    mov edx, [ebp-0x114]           ; compact request index
    lea eax, [ebx+0xa09f48]         ; gameplay/players/default.player
    mov [ebp+edx*8-0x40], eax
    mov dword [ebp+edx*8-0x3c], 0x54
    imul edi, edx, 48
    lea edi, [ebp+edi-0xe0]
    mov [ebp+edx*4-0xf0], edi
    lea eax, [ebx+0xa09f90]         ; native variant vtable
    mov [edi], eax
    lea eax, [esi+60]              ; immutable profile
    mov [edi+8], eax
    mov eax, [esi+24]              ; place
    mov [edi+12], al
    mov eax, [ebp+12]
    mov [edi+16], eax              ; game world
    mov eax, [ebp+16]
    mov [edi+20], eax              ; spatial tree
    mov eax, [ebp+20]
    mov [edi+24], eax              ; dictionary
    mov [edi+28], cl               ; kit, spectator byte remains zero
    mov [edi+40], edi
    mov eax, [ebp-0x11c]
    mov [edi+44], eax
    inc dword [ebp-0x114]
.next:
    inc dword [ebp-0x118]
    cmp dword [ebp-0x118], 3
    jb .kit
    push dword 0
    push esi
    push dword 0
    lea eax, [ebx+query_completed]
    push eax
    lea edi, [ebp-0x110]
    call 0x27d280                  ; native boost member callback, ret 16
    push dword 1                  ; normal asynchronous query
    push dword 0                  ; no parent query
    lea eax, [ebp-0xf0]
    push eax
    push dword [ebx+0x1f1d9c4]
    lea eax, [ebp-0x110]
    push eax
    lea eax, [ebp-0x40]
    mov ecx, [ebp-0x114]
    call 0xe5b60
    add esp, 20
    add esp, 0x120
    pop edi
    pop esi
    pop ebx
    pop ebp
    ret

times 768-($-$$) db 0xcc
; Native boost member callback: ecx=context, [esp+4]=queries_result.
; queries_result owns its resources through the return from this callback.
query_completed:
    push ebp
    mov ebp, esp
    push esi
    push edi
    sub esp, 12
    mov esi, ecx
    mov edi, [ebp+8]
    add edi, 0x12c
    xor eax, eax
    mov [esp], eax
    mov [esp+4], eax
    mov [esp+8], eax
.kit:
    bt [esi+44], eax
    jnc .next
    mov edx, [edi]
    mov [esp+eax*4], edx
    add edi, 0x2e0
.next:
    inc eax
    cmp eax, 3
    jb .kit
    mov eax, esp
    push eax
    push esi
    call LJ_COMPLETE
    add esp, 8
    add esp, 12
    pop edi
    pop esi
    pop ebp
    ret 4

times 896-($-$$) db 0xcc
; cdecl(size): the game's allocator and debug label, ret 4 in the native ABI.
allocate_context:
    push esi
    call .pc
.pc:
    pop edx
    sub edx, .pc
    mov esi, [edx+0x1f1d9c4]
    lea eax, [edx+context_label]
    push eax
    mov eax, [esp+12]
    call 0xe4f90
    pop esi
    ret

times 960-($-$$) db 0xcc
; cdecl(pointer). Native free also clears the supplied local pointer.
free_context:
    call .pc
.pc:
    pop ecx
    sub ecx, .pc
    mov ecx, [ecx+0x1f1d9c4]
    mov eax, [esp+4]
    lea edx, [esp+4]
    push edx
    call 0x1539d0
    add esp, 4
    ret

times 1024-($-$$) db 0xcc
retain_world:
    mov eax, [esp+4]
    lock inc dword [eax+0xd0]
    ret

times 1056-($-$$) db 0xcc
release_world:
    lea ecx, [esp+4]
    jmp 0x11540

context_label: db 'srp late-join profile', 0

times 1120-($-$$) db 0xcc
install_context:
    push ebp
    mov ebp, esp
    call .pc
.pc:
    pop edx
    sub edx, .pc
    lea eax, [edx+0x321790]
    push eax
    lea eax, [edx+player_ready]
    push eax
    lea eax, [edx+bind_animations]
    push eax
    lea eax, [edx+bind_physics]
    push eax
    lea eax, [edx+bind_core]
    push eax
    mov eax, esp
    push eax
    push dword [ebp+8]
    call LJ_INSTALL
    add esp, 28
    test eax, eax
    jz .done
    ; The newcomer has its place: the lists of players made at the match's
    ; start (the scoreboard among them) are made again, with it in them.
    push eax
    mov ecx, [ebp+8]
    mov edx, [ecx+8]              ; context.client
    mov eax, [edx+0x1f1c48]
    add eax, 0xc                  ; the options (the profiles and their count), as
    push eax                      ; the client's vtable+34h gives them (0x219990)
    mov edx, [edx+0x14]           ; game; its game_world is at +18, the hud at +1fc of that
    add edx, 0x214
    push edx                      ; both as 0x1F8542..0x1F855D pass them
    call rebuild_lists
.rebuild_return:
    pop eax
.done:
    pop ebp
    ret
bind_core:
    mov eax, [esp+4]
    push dword [esp+8]
    call bind_slot
    ret
bind_physics:
    mov eax, [esp+4]
    push dword [esp+8]
    call 0x3218a0
    ret
bind_animations:
    push edi
    mov edi, [esp+8]
    push dword [esp+12]
    call register_animations
    pop edi
    ret
player_ready:
    push esi
    mov esi, [esp+8]
    call 0x1f29d0
    pop esi
    ret

times 1312-($-$$) db 0xcc
query_context:
    mov ecx, [esp+4]
    mov edx, [ecx+8]              ; context.client
    mov edx, [edx+0x14]           ; game, its game_world is embedded at +18
    push dword [edx+0x53a4]
    mov eax, [ecx+12]             ; context.world = pvp_match_core
    push dword [eax+0x130]        ; pvp.spatial_tree (constructor 0x2d567b)
    add edx, 0x18
    push edx
    push ecx
    call query_players
    add esp, 16
    ret

times 1408-($-$$) db 0xcc
; cdecl(client, seed, serial, place, status). A full packet pool means retry
; on the server's next Prepare; allocating from an exhausted pool is fatal.
send_prepared:
    push ebp
    mov ebp, esp
    push ebx
    push esi
    push edi
    sub esp, 24
    mov ebx, [ebp+8]
    cmp dword [ebx+0x1f1c64], 2
    jne .done
    mov ecx, [ebx+0x1f1c48]
    test ecx, ecx
    jz .done
    mov eax, [ebp+12]
    cmp [ecx+0x577bc], eax
    jne .done
    mov edx, [ecx+0x578b0]
    mov eax, [edx+0x30]
    add eax, 17
    cmp eax, [edx+0x34]
    ja .done
    mov dword [ebp-36], 0xffffffff
    mov dword [ebp-32], 0x0000ffff
    mov eax, [ebp+12]
    mov [ebp-28], eax
    mov eax, [ebp+16]
    mov [ebp-24], eax
    mov eax, [ebp+20]
    mov [ebp-20], al
    mov eax, [ebp+24]
    mov [ebp-19], al
    mov word [ebp-18], 1
    mov eax, [ecx]
    push 0xe0
    call [eax+0x18]
    mov esi, eax
    and byte [eax+0x98], 0xbf
    lea edx, [ebp-36]
    push edx
    add eax, 0x2a8
    push eax
    mov edi, 20
    call 0x7ac10
    mov ecx, [ebx+0x1f1c48]
    mov eax, [ecx]
    push esi
    call [eax+0x1c]
.done:
    add esp, 24
    pop edi
    pop esi
    pop ebx
    pop ebp
    ret

times 1664-($-$$) db 0xcc
; on_match_ready, before the existing initial-snapshot upload hook. ebx=NC.
ready_hook:
    pushad
    call initialize_api
    mov esi, eax
    mov edi, [ebx+0x1f1c40]
    test edi, edi
    jz .done
    cmp [esi+40], ebx
    jne .activate
    cmp [esi+44], edi
    je .done
.activate:
    mov edx, [edi+0x124]
    movzx eax, byte [edx+0x577b5]
    push eax
    push dword [edx+0x577b0]
    mov eax, [ebx+0x14]
    push dword [eax+0x53a4]
    push edi
    push ebx
    push esi
    call LJ_ACTIVATE
    add esp, 24
.done:
    popad
    jmp UPLOAD_READY

times 1856-($-$$) db 0xcc
; E4 case of on_match_packet_received: esi=NC, ebx=reader.
prepare_hook:
    cmp dword [esi+0x1f1c64], 1
    je .initial
    cmp dword [esi+0x1f1c64], 2
    jne .end
    pushad
    call get_state
    cmp [eax+40], esi
    jne .done
    mov ecx, [ebx+4]
    cmp ecx, [ebx]
    jb .done
    mov edx, [ebx]
    add edx, [ebx+8]
    jc .done
    sub edx, ecx
    jc .done
    push edx
    push ecx
    push eax
    call LJ_PREPARE
    add esp, 12
.done:
    popad
.end:
    jmp 0x27a5ab
.initial:
    pushad
    call get_state
    push eax
    call LJ_DEACTIVATE
    add esp, 4
    popad
    jmp 0x27a385

times 2048-($-$$) db 0xcc
; Replace the call to player::~player in its scalar deleting destructor.
; Save the profile before destruction and notify only after native cleanup.
player_destroyed_hook:
    push ebp
    mov ebp, esp
    push ebx
    push esi
    mov ebx, ecx
    mov esi, [ecx+0x6b8]
    call 0x1f2100
    push eax
    call get_state
    push esi
    push ebx
    push eax
    call LJ_DESTROYED
    add esp, 12
    pop eax
    pop esi
    pop ebx
    pop ebp
    ret

times 2176-($-$$) db 0xcc
initialize_api:
    push ebx
    call .pc
.pc:
    pop ebx
    sub ebx, .pc
    call get_state
    cmp dword [eax+8], 0
    jne .done
%assign field 8
%macro api 1
    lea edx, [ebx+%1]
    mov [eax+field], edx
%assign field field+4
%endmacro
    api allocate_context
    api free_context
    api decode_profile
    api retain_world
    api release_world
    api query_context
    api install_context
    api send_prepared
.done:
    pop ebx
    ret

times 2304-($-$$) db 0xcc
get_state:
    call .pc
.pc:
    pop eax
    add eax, STATE_VA-.pc
    ret

times 2336-($-$$) db 0xcc
; on_match_disconnected, immediately before releasing NC's PVP reference.
disconnect_hook:
    pushad
    call get_state
    cmp [eax+40], edi
    jne .done
    push eax
    call LJ_DEACTIVATE
    add esp, 4
.done:
    popad
    mov ecx, [edi+0x1f1c40]
    jmp 0x27b51d

times 2432-($-$$) db 0xcc
; network_client destructor: retire before destroying match_client or PVP.
client_destroyed_hook:
    pushad
    call get_state
    cmp [eax+40], ecx
    jne .done
    push eax
    call LJ_DEACTIVATE
    add esp, 4
.done:
    popad
    push ebp
    mov ebp, esp
    and esp, -8
    jmp 0x219e66

%ifdef TEST_SESSION
times 2560-($-$$) db 0xcc
; Test-only startup: EDI=game, EAX=NC; stack=(host,port,spectator), ret 12.
direct_match:
    push ebp
    mov ebp, esp
    pushad
    push dword [ebp+16]
    push dword [ebp+12]
    push dword [ebp+8]
    call 0x1ed6e0
    cmp dword [ebp+16], 0
    jne .done
    mov ecx, [edi+0x53ac]
    test ecx, ecx
    jz .done
    mov dword [ecx+0x1ee7ec], TEST_SESSION
    push TEST_SESSION
    push 1
    push dword [ebp+12]
    push dword [ebp+8]
    call 0x21beb0
.done:
    popad
    leave
    ret 12
%endif

times 2816-($-$$) db 0xcc
bind_slot_reserved:
    push ebp
    mov ebp, esp
    push ebx
    push esi
    sub esp, 12
    mov ebx, eax
    mov esi, [ebp+8]
    movzx eax, byte [esi+0x2c]
    inc eax
    shl eax, 16
    mov [ebp-16], eax
    mov eax, [ebx+0x18fb0]
    mov [ebp-12], eax
    lea eax, [ebp-16]
    mov [ebx+0x18fb0], eax
    ; Six weapon IDs per player: three kits, two weapons each.
    mov eax, [ebx+0x18b4c]
    mov edx, [ebx+0x18b50]
    lea ecx, [eax+120*4]
    cmp edx, ecx
    cmovb edx, ecx
    mov [ebp-20], edx
    movzx edx, byte [esi+0x2c]
    imul edx, 24
    add eax, edx
    mov [ebx+0x18b50], eax
    push esi
    mov eax, ebx
    call 0x321820
    mov eax, [ebp-20]
    mov [ebx+0x18b50], eax
    mov eax, [ebp-12]
    mov [ebx+0x18fb0], eax
    add esp, 12
    pop esi
    pop ebx
    pop ebp
    ret 4

times 3072-($-$$) db 0xcc
; Native ABI: EDI=player, stack=(animation table), ret 4.
register_animations:
    push dword [esp+4]
    call 0x2d8100
    push ebx
    call .pc
.pc:
    pop ebx
    sub ebx, .pc-CAVE
    lea eax, [ebx+animation_less-CAVE]
    push eax
    lea eax, [ebx+animation_append-CAVE]
    push eax
    push dword [esp+16]
    call LJ_ANIMATIONS
    add esp, 12
    pop ebx
    ret 4

animation_less:
    push edi
    mov edi, [esp+12]
    push dword [esp+8]
    call 0x2ef060
    movzx eax, al
    pop edi
    ret

animation_append:
    push esi
    mov esi, [esp+8]
    lea eax, [esp+12]
    push eax
    call 0x2ef410
    pop esi
    ret

times 3328-($-$$) db 0xcc
; The hud's lists of players: the scoreboard (root.list_set_players), the
; second list (root.set_players_list) and the enemies' markers
; (root.set_enemy_icon_ids). The game makes them once, in 0x234880 when the
; match is ready, for the players there are then (the loops end at the
; count, options+577B5h); nothing makes them again, so the players of a
; match did not see a newcomer in their scoreboard.
; Stack=(hud, options), ret 8, as 0x234880 itself. This builds that
; function's frame and goes into it where the lists are made (0x234E71);
; lists_hook takes it out again, through the function's own release of
; the two arrays and its own return. Nothing else of the function runs:
; not its subscription, not root.reset, not the hud's items.
rebuild_lists:
    mov eax, [esp+4]
    mov ecx, [esp+8]
    ; Only the hud this match's start made, with its two movies.
    cmp [eax+0x1c], ecx
    jne .not_ours
    cmp dword [eax+8], 0
    je .not_ours
    cmp dword [eax+0x10], 0
    je .not_ours
    sub esp, 0x5d4
    push ebx
    push ebp
    push esi
    push edi
    mov ebp, [esp+0x5e8]          ; the hud, as 0x234888 has it
    lea ebx, [ebp+8]              ; as 0x234AC8
    mov [esp+0x98], ebx           ; as 0x234AE1
    xor eax, eax
    mov [esp+0xdc], eax           ; two values the function's end releases
    mov [esp+0x160], eax          ; when they hold something: they hold nothing
    jmp 0x234e71
.not_ours:
    ret 8

times 3456-($-$$) db 0xcc
; 0x23572B in 0x234880, right after root.set_enemy_icon_ids, the last of
; the three lists: `mov ecx,[esp+98h]`. A run that rebuild_lists began (its
; return address is the one in install_context) ends here.
lists_hook:
    call .pc
.pc:
    pop ecx
    add ecx, install_context.rebuild_return-.pc
    cmp [esp+0x5e4], ecx
    jne .start_of_match
    xor esi, esi                  ; as the function's end has it
    jmp 0x2363ca
.start_of_match:
    mov ecx, [esp+0x98]
    jmp 0x235732

times 3520-($-$$) db 0xcc
; 0x39E52 in loose_oct_tree::insert 0x39E30 (the scene's and the collision
; world's tree), right after the object gave its box: eax -> min xyz, max
; xyz, six floats; edi = the object, which keeps a copy at +4. A box with a
; number in it that is no number or not finite is outside the tree, and
; update_bounds 0x396F0 then doubles the tree's size without end (the size
; to reach works out as infinity): it takes a node each time until none is
; left and reads through a null one (0x399A8; seen live 2026-10-13 in a
; game that was given a newcomer, when a model in the scene was moved to
; such a place). Such a box is taken as a box of two metres at the origin;
; the next move of the object puts it where it is.
box_hook:
    xor ecx, ecx
.next:
    mov edx, [eax+ecx*4]
    and edx, 0x7f800000
    cmp edx, 0x7f800000
    je .not_a_box
    inc ecx
    cmp ecx, 6
    jb .next
.copy:
    movq xmm0, [eax]
    movq [edi+4], xmm0
    jmp 0x39e5b
.not_a_box:
    call .pc
.pc:
    pop eax
    add eax, .box-.pc
    jmp .copy
align 4, db 0xcc
.box: dd -1.0, -1.0, -1.0, 1.0, 1.0, 1.0

times 4096-($-$$) db 0xcc
