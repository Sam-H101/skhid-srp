; UV scrolling for the transparent effects (forward_default) as 0.29 has it
; (build 2777, patch revision 13, README patch 28, client_patch.md section
; 3.26). Three entries at fixed offsets, which patch_026g.py's hooks jump to;
; the .bin ends with the address it was assembled for, which patch_026g.py
; checks.
;   nasm -f bin -DCAVE=0xA425B00 forward_scroll_026g.asm -o forward_scroll_026g.section.bin
;
; effect_fstage_default_materials::compile (0x1D8F40) builds a material's
; forward effect: it reads the material's switches into a 16-byte
; shader_configuration on its stack (esp 16-aligned by its prologue, the
; configuration at [esp+50h]), calls effect_material_base::compile_begin
; (0x1D9126, "vertex_base_z_shifted", "forward_base", &configuration), then
; sets the shader's textures and constants. 0.29's compile (0x7F2600 of
; 0.29a3) also reads use_uv_scrolling, unless use_sequence is on, into bit 6
; of configuration+0Ch, and when that bit is set it sends
;   uv_scrolling_parameters0 = (0, 0, transparency_map_scroll_u_direction,
;                                     transparency_map_scroll_v_direction)
;   uv_scrolling_parameters1 = (emissive_map_scroll_u_direction,
;                               emissive_map_scroll_v_direction, 0, 0)
; (a value the material does not have is 0) after the uv distortion's
; constants. 0.26g's own g_stage effect sets the same bit from the same switch
; (0x1DE51F..0x1DE533) and sends the same two constants (0x1E0815); 0.26g's
; shader_macros::fill_shader_configuration_macros (0x3C9B80) turns that bit
; into CONFIG_USE_UV_SCROLLING (0x3C9F51). The shader key takes the macro
; only when shaders/masks lists it for the shader: forward_base's entry of
; the retail resources.db does not, so with the retail data the bit changes
; no shader the game loads, and a binding the shader has no constant for is
; skipped (shader_constant_table::apply_bindings 0x3D7F40).
;
; forward_scroll_switch, from 0x1D90DC (after use_angle_depended_attenuation
; was read into al, before compile_begin): sets the bit, then does the two
; instructions the jump replaced and goes back.
; forward_scroll_constants, from 0x1D94CB (the end of the uv distortion's
; block, reached from both of its ends): when the bit is set, sends the two
; constants with set_constant<float4> (0x1A8900, ecx = &value, then the
; compiler and the name: `ret 8`), then does the two instructions the jump
; replaced and goes back.
; scene_slomo, from 0x14990F in renderer_context's constructor (0x148570),
; after its last register_constant_binding (scene_time, 0x1498A6..0x14990A):
; binds the constant scene_slomo to a 1.0 of this code, as 0.29 binds it next
; to scene_time (0x74275F of 0.29a3); 0.29's shaders scroll by scene_time *
; scene_slomo. The same calls as the game's for scene_time.
;
; Every register and the flags the game reads after each jump are as the
; game had them (pushad / popad; the flags are written again before they are
; read). The image may be rebased: every address of the exe and of this code
; is taken plus the delta in ebp; calls and jumps into the exe are relative.
BITS 32
ORG CAVE

; The exe's functions.
KEY_EXISTS          equ 0x0F6C00    ; eax = the node, esi = the name: al = whether it has that key
CHILD               equ 0x0F6B10    ; eax = the node, edi = the name: eax = that key's node (it must exist)
SET_FLOAT4          equ 0x1A8900    ; ecx = &float4, push name, push compiler; ret 8
SHARED_STRING       equ 0x0E0B50    ; push the text: eax = its shared string or 0; ret 4
STRING_DESTROY      equ 0x0E0AE0    ; push a shared string whose count is 0; ret 4
REGISTER_BINDING    equ 0x3BCCC0    ; eax = &binding, push the resource manager; ret 4
; The exe's data and texts.
RESOURCE_MANAGER    equ 0x226047C   ; the renderer's resource manager (a pointer in .data)
S_VALUE             equ 0x9E1C48    ; "value"
S_USE_UV_SCROLLING  equ 0x9F8E04    ; "use_uv_scrolling"
S_EMISSIVE_U        equ 0x9F971C    ; "emissive_map_scroll_u_direction"
S_EMISSIVE_V        equ 0x9F973C    ; "emissive_map_scroll_v_direction"
S_PARAMETERS0       equ 0x9F975C    ; "uv_scrolling_parameters0"
S_PARAMETERS1       equ 0x9F9778    ; "uv_scrolling_parameters1"
; Where the game goes on.
SWITCH_RETURN       equ 0x1D90E3
CONSTANTS_RETURN    equ 0x1D94D2
SLOMO_RETURN        equ 0x149919

; compile's frame (no push of its own pending at either hook).
CFG                 equ 0x50        ; the shader_configuration
CFG_SEQUENCE        equ CFG + 4     ; bit 4: use_sequence (set at 0x1D906C)
CFG_SCROLLING       equ CFG + 0x0C  ; bit 6: use_uv_scrolling (CONFIG_USE_UV_SCROLLING)
PUSHAD_SIZE         equ 0x20
PUSHAD_EBP          equ 8           ; pushad's ebp: compile's frame

forward_scroll_switch:                  ; +00h, jmp from 0x1D90DC; ebx = the material
    test byte [esp+CFG_SEQUENCE], 0x10
    jnz .replaced                       ; use_sequence: no scrolling, as in 0.29
    pushad
    call .pc
.pc:
    pop ebp
    sub ebp, .pc                        ; ebp = the rebase delta, 0 if none
    lea esi, [ebp+S_USE_UV_SCROLLING]
    call find_value
    test eax, eax
    jz .done
    cmp dword [eax], 0                  ; a switch, as compile reads its own (0x1D9016)
    je .done
    or byte [esp+PUSHAD_SIZE+CFG_SCROLLING], 0x40
.done:
    popad
.replaced:
    mov dl, [esp+0x56]
    mov esi, [ebp+0x10]
    jmp SWITCH_RETURN

    times 0x40 - ($ - $$) db 0xCC
forward_scroll_constants:               ; +40h, jmp from 0x1D94CB; ebx = the material
    test byte [esp+CFG_SCROLLING], 0x40
    jz .replaced
    pushad
    call .pc
.pc:
    pop ebp
    sub ebp, .pc                        ; ebp = the rebase delta, 0 if none
    sub esp, 0x20                       ; [esp] parameters0, [esp+10h] parameters1
    xorps xmm0, xmm0
    movups [esp], xmm0
    movups [esp+0x10], xmm0
    lea esi, [ebp+transparency_u]
    call read_number
    movss [esp+0x08], xmm0
    lea esi, [ebp+transparency_v]
    call read_number
    movss [esp+0x0C], xmm0
    lea esi, [ebp+S_EMISSIVE_U]
    call read_number
    movss [esp+0x10], xmm0
    lea esi, [ebp+S_EMISSIVE_V]
    call read_number
    movss [esp+0x14], xmm0
    mov edi, [esp+0x20+PUSHAD_EBP]
    mov edi, [edi+8]                    ; compile's first argument: the effect compiler
    lea eax, [ebp+S_PARAMETERS0]
    push eax
    push edi
    lea ecx, [esp+0x08]
    call SET_FLOAT4                     ; keeps ebx, esi, edi, ebp
    lea eax, [ebp+S_PARAMETERS1]
    push eax
    push edi
    lea ecx, [esp+0x18]
    call SET_FLOAT4
    add esp, 0x20
    popad
.replaced:
    mov cl, [esp+0x57]
    and cl, 3
    jmp CONSTANTS_RETURN

    times 0x100 - ($ - $$) db 0xCC
scene_slomo:                            ; +100h, jmp from 0x14990F; ebx = the renderer context
    pushad
    call .pc
.pc:
    pop ebp
    sub ebp, .pc                        ; ebp = the rebase delta, 0 if none
    ; The binding, as the game makes scene_time's on its stack: the value's
    ; address, its size, the name, 0, the size. 20h keeps the stack's
    ; alignment for the call as the game's own has it.
    sub esp, 0x20
    lea eax, [ebp+one]
    mov [esp], eax
    mov dword [esp+0x04], 4
    and dword [esp+0x08], 0
    and dword [esp+0x0C], 0
    mov dword [esp+0x10], 4
    lea eax, [ebp+slomo_name]
    push eax
    call SHARED_STRING
    test eax, eax
    jz .named
    mov [esp+0x08], eax
    lock inc dword [eax]                ; held while the binding is made (0x1498CF)
.named:
    mov eax, esp
    push dword [ebp+RESOURCE_MANAGER]
    call REGISTER_BINDING
    mov eax, [esp+0x08]
    test eax, eax
    jz .done
    lock dec dword [eax]                ; let go (0x149900)
    jnz .done
    push eax
    call STRING_DESTROY
.done:
    add esp, 0x20
    popad
    mov [esp+0x2C], edi
    lea esi, [ebx+0x123C]
    jmp SLOMO_RETURN

; eax = the "value" node of the material's key named by esi, 0 when the
; material has no such key. ebx = the material, ebp = the delta; ecx, edx
; and edi are lost.
find_value:
    mov eax, ebx
    call KEY_EXISTS
    test al, al
    jz .none
    mov edi, esi
    mov eax, ebx
    call CHILD
    lea edi, [ebp+S_VALUE]
    call CHILD
    ret
.none:
    xor eax, eax
    ret

; xmm0 = the material's number named by esi, 0.0 when it has none: a float,
; or an integer made a float, as compile reads its numbers (0x1D9249).
read_number:
    call find_value
    xorps xmm0, xmm0
    test eax, eax
    jz .done
    cmp word [eax+0x14], 2
    jne .integer
    movss xmm0, [eax]
    ret
.integer:
    cvtsi2ss xmm0, dword [eax]
.done:
    ret

    align 4, db 0xCC
one:            dd 1.0                  ; scene_slomo's value, read only
transparency_u: db "transparency_map_scroll_u_direction", 0
transparency_v: db "transparency_map_scroll_v_direction", 0
slomo_name:     db "scene_slomo", 0
    align 4, db 0xCC
                dd CAVE                 ; where this was assembled for
