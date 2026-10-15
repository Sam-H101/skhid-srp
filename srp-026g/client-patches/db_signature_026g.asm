; The resources.db file table signed by Skhid's key (patch revision 12).
;
;   nasm -f bin -DCAVE=0xA425C00 db_signature_026g.asm -o db_signature_026g.section.bin
;
; vostok::crypt::verify_sign (0x13310) finds the survarium.com certificate in
; the exe's own Authenticode signature and, at 0x135D3, calls
; verify_sign_impl (0x12CB0): eax = the certificate's CERT_CONTEXT, then four
; cdecl arguments (the table, its size, the signature, its size); 0 is a
; good signature, any other value an error (log only, the caller shows
; "Resources archive is corrupted"). verify_sign_impl reads two fields of
; the context, the encoding type (+0) and pCertInfo (+0Ch), and of the
; CERT_INFO only the SubjectPublicKeyInfo (+38h), which it gives to
; CryptImportPublicKeyInfo; then SHA-1 (CALG_SHA1) and CryptVerifySignature.
;
; The call at 0x135D3 comes here instead: verify_sign_impl once, with a
; CERT_CONTEXT made on the stack whose public key is Skhid's
; (resources_db_key_026g.der); the survarium.com certificate in eax is not
; used (the caller still frees it). So a revision 12 game starts only with a
; table Skhid's key signed: a retail table, or one signed by nobody, gets
; verify_sign_impl's error and the game stops as before.
;
; Position independent (call/pop for the key's address), so a rebased image
; still runs it. Uses eax, ecx and edx alone, as a cdecl callee may; the
; caller keeps the certificate in esi.

bits 32
org CAVE

VERIFY_SIGN_IMPL        equ 0x12CB0
ENCODING                equ 0x10001     ; X509_ASN_ENCODING | PKCS_7_ASN_ENCODING
CONTEXT_SIZE            equ 0x14        ; CERT_CONTEXT
INFO_SIZE               equ 0x50        ; CERT_INFO up to the end of SubjectPublicKeyInfo
KEY_INFO                equ 0x38        ; CERT_INFO.SubjectPublicKeyInfo
FRAME                   equ CONTEXT_SIZE + INFO_SIZE

verify_skhid:
        ; CERT_CONTEXT at esp, CERT_INFO at esp+CONTEXT_SIZE.
        sub esp, FRAME
        call .here
.here:  pop ecx
        lea edx, [esp + CONTEXT_SIZE]
        mov dword [esp], ENCODING       ; dwCertEncodingType
        mov dword [esp + 4], 0          ; pbCertEncoded
        mov dword [esp + 8], 0          ; cbCertEncoded
        mov [esp + 0Ch], edx            ; pCertInfo
        mov dword [esp + 10h], 0        ; hCertStore
        lea eax, [ecx + rsa_oid - .here]
        mov [edx + KEY_INFO], eax                       ; Algorithm.pszObjId
        mov dword [edx + KEY_INFO + 4], asn_null_end - asn_null
        lea eax, [ecx + asn_null - .here]
        mov [edx + KEY_INFO + 8], eax                   ; Algorithm.Parameters
        mov dword [edx + KEY_INFO + 0Ch], public_key_end - public_key
        lea eax, [ecx + public_key - .here]
        mov [edx + KEY_INFO + 10h], eax                 ; PublicKey.pbData
        mov dword [edx + KEY_INFO + 14h], 0             ; PublicKey.cUnusedBits
        mov eax, esp
        ; The caller's four arguments, at esp+FRAME+4..+10h.
        push dword [esp + FRAME + 10h]  ; signature size
        push dword [esp + FRAME + 10h]  ; signature
        push dword [esp + FRAME + 10h]  ; table size
        push dword [esp + FRAME + 10h]  ; table
        call VERIFY_SIGN_IMPL
        add esp, 10h + FRAME
        ret

rsa_oid:        db "1.2.840.113549.1.1.1", 0    ; szOID_RSA_RSA
asn_null:       db 05h, 00h                     ; its parameters: NULL
asn_null_end:
public_key:     incbin "resources_db_key_026g.der"      ; DER RSAPublicKey, 2048 bits
public_key_end:
