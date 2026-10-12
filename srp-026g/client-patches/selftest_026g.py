"""Self-test of patch_026g.py. Builds every variant from the ORIGINAL 0.26g
exe (read only), writes the results to srp-026g/target-a4/scratch/ (never the
game folder) and checks them. Nothing is run and nothing is installed.

    python selftest_026g.py [<survarium.exe>]

Checks, per variant (cert-only, section, hash-mismatch):
 1. the checked-in .bin files are what nasm makes from the .asm today;
 2. the PE parses: section table, alignments, SizeOfImage, data
    directories inside the file, the certificate table last and intact;
 3. every byte outside the intended sites equals the original;
 4. no active base relocation overlaps a rewritten byte, and the relocation
    table is otherwise untouched;
 5. the hooks and the cave disassemble (ndisasm) to the intended
    instructions: every call/jmp target is a known address;
 6. nothing in the original branches into the middle of a hook;
 7. Windows computes the same PE checksum (imagehlp);
 8. Windows still reads the survarium.com certificate out of the file with
    the exact CryptQueryObject call the client makes at start (0x13310);
 9. full builds only: the default hash interval at 0x27CC4F is 50 ms, one
    byte changed, under no relocation; cert-only keeps the original 100.
10. full builds only: the minimum VRAM check jumps over the fatal warning;
    the detected size and unknown-size fallback remain unchanged.
"""

import ctypes
import hashlib
import os
import re
import struct
import subprocess
import sys
import tempfile

import patch_026g as P

HERE = os.path.dirname(os.path.abspath(__file__))
# What the self-test needs of this machine, each from the environment:
# the original 0.26g survarium.exe (or pass it as the first argument), a
# folder to build into (never the game's), the folder of nasm and ndisasm
# (empty: found on PATH), and optionally an exe built with --cert-only to
# compare with.
ORIGINAL = os.environ.get("SRP_026G_EXE", "survarium.exe")
INSTALLED_CERT_ONLY = os.environ.get("SRP_026G_CERT_ONLY_EXE", "")
SCRATCH = os.environ.get("SRP_026G_SCRATCH", os.path.join(tempfile.gettempdir(), "srp-026g-selftest"))
NASM_DIR = os.environ.get("NASM_DIR", "")


def tool(name):
    return os.path.join(NASM_DIR, name) if NASM_DIR else name

# Every address the injected code may call or jump to outside itself
# (client_patch.md section 3).
EXTERNAL = {
    0x324BE0: "game_state_history_item::serialize",
    0x07AC10: "buffer_writer::w",
    0x424CA0: "_alloca_probe_16",
    0x2D1070: "is_there_event_horizon_item",
    0x2D0B30: "event_horizon_history_item",
    0x27AEA7: "on_match_ready, after the hook",
    0x27A5AB: "0xF6 case: skip the payload and return",
}
INDIRECT = {"call [eax+0x18]": "new_packet", "call [eax+0x1c]": "enqueue"}
# The functions that hold the hooks: [start, end) of straight code.
HOSTS = [(0x27A300, 0x27A738), (0x27AA20, 0x27AEC8)]
JUMP_TABLE = (0x27A738, 20)                   # on_match_packet_received, ids 0xE3..0xF6

failures = []


def check(ok, text):
    print(("  ok    " if ok else "  FAIL  ") + text)
    if not ok:
        failures.append(text)


def ndisasm(code, va):
    """[(address, hex bytes, text)]"""
    with tempfile.NamedTemporaryFile(delete=False, suffix=".bin") as chunk:
        chunk.write(code)
    try:
        out = subprocess.run([tool("ndisasm"), "-b", "32", "-o", str(va), chunk.name],
                             capture_output=True, text=True, check=True, timeout=60).stdout
    finally:
        os.unlink(chunk.name)
    rows = []
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and not parts[0].startswith("-"):
            # ndisasm 3 spells the same 32-bit indirect call with explicit
            # size/distance qualifiers; keep the 2.x spelling for checks.
            text = parts[2].replace("call dword near [", "call [")
            rows.append((int(parts[0], 16), parts[1], text))
    return rows


BRANCH = re.compile(r"^(call|jmp|j[a-z]+|loop[a-z]*)\s+(?:near |short )?(0x[0-9a-f]+)$")


def branch_target(text):
    match = BRANCH.match(text)
    return int(match.group(2), 16) if match else None


class Image:
    def __init__(self, data):
        self.data = data
        self.headers = P.Headers(data)
        self.sections = self.headers.sections(data)

    def offset(self, va):
        rva = va - P.IMAGE_BASE
        for _, srva, vsize, raw, rsize, _ in self.sections:
            if srva <= rva < srva + rsize:
                return rva - srva + raw
        raise ValueError(f"{va:#x} is in no section's file data")

    def read(self, va, size):
        at = self.offset(va)
        return bytes(self.data[at:at + size])

    def directory(self, index):
        return struct.unpack_from("<II", self.data, self.headers.optional + 96 + 8 * index)

    def relocations(self):
        """{file offset of the entry: (type, va)}"""
        rva, size = self.directory(5)
        o = self.offset(P.IMAGE_BASE + rva)
        end, out = o + size, {}
        while o < end:
            page, block = struct.unpack_from("<II", self.data, o)
            if block < 8:
                break
            for i in range((block - 8) // 2):
                e = struct.unpack_from("<H", self.data, o + 8 + 2 * i)[0]
                out[o + 8 + 2 * i] = (e >> 12, P.IMAGE_BASE + page + (e & 0xFFF))
            o += block
        return out


def test_bins():
    print("assembled code")
    for mode, cave in P.CAVES.items():
        path = P.default_bin(mode)
        with tempfile.TemporaryDirectory() as tmp:
            out = os.path.join(tmp, "out.bin")
            subprocess.run([tool("nasm"), "-f", "bin", f"-DCAVE={cave:#x}",
                            os.path.join(HERE, "snapshot_upload_026g.asm"), "-o", out],
                           check=True, timeout=60)
            fresh = open(out, "rb").read()
        check(fresh == open(path, "rb").read(), f"{os.path.basename(path)} is current ({len(fresh)} bytes)")


def test_pe(original, image, mode):
    headers, data = image.headers, image.data
    sections = image.sections
    count = len(sections)
    check(count == (8 if mode == "section" else 7), f"{count} sections")
    check(headers.table + 40 * count <= headers.size_of_headers, "the section table fits in the headers")
    previous_end_rva, previous_end_raw = headers.size_of_headers, headers.size_of_headers
    ordered = True
    for name, rva, vsize, raw, rsize, _ in sections:
        ordered &= raw % headers.file_alignment == 0 and rsize % headers.file_alignment == 0
        ordered &= raw == previous_end_raw
        ordered &= rva == (previous_end_rva + headers.section_alignment - 1) // headers.section_alignment \
            * headers.section_alignment
        previous_end_rva, previous_end_raw = rva + vsize, raw + rsize
    check(ordered, "sections are aligned, contiguous in memory and in the file")
    size_of_image = struct.unpack_from("<I", data, headers.size_of_image_at)[0]
    expected = (previous_end_rva + headers.section_alignment - 1) // headers.section_alignment * headers.section_alignment
    check(size_of_image == expected, f"SizeOfImage {size_of_image:#x} = end of the last section")
    entry = struct.unpack_from("<I", data, headers.optional + 16)[0]
    check(entry == struct.unpack_from("<I", original, headers.optional + 16)[0], "entry point unchanged")

    security_offset, security_size = image.directory(4)
    check(security_offset == previous_end_raw and security_offset + security_size == len(data)
          and security_offset % 8 == 0,
          f"certificate table at {security_offset:#x}, size {security_size:#x}: right after the last section, "
          "8-byte aligned, last in the file")
    old_offset, old_size = struct.unpack_from("<II", original, headers.security_at)
    check(bytes(data[security_offset:]) == bytes(original[old_offset:old_offset + old_size]),
          "certificate table bytes identical to the original")
    length, revision, kind = struct.unpack_from("<IHH", data, security_offset)
    check(length <= security_size and revision == 0x200 and kind == 2,
          f"WIN_CERTIFICATE length {length:#x}, revision {revision:#x}, type {kind} (PKCS#7 signed data)")
    inside = True
    for index in range(16):
        if index == 4:
            continue
        rva, size = image.directory(index)
        if size:
            inside &= any(srva <= rva and rva + size <= srva + max(vsize, rsize)
                          for _, srva, vsize, _, rsize, _ in sections)
        inside &= (rva, size) == struct.unpack_from("<II", original, headers.optional + 96 + 8 * index)
    check(inside, "the other 15 data directories are unchanged and inside sections")
    if mode == "section":
        name, rva, vsize, raw, rsize, characteristics = sections[-1]
        check(name == P.SECTION_NAME and rva == P.SECTION_RVA and characteristics == 0x60000020,
              f".srp at RVA {rva:#x} (VA {P.IMAGE_BASE + rva:#x}), virtual {vsize:#x}, file {raw:#x}+{rsize:#x}, "
              "code | execute | read")
    characteristics = struct.unpack_from("<H", data, headers.pe + 22)[0]
    dll = struct.unpack_from("<H", data, headers.optional + 70)[0]
    check(characteristics == 0x122 and dll == 0x8000,
          "file characteristics 0x122, DLL characteristics 0x8000 (no ASLR flag), as the original")


def allowed_ranges(original, mode, cave_size):
    """File ranges that may differ from the original (same offsets)."""
    headers = P.Headers(original)
    ranges = [(P.file_offset(P.CERT[0]), len(P.CERT[1]))]
    if mode == "cert-only":
        return ranges, []
    ranges.append((P.file_offset(P.MUTEX[0]), len(P.MUTEX[1])))
    rewritten = [(P.CERT[0], P.CERT[0] + 3), (P.MUTEX[0], P.MUTEX[0] + 2)]
    ranges.append((P.file_offset(P.HASH_INTERVAL_BYTE_VA), 1))
    rewritten.append((P.HASH_INTERVAL_BYTE_VA, P.HASH_INTERVAL_BYTE_VA + 1))
    ranges.append((P.file_offset(P.MEMORY_CHECK[0]), 1))
    rewritten.append((P.MEMORY_CHECK[0], P.MEMORY_CHECK[0] + 1))
    for va, old in P.HOOKS:
        ranges.append((P.file_offset(va), len(old)))
        rewritten.append((va, va + len(old)))
    ranges.append((headers.checksum_at, 4))
    if mode == "hash-mismatch":
        ranges.append((P.file_offset(P.HASH_MISMATCH_VA), P.HASH_MISMATCH_REGION))
        rewritten.append((P.HASH_MISMATCH_VA, P.HASH_MISMATCH_VA + P.HASH_MISMATCH_REGION))
    else:
        ranges += [(headers.count_at, 2), (headers.size_of_code_at, 4), (headers.size_of_image_at, 4),
                   (headers.security_at, 4), (headers.table + 40 * 7, 40)]
    return ranges, rewritten


def test_bytes(original, image, mode, cave):
    data = image.data
    ranges, rewritten = allowed_ranges(original, mode, len(cave))
    old_relocations = Image(original).relocations()
    disabled = [at for at, (kind, va) in old_relocations.items()
                if kind == 3 and any(va + 4 > lo and va < hi for lo, hi in rewritten)]
    ranges += [(at, 2) for at in disabled]
    common = len(original)
    if mode == "section":
        common = struct.unpack_from("<I", original, P.Headers(original).security_at)[0]
        raw_size = len(data) - len(original)
        check(raw_size == 0x200 and bytes(data[common:common + raw_size]) == cave + b"\xCC" * (raw_size - len(cave)),
              f"the new section's file data is the cave ({len(cave)} bytes) + int3 padding")
    else:
        check(len(data) == len(original), "file size unchanged")
    allowed = bytearray(common)
    for at, size in ranges:
        allowed[at:at + size] = b"\1" * size
    a, b = bytes(original[:common]), bytes(data[:common])
    stray = changed = 0
    if a != b:
        for i in range(0, common, 1 << 16):
            if a[i:i + (1 << 16)] != b[i:i + (1 << 16)]:
                for j in range(i, min(i + (1 << 16), common)):
                    if a[j] != b[j]:
                        changed += 1
                        stray += not allowed[j]
    check(stray == 0, f"{changed} bytes differ from the original, all inside the {len(ranges)} intended ranges")

    new_relocations = image.relocations()
    active = [va for kind, va in new_relocations.values()
              if kind == 3 and any(va + 4 > lo and va < hi for lo, hi in rewritten)]
    same = all(new_relocations[at] == old for at, old in old_relocations.items() if at not in disabled) \
        and len(new_relocations) == len(old_relocations)
    check(not active and same and all(new_relocations[at][0] == 0 for at in disabled),
          f"{len(disabled)} relocation entries disabled, none active in rewritten bytes, the rest untouched")
    return rewritten


def test_code(original, image, mode, cave):
    cave_va = P.CAVES[mode]
    check(image.read(cave_va, len(cave)) == cave, f"the cave is mapped at {cave_va:#x}")
    rows = ndisasm(image.read(cave_va, len(cave)), cave_va)
    starts = {address for address, _, _ in rows}
    check(rows[-1][2] == "ret" and rows[-1][0] + 1 == cave_va + len(cave), "the cave disassembles cleanly to its last ret")
    seen, bad, indirect = set(), [], []
    for address, _, text in rows:
        target = branch_target(text)
        if target is not None:
            if cave_va <= target < cave_va + len(cave):
                if target not in starts:
                    bad.append(f"{address:#x}: {text} (not an instruction start)")
            elif target in EXTERNAL and (text.startswith("call") or text.startswith("jmp")):
                seen.add(target)
            else:
                bad.append(f"{address:#x}: {text}")
        elif text.startswith(("call", "jmp")):
            indirect.append(text)
            if text not in INDIRECT:
                bad.append(f"{address:#x}: {text}")
        elif re.search(r"\[(dword )?0x", text):
            bad.append(f"{address:#x}: {text} (absolute address operand)")
    check(not bad, "every branch in the cave goes to a known address" + "".join("\n          " + b for b in bad))
    check(seen == set(EXTERNAL), "the cave reaches all 7 external addresses: " +
          ", ".join(f"{va:#x}" for va in sorted(seen)))
    check(sorted(set(indirect)) == sorted(INDIRECT), "indirect calls: " + ", ".join(sorted(set(indirect))))

    entries = []
    for va, old in P.HOOKS:
        hook = ndisasm(image.read(va, len(old)), va)
        target = branch_target(hook[0][2])
        entries.append(target)
        nops = all(text == "nop" for _, _, text in hook[1:])
        check(hook[0][2].startswith("jmp") and target in starts and nops and len(hook) == len(old) - 4,
              f"hook {va:#x}: {hook[0][2]} + {len(hook) - 1} nop")
    # The two hook targets start with pushad, and each path ends by running
    # the replaced instruction or jumping where the original went.
    text_at = {address: text for address, _, text in rows}
    check(all(text_at[e] == "pusha" for e in entries), "both cave entries start with pushad")
    check("mov ebx,[ebx+0x1f1c48]" in text_at.values(), "the cave replays the replaced `mov ebx,[ebx+1F1C48h]`")
    tail = image.read(P.HOOKS[1][0] + 7, 6)
    check(tail == bytes.fromhex("0f85a6000000"), "the dead `jnz` after the request hook is the original's")
    done = Image(original).read(0x27A5AB, 13)
    check(done == bytes.fromhex("8b530803138953 04e944010000".replace(" ", "")),
          "0x27A5AB is `mov edx,[ebx+8]; add edx,[ebx]; mov [ebx+4],edx; jmp 27A6FCh`")


def test_hash_interval(original, image, mode):
    """The default of -hash_check_interval_in_ms (match.md section 14)."""
    va, old, new = P.HASH_INTERVAL
    found = image.read(va, len(old))
    if mode == "cert-only":
        check(found == old, f"{va:#x} is the original `mov dword [esp+0x18],0x64` (cert-only: 100 ms)")
        return
    rows = ndisasm(image.read(va - 10, 10 + len(new) + 5), va - 10)
    texts = [text for _, _, text in rows]
    check(found == new and texts == ["lea ecx,[esp+0x14]", "push ecx", "mov esi,0xaa19f8",
                                     "mov dword [esp+0x18],0x32", "call 0x141b0"],
          f"{va:#x} is `mov dword [esp+0x18],0x32` between `mov esi,0xaa19f8` and `call 0x141b0`: "
          f"the default hash interval is {P.HASH_INTERVAL_MS} ms")
    differing = [i for i in range(len(old)) if found[i] != old[i]]
    check(differing == [P.HASH_INTERVAL_BYTE_VA - va] and found[differing[0]] == P.HASH_INTERVAL_MS,
          f"one byte differs there: {P.HASH_INTERVAL_BYTE_VA:#x} 64 -> 32")
    relocated = sorted(rva for kind, rva in Image(original).relocations().values()
                       if kind == 3 and rva + 4 > va + 4 and rva < va + len(old))
    check(not relocated, "no base relocation of the original covers the immediate (0x27CC53..0x27CC56)")
    # The static the default lands in has one writer and the gate's two
    # reads; the option object is named only here and where it is registered.
    refs = lambda value: sorted(P.TEXT_VA + at - P.TEXT_FILE for at in
                                (m.start() for m in re.finditer(re.escape(struct.pack("<I", value)), original))
                                if P.TEXT_FILE <= at < P.TEXT_FILE + P.TEXT_VSIZE)
    check(refs(0xA2B393C) == [0x27CC62, 0x27CD30, 0x27CD5A] and refs(0xAA19F8) == [0x27CC4B, 0x853F51],
          "the interval static 0xA2B393C is written at 0x27CC60 and read at 0x27CD2E/0x27CD58 only; "
          "the option 0xAA19F8 is used at 0x27CC4A and registered at 0x853F50 only")
    inside = range(va + 1, va + len(old))
    hits = [f"{address:#x}: {text}" for address, _, text in ndisasm(Image(original).read(0x27CC45, 0x30B), 0x27CC45)
            if branch_target(text) in inside]
    check(not hits, "no branch of try_send_all lands inside that instruction")


def test_memory_check(original, image, mode):
    """The launcher's 73 -> EB patch skips only the fatal 320 MiB gate."""
    va, old, new = P.MEMORY_CHECK
    found = image.read(va, len(old))
    check(P.file_offset(va) == 0xF5811, "VRAM patch uses the Linux launcher's file offset 0xF5811")
    if mode == "cert-only":
        check(found == old, "cert-only keeps the original minimum VRAM check")
        return
    rows = ndisasm(found, va)
    check(found == new and len(rows) == 1 and rows[0][2].startswith("jmp ")
          and branch_target(rows[0][2]) == 0x106422,
          "0x106411 unconditionally jumps to the detected-size return at 0x106422")
    check([i for i in range(len(old)) if found[i] != old[i]] == [0],
          "only the branch opcode changes: 73 -> EB; displacement 0F stays")
    # A zero/unknown size returns 320 MiB before reaching the patched gate.
    fallback = bytes.fromhex("8bc30bc7750eb80000001433d25f5e5b83c408c3")
    measured = bytes.fromhex("8bd75f5e8bc35b83c408c3")
    check(image.read(0x1063F1, len(fallback)) == Image(original).read(0x1063F1, len(fallback)) == fallback,
          "unknown-size detection and its 320 MiB fallback are unchanged")
    check(image.read(0x106422, len(measured)) == Image(original).read(0x106422, len(measured)) == measured,
          "the return still uses the detected size in EDI:EBX without inflating it")
    relocated = [rva for kind, rva in Image(original).relocations().values()
                 if kind == 3 and rva + 4 > va and rva < va + len(old)]
    check(not relocated, "no base relocation overlaps the VRAM branch")
    # Exercise the site's byte guard separately from the full-file hash guard.
    bad = bytearray(original)
    bad[P.file_offset(va)] ^= 1
    before = bytes(bad)
    try:
        P.patch(bad, *P.MEMORY_CHECK)
    except SystemExit:
        check(bytes(bad) == before, "a changed VRAM instruction is rejected without modifying it")
    else:
        check(False, "a changed VRAM instruction must be rejected")


def test_no_branch_into_hooks(original):
    print("branches into the hooks (original exe)")
    image = Image(original)
    interiors = [(va + 1, va + len(old)) for va, old in P.HOOKS]
    inside = lambda target: any(lo <= target < hi for lo, hi in interiors)
    hits = []
    for start, end in HOSTS:
        for address, _, text in ndisasm(image.read(start, end - start), start):
            target = branch_target(text)
            if target is not None and inside(target):
                hits.append(f"{address:#x}: {text}")
    check(not hits, "no branch of the two host functions lands inside a hook" + "".join("\n   " + h for h in hits))
    table = struct.unpack_from(f"<{JUMP_TABLE[1]}I", original, image.offset(JUMP_TABLE[0]))
    check(not any(inside(t) for t in table) and table[19] == 0x27A4F4,
          "the dispatcher's jump table has no entry inside a hook; 0xF6 -> 0x27A4F4")
    # The whole .text: any rel32 call/jmp/jcc or rel8 jmp/jcc whose target is
    # inside a hook, and any 4-byte value equal to such an address.
    text = original[P.TEXT_FILE:P.TEXT_FILE + P.TEXT_VSIZE]
    found = []
    for lo, hi in interiors:
        for target in range(lo, hi):
            for at in range(max(0, target - P.TEXT_VA - 0x90), min(len(text), target - P.TEXT_VA + 0x90)):
                op = text[at]
                if (op == 0xEB or 0x70 <= op <= 0x7F or 0xE0 <= op <= 0xE3) and at + 2 <= len(text):
                    if P.TEXT_VA + at + 2 + struct.unpack_from("<b", text, at + 1)[0] == target:
                        found.append((P.TEXT_VA + at, "rel8"))
            pointer = struct.pack("<I", target)
            at = original.find(pointer)
            while at != -1:
                found.append((at, "absolute value (file offset)"))
                at = original.find(pointer, at + 1)
    position = 0
    while True:
        e8, e9 = text.find(b"\xE8", position), text.find(b"\xE9", position)
        candidates = [x for x in (e8, e9) if x != -1]
        if not candidates:
            break
        at = min(candidates)
        position = at + 1
        if at + 5 <= len(text):
            target = (P.TEXT_VA + at + 5 + struct.unpack_from("<i", text, at + 1)[0]) & 0xFFFFFFFF
            if inside(target):
                found.append((P.TEXT_VA + at, "rel32"))
    position = 0
    while True:
        at = text.find(b"\x0F", position)
        if at == -1:
            break
        position = at + 1
        if at + 6 <= len(text) and 0x80 <= text[at + 1] <= 0x8F:
            target = (P.TEXT_VA + at + 6 + struct.unpack_from("<i", text, at + 2)[0]) & 0xFFFFFFFF
            if inside(target):
                found.append((P.TEXT_VA + at, "jcc rel32"))
    # A rel8 hit inside the host function is real only if the linear
    # disassembly above also saw it; anything else is a byte coincidence.
    check(not found, "brute-force scan of .text: no rel8/rel32 branch bytes or pointer values aim inside a hook"
          + "".join(f"\n          {a:#x} {kind}" for a, kind in found))


def test_windows(path, original_path, mode):
    crypt32 = ctypes.WinDLL("crypt32", use_last_error=True)
    imagehlp = ctypes.WinDLL("imagehlp", use_last_error=True)
    header_sum, check_sum = ctypes.c_uint32(), ctypes.c_uint32()
    result = imagehlp.MapFileAndCheckSumW(ctypes.c_wchar_p(path), ctypes.byref(header_sum), ctypes.byref(check_sum))
    if mode == "cert-only":
        check(result == 0, f"imagehlp reads the file (header checksum kept at {header_sum.value:#x}, "
              f"computed {check_sum.value:#x}; the loader does not check it for an exe)")
    else:
        check(result == 0 and header_sum.value == check_sum.value,
              f"PE checksum {header_sum.value:#x} = imagehlp's {check_sum.value:#x}")

    crypt32.CryptQueryObject.argtypes = [ctypes.c_uint32, ctypes.c_void_p] + [ctypes.c_uint32] * 3 + [ctypes.c_void_p] * 6
    crypt32.CertFindCertificateInStore.restype = ctypes.c_void_p
    crypt32.CertFindCertificateInStore.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32,
                                                   ctypes.c_uint32, ctypes.c_void_p, ctypes.c_void_p]

    def certificate(file):
        store, message = ctypes.c_void_p(), ctypes.c_void_p()
        name = ctypes.create_unicode_buffer(file)
        # The client's call at 0x1335B: CERT_QUERY_OBJECT_FILE (1),
        # CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED (0x400),
        # CERT_QUERY_FORMAT_FLAG_BINARY (2), then store and message out.
        if not crypt32.CryptQueryObject(1, name, 0x400, 2, 0, None, None, None,
                                        ctypes.byref(store), ctypes.byref(message), None):
            return None
        # CertFindCertificateInStore(store, X509 | PKCS7, 0, CERT_FIND_SUBJECT_STR_W, L"survarium.com", 0)
        subject = ctypes.create_unicode_buffer("survarium.com")
        context = crypt32.CertFindCertificateInStore(store, 0x10001, 0, 0x00080007, subject, None)
        if not context:
            return None
        # CERT_CONTEXT {DWORD encoding; BYTE *encoded; DWORD size; ...}, native layout
        encoding, encoded, size = struct.unpack_from("@IPI", ctypes.string_at(context, 3 * ctypes.sizeof(ctypes.c_void_p)))
        return ctypes.string_at(encoded, size)

    ours, theirs = certificate(path), certificate(original_path)
    check(ours is not None and ours == theirs,
          "CryptQueryObject (the client's own call) finds the survarium.com certificate, identical to the "
          f"original's ({len(ours or b'')} bytes, sha1 {hashlib.sha1(ours or b'').hexdigest()})")


def main():
    source = sys.argv[1] if len(sys.argv) > 1 else ORIGINAL
    original = open(source, "rb").read()
    os.makedirs(SCRATCH, exist_ok=True)
    game = os.path.abspath(os.path.dirname(source))
    try:
        inside = os.path.commonpath([os.path.abspath(SCRATCH), game]) == game
    except ValueError:  # another drive: not inside
        inside = False
    if inside:
        sys.exit("the scratch directory must not be inside the game folder")
    test_bins()
    test_no_branch_into_hooks(original)
    for mode in ("cert-only", "section", "hash-mismatch"):
        print(f"variant {mode}")
        code = None if mode == "cert-only" else open(P.default_bin(mode), "rb").read()
        data, description = P.build(original, mode, code)
        print("  " + description)
        check(hashlib.sha256(original).hexdigest() == P.EXE_SHA256, "the original in memory is untouched")
        image = Image(data)
        cave = b"" if code is None else P.split_code(code)[0]
        test_pe(original, image, mode)
        test_bytes(original, image, mode, cave)
        test_hash_interval(original, image, mode)
        test_memory_check(original, image, mode)
        if mode == "cert-only":
            if os.path.exists(INSTALLED_CERT_ONLY):
                check(open(INSTALLED_CERT_ONLY, "rb").read() == bytes(data),
                      "identical to the survarium_srp.exe installed in step 0 (read only)")
            check(image.read(P.CERT[0], 3) == P.CERT[2] and ndisasm(P.CERT[2], P.CERT[0])[1][2] == "ret",
                  "0x12A10 is `xor eax,eax; ret`")
        else:
            test_code(original, image, mode, cave)
            check(image.read(P.MUTEX[0], 2) == b"\x90\x90" and image.read(P.CERT[0], 3) == P.CERT[2],
                  "mutex and certificate patches present")
        path = os.path.join(SCRATCH, f"survarium_srp.{mode}.exe")
        with open(path, "wb") as out:
            out.write(data)
        print(f"  written: {path} (sha256 {hashlib.sha256(data).hexdigest()})")
        test_windows(path, source, mode)
    print()
    print(f"{len(failures)} FAILED" if failures else "all checks passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
