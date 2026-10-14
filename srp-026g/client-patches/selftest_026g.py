"""Self-test of patch_026g.py. Builds every variant from the ORIGINAL 0.26g
exe (read only), writes the results to srp-026g/target-a4/scratch/ (never the
game folder) and checks them. Nothing is run and nothing is installed.

    python selftest_026g.py [<survarium.exe>]

Checks, per variant (cert-only, section, hash-mismatch; the last two here
without the late join, as patch revision 4 was, which check 15 builds on):
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
    the detected size and unknown-size fallback remain unchanged;
11. the version field of the first login message, worked out from the
    instructions that fill it (0x380847..0x38088F): "0.26g0" and six zero
    bytes in the original and in cert-only; in the full builds byte 7 is
    patch_026g.REVISION and nothing else differs. The relocation under the
    rewritten instruction is disabled, the two beside it are not, and no
    branch of the function lands inside it.
12. optional embedded login server: the missing-server hook, relative stub,
    address bytes and unchanged explicit-server/replay paths in both caves.
13. full builds only: the battery rule's two instructions (0x313A62,
    0x313B34) are nops between the instructions they stood between, under
    no relocation, and no branch lands inside them; cert-only keeps them.
14. full builds only: mounter::finish leaves at once for a mount without a
    root (0x46B0B, `jmp short 0x46b36` and two nops; the path of a mount
    with a root is the original's), and the default of -record_journal is
    the exe's "0" (0x851CC7; the relocation of the address stays active,
    the option is read only at the game's start); cert-only keeps both.
15. the late join (patch revision 5, the default build): the checked-in
    late_join_026g.section.bin is its .json's, and its first 4096 bytes
    are what nasm makes of late_join_026g.asm with the .json's addresses
    (the C code after them is not built here: late_join_026g.py
    --check-bin does that where a C compiler is). The build with it
    differs from the build without it, of the same server, in the sites
    patch_026g.py names and in nothing else of the original's bytes; each
    rewritten site disassembles to what it is meant to be, each hook to a
    call or jump to its entry and nops; no relocation is under a site;
    `.srp` has the first page unchanged, then the code; `.srpd` is one
    zeroed writable page after it with no bytes in the file; the checks
    of 2 and 7 to 14 hold for it too, and the revision is REVISION.
16. the same build: the default of the game's setting
    time_floating_factor (0xA799C8, in .data) is CLOCK_FACTOR, four bytes
    under no relocation, inside the range the game itself allows; in the
    original the value is read by the `game` constructor alone, into the
    field of the game clock's timer that set_offset multiplies the
    correction's sign by, and the console variable registered for it has
    that name. The build without the late join keeps the original's 0.05.
17. the same build: the statistics handler's start_match (0x26D8D0)
    keeps 20 as its count, `mov al,0x14` and four nops where the original
    and the build without the late join copy the roster; in the original
    that count goes into +3240h alone, start_match tells the hud the score
    of each counted place, the roster still goes into the shared
    statistics' live count (+31E8h) apart, deserialize runs
    recalculate_scores over the counted places, the vtable's slots are
    those functions, +3240h is in .text only in those loops and in stores
    of a zeroed register, and no branch or address aims inside the six
    bytes.
18. the same build: game_world_ui::update_ui's jump of the countdown
    (0x23B328) goes to 0x23B383 instead of the players' rows; in the
    original the countdown shows the pregame and jumps to the rows, play
    shows the match time and goes on at 0x23B383, an instruction of its
    own, from which the code draws the team points, the last players and
    the respawn time, lets go of the player it took and leaves to nowhere
    else; every call before both takes its arguments off the stack; the
    three are the hud movie's set_game_progress, set_last_players and
    set_respawn_time; no branch or address aims inside the jump.
19. the same build: the respawn rule's spawn calls weapon_random_hook at
    0x316EA0 (`call` where the original and the build without the late
    join load the call that puts the player into the world); in the
    original the spawn makes the target kit current, takes its player into
    esi and initialises it with the time in ebp, neither register written
    again before the hook and no flag read after it; the spawn is slot +5Ch
    of every respawn rule and nothing calls it otherwise, the rules' tick
    spawns through it; the player's initialize sets its inventory (23
    slots at +10Ch) up and makes no object; weapon_core's constructor puts
    1 into +5BCh, which .text names only there, in the two spreads (48271
    mod 2^31-1), the serialize and the deserialize; weapon_core's bind is
    in the file only in the item vtables of weapon_core and of the
    client's weapon; the hook's code tests that bind, writes +5BCh alone,
    and ends with the two instructions; no branch or address aims inside
    the five bytes.
"""

import ctypes
import hashlib
import json
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
HOSTS = [(0x27A300, 0x27A738), (0x27AA20, 0x27AEC8), (0x1EA9F0, 0x1EB2C1)]
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
        for stem in ("snapshot_upload_026g", "default_server_026g"):
            path = os.path.join(HERE, f"{stem}.{mode}.bin")
            with tempfile.TemporaryDirectory() as tmp:
                out = os.path.join(tmp, "out.bin")
                subprocess.run([tool("nasm"), "-f", "bin", f"-DCAVE={cave:#x}",
                                os.path.join(HERE, stem + ".asm"), "-o", out],
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


def allowed_ranges(original, mode, cave_size, server=None):
    """File ranges that may differ from the original (same offsets)."""
    headers = P.Headers(original)
    ranges = [(P.file_offset(P.CERT[0]), len(P.CERT[1]))]
    if mode == "cert-only":
        return ranges, []
    ranges.append((P.file_offset(P.MUTEX[0]), len(P.MUTEX[1])))
    rewritten = [(P.CERT[0], P.CERT[0] + 3), (P.MUTEX[0], P.MUTEX[0] + 2)]
    ranges.append((P.file_offset(P.HASH_INTERVAL_BYTE_VA), 1))
    rewritten.append((P.HASH_INTERVAL_BYTE_VA, P.HASH_INTERVAL_BYTE_VA + 1))
    for va, old, _ in P.GATHER:
        ranges.append((P.file_offset(va), len(old)))
        rewritten.append((va, va + len(old)))
    ranges.append((P.file_offset(P.MEMORY_CHECK[0]), 1))
    rewritten.append((P.MEMORY_CHECK[0], P.MEMORY_CHECK[0] + 1))
    ranges.append((P.file_offset(P.MOUNT_NO_ROOT[0]), len(P.MOUNT_NO_ROOT[1])))
    rewritten.append((P.MOUNT_NO_ROOT[0], P.MOUNT_NO_ROOT[0] + len(P.MOUNT_NO_ROOT[1])))
    # The journal default's imm32 changes and keeps its relocation: it is
    # an allowed range, and not a rewritten one (test_start_up_folder).
    ranges.append((P.file_offset(P.JOURNAL_DEFAULT_VA + 1), 4))
    if server is not None:
        va, old = P.DEFAULT_SERVER_HOOK
        ranges.append((P.file_offset(va), len(old)))
        rewritten.append((va, va + len(old)))
    ranges.append((P.file_offset(P.REVISION_SITE[0]), len(P.REVISION_SITE[1])))
    rewritten.append((P.REVISION_SITE[0], P.REVISION_SITE[0] + len(P.REVISION_SITE[1])))
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


def test_bytes(original, image, mode, cave, server=None):
    data = image.data
    ranges, rewritten = allowed_ranges(original, mode, len(cave), server)
    old_relocations = Image(original).relocations()
    disabled = [at for at, (kind, va) in old_relocations.items()
                if kind == 3 and any(va + 4 > lo and va < hi for lo, hi in rewritten)]
    ranges += [(at, 2) for at in disabled]
    common = len(original)
    if mode == "section":
        common = struct.unpack_from("<I", original, P.Headers(original).security_at)[0]
        raw_size = len(data) - len(original)
        alignment = image.headers.file_alignment
        check(raw_size == (len(cave) + alignment - 1) // alignment * alignment
              and bytes(data[common:common + raw_size]) == cave + b"\xCC" * (raw_size - len(cave)),
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


def test_gather(original, image, mode):
    """The battery rule's container event handler (match.md section 26)."""
    # The handler: 0x313A50 to its last `ret 0Ch` at 0x313B3B.
    start, end = 0x313A50, 0x313B3E
    before = {text for _, _, text in ndisasm(Image(original).read(start, end - start), start)}
    check({"cmp dword [ecx+0x15c],byte -0x1", "jnz near 0x313b3b", "mov [ecx+0x15c],edx",
           "mov byte [ecx+0x160],0x0", "ret 0xc"} <= before,
          "the original handler has `cmp [ecx+0x15c],-1` / `jnz 0x313b3b` and `mov [ecx+0x15c],edx` / "
          "`mov byte [ecx+0x160],0` / `ret 0xc`")
    rows = ndisasm(image.read(start, end - start), start)
    at = {address: text for address, _, text in rows}
    if mode == "cert-only":
        check(all(image.read(va, len(old)) == old for va, old, _ in P.GATHER),
              "cert-only: the handler is the original's")
        return
    (events_va, events_old, _), (lock_va, lock_old, _) = P.GATHER
    nops = lambda va, size: all(at.get(va + i) == "nop" for i in range(size))
    check(at.get(0x313A5B) == "cmp dword [ecx+0x15c],byte -0x1" and nops(events_va, len(events_old))
          and at.get(0x313A68) == "mov eax,[esp+0x8]",
          f"{events_va:#x}: six nops between `cmp dword [ecx+0x15c],-1` and `mov eax,[esp+0x8]`: "
          "an event after the completion is counted")
    check(at.get(0x313B2E) == "mov [ecx+0x15c],edx" and nops(lock_va, len(lock_old))
          and at.get(0x313B3B) == "ret 0xc",
          f"{lock_va:#x}: seven nops between `mov [ecx+0x15c],edx` and `ret 0xc`: "
          "the containers stay usable")
    relocated = sorted(rva for kind, rva in Image(original).relocations().values()
                       if kind == 3 and any(rva + 4 > va and rva < va + len(old) for va, old, _ in P.GATHER))
    check(not relocated, "no base relocation of the original covers either instruction")
    interiors = [(va + 1, va + len(old)) for va, old, _ in P.GATHER]
    hits = [f"{address:#x}: {text}" for address, _, text in ndisasm(Image(original).read(start, end - start), start)
            if branch_target(text) is not None
            and any(lo <= branch_target(text) < hi for lo, hi in interiors)]
    check(not hits, "no branch of the handler lands inside either instruction")
    # The byte the second one cleared: set by the rule's constructor,
    # tested by the container's can_use, and written nowhere else as a
    # `mov byte [reg+160h],imm` of this class.
    text = original[P.TEXT_FILE:P.TEXT_FILE + P.TEXT_VSIZE]
    clears = sorted(P.TEXT_VA + m.start() for m in re.finditer(rb"\xC6[\x80-\x87]\x60\x01\x00\x00\x00", text))
    sets = sorted(P.TEXT_VA + m.start() for m in re.finditer(rb"\xC6[\x80-\x87]\x60\x01\x00\x00\x01", text))
    check(lock_va in clears and 0x313A1F in sets
          and not any(0x313000 <= va < 0x315000 for va in clears if va != lock_va),
          f"`mov byte [reg+0x160],0` is at {lock_va:#x} and nowhere else in the rule's code "
          "(the constructor sets it at 0x313A1F)")


def test_start_up_folder(original, image, mode):
    """A Documents folder the game may not write to (client_patch.md
    section 3.10): the mount without a root, and no journal by default."""
    stock = Image(original)
    va, old, new = P.MOUNT_NO_ROOT
    journal_va, journal_old, journal_new = P.JOURNAL_DEFAULT
    if mode == "cert-only":
        check(image.read(va, len(old)) == old and image.read(journal_va, 5) == journal_old,
              "cert-only: mounter::finish and the journal's default are the original's")
        return
    # mounter::finish from the read of the mount's root to its `ret 0Ch`.
    start, end = 0x46B04, 0x46B3D
    tail = ["mov eax,[eax+0x34]", "test eax,eax", "jnz 0x46b0f", "xor ecx,ecx", "jmp short 0x46b12",
            "mov ecx,[eax+0x40]", "mov ebp,[ebp+0x524]", "mov edx,[ebp+0x201c8]", "lea eax,[ebp+0x201c8]",
            "neg edx", "sbb edx,edx", "test edx,0x1af330", "jz 0x46b36", "push ecx", "call 0x10e2a0",
            "pop edi", "pop esi", "pop ebp", "pop ecx", "ret 0xc"]
    before = [text for _, _, text in ndisasm(stock.read(start, end - start), start)]
    check(before == tail,
          "the original mounter::finish ends: root = [mount+0x34]; ecx = root ? [root+0x40] : 0; "
          "the on_mounted callback, when there is one, is called with ecx")
    after = [text for _, _, text in ndisasm(image.read(start, end - start), start)]
    check(after == tail[:3] + [f"jmp short {P.MOUNT_NO_ROOT_END:#x}", "nop", "nop"] + tail[5:],
          f"{va:#x}: `jmp short {P.MOUNT_NO_ROOT_END:#x}` and two nops: a mount without a root goes to the "
          "function's end (the four pops and `ret 0xc`), the path of a mount with a root is the original's")
    relocated = [rva for kind, rva in stock.relocations().values()
                 if kind == 3 and rva + 4 > va and rva < va + len(old)]
    check(not relocated, "no base relocation of the original covers the four bytes")
    whole = ndisasm(stock.read(0x469F0, 0x14D), 0x469F0)
    hits = [f"{address:#x}: {text}" for address, _, text in whole
            if branch_target(text) is not None and va < branch_target(text) < va + len(old)]
    targets = sorted({address for address, _, text in whole if branch_target(text) == P.MOUNT_NO_ROOT_END})
    check(not hits and len(targets) >= 3,
          f"no branch of mounter::finish lands inside them; {P.MOUNT_NO_ROOT_END:#x} is where its own "
          f"{len(targets)} early exits go")
    # The callback this protects: it reads the node's flags without looking.
    check(stock.read(0xF9A8B, 8) == bytes.fromhex("8b74240c0fb74630"),
          "resources_manager::on_mounted 0xF9A80 does `mov esi,[esp+0xc]` / `movzx eax,word [esi+0x30]`: "
          "a null node is an access violation at 0xF9A8F")

    # The journal: the option's initializer.
    literal = lambda value: stock.read(value - 1, 3)
    check(literal(P.JOURNAL_DEFAULT_ON) == b"\x001\x00" and literal(P.JOURNAL_DEFAULT_OFF) == b"\x000\x00",
          f'{P.JOURNAL_DEFAULT_ON:#x} is the text "1" and {P.JOURNAL_DEFAULT_OFF:#x} the text "0", '
          "both in the original's read-only data")
    initializer = lambda source, value: [text for _, _, text in ndisasm(source.read(journal_va - 5, 16), journal_va - 5)] \
        == [f"push dword {P.JOURNAL_OPTION:#x}", f"mov eax,{value:#x}", "call 0xddff0", "ret"]
    check(initializer(stock, P.JOURNAL_DEFAULT_ON) and initializer(image, P.JOURNAL_DEFAULT_OFF),
          f"{journal_va:#x}: the initializer of -record_journal ({P.JOURNAL_OPTION:#x}) gives \"0\" "
          "in the place of \"1\"; the push before it and the call after it are the original's")
    others = [at for at in (0x84CF92, 0x84E4F2, 0x853B02)
              if [text for _, _, text in ndisasm(stock.read(at, 16), at)][1:]
              == [f"mov eax,{P.JOURNAL_DEFAULT_OFF:#x}", "call 0xddff0", "ret"]]
    check(len(others) == 3, "three other options of the original are given that \"0\" by the same three instructions")
    name, description = stock.read(0x9FB560, 15), stock.read(0x9FB544, 26)
    check(name == b"record_journal\0" and description == b"disable journal recording\0"
          and stock.read(P.JOURNAL_OPTION + 0x210, 4) == struct.pack("<I", 0x9FB560),
          "the option object's name is `record_journal` (its description: \"disable journal recording\")")
    kinds = lambda source: sorted(kind for kind, rva in source.relocations().values() if rva == journal_va + 1)
    check(kinds(stock) == [3] and kinds(image) == [3],
          f"the base relocation of the address ({journal_va + 1:#x}) is active in the original and stays so")
    # One reader: the game's start decides between replay, record and none.
    text = original[P.TEXT_FILE:P.TEXT_FILE + P.TEXT_VSIZE]
    readers = sorted(P.TEXT_VA + m.start() for m in re.finditer(re.escape(struct.pack("<I", P.JOURNAL_OPTION)), text))
    check(readers == [0x1F69B3, 0x851CB1, 0x851CC3] and stock.read(0x1F69B2, 19) == bytes.fromhex(
        "be28d6a900e804dce1ff33d284c00f95c28bf2"),
          "the option is named where it is registered and at 0x1F69B2 only: `mov esi,0xa9d628` / "
          "`call 0x145c0` / `setnz dl` / `mov esi,edx`, the journal's use (0: none, 1: record)")
    check(stock.read(0xEB317, 4) == bytes.fromhex("85f67463"),
          "journaling's start 0xEB310 makes no journal for use 0 (`test esi,esi` / `jz 0xeb37e`)")


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


def test_default_server(original, image, mode, server):
    va, old = P.DEFAULT_SERVER_HOOK
    if server is None:
        check(image.read(va, len(old)) == old, "without an embedded server the original missing-server path stays")
        return
    start = P.CAVES[mode] + P.DEFAULT_SERVER_OFFSET
    stub = open(P.default_server_bin(mode), "rb").read()
    hook = ndisasm(image.read(va, len(old)), va)
    check(hook[0][2].startswith("jmp ") and branch_target(hook[0][2]) == start
          and all(text == "nop" for _, _, text in hook[1:]),
          "missing-server error jumps to the embedded-address stub")
    rows = ndisasm(image.read(start, len(stub)), start)
    key_delta = P.DEFAULT_SERVER_OPTION - (start + 9)
    key_operand = f"esi{'+' if key_delta >= 0 else '-'}{abs(key_delta):#x}"
    check([text.replace(",byte +", ",") for _, _, text in rows] == [
        "push ecx", "push edx", "push esi", "push edi", f"call {start + 9:#x}", "pop esi",
        f"lea edx,[{key_operand}]", "add esi,0x2a", "mov edi,[edx]", "cld", "lodsb", "stosb",
        "test al,al", f"jnz {start + 22:#x}", "dec edi", "mov [edx+0x4],edi",
        "mov dword [edx+0x224],0x4", "pop edi", "pop esi", "pop edx", "pop ecx",
        f"jmp {P.DEFAULT_SERVER_RETURN:#x}"],
        "stub initializes the server key and type_string, preserves registers and repeats the original option read")
    # Re-enter before all string initialization. At the hook EDX no longer
    # holds the server buffer limit; entering at 0x1EAE47 stores a stale EDX.
    check(P.DEFAULT_SERVER_RETURN == 0x1EAE2D and image.read(P.DEFAULT_SERVER_RETURN, 50)
          == bytes.fromhex("8d8578fbffff8bc889856cfbffff898d70fbffff"
                           "8d9578fdffff8d856cfbffffb950c5a900"
                           "899574fbffffc68578fbffff00"),
          "fallback rebuilds the stack string, including its 512-byte buffer limit, before reading the key")
    encoded = P.parse_server(server).encode("ascii") + b"\0"
    check(image.read(start + len(stub), len(encoded)) == encoded,
          "embedded host:port is terminated and fits the client buffers")
    # Preserve all option checks, address parsing and client creation up to
    # the old error. Explicit -server and -replay never enter our stub.
    begin = 0x1EAE2D
    check(image.read(begin, va - begin) == Image(original).read(begin, va - begin),
          "explicit-server and replay selection, parsing and connection code are unchanged")
    missing = ndisasm(image.read(0x1EB0AB, 6), 0x1EB0AB)
    check(missing[0][2].startswith("jz ") and branch_target(missing[0][2]) == va,
          "the original no-server/no-replay branch is what reaches the hook")


def test_server_options(original):
    import make_client_026g as overlay

    for value in ("026g.skhid.link:1234", "localhost:1", "a" * 63 + ":65535"):
        check(P.parse_server(value) == value, f"accepts build-time server {value}")
    for value in ("host", "host:0", "host:65536", "host:-1", "host:１２３４",
                  "a" * 64 + ":1234", "host:1234\0", "host:1234\n", "host;other:1234"):
        try:
            P.parse_server(value)
        except SystemExit:
            pass
        else:
            check(False, f"invalid address accepted: {value!r}")
    try:
        P.build(original, "cert-only", server="localhost:1234")
    except SystemExit:
        check(True, "cert-only refuses an embedded server")
    else:
        check(False, "cert-only must refuse an embedded server")
    for embedded in (False, True):
        line = overlay.launch_line("localhost:1234", "english", embedded)
        batch = overlay.bat("localhost:1234", "english", embedded).decode("ascii")
        start = next(line for line in batch.splitlines() if line.startswith('start "" '))
        check(("-server=" in line) != embedded and ("-server=" in start) != embedded,
              f"overlay launch line and batch use {'embedded' if embedded else 'explicit'} server")


# sign_in_on_connected: from the copy of the version string to the write of
# the byte after the field (client_patch.md section 6).
VERSION_FIELD = (0x380847, 0x38088F)
SIGN_IN_ON_CONNECTED = (0x3807D0, 0x380940)   # to `sign_in`
REGISTERS = {"eax": ("eax", 4), "ax": ("eax", 2), "al": ("eax", 1),
             "ecx": ("ecx", 4), "cx": ("ecx", 2), "cl": ("ecx", 1),
             "edx": ("edx", 4), "dx": ("edx", 2), "dl": ("edx", 1)}


def version_field(image):
    """The 13 bytes sign_in_on_connected writes at edi after the user name
    (the 12-byte version field and the byte after it), worked out from its
    instructions. A byte is an int, "eula" (the flag read from [ebp+10h]) or
    None (not known). Returns None and says why when an instruction is not
    one of the forms the original has."""
    start, end = VERSION_FIELD
    registers = {"eax": [None] * 4, "ecx": [None] * 4, "edx": [None] * 4}
    xmm0, stack, out = [None] * 8, {}, [None] * 13

    def get(name):
        register, size = REGISTERS[name]
        return registers[register][:size]

    def put(name, values):
        register, size = REGISTERS[name]
        assert len(values) == size
        registers[register][:size] = values

    absolute = re.compile(r"^mov (\w+),\[(?:dword )?0x([0-9a-f]+)\]$")
    to_stack = re.compile(r"^mov \[esp\+0x([0-9a-f]+)\],(\w+)$")
    immediate = re.compile(r"^mov byte \[esp\+0x([0-9a-f]+)\],0x([0-9a-f]+)$")
    to_field = re.compile(r"^mov \[edi\+0x([0-9a-f]+)\],(\w+)$")
    for address, _, text in ndisasm(image.read(start, end - start), start):
        if m := absolute.match(text):
            size = REGISTERS[m.group(1)][1]
            put(m.group(1), list(image.read(int(m.group(2), 16), size)))
        elif text == "pxor xmm0,xmm0":
            xmm0 = [0] * 8
        elif m := re.match(r"^movq \[esp\+0x([0-9a-f]+)\],xmm0$", text):
            for i, value in enumerate(xmm0):
                stack[int(m.group(1), 16) + i] = value
        elif m := re.match(r"^movq xmm0,\[esp\+0x([0-9a-f]+)\]$", text):
            xmm0 = [stack.get(int(m.group(1), 16) + i) for i in range(8)]
        elif m := to_stack.match(text):
            for i, value in enumerate(get(m.group(2))):
                stack[int(m.group(1), 16) + i] = value
        elif m := immediate.match(text):
            stack[int(m.group(1), 16)] = int(m.group(2), 16)
        elif text == "xor eax,eax":
            put("eax", [0] * 4)
        elif text == "mov ecx,[ebp+0xc]":
            put("ecx", [None] * 4)
        elif text == "mov dl,[ebp+0x10]":
            put("dl", ["eula"])
        elif text == "movq [edi],xmm0":
            out[:8] = xmm0
        elif m := to_field.match(text):
            for i, value in enumerate(get(m.group(2))):
                out[int(m.group(1), 16) + i] = value
        elif text not in ("nop", "add edi,esi"):
            check(False, f"{address:#x}: `{text}` is not an instruction of the version field's code")
            return None
    return out


def test_revision(original, image, mode, revision):
    """The patch revision in the sign-in (client_patch.md section 6)."""
    va, old, new = P.revision_site(revision)
    stock = list(b"0.26g0" + bytes(6)) + ["eula"]
    check(version_field(Image(original)) == stock,
          'the original fills the version field with "0.26g0" and six zero bytes, then the EULA flag')
    field = version_field(image)
    if mode == "cert-only":
        check(image.read(va, len(old)) == old and field == stock, "cert-only: the version field's code is the original's")
        return
    check(1 <= revision <= 255 and new == bytes([0xC6, 0x44, 0x24, 0x27, revision, 0x90]),
          f"the revision is {revision}: a byte, not 0")
    rows = {address: text for address, _, text in ndisasm(image.read(*[VERSION_FIELD[0], VERSION_FIELD[1] - VERSION_FIELD[0]]),
                                                         VERSION_FIELD[0])}
    check(rows.get(0x38085E) == "mov [esp+0x20],ecx" and rows.get(va) == f"mov byte [esp+0x27],{revision:#x}"
          and rows.get(va + 5) == "nop" and rows.get(0x380868) == "xor eax,eax",
          f"{va:#x}: `mov byte [esp+0x27],{revision:#x}` / `nop` between `mov [esp+0x20],ecx` and `xor eax,eax`")
    expected = list(b"0.26g0" + bytes([0, revision]) + bytes(4)) + ["eula"]
    check(field == expected,
          f'the version field is "0.26g0", a zero, the revision {revision} in byte 7 and four zero bytes; '
          "the EULA flag follows as before")
    inside = lambda relocations: sorted((address, kind) for kind, address in relocations.values()
                                        if VERSION_FIELD[0] <= address < VERSION_FIELD[1])
    check(inside(Image(original).relocations()) == [(0x380849, 3), (0x380850, 3), (0x380864, 3)],
          "the original has three base relocations in this code: 0x380849, 0x380850, 0x380864")
    check(inside(image.relocations()) == [(0x380849, 3), (0x380850, 3), (0x380864, 0)],
          "the one under the rewritten instruction (0x380864) is disabled, the other two are untouched")
    start, end = SIGN_IN_ON_CONNECTED
    hits = [f"{address:#x}: {text}" for address, _, text in ndisasm(Image(original).read(start, end - start), start)
            if branch_target(text) is not None and va < branch_target(text) < va + len(old)]
    check(not hits, "no branch of sign_in_on_connected lands inside that instruction")


def test_no_branch_into_hooks(original):
    print("branches into the hooks (original exe)")
    image = Image(original)
    interiors = [(va + 1, va + len(old)) for va, old in [*P.HOOKS, P.DEFAULT_SERVER_HOOK]]
    inside = lambda target: any(lo <= target < hi for lo, hi in interiors)
    hits = []
    for start, end in HOSTS:
        for address, _, text in ndisasm(image.read(start, end - start), start):
            target = branch_target(text)
            if target is not None and inside(target):
                hits.append(f"{address:#x}: {text}")
    check(not hits, "no branch of the host functions lands inside a hook" + "".join("\n   " + h for h in hits))
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


# What the late join's rewritten sites are meant to disassemble to.
LATE_JOIN_TEXT = {
    0x279610: "mov eax,0x3", 0x27A2E0: "mov eax,0x3", 0x3237B4: "mov dl,0x14", 0x26D8F3: "mov al,0x14",
    0x2F5ACF: "cmp bl,0x14", 0x2F5AF4: "cmp bl,0x14", 0x2F5BE6: "cmp bl,0x14", 0x2F5C03: "cmp bl,0x14",
}


def late_join_sites():
    """[(address, length)] of every site the late join changes beside the
    revision and the ready hook, which the build without it changes too."""
    return [(va, len(old)) for va, old, _ in P.LATE_JOIN_PATCHES] + \
        [(va, len(old)) for va, old, _, _ in P.LATE_JOIN_HOOKS] + \
        [(P.RUN_UNFOCUSED_VA, len(P.RUN_UNFOCUSED[1])), (P.REMOVE_MODEL_VA, len(P.REMOVE_MODEL[1])),
         (P.PREGAME_PROGRESS_VA, len(P.PREGAME_PROGRESS[1]))]


def test_late_join_bin():
    print("late-join code (late_join_026g.section.bin)")
    code = open(P.late_join_bin(), "rb").read()
    kept = json.load(open(os.path.splitext(P.late_join_bin())[0] + ".json", encoding="utf-8"))
    check(kept["size"] == len(code) and kept["sha256"] == hashlib.sha256(code).hexdigest(),
          f"the .bin is the one its .json describes ({len(code)} bytes, built with {kept['built_with']['cc']})")
    names = {"LJ_COMPLETE": "lj_complete", "LJ_PREPARE": "lj_prepare", "LJ_ACTIVATE": "lj_activate",
             "LJ_DEACTIVATE": "lj_deactivate", "LJ_DESTROYED": "lj_player_destroyed",
             "LJ_INSTALL": "lj_native_install", "LJ_ANIMATIONS": "lj_native_animations"}
    c_code = P.LATE_JOIN_VA + P.LATE_JOIN_ASM_SIZE
    defines = [f"-DCAVE={P.LATE_JOIN_VA}", f"-DC_CODE={c_code}"]
    defines += [f"-D{define}={c_code + kept['symbols'][name]}" for define, name in names.items()]
    defines += [f"-DSTATE_VA={P.late_join_state_va(code)}", f"-DUPLOAD_READY={P.SECTION_VA}"]
    with tempfile.TemporaryDirectory() as tmp:
        out = os.path.join(tmp, "out.bin")
        subprocess.run([tool("nasm"), "-f", "bin", *defines, os.path.join(HERE, "late_join_026g.asm"), "-o", out],
                       check=True, timeout=60)
        fresh = open(out, "rb").read()
    check(len(fresh) == P.LATE_JOIN_ASM_SIZE and fresh == code[:P.LATE_JOIN_ASM_SIZE],
          f"its first {P.LATE_JOIN_ASM_SIZE} bytes are what nasm makes of late_join_026g.asm today")
    check(all(0 < offset < len(code) - P.LATE_JOIN_ASM_SIZE for offset in kept["symbols"].values()),
          "the C functions the assembly calls are inside the C code")
    entries = sorted({offset for _, _, offset, _ in P.LATE_JOIN_HOOKS} | {P.LATE_JOIN_READY_OFFSET})
    check(all(code[offset] != 0xCC and (offset == 0 or code[offset - 1] == 0xCC) for offset in entries),
          f"the {len(entries)} entries the hooks go to start code after int3 padding")
    return code


def test_clock_factor(original, image, before):
    """Check 16: the game's time_floating_factor."""
    source = Image(original)
    va, old, new = P.CLOCK_FACTOR_PATCH
    value = lambda blob: struct.unpack("<f", blob)[0]
    check(source.read(va, 4) == old and before.read(va, 4) == old and image.read(va, 4) == new
          and abs(value(new) - P.CLOCK_FACTOR) < 1e-7 and abs(value(old) - P.CLOCK_FACTOR_ORIGINAL) < 1e-8,
          f"{va:#x}: the default of time_floating_factor is {value(image.read(va, 4)):g}; the original and the "
          f"build without the late join have {value(source.read(va, 4)):g}")
    low, high = value(source.read(P.CLOCK_FACTOR_MIN_VA, 4)), value(source.read(P.CLOCK_FACTOR_MAX_VA, 4))
    register = [text for _, _, text in ndisasm(source.read(P.CLOCK_FACTOR_REGISTER_VA - 0x39, 0x63),
                                               P.CLOCK_FACTOR_REGISTER_VA - 0x39)]
    wanted = [f"mov ecx,0x{P.CLOCK_FACTOR_OPTION:x}",
              f"movss xmm0,dword [dword 0x{P.CLOCK_FACTOR_MIN_VA:x}]",
              f"movss dword [dword 0x{P.CLOCK_FACTOR_OPTION + 0x44:x}],xmm0",
              f"movss xmm0,dword [dword 0x{P.CLOCK_FACTOR_MAX_VA:x}]",
              f"mov dword [dword 0x{P.CLOCK_FACTOR_OPTION + 0x40:x}],0x{va:x}",
              f"movss dword [dword 0x{P.CLOCK_FACTOR_OPTION + 0x48:x}],xmm0"]
    name_at = struct.unpack("<I", source.read(P.CLOCK_FACTOR_OPTION + 0x10, 4))[0]
    name = source.read(name_at, 21)
    check(all(text in register for text in wanted) and register[-1] == "ret"
          and name == b"time_floating_factor\0",
          f"the console variable at {P.CLOCK_FACTOR_OPTION:#x} is `time_floating_factor`: its value is the float at "
          f"{va:#x}, its least {low:g} and its most {high:g} (registered at {P.CLOCK_FACTOR_REGISTER_VA - 0x39:#x})")
    check(abs(low - 0.001) < 1e-9 and high == 0.5 and low <= P.CLOCK_FACTOR <= high,
          f"{P.CLOCK_FACTOR:g} is inside the range the game allows the setting")
    text_start = P.file_offset(P.TEXT_VA)
    text = bytes(original[text_start:text_start + P.TEXT_VSIZE])
    needle, held, at = struct.pack("<I", va), [], 0
    while (at := text.find(needle, at)) >= 0:
        held.append(P.TEXT_VA + at)
        at += 1
    read = [text for _, _, text in ndisasm(source.read(P.CLOCK_FACTOR_READ_VA, 0x2A), P.CLOCK_FACTOR_READ_VA)]
    check(held == [P.CLOCK_FACTOR_READ_VA + 4, P.CLOCK_FACTOR_REGISTER_VA + 6]
          and read[0] == f"movss xmm0,dword [dword 0x{va:x}]" and read[1] == "lea esi,[ebx+0x52d8]"
          and "call 0x146b60" in read and read[-1] == "movss dword [esi+0x28],xmm0",
          f"the value's address is in the code twice: the registration, and the `game` constructor "
          f"({P.CLOCK_FACTOR_READ_VA:#x}), which puts it into the game clock's timer (game+52D8h) at +28h")
    offset = [text for _, _, text in ndisasm(source.read(0x1EE9E0, 0x4C), 0x1EE9E0)]
    handler = [text for _, _, text in ndisasm(source.read(0x27B7F0, 0x34), 0x27B7F0)]
    check("movss xmm0,dword [esi+0x28]" in offset and "mulss xmm0,xmm1" in offset
          and "movss dword [esi+0x24],xmm0" in offset
          and "add esi,0x52d8" in handler and "call 0x1ee9e0" in handler,
          "set_offset (0x1EE9E0) makes the timer's rate of that field and the correction's sign, and the 0xE7 "
          "handler (0x27B7F0) calls it for the game clock's timer with the offset as it came")
    under = lambda relocations: [at for kind, at in relocations.values() if kind == 3 and at + 4 > va and at < va + 4]
    check(not under(source.relocations()) and not under(image.relocations()),
          "no base relocation is under the four bytes, in the original or in the build")


def test_statistics_count(original, image, before):
    """Check 17: the statistics handler keeps the score of all twenty places."""
    source = Image(original)
    va, old, new = P.STATISTICS_COUNT
    check(source.read(va, len(old)) == old and before.read(va, len(old)) == old and image.read(va, len(old)) == new,
          f"{va:#x}: start_match's count is `mov al,0x14` + 4 nop; the original and the build without the late join "
          "copy the roster's")
    start = P.STATISTICS_START_MATCH_VA
    texts = lambda blob, at: [text for _, _, text in ndisasm(blob.read(at, 0xE5), at)]
    was, now = texts(source, start), texts(image, start)
    wanted = ["mov [esi+0x3238],ecx", "mov [ecx+0x18fa8],esi", "mov [esi+0x323c],edx", "mov al,[edx+0x577b5]",
              "push edi", "mov [esi+0x3240],al", "mov eax,[esi+0x3218]"]
    patched = wanted[:3] + ["mov al,0x14"] + ["nop"] * 4 + wanted[4:]
    at = was.index(wanted[0]) if wanted[0] in was else -1
    loop = ["cmp bl,[esi+0x3240]", "movzx edx,word [esi+ecx*2+0x31f0]", "call 0x4d370"]
    live = ["mov al,[edx+0x577b5]", "mov [esi+0x31e8],al"]
    pairs = lambda rows: [i for i in range(len(rows) - 1) if rows[i:i + 2] == live]
    check(at >= 0 and was[at:at + len(wanted)] == wanted and all(text in was for text in loop)
          and len(pairs(was)) == 1 and len(pairs(now)) == 1 and pairs(was)[0] + 4 == pairs(now)[0]
          and now[at:at + len(patched)] == patched and was[at + len(wanted):] == now[at + len(patched):],
          f"start_match ({start:#x}) puts the loaded count in +3240h alone, tells the hud the score of every counted "
          "place (+31F0h, through 0x4D370), and copies the roster into the shared statistics' live count (+31E8h) "
          "apart; the build keeps that copy")
    deserialize = texts(source, 0x26DB60)[:7]
    recalculate = [text for _, _, text in ndisasm(source.read(0x26DC30, 0x190), 0x26DC30)]
    slots = [struct.unpack("<I", image.read(0xA0755C + slot, 4))[0] for slot in (0x34, 0x3C, 0x44)]
    check(deserialize[5:7] == ["call 0x2f5be0", "call 0x26dc30"] and "cmp al,[edi+0x3240]" in recalculate
          and "mov [edi+eax*2+0x31f0],bx" in recalculate and slots == [0x26DB60, start, 0x26DB80],
          "the handler's deserialize (slot +34h) reads the shared statistics and runs recalculate_scores (0x26DC30), "
          "which works out every counted place's score into +31F0h; slot +3Ch is start_match, +44h on_event")
    # Every instruction of .text with +3240h on a [register+disp32] operand.
    text_start = P.file_offset(P.TEXT_VA)
    text = bytes(original[text_start:text_start + P.TEXT_VSIZE])
    sites, at = [], 0
    while (at := text.find(b"\x40\x32\x00\x00", at)) >= 0:
        if at >= 2 and text[at - 1] & 0xC0 == 0x80 and text[at - 1] & 7 != 4:
            sites.append(P.TEXT_VA + at - 2)
        at += 1
    first = lambda address: ndisasm(source.read(address, 8), address)[0][2]
    zeroing = {0x26D78E: (0x26D76A, "xor ebx,ebx"), 0x26D8C4: (0x26D89B, "xor edx,edx"),
               0x26D9E0: (0x26D9DE, "xor eax,eax"), 0x278CD0: (0x278CCE, "xor eax,eax")}
    readers = {0x26D91D: "cmp bl,[esi+0x3240]", 0x26DC54: "cmp al,[edi+0x3240]", 0x26D8FA: "mov [esi+0x3240],al"}
    check(sorted(sites) == sorted([*zeroing, *readers])
          and all(first(site) == text_ for site, text_ in readers.items())
          and all(first(site).startswith("mov [esi+0x3240],") and first(zero) == text_
                  for site, (zero, text_) in zeroing.items()),
          f"+3240h is in .text {len(sites)} times: start_match's store and loop, recalculate_scores' loop, and four "
          "stores of a register zeroed before them (the constructor, clear, and the two finish_match); on_event "
          "(0x26DB80, any place) has none")
    inside = lambda target: va < target < va + len(old)
    found = []
    for at in range(len(text) - 6):
        op, here = text[at], P.TEXT_VA + at
        if op in (0xE8, 0xE9) and inside(here + 5 + struct.unpack_from("<i", text, at + 1)[0]):
            found.append(here)
        elif op == 0x0F and 0x80 <= text[at + 1] <= 0x8F and inside(here + 6 + struct.unpack_from("<i", text, at + 2)[0]):
            found.append(here)
        elif (op == 0xEB or 0x70 <= op <= 0x7F or 0xE0 <= op <= 0xE3) and va - 0x82 <= here < va + len(old) \
                and inside(here + 2 + struct.unpack_from("<b", text, at + 1)[0]):
            found.append(here)
    found += [va + 1 + i for i in range(len(old) - 1) if original.find(struct.pack("<I", va + 1 + i)) != -1]
    # clear (0x26D880) and start_match, as the game runs them.
    rows = ndisasm(source.read(0x26D880, start + 0xE5 - 0x26D880), 0x26D880)
    check(not found and not any(inside(branch_target(text) or 0) for _, _, text in rows),
          "no rel32 branch of .text, no rel8 branch byte near them, no branch of clear or start_match and no 4-byte "
          "value aims inside the six bytes" + "".join(f"\n          {a:#x}" for a in found))


def aimed_inside(original, va, size, near):
    """Every rel32 branch of .text, rel8 branch byte within `near` bytes
    and 4-byte value of the file that aims inside bytes 1.. of [va, va+size)."""
    text_start = P.file_offset(P.TEXT_VA)
    text = bytes(original[text_start:text_start + P.TEXT_VSIZE])
    inside = lambda target: va < target < va + size
    found = []
    for at in range(len(text) - 6):
        op, here = text[at], P.TEXT_VA + at
        if op in (0xE8, 0xE9) and inside(here + 5 + struct.unpack_from("<i", text, at + 1)[0]):
            found.append(here)
        elif op == 0x0F and 0x80 <= text[at + 1] <= 0x8F and inside(here + 6 + struct.unpack_from("<i", text, at + 2)[0]):
            found.append(here)
        elif (op == 0xEB or 0x70 <= op <= 0x7F or 0xE0 <= op <= 0xE3) and va - near <= here < va + size \
                and inside(here + 2 + struct.unpack_from("<b", text, at + 1)[0]):
            found.append(here)
    return found + [va + i for i in range(1, size) if original.find(struct.pack("<I", va + i)) != -1]


def test_pregame_progress(original, image, before):
    """Check 18: the hud draws the team points while a match counts down."""
    source = Image(original)
    va, old, new = P.PREGAME_PROGRESS
    target = P.PREGAME_PROGRESS_TARGET
    rows = lambda blob, at, size: ndisasm(blob.read(at, size), at)
    check(source.read(va, len(old)) == old and before.read(va, len(old)) == old and image.read(va, len(old)) == new
          and [text for _, _, text in rows(image, va, len(new))] == [f"jmp 0x{target:x}"]
          and [text for _, _, text in rows(source, va, len(old))] == ["jmp 0x23b421"],
          f"{va:#x}: the countdown's `jmp 0x23b421` is `jmp 0x{target:x}`; the original and the build without the "
          "late join keep it")
    # update_ui from the test of the countdown flag to the players' rows.
    was = rows(source, 0x23B307, 0x23B421 - 0x23B307)
    texts = [text for _, _, text in was]
    at = {address: i for i, (address, _, _) in enumerate(was)}
    countdown = texts[at[0x23B307]:at[va] + 1]
    play = texts[at[0x23B371]:at[target]]
    check(countdown == ["cmp byte [edi+0x15c],0x0", "jz 0x23b32d", "mov al,0x1", "mov ecx,edi", "call 0x236ad0",
                        "mov eax,[edi+0xb0]", "lea ecx,[edi+0x30]", "push edi", "call 0x236b50", "jmp 0x23b421"]
          and play == ["xor al,al", "mov ecx,edi", "call 0x236ad0", "mov eax,[edi+0x28]", "push edi", "call 0x236a00"]
          and target in at,
          f"in the original the countdown shows the pregame (0x236AD0 with 1, 0x236B50) and jumps to the rows; play "
          f"shows the match time (0x236AD0 with 0, 0x236A00) and goes on at {target:#x}, an instruction of its own")
    after = texts[at[target]:]
    calls = [text for text in after if text.startswith("call")]
    leaving = [text for text in after if branch_target(text) is not None and not text.startswith("call")
               and not target <= branch_target(text) <= 0x23B421]
    check(calls == ["call 0x2364c0", "call 0x236610", "call 0x236c30", "call 0xe0210"] and not leaving
          and after[0] == "mov eax,[edi+0x18]",
          f"from {target:#x} to the rows the code draws the team points (0x2364C0), in Team Deathmatch the last "
          "players (0x236610) and the respawn time (0x236C30), lets go of the player it took (0xE0210 at the last "
          "count) and leaves to nowhere else")
    ret = lambda function: next(text for _, _, text in ndisasm(source.read(function, 0x400), function)
                                if text.startswith("ret"))
    pushed = lambda function: {source.read(int(text.split()[-1], 16), 32).split(b"\0")[0]
                               for _, _, text in ndisasm(source.read(function, 0x200), function)
                               if text.startswith("push dword 0xa")}
    rets = {function: ret(function) for function in (0x236AD0, 0x236B50, 0x236A00, 0x2375C0)}
    check(rets == {0x236AD0: "ret", 0x236B50: "ret 0x4", 0x236A00: "ret 0x4", 0x2375C0: "ret 0x10"}
          and b"root.set_game_progress" in pushed(0x2364C0) and b"root.set_last_players" in pushed(0x236610)
          and b"root.set_respawn_time" in pushed(0x236C30),
          "each call before the jump and before the target takes its arguments off the stack (both paths reach the "
          "target as deep), and the three after it are the hud movie's set_game_progress, set_last_players and "
          "set_respawn_time")
    found = aimed_inside(original, va, len(old), 0x82)
    check(not found, "no rel32 branch of .text, no rel8 branch byte near it and no 4-byte value aims inside the jump"
          + "".join(f"\n          {a:#x}" for a in found))


# The respawn rules' vtables: player_respawn_rule_base, _core, the client's
# player_respawn_rule, single_player_respawn_rule.
RESPAWN_RULE_VTABLES = (0xA16014, 0xA16084, 0xA06D1C, 0xA06D8C)


def branches_to(original, target):
    """Every rel32 call, jmp or jcc of .text whose target is `target`."""
    text_start = P.file_offset(P.TEXT_VA)
    text = bytes(original[text_start:text_start + P.TEXT_VSIZE])
    found = []
    for at in range(len(text) - 6):
        op, here = text[at], P.TEXT_VA + at
        if op in (0xE8, 0xE9) and here + 5 + struct.unpack_from("<i", text, at + 1)[0] == target:
            found.append(here)
        elif op == 0x0F and 0x80 <= text[at + 1] <= 0x8F and here + 6 + struct.unpack_from("<i", text, at + 2)[0] == target:
            found.append(here)
    return found


def rtti_name(image, vtable):
    """(the class name, the subobject's offset) of a vtable, from its
    complete object locator at vtable-4."""
    locator = struct.unpack("<I", image.read(vtable - 4, 4))[0]
    offset, descriptor = struct.unpack("<I", image.read(locator + 4, 4))[0], struct.unpack("<I", image.read(locator + 12, 4))[0]
    return image.read(descriptor + 8, 64).split(b"\0")[0].decode("latin1"), offset


def test_weapon_random(original, image, before):
    """Check 19: a spawn gives the new kit's weapons a random state of the
    match's own."""
    source = Image(original)
    va, old, offset, _ = P.WEAPON_RANDOM_HOOK
    entry = P.LATE_JOIN_VA + offset
    rows = lambda blob, at, size: ndisasm(blob.read(at, size), at)
    texts = lambda blob, at, size: [text for _, _, text in rows(blob, at, size)]
    check(source.read(va, len(old)) == old and before.read(va, len(old)) == old
          and texts(source, va, len(old)) == ["mov edx,[esi]", "mov eax,[edx+0x20]"]
          and texts(image, va, len(old)) == [f"call 0x{entry:x}"],
          f"{va:#x}: `mov edx,[esi]` / `mov eax,[edx+0x20]` is `call 0x{entry:x}`; the original and the build without "
          "the late join keep them")
    # The spawn, 0x316DA0 to its `ret 0Ch`.
    spawn = rows(source, P.WEAPON_SPAWN_VA, 0x316EB3 - P.WEAPON_SPAWN_VA)
    at = {address: text for address, _, text in spawn}
    order = [address for address, _, _ in spawn]
    between = lambda lo, hi: [at[a] for a in order if lo <= a < hi]
    writes = lambda register, lo, hi: [text for text in between(lo, hi)
                                       if re.match(rf"(mov|lea|pop|xchg|add|sub|xor|or|and|inc|dec) {register}\b", text)]
    check(at.get(0x316DC8) == "mov ebp,[esp+0x20]" and at.get(0x316DE3) == "mov [eax+0xc],dl"
          and at.get(0x316E50) == "mov esi,[eax+ecx*4]" and at.get(0x316EB0) == "ret 0xc"
          and between(0x316E57, 0x316E72) == ["mov edx,[esi]", "mov edx,[edx+0x1c]", "sub esp,byte +0x8",
                                               "lea eax,[edi+0x8]", "mov ecx,esi", "fstp dword [esp+0x4]",
                                               "fld dword [edi+0x14]", "fstp dword [esp]", "push eax", "push ebp",
                                               "call edx"]
          and at.get(0x316E76) == "jna 0x316ea0"
          and between(0x316EA0, 0x316EB3) == ["mov edx,[esi]", "mov eax,[edx+0x20]", "push byte +0x1", "mov ecx,esi",
                                               "call eax", "pop edi", "pop esi", "pop ebp", "pop ebx", "pop ecx",
                                               "ret 0xc"]
          and not writes("ebp", 0x316DCC, 0x316EAD) and not writes("esi", 0x316E52, 0x316EAC),
          "the spawn (0x316DA0) makes the target kit current (0x316DE3), takes its player into esi (0x316E50), "
          "initialises it (vtable+1Ch) with the time of its third argument (ebp), and at 0x316EA0, which the path "
          "past the protection's timer also jumps to, loads the call that puts it into the world (vtable+20h); "
          "esi and ebp are written nowhere between, and nothing after the hook reads a flag")
    slot = lambda vtable, number: struct.unpack("<I", source.read(vtable + number, 4))[0]
    holders = []
    position = original.find(struct.pack("<I", P.WEAPON_SPAWN_VA))
    while position != -1:
        holders.append(position)
        position = original.find(struct.pack("<I", P.WEAPON_SPAWN_VA), position + 1)
    tick = texts(source, 0x316820, 0x316A24 - 0x316820)
    check(all(slot(vtable, 0x5C) == P.WEAPON_SPAWN_VA and slot(vtable, 0x50) in (0x316820, 0x267980)
              for vtable in RESPAWN_RULE_VTABLES)
          and sorted(holders) == sorted(source.offset(vtable + 0x5C) for vtable in RESPAWN_RULE_VTABLES)
          and not branches_to(original, P.WEAPON_SPAWN_VA)
          and texts(source, 0x317D30, 5) == ["mov eax,[ecx]", "jmp [eax+0x50]"]
          and "mov dword [esi+0x120],0x317d30" in texts(source, 0x316700, 0x60)
          and all(text in tick for text in ("mov edx,[ebp+0xc]", "mov eax,[eax+0x5c]", "call eax")),
          "the spawn is slot +5Ch of every respawn rule's vtable and nowhere else (no call aims at it); the rules "
          "of a match tick through 0x317D30 (slot +50h, 0x316820), which spawns each place through slot +5Ch with "
          "the time it was given")
    player = struct.unpack("<I", source.read(0x9FB35C + 0x1C, 4))[0]
    initialize = texts(source, 0x2DAC00, 0x350)
    check(player == 0x1F2540 and "call 0x2dac00" in texts(source, player, 0x100)
          and "mov edi,[esi+0x10c]" in initialize and "call 0x2fcb70" in initialize
          and initialize.index("call 0x2fcb70") == initialize.index("mov edi,[esi+0x10c]") + 3
          and all(text in texts(source, 0x2FD19A, 0x30) for text in ("lea esi,[edi+0x10c]", "mov ebp,0x17")),
          "the player's initialize (vtable+1Ch, 0x1F2540) runs base_player's (0x2DAC00), which sets the inventory "
          "(player+10Ch) up from the profile (0x2FCB70) and makes no object; the inventory's 23 slots are at +10Ch")
    weapon_core = texts(source, 0x2DF0D0, 0x400)
    weapon = texts(source, 0x243640, 0x30)
    bound = []
    position = original.find(struct.pack("<I", P.WEAPON_BIND_VA))
    while position != -1:
        bound.append(position)
        position = original.find(struct.pack("<I", P.WEAPON_BIND_VA), position + 1)
    names = {rtti_name(source, item_vtable) for item_vtable in (0xA0422C, 0xA1114C)}
    check(all(text in weapon_core for text in ("lea edi,[ebp+0x10]", "mov dword [edi],0xa1114c",
                                               "mov dword [ebp+0x0],0xa11094", "mov dword [ebp+0x5bc],0x1"))
          and weapon[weapon.index("call 0x2df0d0"):][1:3] == ["mov dword [ebx],0xa04174", "mov dword [ebx+0x10],0xa0422c"]
          and sorted(bound) == sorted(source.offset(item_vtable + 0x70) for item_vtable in (0xA0422C, 0xA1114C))
          and names == {(".?AVweapon@survarium@@", 0x10), (".?AVweapon_core@survarium@@", 0x10)},
          f"weapon_core's constructor puts its item at +10h and 1 into +5BCh; the client's weapon is one; "
          f"{P.WEAPON_BIND_VA:#x} (weapon_core's bind) is in the file only as +70h of their two item vtables")
    # Every [register+5BCh] of .text that is not on the stack.
    text_start = P.file_offset(P.TEXT_VA)
    text = bytes(original[text_start:text_start + P.TEXT_VSIZE])
    sites, position = [], 0
    while (position := text.find(struct.pack("<I", P.WEAPON_RANDOM_FIELD), position)) >= 0:
        modrm, sib = text[position - 1], text[position - 2]
        if modrm & 0xC0 == 0x80 and modrm & 7 != 4:
            sites.append(P.TEXT_VA + position - 2)
        elif sib & 0xC0 == 0x80 and sib & 7 == 4 and modrm & 7 != 4:
            sites.append(P.TEXT_VA + position - 3)
        position += 1
    field = {0x2DF429: "mov dword [ebp+0x5bc],0x1", 0x2E1DCC: "lea esi,[ebx+0x5bc]", 0x2E2195: "lea eax,[esi+0x5bc]",
             0x2E315D: "lea eax,[ebx+0x5bc]", 0x2E34C1: "mov [edi+0x5bc],ecx"}
    check(sorted(sites) == sorted(field) and all(texts(source, site, 10)[0] == want for site, want in field.items())
          and "call 0x2e4850" in texts(source, 0x2E1DD2, 0x30) and "mov [esp+0x78],eax" in texts(source, 0x2E219B, 0x30)
          and all("mov ecx,0xbc8f" in texts(source, draw, 0x40) and "push dword 0x7fffffff" in texts(source, draw, 0x50)
                  for draw in (0x2AB6E0, 0x2E49F0)),
          "+5BCh of a register is in .text five times: the constructor's 1, the two spreads that draw from it "
          "(minstd_rand: 48271 mod 2^31-1, 0x2AB6E0 and 0x2E49F0), the serialize and the deserialize's store")
    hook = rows(image, entry, 0x80)
    hook = hook[:next(i for i, (_, _, text) in enumerate(hook) if text == "ret") + 1]
    hook_texts = [text for _, _, text in hook]
    pc = next(i for i, (_, _, text) in enumerate(hook) if text.startswith("call"))
    add = re.match(r"add ebx,0x([0-9a-f]+)$", hook_texts[pc + 2])
    stores = [text for text in hook_texts if re.match(r"mov \[", text)]
    inside = all(entry <= branch_target(text) < hook[-1][0] + 1 for text in hook_texts
                 if branch_target(text) is not None)
    check(hook_texts[0] == "pusha" and hook_texts[-4:] == ["popa", "mov edx,[esi]", "mov eax,[edx+0x20]", "ret"]
          and hook_texts[pc + 1] == "pop ebx" and branch_target(hook_texts[pc]) == hook[pc + 1][0]
          and add is not None and (hook[pc + 1][0] + int(add.group(1), 16)) & 0xFFFFFFFF == P.WEAPON_BIND_VA
          and "cmp [eax+0x70],ebx" in hook_texts and "mov ecx,[esi+0x10c]" in hook_texts
          and "cmp edi,byte +0x17" in hook_texts and "mov ecx,0x7ffffffe" in hook_texts
          and stores == [f"mov [eax+0x{P.WEAPON_RANDOM_FIELD - 0x10:x}],edx"] and inside,
          f"weapon_random_hook ({entry:#x}): pushad; for the 23 slots of the player's inventory, an item whose "
          f"vtable+70h is {P.WEAPON_BIND_VA:#x} (taken relative to the code) gets a state at item+5ACh (weapon+5BCh) "
          "and nothing else is written; popad, the two instructions, ret; no branch leaves it")
    found = aimed_inside(original, va, len(old), 0x82)
    check(not found, "no rel32 branch of .text, no rel8 branch byte near it and no 4-byte value aims inside the five "
          "bytes (0x316E76 aims at their start)" + "".join(f"\n          {a:#x}" for a in found))


def test_late_join(original, without, server, source):
    """The default build with the late join against `without`, the build
    of the same server without it (checked by everything above)."""
    print(f"  the late join on top (patch revision {P.REVISION})")
    code = open(P.late_join_bin(), "rb").read()
    upload = open(P.default_bin("section"), "rb").read()
    data, description = P.build(original, "section", upload, server, code)
    print("  " + description)
    image, before = Image(data), Image(without)
    headers = image.headers
    sections, old_sections = image.sections, before.sections
    name, rva, vsize, raw, rsize, flags = sections[-2]
    align = lambda value, to: (value + to - 1) // to * to
    first_page = bytes(without[old_sections[-1][3]:old_sections[-1][3] + old_sections[-1][2]])
    check(len(sections) == 9 and sections[:7] == old_sections[:7], "9 sections; the original's seven are untouched")
    check(name == P.SECTION_NAME and rva == P.SECTION_RVA and flags == P.SECTION_CHARACTERISTICS
          and vsize == P.LATE_JOIN_OFFSET + len(code) and rsize == align(vsize, headers.file_alignment)
          and raw == old_sections[-1][3],
          f".srp: virtual {vsize:#x} = one page and the {len(code)} bytes of late-join code, file {raw:#x}+{rsize:#x}")
    body = bytes(data[raw:raw + rsize])
    check(body[:len(first_page)] == first_page
          and set(body[len(first_page):P.LATE_JOIN_OFFSET]) <= {0xCC}
          and body[P.LATE_JOIN_OFFSET:P.LATE_JOIN_OFFSET + len(code)] == code
          and set(body[P.LATE_JOIN_OFFSET + len(code):]) <= {0xCC},
          f"its first page is the build's without the late join ({len(first_page)} bytes, then int3), "
          "the code follows, then int3 padding")
    name, state_rva, state_size, state_raw, state_rsize, flags = sections[-1]
    check(name == P.LATE_JOIN_STATE_NAME and flags == P.LATE_JOIN_STATE_CHARACTERISTICS
          and state_size == P.LATE_JOIN_STATE_SIZE and (state_raw, state_rsize) == (0, 0)
          and state_rva == align(rva + vsize, headers.section_alignment)
          and P.IMAGE_BASE + state_rva == P.late_join_state_va(code),
          f".srpd at VA {P.IMAGE_BASE + state_rva:#x}: one page, uninitialized | read | write, no bytes in the "
          "file, on the page after the code (where the code looks for it)")
    get = lambda blob, at: struct.unpack_from("<I", blob, at)[0]
    check(get(data, headers.size_of_image_at) == state_rva + state_size
          and get(data, headers.optional + 12) == get(without, headers.optional + 12) + state_size
          and get(data, headers.size_of_code_at) == get(original, headers.size_of_code_at) + rsize
          and headers.table + 40 * 9 <= headers.size_of_headers,
          "SizeOfImage ends at .srpd, SizeOfUninitializedData grew by its page, SizeOfCode by .srp's file size; "
          "the section table fits in the headers")
    security_offset, security_size = image.directory(4)
    old_offset, old_size = struct.unpack_from("<II", original, headers.security_at)
    check(security_offset == raw + rsize and security_offset + security_size == len(data) and security_offset % 8 == 0
          and bytes(data[security_offset:]) == bytes(original[old_offset:old_offset + old_size]),
          f"certificate table at {security_offset:#x}: right after .srp's bytes, 8-byte aligned, last in the file, "
          "identical to the original's")
    same = all(image.directory(index) == before.directory(index) for index in range(16) if index != 4)
    check(same, "the other 15 data directories are those of the build without it")

    # Every byte before .srp's file data, against the build without.
    ranges = [(P.file_offset(va), size) for va, size in late_join_sites()]
    ranges.append((P.data_file_offset(P.CLOCK_FACTOR_VA), 4))
    ranges += [(P.file_offset(P.REVISION_VA) + 4, 1), (P.file_offset(P.LATE_JOIN_READY_VA) + 1, 4),
               (headers.count_at, 2), (headers.size_of_code_at, 4), (headers.optional + 12, 4),
               (headers.size_of_image_at, 4), (headers.checksum_at, 4), (headers.security_at, 4),
               (headers.table + 40 * 7 + 8, 4), (headers.table + 40 * 7 + 16, 4), (headers.table + 40 * 8, 40)]
    # The entry of the relocation table that is disabled (scene::remove_model's branch).
    ranges += [(at, 2) for at, (kind, va) in before.relocations().items()
               if kind == 3 and va == P.REMOVE_MODEL_VA + 2]
    allowed = bytearray(raw)
    for at, size in ranges:
        allowed[at:at + size] = b"\1" * size
    a, b = bytes(without[:raw]), bytes(data[:raw])
    changed = [i for i in range(raw) if a[i] != b[i]] if a != b else []
    stray = [i for i in changed if not allowed[i]]
    check(not stray, f"{len(changed)} bytes before .srp differ from the build without the late join, all inside the "
          f"{len(ranges)} intended ranges" + "".join(f"\n          file {i:#x}" for i in stray[:8]))

    rows_at = lambda va, size: ndisasm(image.read(va, size), va)
    for va, old, new in P.LATE_JOIN_PATCHES:
        rows = rows_at(va, len(old))
        check(image.read(va, len(old)) == new and Image(original).read(va, len(old)) == old
              and rows[0][2] == LATE_JOIN_TEXT[va] and all(text == "nop" for _, _, text in rows[1:])
              and sum(len(code_hex) // 2 for _, code_hex, _ in rows) == len(old),
              f"{va:#x}: `{rows[0][2]}` + {len(rows) - 1} nop")
    for va, old, offset, opcode in P.LATE_JOIN_HOOKS + ((P.LATE_JOIN_READY_VA, P.HOOKS[0][1], P.LATE_JOIN_READY_OFFSET, 0xE9),):
        rows = rows_at(va, len(old))
        word = "call" if opcode == 0xE8 else "jmp"
        check(Image(original).read(va, len(old)) == old and rows[0][2].startswith(word)
              and branch_target(rows[0][2]) == P.LATE_JOIN_VA + offset
              and all(text == "nop" for _, _, text in rows[1:]) and len(rows) == len(old) - 4,
              f"hook {va:#x}: {rows[0][2]} + {len(rows) - 1} nop (entry {offset})")
    # The game that goes on without the focus: the same instructions up to
    # the test, then two nops where the original (and the build without the
    # late join) jumps to the branch that does not call game::tick.
    va, old, new = P.RUN_UNFOCUSED
    rows, original_rows = rows_at(va, len(old)), ndisasm(Image(original).read(va, len(old)), va)
    texts = [text for _, _, text in rows]
    check(image.read(va, len(old)) == new and Image(original).read(va, len(old)) == old
          and before.read(va, len(old)) == old
          and texts[:-2] == [text for _, _, text in original_rows[:-1]]
          and texts[-3:] == ["test al,al", "nop", "nop"] and texts[-4] == "call eax"
          and original_rows[-1][0] == P.RUN_UNFOCUSED_BRANCH_VA and original_rows[-1][2].startswith("jz")
          and branch_target(original_rows[-1][2]) == 0x1A113,
          f"{va:#x}: the active check's `{original_rows[-1][2]}` at {P.RUN_UNFOCUSED_BRANCH_VA:#x} is two nops; "
          "the call and the test before it are the original's; the build without the late join keeps the jump")
    # scene::remove_model: the branch of a model that is not in the scene
    # is a jump to the function's own end, where it lets go of the model it
    # was given and returns.
    va, old, new = P.REMOVE_MODEL
    rows, original_rows = rows_at(va, len(old)), ndisasm(Image(original).read(va - 8, len(old) + 8), va - 8)
    check(image.read(va, len(old)) == new and Image(original).read(va, len(old)) == old
          and before.read(va, len(old)) == old
          and [text for _, _, text in original_rows[:2]] == ["cmp edi,esi", "jnz near 0x15006c"]
          and rows[0][2].startswith("jmp") and branch_target(rows[0][2]) == P.REMOVE_MODEL_END
          and [text for _, _, text in rows[1:]] == ["nop"]
          and [text for _, _, text in ndisasm(Image(original).read(P.REMOVE_MODEL_END, 0x2C), P.REMOVE_MODEL_END)]
          == ["mov eax,[ebp+0xc]", "test eax,eax", "jz 0x150215", "add eax,0xd0", "or ecx,byte -0x1",
              "lock xadd [eax],ecx", "jnz 0x150215", "mov ecx,[ebp+0xc]", "lea eax,[ecx+0xd0]", "call 0xe0210",
              "pop edi", "pop esi", "pop ebx", "mov esp,ebp", "pop ebp", "ret 0x8"],
          f"{va:#x}: scene::remove_model's branch for a model that is not in the scene is `{rows[0][2]}` + nop, "
          "to the function's own end (it lets go of the model it was given, then returns); the build without "
          "the late join keeps the original")
    ready = ndisasm(image.read(P.LATE_JOIN_VA + P.LATE_JOIN_READY_OFFSET, 128), P.LATE_JOIN_VA + P.LATE_JOIN_READY_OFFSET)
    onward = [branch_target(text) for _, _, text in ready if text.startswith("jmp")]
    check(ready[0][2] == "pusha" and onward[:1] == [P.SECTION_VA],
          f"the ready hook's code starts with pushad and jumps on to the upload code at {P.SECTION_VA:#x}")

    sites = [(va, va + size) for va, size in late_join_sites()]
    under = lambda relocations: [va for kind, va in relocations.values()
                                 if kind == 3 and any(va + 4 > lo and va < hi for lo, hi in sites)]
    kept = [(lo, hi) for lo, hi in sites if lo != P.REMOVE_MODEL_VA]
    under_kept = [va for kind, va in Image(original).relocations().values()
                  if kind == 3 and any(va + 4 > lo and va < hi for lo, hi in kept)]
    now, was = image.relocations(), before.relocations()
    differing = sorted(key for key in set(now) | set(was) if now.get(key) != was.get(key))
    check(not under_kept and len(differing) == 1 and was[differing[0]] == (3, P.REMOVE_MODEL_VA + 2)
          and now[differing[0]][0] == 0,
          "no base relocation is under a late-join site but the one of scene::remove_model's branch "
          f"({P.REMOVE_MODEL_VA + 2:#x}), which is disabled; the rest of the table is the build's without it")

    test_clock_factor(original, image, before)
    test_statistics_count(original, image, before)
    test_pregame_progress(original, image, before)
    test_weapon_random(original, image, before)
    test_revision(original, image, "section", P.REVISION)
    test_hash_interval(original, image, "section")
    test_memory_check(original, image, "section")
    test_default_server(original, image, "section", server)
    test_gather(original, image, "section")
    test_start_up_folder(original, image, "section")
    suffix = "" if server is None else (".embedded-max" if len(server) > 63 else ".embedded")
    path = os.path.join(SCRATCH, f"survarium_srp.section{suffix}.late-join.exe")
    with open(path, "wb") as out:
        out.write(data)
    print(f"  written: {path} (sha256 {hashlib.sha256(data).hexdigest()})")
    test_windows(path, source, "section")


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
    test_late_join_bin()
    test_no_branch_into_hooks(original)
    test_server_options(original)
    variants = [(mode, None) for mode in ("cert-only", "section", "hash-mismatch")]
    variants += [(mode, "026g.skhid.link:1234") for mode in P.CAVES]
    variants += [(mode, "a" * 63 + ":65535") for mode in P.CAVES]
    for mode, server in variants:
        print(f"variant {mode}, embedded server {server}")
        code = None if mode == "cert-only" else open(P.default_bin(mode), "rb").read()
        data, description = P.build(original, mode, code, server)
        print("  " + description)
        check(hashlib.sha256(original).hexdigest() == P.EXE_SHA256, "the original in memory is untouched")
        image = Image(data)
        cave = b"" if code is None else P.split_code(code)[0]
        payload = cave if server is None else cave + open(P.default_server_bin(mode), "rb").read() + server.encode("ascii") + b"\0"
        test_pe(original, image, mode)
        test_bytes(original, image, mode, payload, server)
        test_hash_interval(original, image, mode)
        test_memory_check(original, image, mode)
        test_default_server(original, image, mode, server)
        test_revision(original, image, mode, P.REVISION_WITHOUT_LATE_JOIN)
        test_gather(original, image, mode)
        test_start_up_folder(original, image, mode)
        if mode == "section":
            test_late_join(original, data, server, source)
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
        suffix = "" if server is None else (".embedded-max" if len(server) > 63 else ".embedded")
        path = os.path.join(SCRATCH, f"survarium_srp.{mode}{suffix}.exe")
        with open(path, "wb") as out:
            out.write(data)
        print(f"  written: {path} (sha256 {hashlib.sha256(data).hexdigest()})")
        test_windows(path, source, mode)
    print()
    print(f"{len(failures)} FAILED" if failures else "all checks passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
