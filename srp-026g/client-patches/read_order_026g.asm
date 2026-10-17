; Patch 33: the order of the archive reads is a strict weak ordering (build
; 2777, client_patch.md section 3.31). This replaces the body of
; vostok::resources::hdd_manager_sorting_predicate::operator() (0x10BCC0,
; 0x1A0 bytes to the next function, 0x10BE60) in place: the same entry, the
; same convention, int3 after the code. patch_026g.py checks the bytes it
; replaces.
;   nasm -f bin read_order_026g.asm -o read_order_026g.bin
;
; hdd_manager::grab_sorted_queries (0x10BE80) takes the pending read queries
; (query_result, linked by +270h) into an array on its stack and sorts them
; with std::sort (introsort 0x10BF70, final insertion sort 0x10C060), whose
; partition (0x10C0B0) and insertion (0x10C280) are unguarded: they rely on
; the predicate being a strict weak ordering. The original predicate (the
; same in 0.21d, 0xCD023, where its name is known) is not one when a batch
; holds reads of two archives (resources.db and a resources.db.patch_<n>
; mounted over it):
;   1. a query whose iterator (+A4h) is null first, nulls by address;
;   2. the same +158h (the parent query, 0 for none): +2B8h descending;
;   3. both nodes in an archive (node +30h bit 4; node = +A8h, else +A4h) and
;      the same archive's path: the file offset (get_file_offs 0x47160);
;   4. else +158h, or the query itself when it is 0.
; Two reads of one archive go by offset, a read of another archive by step
; 4: three reads can make a cycle, and the unguarded loops then read past
; the array (round 4d: the word before a 58-slot buffer, 0, read as a query).
;
; Here, lexicographically:
;   1. a null iterator first, nulls by address          (the original's 1)
;   2. +2B8h descending                                 (the original's 2)
;   3. a read from an archive before one that is not
;   4. archive reads: the archive's physical path, then the file offset
;      (the original's 3); others: +158h or the query (the original's 4).
; Reads of one archive keep the original's order (within a priority, in file
; order); a batch whose queries of different priorities share +158h, all of
; one archive, sorts as the original.
BITS 32
ORG 0x10BCC0

%define GET_PATH        0x4BC10     ; vfs_iterator::get_physical_path: [esp+4] the node, edi = the path's buffer
%define STRCMP          0x3B5A0     ; strcmp_s: eax, ecx -> eax -1, 0, 1 (a null is "")
%define GET_OFFS        0x47160     ; get_file_offs: ecx = the node -> edx:eax
%define ITERATOR        0xA4        ; query_result: the iterator's node (0: none)
%define FILE_NODE       0xA8        ; query_result: the file's node (0: the iterator's)
%define PARENT          0x158
%define PRIORITY        0x2B8
%define IN_ARCHIVE      4           ; node +30h

; eax = A, [esp+4] = B: al = A before B. Keeps ebx, esi, edi, ebp; `ret 4`.
less:
    sub esp, 0x250
    push ebx
    push ebp
    push esi
    push edi
    mov esi, eax                    ; A
    mov ebx, [esp+0x264]            ; B
    mov ebp, [esi+ITERATOR]
    mov ecx, [ebx+ITERATOR]
    test ebp, ebp
    jnz .a_set
    test ecx, ecx
    jnz .true                       ; null before set
    cmp esi, ebx
    jmp .below                      ; two nulls: by address
.a_set:
    test ecx, ecx
    jz .false
    mov eax, [ebx+PRIORITY]
    cmp eax, [esi+PRIORITY]
    jb .true                        ; A's priority is higher
    ja .false
    mov [esp+0x18], ecx             ; B's iterator node
    mov eax, [esi+FILE_NODE]
    test eax, eax
    jnz .a_file
    mov eax, ebp
.a_file:
    mov edx, [ebx+FILE_NODE]
    test edx, edx
    jnz .b_file
    mov edx, ecx
.b_file:
    mov [esp+0x10], eax             ; A's file node
    mov [esp+0x14], edx             ; B's file node
    mov al, [eax+0x30]
    mov dl, [edx+0x30]
    and al, IN_ARCHIVE
    and dl, IN_ARCHIVE
    cmp al, dl
    je .same_kind
    test al, al
    jnz .true                       ; A in an archive, B not
    jmp .false
.same_kind:
    test al, al
    jz .loose
    lea edi, [esp+0x20]             ; A's path
    push ebp
    call GET_PATH
    add esp, 4
    lea edi, [esp+0x138]            ; B's path
    push dword [esp+0x18]
    call GET_PATH
    add esp, 4
    mov eax, [esp+0x20]
    mov ecx, [esp+0x138]
    call STRCMP
    test eax, eax
    js .true
    jnz .false
    mov ecx, [esp+0x10]
    call GET_OFFS
    mov edi, eax
    mov ebp, edx                    ; A's offset
    mov ecx, [esp+0x14]
    call GET_OFFS                   ; B's
    cmp ebp, edx
    jb .true
    ja .false
    cmp edi, eax
    jmp .below
.loose:
    mov eax, [esi+PARENT]
    test eax, eax
    jnz .a_key
    mov eax, esi
.a_key:
    mov edx, [ebx+PARENT]
    test edx, edx
    jnz .b_key
    mov edx, ebx
.b_key:
    cmp eax, edx
.below:
    jb .true
.false:
    xor eax, eax
    jmp .done
.true:
    mov eax, 1
.done:
    pop edi
    pop esi
    pop ebp
    pop ebx
    add esp, 0x250
    ret 4

times 0x1A0-($-$$) db 0xCC
