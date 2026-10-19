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
; kits before the start, the death screen. The player's handler queues
; each kind it is told with the action (handler+98h, 0x23FF50, 0x242580),
; and action_present (0x242460) answers 1 for an action with any kind
; queued, or one held. The play step (0x241EF1 on) asks three actions for
; a press (kind 0) and takes the rest at any kind: the weapon fires at a
; press, a held state and a release alike, aims the same with the right
; button.
;
; So a button pressed while the cursor was shown (a drag of the warmup
; camera, a pick on the death screen) and still held when the cursor goes
; (the start, the respawn) came to the player's handler as kind 2 at the
; next frame, and its release as kind 1: the weapon fired, without a press
; of its own. Here a button that goes down while the cursor is shown is
; marked; while the cursor is not shown a marked button's kind 2 is taken
; here (an answer not 0) and goes no further, and so is its kind 1, which
; unmarks it. A kind 0 while the cursor is not shown (a fresh press, should
; a release have been missed) unmarks it and is the original's. Anything
; else is the original's. The game world takes a shown cursor's keys as
; before. Nothing a match computes: the player's input only misses the
; held states and the release of a press it never had.
BITS 32
ORG CAVE

%define SHOWN       0x2E0          ; handler: the cursor shown (the game world takes the mouse's keys)
%define FIRST       0x151          ; the mouse's first button (0x3B9CA0)
%define BUTTONS     8              ; codes 151h..158h
%define UP          1              ; kinds of the mouse's keys (0x3B9D90): 0 down, 1 up, 2 held
%define KIND        0Ch            ; [esp+KIND]: the kind (the input world, the code before it)
%define RESUME      0x25CF6A       ; on_mouse_key_action after the ten bytes

; edx is free here, as at any thiscall's entry; eax is the button (0..7),
; so ah is 0.
entry:
        mov     eax, [esp+8]               ; the key's code
        sub     eax, FIRST
        cmp     eax, BUTTONS
        jae     original                   ; a key of the keyboard's, or no button
        call    .here
.here:  pop     edx
        add     edx, STATE - .here         ; the buttons marked
        cmp     byte [esp+KIND], UP
        jb      .press
        je      .release
        bt      dword [edx], eax           ; held: marked?
        jmp     .marked
.release:
        btr     dword [edx], eax           ; let go: unmarked; marked before?
.marked:
        jnc     original                   ; not marked: the original's
        cmp     [ecx+SHOWN], ah
        jne     original                   ; the cursor shown: the game world's, as before
        inc     eax                        ; taken (al 1..8, not 0): no handler after this one
        ret     0Ch
.press:
        cmp     [ecx+SHOWN], ah
        je      .clear                     ; the cursor not shown: a press anew, unmarked
        bts     dword [edx], eax           ; the cursor shown: marked
        jmp     original
.clear:
        btr     dword [edx], eax
original:
        push    ebp                        ; the ten bytes the jump replaced
        push    esi
        db      0x8B, 0xF1                 ; mov esi,ecx in the original's encoding
        mov     ecx, [esp+0Ch]
        mov     eax, [ecx]
        jmp     RESUME
