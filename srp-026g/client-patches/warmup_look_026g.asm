; Patch 31: the warmup camera before the start, while the kits are shown, as
; the lobby's character view: a drag turns it, the wheel brings it nearer or
; further (build 2777, client_patch.md section 3.29; the rules:
; ../docs/notes/kit-change-before-start.md). One entry, jumped to from the
; first ten bytes of the game world's on_mouse_move (WARMUP_LOOK_HOOK in
; patch_026g.py); its state, 12 bytes at STATE (WARMUP_LOOK_STATE_OFFSET of
; the late join's state page), found from its own address; the end of a drag
; and the camera's limits are warmup_limits_026g.asm at LIMITS.
;   nasm -f bin -DCAVE=0xA425D40 -DSTATE=0xA428E20 -DLIMITS=0xA425A98 warmup_look_026g.asm -o warmup_look_026g.section.bin
;
; The input world gives a mouse move to its handlers by priority, lowest
; first, until one answers 1 (0x3B9F10): the game world's (game world+1F8h,
; priority 7) before the player's input handler (game world+4D58h, priority
; 10), whose warmup mode turns the warmup camera by the moves it adds up and
; moves it by the wheel (0x240200, 0x242200). The game world's on_mouse_move
; (0x25D0E0, ecx = the handler; the input world, x, y, z on the stack, `ret
; 10h`) moves the hud's cursor (game world vtable+30h) and answers whether
; the cursor is shown; the kits show it with them (0x2383B0 -> 0x23A930), so
; before the start every move was the cursor's and the camera stood still.
;
; The lobby (lobby_camera, vtable 0xA09EB4, priority 10, before the lobby
; menu's 100; on_mouse_move 0x283D30): while one of the mouse buttons 2, 5
; or 7 is held (52h of the buttons held, the mouse's +10h), a move turns the
; character's camera and is not the menu's, so the cursor stays; the first
; move after the release puts the system's cursor back where the drag began
; (0x29A680). Its wheel moves the distance by 0.001 a unit, nearer when the
; wheel turns forward, as the player's handler does (+348h).
;
; Here, while the kits are shown and the player's handler is in the warmup
; mode (+358h = 3, which the warmup camera 0x1F8190 alone sets: before
; play), at every move (the mouse is the input world's +28h):
; - a button of the lobby's held: a drag at once; the left button (the
;   kits' click) held: a drag once its moves since it went down add up to
;   more than CLICK (|x| + |y|), the cursor's before that, so that a click
;   stays a click. A drag begins by keeping the hud's pointer's place and
;   taking the pointer off the screen (game world vtable+30h to -128,-128):
;   a press on a kit is let go outside it and picks nothing. In a drag the
;   move is not the cursor's and goes on (0) to the player's handler: the
;   camera turns (and the wheel moves it).
; - no button held: a drag ends with the pointer back (LIMITS+0, as the
;   lobby). A move of the wheel goes on to the player's handler with no
;   turn (the mouse's x and y of the frame 0): the camera moves nearer or
;   further. Any other move is the original's: the cursor's (1).
; Any other time it is the original's. Nothing a match computes: the warmup
; mode gives the player's input no turn.
BITS 32
ORG CAVE

%define KITS        0x2E3          ; handler: the kits shown (hud+2DFh, 0x2383B0); the cursor with them
%define INPUT       0x4B60         ; handler: the player's input handler (game world+4D58h)
%define WORLD       -0x1F8         ; handler: the game world
%define POINTER     -0x16C         ; handler: the hud's pointer, x and y (game world+8Ch, +90h)
%define MODE        0x358          ; player's input handler: its mode
%define WARMUP      3              ; the warmup mode (0x1F8190)
%define LEFT        0x01           ; the mouse's +10h: the buttons held
%define LOBBY       0x52           ; the lobby camera's buttons (0x283D81)
%define CLICK       8              ; a left press moved no more is a click
%define AWAY        -128           ; the hud's pointer in a drag

; The state: +0 the moves of the buttons held (above CLICK: a drag); +4,
; +8 the hud's pointer when the drag began.
%define S_TRAVEL    0
%define S_POINTER   4

; The first four instructions of 0x25D0E0 (`push ebx` / `push esi` / `mov
; esi,ecx` / `mov ecx,[esi+4B70h]`) jump here.
warmup_look:
    cmp byte [ecx+KITS], 0
    je .original
    mov eax, [ecx+INPUT]
    test eax, eax
    jz .original
    cmp dword [eax+MODE], WARMUP
    je .warmup
.original:
    push ebx
    push esi
    mov esi, ecx
    mov ecx, [esi+0x4b70]
    jmp 0x25d0ea
.cursor:
    mov ecx, esi
    pop edi
    pop esi
    pop ebx
    jmp .original
.warmup:
    push ebx
    push esi
    push edi
    mov esi, ecx                    ; the handler
    call .here
.here:
    pop ebx
    add ebx, STATE - .here          ; the state
    mov edi, [esp+12+4]             ; the input world
    mov edi, [edi+0x28]             ; its mouse: +4 x, +8 y, +0Ch the wheel
    mov al, [edi+0x10]
    mov ecx, [ebx+S_TRAVEL]
    test al, LOBBY | LEFT
    jz .up
    cmp ecx, CLICK
    ja .turn                        ; in a drag
    test al, LOBBY
    jnz .drag                       ; the lobby's buttons: at once
    mov eax, [edi+4]                ; the left button: |x| + |y| since it went down
    cdq
    xor eax, edx
    sub eax, edx
    add ecx, eax
    mov eax, [edi+8]
    cdq
    xor eax, edx
    sub eax, edx
    add ecx, eax
    mov [ebx+S_TRAVEL], ecx
    cmp ecx, CLICK
    jbe .cursor                     ; still a click
.drag:
    or dword [ebx+S_TRAVEL], -1
    movq xmm0, [esi+POINTER]
    movq [ebx+S_POINTER], xmm0
    push 0
    push AWAY
    push AWAY
    lea ecx, [esi+WORLD]
    mov eax, [ecx]
    call [eax+0x30]                 ; the hud's pointer off the screen
.turn:
    xor al, al                      ; on to the player's handler
    pop edi
    pop esi
    pop ebx
    ret 0x10
.up:
    call LIMITS                     ; the end of a drag
    xor eax, eax
    cmp [edi+0xc], eax
    je .cursor
    mov [edi+4], eax                ; the wheel alone on to the player's handler
    mov [edi+8], eax
    jmp .turn
