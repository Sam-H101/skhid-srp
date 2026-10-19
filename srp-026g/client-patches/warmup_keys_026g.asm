; Patch 31: a mouse button held from before the start is not the player's
; until it is let go (build 2777, client_patch.md section 3.29; the rules:
; ../docs/notes/kit-change-before-start.md). One entry, jumped to from the
; first ten bytes of the game world's on_mouse_key_action
; (WARMUP_KEYS_HOOK in patch_026g.py); its state, the buttons marked (one
; bit each), at STATE (WARMUP_KEYS_STATE_OFFSET of the late join's state
; page), found from its own address.
;   nasm -f bin -DCAVE=0xA4259A8 -DSTATE=0xA428E30 warmup_keys_026g.asm -o warmup_keys_026g.section.bin
;
; The input world tells its handlers of the mouse's buttons each frame, by
; priority, until one answers 1 (vostok::input::platform::mouse::process
; 0x3B9D90; the buttons held now +10h, the frame before +20h): a button
; gone down, kind 0 and then 2; one let go, kind 1; one still held, kind 2,
; at every frame. The button's code is 151h + its bit (0x3B9CA0); the left
; one 151h. The game world's handler (game world+1F8h, priority 7) comes
; before the player's input handler (priority 10); its on_mouse_key_action
; (0x25CF60, ecx = the handler; the input world, the code, the kind on the
; stack, `ret 0Ch`) takes them (1) while the cursor is shown (+2E0h): the
; kits before the start, the death screen. The player's handler keeps each
; kind it is told as the action's state (handler+98h, 0x23FF50), and
; action_present (0x242460) makes a state 2 a held action: the weapon fires
; while the left button's state is 2, aims while the right one's is.
;
; So a button pressed while the cursor was shown (a drag of the warmup
; camera, a pick on the death screen) and still held when the cursor goes
; (the start, the respawn) came to the player's handler as kind 2 at the
; next frame: the weapon fired, without a press of its own. Here a button
; that goes down while the cursor is shown is marked; while the cursor is
; not shown a marked button's kind 2 is taken here (1) and goes no
; further; its kind 1 (or a kind 0, should a release have been missed)
; unmarks it and is the original's, so the release reaches the player's
; handler as before. Anything else is the original's. The game world takes
; a shown cursor's keys as before. Nothing a match computes: the player's
; input only misses held states of a press it never had.
BITS 32
ORG CAVE

%define SHOWN       0x2E0          ; handler: the cursor shown (the game world takes the mouse's keys)
%define FIRST       0x151          ; the mouse's first button (0x3B9CA0)
%define BUTTONS     8              ; codes 151h..158h
%define UP          1              ; kinds of the mouse's keys (0x3B9D90): 0 down, 1 up, 2 held
%define HELD        2
%define RESUME      0x25CF6A       ; on_mouse_key_action after the ten bytes

entry:
        mov     eax, [esp+8]               ; the key's code
        sub     eax, FIRST
        cmp     eax, BUTTONS
        jae     original                   ; a key of the keyboard's, or no button
        push    edx
        call    .here
.here:  pop     edx
        add     edx, STATE - .here         ; the buttons marked
        cmp     byte [ecx+SHOWN], 0
        je      .hidden
        cmp     byte [esp+4+0Ch], UP       ; the cursor shown (the kind is 0, 1 or 2):
        ja      .back                      ; held: nothing
        je      .clear                     ; a release unmarks
        bts     dword [edx], eax           ; a press marks
        jmp     .back
.hidden:
        bt      dword [edx], eax
        jnc     .back                      ; not marked: the original's
        cmp     byte [esp+4+0Ch], HELD
        je      take                       ; held since before: not the player's
.clear:
        btr     dword [edx], eax           ; let go (or pressed anew): unmarked, the original's
.back:
        pop     edx
original:
        push    ebp                        ; the ten bytes the jump replaced
        push    esi
        db      0x8B, 0xF1                 ; mov esi,ecx in the original's encoding
        mov     ecx, [esp+0Ch]
        mov     eax, [ecx]
        jmp     RESUME
take:
        pop     edx
        mov     al, 1                      ; taken: no handler after this one
        ret     0Ch
