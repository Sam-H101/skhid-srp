; Patch 31: the warmup camera starts in front of the player, looking at
; them, not at their back (build 2777, client_patch.md section 3.29; the
; rules: ../docs/notes/kit-change-before-start.md). One entry, called from
; switch_to_warmup_camera's store of the mode (WARMUP_FACE_HOOK in
; patch_026g.py).
;   nasm -f bin -DCAVE=0xA4257C0 warmup_face_026g.asm -o warmup_face_026g.section.bin
;
; The warmup camera (the player's input handler in mode 3, set by
; switch_to_warmup_camera 0x1F8190 alone) looks at the player from its yaw
; (+34Ch) and pitch (+350h) about the player's own facing, at its distance
; (0x2422B0: the player's matrix turned by the yaw, then the pitch); a yaw
; of 0 is behind the back. The constructor puts both at 0 (0x23FDD7), and
; only process_warmup_mode (0x242200) changes them, by a drag. A player
; spawns with a wall often behind, so the camera began in the wall. Modes 1
; and 2 place the camera from the same yaw without turning it; they are left
; as they were.
;
; switch_to_warmup_camera has cl 1 when the camera changes (another mode
; before, or a change not yet played, +35Ch) and stores it at +35Ch; then
; `mov [eax+358h],esi` (esi = 3), the six bytes this call replaced. Here a
; change also puts the yaw at pi and the pitch at 0: the camera in front,
; level. A second call while the mode is already 3 and played (cl 0) keeps
; the drag. Keeps every register; the flags are not read after the call.
BITS 32
ORG CAVE

%define YAW         0x34C          ; player's input handler: the warmup camera's yaw
%define PITCH       0x350          ; and its pitch
%define MODE        0x358
%define PI          0x40490FDB     ; 3.14159265f

warmup_face:
        mov     [eax+MODE], esi            ; the six bytes the call replaced
        test    cl, cl
        jz      .kept                      ; the warmup camera already, played: the drag kept
        mov     dword [eax+YAW], PI        ; in front of the player
        and     dword [eax+PITCH], 0       ; level (0.0f)
.kept:
        ret
