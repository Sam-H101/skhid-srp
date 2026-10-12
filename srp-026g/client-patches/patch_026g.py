"""Builds the srp copy of the 0.26g client (build 2777) from the original
survarium.exe. The full patch includes the startup VRAM-check bypass used
by the Linux launcher.

USAGE (this script never touches the source exe):

    cd srp-026g/client-patches
    python patch_026g.py <game>/binaries/x86/survarium.exe <game>/binaries/x86/survarium_srp.exe
        full patch, upload code in a new PE section (the default)
    python patch_026g.py <survarium.exe> <survarium_srp.exe> --cave=hash-mismatch
        full patch, upload code over game_world_core::on_hash_mismatch
        (the fallback if a client with one more section misbehaves)
    python patch_026g.py <survarium.exe> <survarium_srp.exe> --cert-only
        only the certificate patch (what step 0 installed)
    python patch_026g.py <survarium.exe> --check [--cave=...] [--cert-only]
        every check and patch in memory, nothing written
    python selftest_026g.py
        builds every variant, re-parses and disassembles it, writes the
        results to SRP_026G_SCRATCH (never the game folder)

    <survarium.exe> must be the ORIGINAL 0.26g exe (the sha256 is checked);
    the output must be another file. Close the game before replacing an exe
    that is in use. The upload code comes from snapshot_upload_026g.asm:
        nasm -f bin -DCAVE=0xA425000 snapshot_upload_026g.asm -o snapshot_upload_026g.section.bin
        nasm -f bin -DCAVE=0x325A60  snapshot_upload_026g.asm -o snapshot_upload_026g.hash-mismatch.bin
    (both .bin files are checked in; --bin=<file> names another one).

Port of ../srp-021d/client-patches/patch.py; the evidence for every address
is in docs/notes/client_patch.md.

Patches:
- the code-signing certificate check (0x12A10 -> `xor eax,eax; ret`). At
  start the client takes the survarium.com certificate from its own
  Authenticode signature, asks Windows whether its chain is valid now, and
  only then verifies the resources.db file table signature with its key.
  The certificate expired on 2015-06-06, so the chain is "not time valid"
  and every stock 0.26g client stops with "Resources archive is
  corrupted". The patch skips only the chain check; the file table
  signature is still verified, so the signature blob must stay readable
  (see --cave=section below);
- the single-instance check (0x124F9 `jz fail` -> two nops), so several
  clients run on one PC;
- the minimum VRAM check (0x106411 `jae` -> `jmp`), skipping the fatal
  warning when detected video memory is below 320 MiB. The detected size
  and the original fallback for an unknown size are kept;
- the default of -hash_check_interval_in_ms, 100 -> 50 (the imm32 of
  `mov dword [esp+18h],64h` at 0x27CC4F, one byte at 0x27CC53). A stock
  client reports its world hash (0xDF) for every second 50 ms tick, on a
  phase of its own; two clients on opposite phases never name the same
  tick, and the match server then cannot compare their worlds (match.md
  section 14). With 50 every client reports every tick. The key on the
  command line still overrides it;
- the battery rule of modes 2 and 3 (gather_victory_items_rule_core's
  container event handler 0x313A50): two instructions become nops. When a
  team's count reaches the number of batteries the handler stores the
  completion time (+0x15C) and clears the "containers can be used" byte
  (+0x160); from then on it counts no event (`jnz` at 0x313A62) and no
  container can be used (0x31CB16). Neither field is in the rule's
  serialize/deserialize, so a rollback takes the tenth battery out of the
  container again and leaves both as they were: the replay of the same
  tick cannot put it in, and nobody can use a container for the rest of
  the match (9 of 9 live matches in which a team had nine; match.md
  section 26). Without the two instructions every event is counted and
  the containers stay usable, so the replay ends as the first run did.
  The match's end is the server's (it reads the points);
- the option B snapshot upload: two 5-byte jumps (0x27AEA1 in
  on_match_ready, 0x27A505 in the 0xF6 case of on_match_packet_received)
  into the upload code, and the upload code itself:
  --cave=section (default): a new section `.srp` (RVA 0xA415000, VA
    0xA425000, 0x200 bytes of file) after .reloc. The section table gets an
    8th entry (40 of the 216 free header bytes), SizeOfImage grows by one
    page, and the Authenticode blob moves 0x200 bytes towards the end of
    the file, where the security data directory finds it again. Nothing of
    the original code is lost: -debug_hash_mismatch keeps its function
    (but the 0xF6 branch that called it now uploads instead);
  --cave=hash-mismatch: the code overwrites game_world_core::on_hash_mismatch
    (0x325A60, 0x47B bytes, reachable only through the branch the request
    hook replaces); the rest of it becomes int3. No header changes;
- base relocations that overlap rewritten bytes become no-op (type 0)
  entries, so a rebase could not corrupt the new code;
- the PE checksum is recomputed (full patch only; --cert-only keeps the
  file byte-identical to the copy installed in step 0).
The 0.21d mode 3 battery patch is NOT ported: 0.26g serializes the
next-battery time (match_time_vi_spawn_rule_core +0x16C), the 0.21d bug.

Each site is checked against the original bytes first, so a different build
or an already patched file is refused rather than patched twice.
"""

import hashlib
import os
import struct
import sys

IMAGE_BASE = 0x10000
TEXT_VA, TEXT_FILE = 0x11000, 0x400          # same in 0.26g as in 0.21d
TEXT_VSIZE = 0x85E5A3
EXE_SHA256 = "ac33d9937015c2299bf62cc6b127cb308b48a3475b30fff2ddd65479ae3bf153"

# vostok::crypt chain check of the signing certificate: sub esp,2Ch -> xor eax,eax; ret
CERT = (0x12A10, bytes.fromhex("83ec2c"), bytes.fromhex("31c0c3"))

# survarium_or_its_installer_is_already_running: cmp eax,0B7h / jz fail
MUTEX = (0x124F9, bytes.fromhex("7409"), bytes.fromhex("9090"))

# Skip the fatal low-VRAM warning after cmp ebx,14000000h (320 MiB).
# Same one-byte change as play_skhid.sh: file offset 0xF5811, 73 -> EB.
# The branch still targets 0x106422, which returns the detected size.
MEMORY_CHECK = (0x106411, bytes.fromhex("730f"), bytes.fromhex("eb0f"))

# network_client::try_send_all, the one-time read of the option object
# 0xAA19F8 (-hash_check_interval_in_ms) into the static 0xA2B393C:
# mov dword [esp+18h],64h (the default, kept when the key is absent) -> 32h.
# The whole instruction is checked; only the imm32's low byte changes.
HASH_INTERVAL_VA = 0x27CC4F
HASH_INTERVAL = (HASH_INTERVAL_VA, bytes.fromhex("c744241864000000"), bytes.fromhex("c744241832000000"))
HASH_INTERVAL_BYTE_VA = HASH_INTERVAL_VA + 4  # the one byte that differs
HASH_INTERVAL_MS = 50                         # = one tick

# gather_victory_items_rule_core's container event handler 0x313A50:
# `cmp dword [ecx+15Ch],-1` / `jnz 313B3Bh` (an event after the completion
# is not counted) -> the jnz becomes nops;
GATHER_EVENTS = (0x313A62, bytes.fromhex("0f85d3000000"), b"\x90" * 6)
# `mov [ecx+15Ch],edx` / `mov byte [ecx+160h],0` / `ret 0Ch` (at the
# completion no container can be used any more) -> the mov byte becomes nops.
GATHER_LOCK = (0x313B34, bytes.fromhex("c6816001000000"), b"\x90" * 7)
GATHER = (GATHER_EVENTS, GATHER_LOCK)

# game_world_core::on_hash_mismatch, 0x47B bytes of code + 5 int3 of padding
HASH_MISMATCH_VA, HASH_MISMATCH_SIZE, HASH_MISMATCH_REGION = 0x325A60, 0x47B, 0x480
HASH_MISMATCH_SHA256 = "f757a41458ada034433e7476bb47447a433c7c0a33f5b403cd4168e0688a040c"

# The new section of --cave=section, right after .reloc.
SECTION_NAME = b".srp\0\0\0\0"
SECTION_RVA = 0xA415000                       # = the original SizeOfImage
SECTION_VA = IMAGE_BASE + SECTION_RVA         # 0xA425000, the CAVE of the .section.bin
SECTION_CHARACTERISTICS = 0x60000020          # code, execute, read (as .text)

CAVES = {"section": SECTION_VA, "hash-mismatch": HASH_MISMATCH_VA}

HOOKS = [
    # network_client::on_match_ready, ebx = network_client: mov ebx,[ebx+1F1C48h]
    (0x27AEA1, bytes.fromhex("8b9b481c1f00")),
    # on_match_packet_received, case 0xF6: cmp byte [g_debug_hash_mismatches],0
    (0x27A505, bytes.fromhex("803d2cd3f10100")),
]


def file_offset(va):
    assert TEXT_VA <= va < TEXT_VA + TEXT_VSIZE
    return va - TEXT_VA + TEXT_FILE


def patch(data, va, expected, new):
    at = file_offset(va)
    found = bytes(data[at:at + len(expected)])
    if found != expected:
        sys.exit(f"{va:#x}: expected {expected.hex()}, found {found.hex()}")
    data[at:at + len(new)] = new


class Headers:
    """The few PE header fields the patch reads or changes (PE32 only)."""

    def __init__(self, data):
        self.pe = struct.unpack_from("<I", data, 0x3C)[0]
        if data[self.pe:self.pe + 4] != b"PE\0\0":
            sys.exit("not a PE file")
        self.count_at = self.pe + 6
        self.optional = self.pe + 24
        if struct.unpack_from("<H", data, self.optional)[0] != 0x10B:
            sys.exit("not a PE32 image")
        self.size_of_code_at = self.optional + 4
        self.size_of_image_at = self.optional + 56
        self.checksum_at = self.optional + 64
        self.security_at = self.optional + 96 + 8 * 4
        self.reloc_at = self.optional + 96 + 8 * 5
        self.debug_at = self.optional + 96 + 8 * 6
        self.table = self.optional + struct.unpack_from("<H", data, self.pe + 20)[0]
        self.section_alignment, self.file_alignment = struct.unpack_from("<II", data, self.optional + 32)
        self.size_of_headers = struct.unpack_from("<I", data, self.optional + 60)[0]

    def sections(self, data):
        """[(name, rva, virtual size, raw offset, raw size, characteristics)]"""
        out = []
        for i in range(struct.unpack_from("<H", data, self.count_at)[0]):
            o = self.table + 40 * i
            vsize, rva, rsize, raw = struct.unpack_from("<IIII", data, o + 8)
            out.append((bytes(data[o:o + 8]), rva, vsize, raw, rsize, struct.unpack_from("<I", data, o + 36)[0]))
        return out


def pe_checksum(data, checksum_at):
    """The checksum of IMAGEHLP CheckSumMappedFile: a 16-bit one's-complement
    sum of the file with the checksum field as zero, plus the file size."""
    total = 0
    padded = bytes(data) + b"\0" * (len(data) % 2)
    for (word,) in struct.iter_unpack("<H", padded[:checksum_at] + b"\0\0\0\0" + padded[checksum_at + 4:]):
        total += word
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return total + len(data)


def neutralize_relocs(data, ranges):
    """Turn HIGHLOW base relocations that overlap a [lo, hi) VA range into
    type 0 (skip). Returns the VAs of the changed entries."""
    headers = Headers(data)
    rva, size = struct.unpack_from("<II", data, headers.reloc_at)
    base_off = None
    for _, srva, vsize, raw, rsize, _ in headers.sections(data):
        if srva <= rva < srva + max(vsize, rsize):
            base_off = rva - srva + raw
    if base_off is None:
        sys.exit("no .reloc section")
    o, end, changed = base_off, base_off + size, []
    while o < end:
        page, block = struct.unpack_from("<II", data, o)
        if block < 8:
            break
        for i in range((block - 8) // 2):
            at = o + 8 + 2 * i
            e = struct.unpack_from("<H", data, at)[0]
            va = IMAGE_BASE + page + (e & 0xFFF)
            if e >> 12 == 3 and any(va + 4 > lo and va < hi for lo, hi in ranges):
                struct.pack_into("<H", data, at, e & 0xFFF)
                changed.append(va)
        o += block
    return changed


def split_code(code):
    sizes = [len(original) for _, original in HOOKS]
    return code[:-sum(sizes)], code[-sum(sizes):], sizes


def add_section(data, cave):
    """Appends the .srp section holding `cave` and moves the Authenticode
    blob behind it. Returns the new file contents."""
    headers = Headers(data)
    sections = headers.sections(data)
    _, last_rva, last_vsize, last_raw, last_rsize, _ = sections[-1]
    align = lambda value, to: (value + to - 1) // to * to
    slot = headers.table + 40 * len(sections)
    if slot + 40 > headers.size_of_headers or any(data[slot:slot + 40]):
        sys.exit("no free section table entry")
    if align(last_rva + last_vsize, headers.section_alignment) != SECTION_RVA \
            or struct.unpack_from("<I", data, headers.size_of_image_at)[0] != SECTION_RVA:
        sys.exit("the image does not end where the new section starts")
    raw = last_raw + last_rsize
    security_offset, security_size = struct.unpack_from("<II", data, headers.security_at)
    # The attribute certificate table is the only thing after the last
    # section, and its data directory holds a FILE offset: it must move with
    # the bytes, and stays the last thing in the file (8-byte aligned), as
    # the PE format asks. vostok::crypt::verify_sign reads it at every start.
    if security_offset != raw or security_offset + security_size != len(data) or raw % headers.file_alignment:
        sys.exit("unexpected layout after the last section")
    if struct.unpack_from("<I", data, security_offset)[0] > security_size:
        sys.exit("the certificate table entry is larger than the table")
    debug_rva, debug_size = struct.unpack_from("<II", data, headers.debug_at)
    # IMAGE_DEBUG_DIRECTORY.PointerToRawData is the other file offset in the
    # headers; it must point before the bytes that move.
    debug_at = next(debug_rva - rva + offset for _, rva, vsize, offset, rsize, _ in sections
                    if rva <= debug_rva < rva + rsize)
    for entry in range(debug_size // 28):
        if struct.unpack_from("<I", data, debug_at + 28 * entry + 24)[0] >= raw:
            sys.exit("debug data after the last section would move")
    raw_size = align(len(cave), headers.file_alignment)
    if (raw + raw_size) % 8:
        sys.exit("the certificate table would lose its 8-byte alignment")

    out = bytearray(data[:raw]) + cave + b"\xCC" * (raw_size - len(cave)) + data[raw:]
    out[slot:slot + 40] = SECTION_NAME + struct.pack(
        "<IIIIIIHHI", len(cave), SECTION_RVA, raw_size, raw, 0, 0, 0, 0, SECTION_CHARACTERISTICS)
    struct.pack_into("<H", out, headers.count_at, len(sections) + 1)
    struct.pack_into("<I", out, headers.size_of_image_at,
                     align(SECTION_RVA + len(cave), headers.section_alignment))
    struct.pack_into("<I", out, headers.size_of_code_at,
                     struct.unpack_from("<I", data, headers.size_of_code_at)[0] + raw_size)
    struct.pack_into("<I", out, headers.security_at, security_offset + raw_size)
    return out


def build(original, mode, code=None):
    """Returns (patched file, description). mode: "cert-only", "section" or
    "hash-mismatch"; code = the assembled .bin for that cave."""
    if hashlib.sha256(original).hexdigest() != EXE_SHA256:
        sys.exit("the source is not the original 0.26g survarium.exe (sha256 mismatch)")
    data = bytearray(original)
    patch(data, *CERT)
    if mode == "cert-only":
        return data, "certificate check only"

    cave_va = CAVES[mode]
    cave, hooks, sizes = split_code(code)
    patch(data, *MUTEX)
    patch(data, *MEMORY_CHECK)
    patch(data, *HASH_INTERVAL)
    rewritten = [(HASH_INTERVAL_BYTE_VA, HASH_INTERVAL_BYTE_VA + 1),
                 (MEMORY_CHECK[0], MEMORY_CHECK[0] + 1)]
    for site in GATHER:
        patch(data, *site)
        rewritten.append((site[0], site[0] + len(site[1])))
    if mode == "hash-mismatch":
        if len(cave) > HASH_MISMATCH_SIZE:
            sys.exit(f"the cave code is {len(cave)} bytes; {HASH_MISMATCH_SIZE} fit")
        at = file_offset(HASH_MISMATCH_VA)
        if hashlib.sha256(data[at:at + HASH_MISMATCH_REGION]).hexdigest() != HASH_MISMATCH_SHA256:
            sys.exit(f"{HASH_MISMATCH_VA:#x}: on_hash_mismatch is not the original")
        data[at:at + HASH_MISMATCH_REGION] = cave + b"\xCC" * (HASH_MISMATCH_REGION - len(cave))
        rewritten.append((HASH_MISMATCH_VA, HASH_MISMATCH_VA + HASH_MISMATCH_REGION))
        where = "over on_hash_mismatch"
    else:
        if len(cave) > Headers(data).section_alignment:
            sys.exit(f"the cave code is {len(cave)} bytes; one page fits")
        where = f"in the new section .srp at {SECTION_VA:#x}"

    pos = 0
    for (va, original_bytes), size in zip(HOOKS, sizes):
        hook = hooks[pos:pos + size]
        pos += size
        if hook[0] != 0xE9 or set(hook[5:]) != {0x90}:
            sys.exit(f"{va:#x}: the hook is not jmp + nops: {hook.hex()}")
        target_va = va + 5 + struct.unpack_from("<i", hook, 1)[0]
        if not cave_va <= target_va < cave_va + len(cave):
            sys.exit(f"{va:#x}: the hook jumps to {target_va:#x}, outside the cave "
                     f"(was the .bin assembled with -DCAVE={cave_va:#x}?)")
        patch(data, va, original_bytes, hook)
        rewritten.append((va, va + size))
    rewritten += [(CERT[0], CERT[0] + len(CERT[1])), (MUTEX[0], MUTEX[0] + len(MUTEX[1]))]
    relocs = neutralize_relocs(data, rewritten)

    if mode == "section":
        data = add_section(data, cave)
    headers = Headers(data)
    struct.pack_into("<I", data, headers.checksum_at, pe_checksum(data, headers.checksum_at))
    return data, (f"certificate check, mutex, minimum VRAM check, hash interval {HASH_INTERVAL_MS} ms, battery rule, "
                  f"{len(cave)} bytes of upload code {where} "
                  f"({len(relocs)} relocations disabled), {len(HOOKS)} hooks")


def default_bin(mode):
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), f"snapshot_upload_026g.{mode}.bin")


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--") or a == "--check"]
    options = [a for a in sys.argv[1:] if a.startswith("--") and a != "--check"]
    mode, bin_path = "section", None
    for option in options:
        if option == "--cert-only":
            mode = "cert-only"
        elif option.startswith("--cave=") and option[7:] in CAVES:
            mode = option[7:]
        elif option.startswith("--bin="):
            bin_path = option[6:]
        else:
            sys.exit(f"unknown option {option}\n\n{__doc__}")
    if len(args) == 3:                                     # the draft's form: <exe> <out> <bin>
        bin_path = args.pop()
    if len(args) != 2:
        sys.exit(__doc__)
    source, target = args
    if target != "--check" and os.path.exists(target) and os.path.samefile(source, target):
        sys.exit("the output must not be the original exe")
    code = None
    if mode != "cert-only":
        code = open(bin_path or default_bin(mode), "rb").read()
    data, description = build(open(source, "rb").read(), mode, code)
    if target != "--check":
        with open(target, "wb") as out:
            out.write(data)
    print(f"{target}: {description}")


if __name__ == "__main__":
    main()
