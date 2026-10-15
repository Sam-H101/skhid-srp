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
20. the same build: the two calls of a game's first world (0x2D1050,
    0x2D1055) go to first_load_input and first_load_kit; in the original
    they are the only calls of 0x2D4AB0 and 0x2D4B50, in deserialize's
    block of the first world after the world is read (which puts the
    world's next kit into place+0Dh), the kit choice is the kit in use and
    both append at the newest end; each tick takes the next kit from the
    newest entry at or before it, and the network inserts in time order;
    the new code reads place+0Dh, calls the game's allocator and the
    list's insert only, writes a node's value and time only and leaves to
    nowhere else; nothing aims inside the two calls.
22. the same build: Team Deathmatch's death handler (0x316090) goes on at
    0x3160F1 where the original and the build without the late join jump
    to its return when the rule's completion time (+12Ch) is not -1: six
    nops between `cmp dword [esi+12Ch],-1` and `mov ecx,[esi+128h]`; the
    handler's only jumps to its return are that one and the one of the
    other event; no relocation under, no branch into; the rule's code
    touches +12Ch at seven places (the constructor, the guard, the end
    test's four, rule_completed). run_tdm_rule.ps1 runs the handler.
23. the same build: the field-of-view setting (patch 22). The video
    options' option 7 names "uro_fov" (0x262930) where the original names
    "fov", a string nothing else points to; "uro_fov" is the name of the
    console variable at 0xAA6510, whose value (0xA6FA28, 65.0) only its
    registration pointed to; the camera's projection (0x242900, slot 0 of
    the vtables of player_input_handler, game_camera, free_fly_camera,
    demo_camera and lobby_camera, which RTTI names, and pointed to from
    nowhere else) jumps at its first instruction to fov_projection, the
    rest of its body (the only reader of the 65-degree constant 0xA2D9EC)
    the original's; fov_projection reads uro_fov, takes the scope's blend
    with its four constants, calls the same matrix builder, writes no XMM
    register but xmm0 and returns as the original (`ret 8`); the
    registration stores 50.0 as the least, the most stays the float 100 at
    0xA710D0; the build without the late join keeps all three; no branch or
    address aims inside the two sites rewritten in place.
24. the same build: a battery under the floor (patch 23). The scan's
    frame and its three hooks; the code (.srp+800h) leaves only back to the
    scan and calls only a battery's get_transform (vtable+44h; only
    batteries' vtables have victory_item_core::can_use at +30h); nothing
    aims inside the four sites.
25. the same build: verify_sign's call of verify_sign_impl (0x135D3) is
    `xor eax,eax` and three nops (`mov eax,esi` before it and
    `add esp,10h` after it are the original's; no relocation under it, no
    branch aims inside it): no resources.db signature is checked; the build
    without the late join keeps the original call.
26. the same build: sign_in_on_connected's `mov [edi+8],eax` /
    `mov [edi+0Ch],dl` (0x380889) are `call` stale_dll at 0xE00 of `.srp`
    and a nop (no relocation under them, no branch aims inside them);
    stale_dll_026g.section.bin is what nasm makes and sits there, after the
    seek code and before the late join; it saves every
    register (pushad / popad), calls itself once for its address, then
    GetModuleHandleA, GetProcAddress, CreateFileA, CloseHandle through the
    import table and GetTempPathA, branches only inside itself, does the two
    writes it replaced and ends with `ret`; it names kernel32.dll,
    GetTempPathA and shell_32.dll and nothing else. Check 11 works out its
    field: byte 8 the check's. The build without the late join keeps the
    original instructions.
27. the same build: the archive the game opens. mount_by_config_helper's
    mount (0xE7D90) pushes the path's format at 0xE7DAD: `push 0x9e705c`
    ("../../%s.db", named by nothing else) in the original and the build
    without the late join, `push` .srp+C00h in this one, which holds
    exactly "../../%s_skhid.db" and its zero between int3 (no overlap with
    the seek code before it or the shell_32.dll check after it: check 15);
    the helper's constructor gives "resources" as the name, so the game
    opens ../../resources_skhid.db, which fits the 260-byte buffer and
    still ends in ".db"; the build names 0x9E705C nowhere; the imm32's
    relocation stays active in both; no branch or address aims inside the
    push.
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
    with tempfile.TemporaryDirectory() as tmp:
        out = os.path.join(tmp, "out.bin")
        subprocess.run([tool("nasm"), "-f", "bin", f"-DCAVE={P.PICKUP_VA:#x}",
                        os.path.join(HERE, "pickup_below_026g.asm"), "-o", out], check=True, timeout=60)
        fresh = open(out, "rb").read()
    check(fresh == open(P.pickup_bin(), "rb").read(),
          f"{os.path.basename(P.pickup_bin())} is current ({len(fresh)} bytes)")
    with tempfile.TemporaryDirectory() as tmp:
        out = os.path.join(tmp, "out.bin")
        subprocess.run([tool("nasm"), "-f", "bin", f"-DCAVE={P.SEEK_CLEAR_VA:#x}",
                        os.path.join(HERE, "replay_seek_026g.asm"), "-o", out], check=True, timeout=60)
        fresh = open(out, "rb").read()
    check(fresh == open(P.seek_clear_bin(), "rb").read(),
          f"{os.path.basename(P.seek_clear_bin())} is current ({len(fresh)} bytes)")
    path = P.stale_dll_bin()
    with tempfile.TemporaryDirectory() as tmp:
        out = os.path.join(tmp, "out.bin")
        subprocess.run([tool("nasm"), "-f", "bin", f"-DCAVE={P.STALE_DLL_ENTRY:#x}",
                        os.path.join(HERE, "stale_dll_026g.asm"), "-o", out], check=True, timeout=60)
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


def test_tdm_deaths(original, image, before):
    """Team Deathmatch's rule counts a death after its own end (patch
    revision 11, client_patch.md section 3.21): the `jnz` after `cmp dword
    [esi+12Ch],-1` in the death handler 0x316090 is six nops; the build
    without the late join keeps it."""
    va, old, new = P.TDM_DEATHS
    stock = Image(original)
    start, end = 0x316090, 0x316200
    rows = {address: text for address, _, text in ndisasm(image.read(start, end - start), start)}
    original_rows = ndisasm(stock.read(start, end - start), start)
    original_at = {address: text for address, _, text in original_rows}
    check(stock.read(va, len(old)) == old and before.read(va, len(old)) == old and image.read(va, len(old)) == new,
          f"{va:#x}: the original's six bytes `{old.hex()}`, kept by the build without the late join, "
          "six 0x90 in this one")
    check(original_at.get(0x3160E1) == "cmp eax,byte +0x1" and original_at.get(0x3160E4) == "jnz near 0x3161dd"
          and original_at.get(0x3160EA) == "cmp dword [esi+0x12c],byte -0x1"
          and original_at.get(va) == "jnz near 0x3161dd" and original_at.get(0x3160F7) == "mov ecx,[esi+0x128]",
          "the original's handler: event 1 (`cmp eax,1` / `jnz 0x3161dd`), then `cmp dword [esi+0x12c],-1` / "
          "`jnz 0x3161dd` / `mov ecx,[esi+0x128]`")
    check(rows.get(0x3160EA) == "cmp dword [esi+0x12c],byte -0x1"
          and all(rows.get(va + i) == "nop" for i in range(len(old))) and rows.get(0x3160F7) == "mov ecx,[esi+0x128]",
          f"{va:#x}: six nops between `cmp dword [esi+0x12c],-1` and `mov ecx,[esi+0x128]`: a death after the "
          "rule's end is counted")
    # What the patch changes is one thing: the handler goes on to the
    # ticket, the alive mask and the end test where it returned at once.
    leaves = sorted(address for address, text in original_at.items() if branch_target(text) == 0x3161DD)
    check(leaves == [0x3160E4, va], "the handler's jumps to its return (0x3161dd) are those of the other event "
          f"(0x3160e4) and the one patched ({va:#x}): {[hex(a) for a in leaves]}")
    relocated = [rva for kind, rva in stock.relocations().values() if kind == 3 and rva + 4 > va and rva < va + len(old)]
    check(not relocated, "no base relocation of the original is under the six bytes")
    hits = [f"{address:#x}: {text}" for address, _, text in original_rows
            if branch_target(text) is not None and va + 1 <= branch_target(text) < va + len(old)]
    check(not hits, "no branch of the handler lands inside the six bytes")
    # The completion time: set to -1 by the rule's constructor, set by the
    # end test, read by the end test, `rule_completed` and this guard, and
    # in no other function of the rule (so nothing else reads it as
    # "the rule has ended").
    text = original[P.TEXT_FILE:P.TEXT_FILE + P.TEXT_VSIZE]
    users = sorted(P.TEXT_VA + m.start() for m in re.finditer(rb"[\x80-\xBF]\x2C\x01\x00\x00", text)
                   if 0x315F00 <= P.TEXT_VA + m.start() < 0x316700)
    check(len(users) == 7, "the rule's code touches the completion time (+0x12c) at seven places "
          f"(constructor, this guard, the end test's four, rule_completed): {[hex(a) for a in users]}")


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
        elif text == f"call {P.STALE_DLL_ENTRY:#x}":
            # stale_dll_026g.asm: what `mov [edi+8],eax` / `mov [edi+0Ch],dl`
            # did, with its flags in byte 8 (check 26).
            out[8:13] = ["stale_dll", 0, 0, 0] + get("dl")
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
    stale_dll = revision >= 13
    expected = list(b"0.26g0" + bytes([0, revision])) + (["stale_dll"] if stale_dll else [0]) + [0, 0, 0, "eula"]
    check(field == expected,
          f'the version field is "0.26g0", a zero, the revision {revision} in byte 7, '
          + ("the shell_32.dll check's flags in byte 8 and three zero bytes" if stale_dll else "four zero bytes")
          + "; the EULA flag follows as before")
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
         (P.PREGAME_PROGRESS_VA, len(P.PREGAME_PROGRESS[1])), (P.TDM_DEATHS[0], len(P.TDM_DEATHS[1])),
         (P.REPLAY_REATTACH_THUNK[0], len(P.REPLAY_REATTACH_THUNK[1])),
         (P.SOUND_QUEUE_CAVE_SITE[0], len(P.SOUND_QUEUE_CAVE_SITE[1])),
         (P.SOUND_QUEUE_HOOK[0], len(P.SOUND_QUEUE_HOOK[1])),
         (P.SEEK_CLEAR_HOOK[0], len(P.SEEK_CLEAR_HOOK[1]))] + \
        [(va, len(old)) for va, old, _ in P.FOV_PATCHES] + pickup_sites()


def pickup_sites():
    """[(address, length)] of patch 23's sites: the frame and the three hooks."""
    return [(P.PICKUP_FRAME[0], len(P.PICKUP_FRAME[1]))] + [(va, len(old)) for va, old, _, _ in P.PICKUP_HOOKS]


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


def starts_instruction(source, at):
    """Whether `at` starts an instruction of the function around it, read
    from the function's start (after the int3 before it)."""
    start = at
    while source.read(start - 1, 1) != b"\xcc" or start % 16:
        start -= 1
    return at in {address for address, _, _ in ndisasm(source.read(start, at - start + 16), start)}


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


# The vtables whose slot 0 is the camera's projection, by RTTI.
FOV_VTABLES = {0xA0406C: b".?AVplayer_input_handler@survarium@@", 0xA04084: b".?AVgame_camera@survarium@@",
               0xA047F0: b".?AVfree_fly_camera@survarium@@", 0xA0482C: b".?AVdemo_camera@survarium@@",
               0xA09ED8: b".?AVlobby_camera@survarium@@"}


def test_fov(original, image, before):
    """Check 23: the field-of-view setting (patch 22)."""
    source = Image(original)
    pack = lambda value: struct.pack("<I", value)

    def refs(value):
        """The addresses of every 4-byte copy of `value` in the file's sections."""
        found, at = [], original.find(pack(value))
        while at != -1:
            found += [P.IMAGE_BASE + rva + at - raw for _, rva, _, raw, rsize, _ in source.sections
                      if raw <= at < raw + rsize]
            at = original.find(pack(value), at + 1)
        return sorted(found)
    for va, old, new in P.FOV_PATCHES:
        check(source.read(va, len(old)) == old and before.read(va, len(old)) == old and image.read(va, len(new)) == new,
              f"{va:#x}: {len(old)} bytes rewritten; the original and the build without the late join keep them")
    string = lambda blob, at: blob.read(at, 64).split(b"\0")[0]
    name = struct.unpack("<I", P.FOV_OPTION_NAME[2][1:])[0]
    check(string(source, 0xA0659C) == b"fov" and refs(0xA0659C) == [P.FOV_OPTION_NAME[0] + 1]
          and string(image, name) == b"uro_fov" and source.read(0xAA6510 + 0x10, 4) == pack(name)
          and refs(name) == [0xAA6510 + 0x10] and refs(P.FOV_VALUE) == [0x85AE24 + 6]
          and source.read(0x85AE24, 6) == bytes.fromhex("c70550 65aa00".replace(" ", ""))
          and struct.unpack("<f", source.read(P.FOV_VALUE, 4))[0] == 65.0,
          "option 7 names \"uro_fov\" for \"fov\" (pointed to from there alone); \"uro_fov\" names the console variable "
          f"at 0xAA6510, whose value ({P.FOV_VALUE:#x}, 65.0) only its registration pointed to")
    va, old, offset, _ = P.FOV_PROJECTION_HOOK
    entry = P.LATE_JOIN_VA + offset
    hook = [text for _, _, text in ndisasm(image.read(va, len(old)), va)]
    rest = len(P.FOV_PROJECTION_BODY)
    check(source.read(va, len(old)) == old and before.read(va, len(old)) == old
          and branch_target(hook[0]) == entry and hook[1:] == ["nop"]
          and image.read(va + len(old), rest - len(old)) == source.read(va + len(old), rest - len(old))
          and source.read(va, rest) == P.FOV_PROJECTION_BODY and refs(0xA2D9EC) == [va + 0x3A],
          f"the projection's first instruction jumps to fov_projection ({entry:#x}); the rest of its body, which "
          "alone reads the 65-degree constant 0xA2D9EC, is the original's; the build without the late join keeps it")
    rows = ndisasm(image.read(entry, 0x80), entry)
    end_at = next(i for i, (_, _, text) in enumerate(rows) if text.startswith("ret"))
    texts = [text for _, _, text in rows[:end_at + 1]]
    data = image.read(rows[end_at][0] + len(rows[end_at][1]) // 2, 16)
    returns = lambda text: text.replace("ret word ", "ret ")  # ndisasm 3 spells `ret 0x8` so
    was = [text for _, _, text in ndisasm(source.read(va, rest), va)]
    calls = lambda texts: [branch_target(text) for text in texts if text.startswith("call")]
    check(calls(texts) == calls(was) == [0xEEE30] and returns(texts[-1]) == "ret 0x8"
          and f"fld dword [0x{P.FOV_VALUE:x}]" in [t.replace("[dword 0x", "[0x") for t in texts]  # ndisasm 2.x spells [dword 0x..]
          and "fcmovb st1" in texts
          and not any("xmm" in text and "xmm0" not in text for text in texts)
          and data == struct.pack("<4f", 0.25, 1.3333334, 65.0, 0.017453292),
          "fov_projection reads uro_fov, takes max(0, (z - 0.25) / 0.75) (0.25, 4/3, 65 and pi/180 after its code), "
          "calls the matrix builder 0xEEE30 as the original, writes no XMM register but xmm0 and returns with `ret 0x8`")
    check(all(source.read(vtable, 4) == pack(P.FOV_PROJECTION_VA)
              and string(source, struct.unpack("<I", source.read(struct.unpack("<I", source.read(vtable - 4, 4))[0] + 12,
                                                                  4))[0] + 8) == klass
              for vtable, klass in FOV_VTABLES.items())
          and refs(P.FOV_PROJECTION_VA) == sorted(FOV_VTABLES),
          "the projection is slot 0 of the vtables of " + ", ".join(k[4:].split(b"@")[0].decode()
                                                                   for k in FOV_VTABLES.values())
          + " and pointed to from nowhere else")
    least = struct.unpack("<I", P.FOV_MINIMUM[1][7:11])[0]
    check(image.read(P.FOV_MINIMUM[0] + 2, 8) == struct.pack("<If", least, P.FOV_MIN) and least == 0xAA6510 + 0x44
          and source.read(0x85AE11, 8) == bytes.fromhex("f30f1005d010a700")
          and struct.unpack("<f", source.read(0xA710D0, 4))[0] == 100.0,
          f"uro_fov's registration stores {P.FOV_MIN:g} as its least (+44h); its most stays the float 100 at 0xA710D0")

    # A rel8 byte is a branch only where the function's own instructions
    # have one; the others are bytes of other instructions.
    found = [hit for va, old, _ in P.FOV_PATCHES for hit in aimed_inside(original, va, len(old), 0x82)
             if not (va <= hit < va + len(old)) and starts_instruction(source, hit)]
    check(not found, "no rel32 branch of .text, no rel8 branch near them (read as the functions' instructions) and "
          "no 4-byte value aims inside the two sites rewritten in place" + "".join(f"\n          {a:#x}" for a in found))


# A battery's usable-object vtables: can_use at +30h, get_transform at +44h.
BATTERY_VTABLES = {0xA13E8C: b".?AVvictory_item_core@survarium@@", 0xA0437C: b".?AVvictory_item@survarium@@"}


def test_pickup(original, image, before):
    """Check 24: a battery under the floor one stands on (patch 23)."""
    source = Image(original)
    word = lambda blob, at: struct.unpack("<I", blob.read(at, 4))[0]
    va, old, new = P.PICKUP_FRAME
    check(source.read(va, len(old)) == old and before.read(va, len(old)) == old and image.read(va, len(new)) == new,
          f"{va:#x}: the scan's frame grows by 8 (sub esp,34h -> 3Ch); the build without the late join keeps it")
    for va, old, offset, opcode in P.PICKUP_HOOKS:
        rows = [text for _, _, text in ndisasm(image.read(va, len(old)), va)]
        check(source.read(va, len(old)) == old and before.read(va, len(old)) == old
              and rows[0].split()[0] == ("call" if opcode == 0xE8 else "jmp")
              and branch_target(rows[0]) == P.PICKUP_VA + offset and set(rows[1:]) <= {"nop"},
              f"{va:#x}: {rows[0]} (+{offset}) and nops; the original and the build without the late join keep it")
    code = open(P.pickup_bin(), "rb").read()
    rows = ndisasm(image.read(P.PICKUP_VA, len(code)), P.PICKUP_VA)
    outside = sorted({branch_target(text) for _, _, text in rows
                      if branch_target(text) is not None
                      and not P.PICKUP_VA <= branch_target(text) < P.PICKUP_VA + len(code)})
    check(image.read(P.PICKUP_VA, len(code)) == code and outside == [0x2D85C6, 0x2D85FC, 0x2D8622]
          and [text for _, _, text in rows if text.startswith("call")] == ["call [edx+0x44]"]
          and struct.unpack("<f", source.read(0xA710D8, 4))[0] == 2.0
          and code[-4:] == struct.pack("<f", P.PICKUP_ACROSS ** 2),
          f"the code at {P.PICKUP_VA:#x} is the .bin; it leaves only to the scan (0x2D85C6, 0x2D85FC, 0x2D8622), "
          f"calls only get_transform (vtable+44h); the distance (0xA710D8) is 2.0; across, {P.PICKUP_ACROSS:g} m")
    string = lambda at: source.read(at, 64).split(b"\0")[0]
    klass = lambda vtable: string(word(source, word(source, vtable - 4) + 12) + 8)
    can_use = struct.pack("<I", 0x300710)
    at, holders = original.find(can_use), []
    while at != -1:
        holders += [P.IMAGE_BASE + rva + at - raw for _, rva, _, raw, rsize, _ in source.sections if raw <= at < raw + rsize]
        at = original.find(can_use, at + 1)
    check(sorted(holders) == sorted(vtable + 0x30 for vtable in BATTERY_VTABLES)
          and all(klass(vtable) == name and word(source, vtable + 0x44) == 0x246ED0
                  for vtable, name in BATTERY_VTABLES.items())
          and source.read(0x246ED0, 8) == bytes.fromhex("83e914e9") + struct.pack("<i", 0x300A20 - 0x246ED8),
          "victory_item_core::can_use (0x300710) is at +30h of victory_item_core's and victory_item's usable "
          "vtables and nowhere else; their +44h, get_transform, is the battery's (sub ecx,14h; jmp 0x300A20)")
    found = [hit for va, size in pickup_sites() for hit in aimed_inside(original, va, size, 0x82)
             if not (va <= hit < va + size) and starts_instruction(source, hit)]
    check(not found, "no rel32 branch of .text, no rel8 branch near them and no 4-byte value aims inside the "
          "four sites" + "".join(f"\n          {a:#x}" for a in found))


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

def register_sites(original, field):
    """Every [register+field] (disp32, not esp-based) of .text, by the
    address of its instruction's opcode (a 0x66 prefix not counted)."""
    text_start = P.file_offset(P.TEXT_VA)
    text = bytes(original[text_start:text_start + P.TEXT_VSIZE])
    sites, position = [], 0
    while (position := text.find(struct.pack("<I", field), position)) >= 0:
        modrm, sib = text[position - 1], text[position - 2]
        if modrm & 0xC0 == 0x80 and modrm & 7 != 4:
            sites.append(P.TEXT_VA + position - 2)
        elif sib & 0xC0 == 0x80 and sib & 7 == 4 and modrm & 7 != 4:
            sites.append(P.TEXT_VA + position - 3)
        position += 1
    return sites


def test_kit_state(original, image, before):
    """Check 21: a spawn gives the new kit's weapons the state a weapon
    every game has alike."""
    source = Image(original)
    va, old, offset, _ = P.KIT_STATE_HOOK
    entry = P.LATE_JOIN_VA + offset
    rows = lambda blob, at, size: ndisasm(blob.read(at, size), at)
    texts = lambda blob, at, size: [text for _, _, text in rows(blob, at, size)]
    check(source.read(va, len(old)) == old and before.read(va, len(old)) == old
          and texts(source, va, len(old)) == ["mov edx,[esi]", "mov edx,[edx+0x1c]"]
          and texts(image, va, len(old)) == [f"call 0x{entry:x}"],
          f"{va:#x}: `mov edx,[esi]` / `mov edx,[edx+0x1c]` is `call 0x{entry:x}`; the original and the build "
          "without the late join keep them")
    spawn = rows(source, P.WEAPON_SPAWN_VA, 0x316EB3 - P.WEAPON_SPAWN_VA)
    at = {address: text for address, _, text in spawn}
    order = [address for address, _, _ in spawn]
    between = lambda lo, hi: [at[a] for a in order if lo <= a < hi]
    check(at.get(0x316DDD) == "mov cl,[ecx+0x115]" and at.get(0x316DE3) == "mov [eax+0xc],dl"
          and at.get(0x316DFC) == "mov al,[eax+0x115]" and at.get(0x316E02) == "mov [esp+0x10],cl"
          and at.get(0x316E06) == "mov [esp+0x18],al" and at.get(0x316E0A) == "cmp cl,al"
          and at.get(0x316E0C) == "jz 0x316e3b" and at.get(0x316E53) == "mov edi,[esp+0x1c]"
          and between(0x316E3B, 0x316E57) == ["mov eax,[esi+0x134]", "fldz", "mov eax,[eax+0x18458]",
                                               "movzx ecx,byte [eax+edi+0xc]", "add eax,edi",
                                               "mov esi,[eax+ecx*4]", "mov edi,[esp+0x1c]"]
          and not [text for text in between(0x316E0A, 0x316E57) if re.match(r"mov (byte )?\[esp", text)],
          "the spawn keeps the kit of the player that was current at [esp+10h] (0x316E02) and the new one's at "
          "[esp+18h] (0x316E06) and tests them for a change of kit (0x316E0A); both paths read the spawn point at "
          "[esp+1Ch] at 0x316E53 with the same esp, nothing writes the two bytes again, and esi is the new kit's "
          "player at 0x316E57")
    ctor = texts(source, 0x2DF0D0, 0x480)
    check(all(text in ctor for text in (
        "or eax,byte -0x1", "mov [ebp+0x5a4],eax", "mov [ebp+0x5a8],eax", "mov [ebp+0x5b1],bl",
        "mov [ebp+0x5b2],bl", "mov [ebp+0x5e5],bl", "mov [ebp+0x5e6],bl", "mov [ebp+0x5e8],bl",
        "mov [ebp+0x5f1],bl", "mov [ebp+0x5f5],bl", "mov dword [ebp+0x5ec],0x6", "movss dword [ebp+0x5fc],xmm0",
        "movss dword [ebp+0x600],xmm0", "movss dword [ebp+0x604],xmm0", "movss dword [ebp+0x608],xmm0",
        "mov dword [ebp+0x60c],0xffffffff", "xor ebx,ebx"))
          and ctor.index("xor ebx,ebx") < ctor.index("mov [ebp+0x5b1],bl")
          and not [text for text in ctor[ctor.index("xor ebx,ebx") + 1:ctor.index("mov [ebp+0x5f5],bl")]
                   if re.match(r"(mov|xor|or|and|add|sub|inc|dec|pop|lea|movzx|setn?z) (ebx|bl|bx),", text)]
          and ctor[ctor.index("mov [ebp+0x5a4],eax") - 1] == "or eax,byte -0x1"
          and ctor[ctor.index("mov [ebp+0x5a8],eax") - 1] == "mov [ebp+0x5a4],eax"
          and "xorps xmm0,xmm0" in ctor[ctor.index("mov [ebp+0x5e0],ebx"):ctor.index("movss dword [ebp+0x5fc],xmm0")]
          and not [text for text in ctor[ctor.index("mov [ebp+0x5e0],ebx"):ctor.index("movss dword [ebp+0x608],xmm0")]
                   if re.match(r"\w+ xmm0,", text) and text != "xorps xmm0,xmm0"],
          "weapon_core's constructor (0x2DF0D0) gives +5A4h and +5A8h -1, +5B1h, +5B2h, +5E5h, +5E6h, +5E8h, "
          "+5F1h and +5F5h 0, +5ECh 6, +5FCh..+608h 0.0 and +60Ch -1: KIT_STATE_FIELDS")
    initialize = texts(source, 0x2DAC00, 0x260)
    activate = texts(source, 0x2E2A30, 0x290)
    core_vtable = lambda number: struct.unpack("<I", source.read(0xA11094 + number, 4))[0]
    check("mov [esi+0x324],ebx" in initialize and initialize.index("mov ebx,[ebp+0x8]") < initialize.index(
        "mov [esi+0x324],ebx") and "call 0x2fcb70" in initialize and "mov [esi+0x128],ecx" in initialize
          and initialize.index("call 0x2fcb70") < initialize.index("mov [esi+0x128],ecx")
          and "mov eax,[edx+0x14]" in initialize[initialize.index("mov [esi+0x128],ecx"):]
          and core_vtable(0x14) == 0x2E2A30
          and "mov esi,[eax+0x324]" in activate and "mov [edi+0x5a4],esi" in activate
          and activate.index("mov esi,[eax+0x324]") < activate.index("mov [edi+0x5a4],esi")
          and all(text in activate for text in ("mov byte [edi+0x5e8],0x0", "mov dword [edi+0x5ec],0x6",
                                                "mov byte [edi+0x5f5],0x0", "mov dword [edi+0x60c],0xffffffff",
                                                "mov dword [edi+0x5a8],0xffffffff")),
          "the initialize keeps the spawn's time at +324h, sets the inventory up (0x2FCB70) and activates the item "
          "it puts in hand (+128h, vtable+14h: weapon_core's 0x2E2A30), which sets +5A4h to that time and the aim, "
          "+5ECh, +5F5h, +60Ch and +5A8h as a new weapon has them")
    tick_sites = {0x244E20: "mov eax,[ebx+0x5a4]", 0x26D028: "mov ecx,[esi+0x5a4]", 0x2DF405: "mov [ebp+0x5a4],eax",
                  0x2E09EF: "sub edi,[ebp+0x5a4]", 0x2E09F5: "mov [ebp+0x5a4],ebx", 0x2E1BAF: "mov ecx,[edi+0x5a4]",
                  0x2E1C4B: "mov ecx,[edi+0x5a4]", 0x2E2696: "mov eax,[ecx+0x5a4]", 0x2E2C2B: "mov [edi+0x5a4],esi",
                  0x2E317D: "lea eax,[ebx+0x5a4]", 0x2E34D2: "mov [edi+0x5a4],ecx"}
    sites = [site for site in register_sites(original, 0x5A4) if "+0x5a4]" in texts(source, site, 8)[0]]
    check(sorted(sites) == sorted(tick_sites)
          and all(texts(source, site, 8)[0] == want for site, want in tick_sites.items())
          and core_vtable(0x28) == 0x2E09E0,
          "+5A4h of a register is in .text eleven times: the constructor, the weapon's tick (vtable+28h, 0x2E09E0: "
          "the time since it, then the tick's time), the activation, the serialize and the deserialize, the "
          "aim's two (0x2E1B50, 0x2E1BF0, of the weapon in hand), the change of ammunition (0x2E2650, by the "
          "player's input), the client's draw (0x244E10) and 0x26D028 (another class's +5A4h, read by hand)")
    fire_mode = {0x2DF4A0: "mov [ebp+0x5e6],bl", 0x2E0CE1: "mov [ecx+0x5e6],dl", 0x2E3504: "mov [edi+0x5e6],cl"}
    sites = [site for site in register_sites(original, 0x5E6) if "+0x5e6]" in texts(source, site, 8)[0]]
    writers = [site for site in sites if not re.match(r"(movzx|cmp|lea|test|push) ", texts(source, site, 8)[0])]
    check(sorted(writers) == sorted(fire_mode)
          and all(texts(source, site, 8)[0] == want for site, want in fire_mode.items()),
          "the fire mode (+5E6h) is written by the constructor, the player's input (0x2E0CA0) and the deserialize "
          "alone: it is kept through deaths and spawns")
    hook = rows(image, entry, 0xA0)
    hook = hook[:next(i for i, (_, _, text) in enumerate(hook) if text == "ret") + 1]
    hook_texts = [text for _, _, text in hook]
    pc = next(i for i, (_, _, text) in enumerate(hook) if text.startswith("call"))
    add = re.match(r"add ebx,0x([0-9a-f]+)$", hook_texts[pc + 2])
    base = 0x594 + 0x10
    stores = {}
    for text in hook_texts:
        match = re.match(r"(or|mov) (byte |dword )?\[edx(?:\+0x([0-9a-f]+))?\],(.*)$", text)
        if not match:
            continue
        field = base + int(match.group(3) or "0", 16)
        operand = match.group(4)
        if operand == "byte -0x1":
            stores[field] = (4, 0xFFFFFFFF)
        elif operand == "bp":
            stores[field], stores[field + 1] = (1, 0), (1, 0)
        elif operand == "ebp":
            stores[field] = (4, 0)
        elif match.group(2) == "byte " and operand == "0x0":
            stores[field] = (1, 0)
        elif match.group(2) == "dword ":
            stores[field] = (4, int(operand, 16))
        else:
            stores[field] = None
    wanted = {offset: (size, value) for offset, size, value in P.KIT_STATE_FIELDS}
    wanted[P.KIT_STATE_TICK_FIELD] = (4, 0xFFFFFFFF)
    tick_store = hook_texts.index("or dword [edx],byte -0x1")
    inside = all(entry <= branch_target(text) < hook[-1][0] + 1 for text in hook_texts
                 if branch_target(text) is not None)
    other_writes = [text for text in hook_texts if re.match(r"(mov|or|and|add|sub|xor|inc|dec) [a-z ]*\[", text)
                    and "[edx" not in text]
    check(hook_texts[0] == "pusha" and hook_texts[-4:] == ["popa", "mov edx,[esi]", "mov edx,[edx+0x1c]", "ret"]
          and hook_texts[pc + 1] == "pop ebx" and branch_target(hook_texts[pc]) == hook[pc + 1][0]
          and add is not None and (hook[pc + 1][0] + int(add.group(1), 16)) & 0xFFFFFFFF == P.WEAPON_BIND_VA
          and "cmp [ebp+0x70],ebx" in hook_texts and "mov ecx,[esi+0x10c]" in hook_texts
          and "mov al,[esp+0x34]" in hook_texts and "cmp al,[esp+0x3c]" in hook_texts and "setnz al" in hook_texts
          and "add edx,0x594" in hook_texts and "cmp edi,byte +0x17" in hook_texts
          and hook_texts[tick_store + 1:tick_store + 3] == ["test al,al", f"jz 0x{hook[-7][0]:x}"]
          and hook_texts[-7:-4] == ["inc edi", "cmp edi,byte +0x17", f"jc 0x{hook[hook_texts.index('mov edx,[ecx+edi*4+0x10c]')][0]:x}"]
          and stores == wanted and not other_writes and inside,
          f"kit_state_hook ({entry:#x}): pushad; for the 23 slots of the player's inventory, an item whose "
          f"vtable+70h is {P.WEAPON_BIND_VA:#x} gets -1 at weapon+5A4h and, when the kit before (the spawn's "
          "[esp+10h]) is not the new one ([esp+18h]), KIT_STATE_FIELDS; nothing else is written; popad, the two "
          "instructions, ret; no branch leaves it")
    found = aimed_inside(original, va, len(old), 0x82)
    check(not found, "no rel32 branch of .text, no rel8 branch byte near it and no 4-byte value aims inside the five "
          "bytes" + "".join(f"\n          {a:#x}" for a in found))


def test_first_load(original, image, before):
    """Check 20: a first world's inputs and kit choices, in time order."""
    source = Image(original)
    texts = lambda blob, at, size: [text for _, _, text in ndisasm(blob.read(at, size), at)]
    sites = []
    for (va, old, offset, _), was in ((P.FIRST_LOAD_INPUT_HOOK, 0x2D4AB0), (P.FIRST_LOAD_KIT_HOOK, 0x2D4B50)):
        entry = P.LATE_JOIN_VA + offset
        sites.append((va, old, entry, was))
        check(source.read(va, 5) == old and before.read(va, 5) == old
              and texts(source, va, 5) == [f"call 0x{was:x}"] and texts(image, va, 5) == [f"call 0x{entry:x}"]
              and branches_to(original, was) == [va],
              f"{va:#x}: `call 0x{was:x}` is `call 0x{entry:x}`; it was the only call of 0x{was:x}; the build "
              "without the late join keeps it")
    block = texts(source, 0x2D0FBE, 0x2D1067 - 0x2D0FBE)
    check(block[0] == "cmp byte [edi+0x18fdf],0x0" and "jz near 0x2d105e" in block
          and "mov byte [edi+0x18fdf],0x0" in block
          and all(text in block for text in ("mov [esp+0x24],ecx", "mov [esp+0x28],ecx", "mov [esp+0x2c],dl",
                                             "lea edi,[esp+0x24]", "and esi,ebx", "jnz 0x2d0ff2"))
          and block[block.index("lea edi,[esp+0x24]") + 1:block.index("lea edi,[esp+0x24]") + 4]
          == ["mov [esp+0x2c],dl", "call 0x2d4ab0", "call 0x2d4b50"]
          and texts(source, 0x2D0FB9, 5) == ["call 0x2d2770"]
          and "mov [edi+0xd],al" in texts(source, 0x2D2A30, 0x10),
          "the two calls are in deserialize's block of a game's first world (the frozen flag, +18FDFh, cleared), "
          "for each place of the world's mask, with edi -> {core, the world's time, place}, after the world is "
          "read (0x2D2770, which puts the world's next kit into place+0Dh at 0x2D2A37)")
    kit, given = texts(source, 0x2D4B50, 0x87), texts(source, 0x2D4AB0, 0x95)
    check("mov bl,[edx+0x115]" in kit and "add esi,[ecx+0x89d8]" in kit and "call 0x1fcb60" in kit
          and "add esi,[ecx+0x2fcc]" in given and "mov ebx,[eax+0x32c]" in given and "mov ebp,[eax+0x330]" in given
          and "call 0x1fcb60" in given and "call 0x2d4850" not in kit + given,
          "in the original the kit choice is the kit in use (player+115h) and both put a new entry at the newest "
          "end themselves, not through the list's insert (0x2D4850)")
    tick = texts(source, 0x321790, 0x4B)
    network = texts(source, 0x1FBF00, 0x70)
    inputs = texts(source, 0x1FB41B, 0x60)
    check(all(text in tick for text in ("mov edx,[edx+0x89d8]", "mov eax,[eax+edx+0x8]", "cmp [eax+0x20],edx",
                                        "mov eax,[eax+0x4]", "mov [ecx+0xd],al"))
          and all(text in network for text in ("add ebx,[edi+0x89d8]", "mov eax,[ebx+0x4]", "cmp [eax+0x20],esi",
                                               "mov eax,[eax]", "call 0x2d4850"))
          and all(text in inputs for text in ("add eax,[ebx+0x2fcc]", "mov esi,[eax+0x4]", "cmp [esi+0x20],edi",
                                              "mov esi,[esi]", "call 0x1fcb60")),
          "each tick (0x321790) takes the next kit from the newest entry at or before it, looked for from the "
          "newest end; the network puts kit choices and inputs before the oldest newer entry (0x1FBF00, 0x1FB41B)")
    code = ndisasm(image.read(sites[0][2], 0xC0), sites[0][2])
    code = code[:next(i for i, (address, _, text) in enumerate(code)
                      if text == "ret" and address > sites[1][2] + 0x2E) + 1]
    code_texts = [text for _, _, text in code]
    end = code[-1][0] + 1
    calls = sorted({branch_target(text) for text in code_texts if text.startswith("call")})
    leaving = [text for text in code_texts if branch_target(text) is not None and not text.startswith("call")
               and not sites[0][2] <= branch_target(text) < end]
    stores = sorted({text for text in code_texts if text.startswith("mov [")})
    check(code[0][0] == sites[0][2] and any(address == sites[1][2] for address, _, _ in code)
          and "mov bl,[eax+0xd]" in code_texts and "mov ebx,[eax+0x32c]" in code_texts
          and calls[:2] == [0x1FCB60, 0x2D4850] and all(sites[0][2] <= c < end for c in calls[2:])
          and not leaving and stores == ["mov [eax+0x20],ecx", "mov [eax+0x8],bl", "mov [eax+0x8],ebx",
                                         "mov [eax+0xc],ebp"],
          f"first_load_input ({sites[0][2]:#x}) and first_load_kit ({sites[1][2]:#x}): the kit choice is place+0Dh; "
          "they call the game's allocator (0x1FCB60) and the list's insert (0x2D4850) and their own search alone, "
          "write a node's value and time alone, and no branch leaves them")
    found = [address for va, old, _, _ in sites for address in aimed_inside(original, va, len(old), 0x82)]
    check(not found, "no rel32 branch of .text, no rel8 branch byte near them and no 4-byte value aims inside the "
          "two calls" + "".join(f"\n          {a:#x}" for a in found))


def test_db_signature(original, image, before):
    """Check 25: no resources.db signature is checked."""
    va, old, new = P.DB_SIGNATURE
    # ndisasm 2.x spells the last one `add esp,byte +0x10`, 3.x `add esp,0x10`.
    texts_at = lambda blob: [text.replace("byte +", "") for _, _, text in ndisasm(blob.read(va - 2, 10), va - 2)]
    check(texts_at(Image(original)) == ["mov eax,esi", f"call {P.DB_SIGNATURE_IMPL:#x}", "add esp,0x10"]
          and texts_at(image) == ["mov eax,esi", "xor eax,eax", "nop", "nop", "nop", "add esp,0x10"]
          and image.read(va, len(new)) == new and before.read(va, len(old)) == old,
          f"{va:#x}: verify_sign's `call {P.DB_SIGNATURE_IMPL:#x}` is `xor eax,eax` and three nops between the "
          "original's instructions; the build without the late join keeps the original call")
    under = [v for kind, v in Image(original).relocations().values() if kind == 3 and va - 3 < v < va + len(old)]
    # rel32 branches of .text and 4-byte values of the file; the short
    # branches of verify_sign itself (0x13310..0x13608), instruction by
    # instruction (a byte-wise scan reads `lea esi,[esp+10h]` at 0x135AD
    # as a jz into the call).
    found = aimed_inside(original, va, len(old), 0)
    verify_sign = ndisasm(Image(original).read(0x13310, 0x2F8), 0x13310)
    found += [address for address, _, text in verify_sign
              if (target := branch_target(text)) is not None and va < target < va + len(old)]
    check(not under and not found and any(address == va for address, _, _ in verify_sign),
          "no base relocation under the call, no branch or address aims inside it" + "".join(f"\n          {a:#x}" for a in found))


# mount_by_config_helper's mount of the archive (0xE7D90..0xE7E75), which
# formats the path; its constructor (0xE7D20..0xE7D81) names it.
ARCHIVE_MOUNT = (0xE7D90, 0xE7E75)
ARCHIVE_HELPER = (0xE7D20, 0xE7D81)


def test_archive_path(original, image, before):
    """Check 27: the archive the game opens is resources_skhid.db."""
    va, old, new = P.ARCHIVE_PATH_SITE
    stock = Image(original)
    string = P.ARCHIVE_PATH_STRING_VA
    # ndisasm spells a push of an imm32 `push dword 0x...`.
    push = lambda blob: [text.replace("dword ", "") for _, _, text in ndisasm(blob.read(va, 5), va)]
    cstring = lambda blob, at: bytes(blob.read(at, 64)).split(b"\0")[0]
    check(push(stock) == [f"push {P.ARCHIVE_PATH_ORIGINAL:#x}"] and push(image) == [f"push {string:#x}"]
          and image.read(va, len(new)) == new and before.read(va, len(old)) == old
          and cstring(stock, P.ARCHIVE_PATH_ORIGINAL) == b"../../%s.db",
          f"{va:#x}: the path's format is `push {string:#x}` where the original pushes "
          f"{P.ARCHIVE_PATH_ORIGINAL:#x} (\"../../%s.db\"); the build without the late join keeps it")
    check(image.read(string, len(P.ARCHIVE_PATH)) == P.ARCHIVE_PATH
          and image.read(string - 1, 1) == b"\xCC" and image.read(string + len(P.ARCHIVE_PATH), 1) == b"\xCC",
          f"{string:#x} (.srp+{P.ARCHIVE_PATH_OFFSET:#x}) holds exactly {P.ARCHIVE_PATH!r}, int3 on both sides")
    # The %s: the helper's constructor copies "resources" into its name
    # (+20h), which the mount formats with and mounts the archive as.
    helper = [text for _, _, text in ndisasm(stock.read(ARCHIVE_HELPER[0], ARCHIVE_HELPER[1] - ARCHIVE_HELPER[0]),
                                             ARCHIVE_HELPER[0])]
    name = 0x9DB880
    path = (P.ARCHIVE_PATH[:-1] % P.ARCHIVE_NAME.encode("ascii")).decode("ascii")
    check(f"mov ecx,{name:#x}" in helper and "lea edi,[esi+0x20]" in helper
          and cstring(stock, name) == P.ARCHIVE_NAME.encode("ascii")
          and path == "../../resources_skhid.db" and len(path) < 0x104
          and cstring(stock, 0x9DE5A0) == b".db" and path.endswith(".db"),
          f"the name it is formatted with is {P.ARCHIVE_NAME!r} ({name:#x}): the game opens {path} "
          "(in the 260-byte buffer), and the name still ends in \".db\", as windows_hdd_file_system::open "
          "tests to keep the archive's handle")
    # The one reference to "../../%s.db" in the original is this push; the
    # build names it nowhere.
    refs = lambda data: [m.start() for m in re.finditer(re.escape(struct.pack("<I", P.ARCHIVE_PATH_ORIGINAL)), bytes(data))]
    check(refs(original) == [P.file_offset(va) + 1] and refs(image.data) == [],
          f"the original names {P.ARCHIVE_PATH_ORIGINAL:#x} at {va + 1:#x} alone; the build names it nowhere")
    relocations = lambda blob: [v for kind, v in blob.relocations().values() if kind == 3 and va - 3 < v < va + len(old)]
    start, end = ARCHIVE_MOUNT
    found = aimed_inside(original, va, len(old), 0)
    found += [address for address, _, text in ndisasm(stock.read(start, end - start), start)
              if (target := branch_target(text)) is not None and va < target < va + len(old)]
    check(relocations(stock) == [va + 1] and relocations(image) == [va + 1] and not found,
          f"the base relocation of the imm32 ({va + 1:#x}) stays active, an address for an address; no branch "
          "or address aims inside the push" + "".join(f"\n          {a:#x}" for a in found))


def test_stale_dll(original, image, before):
    r"""Check 26: the sign-in says whether %TEMP%\shell_32.dll exists."""
    va, old, new = P.STALE_DLL
    entry = P.STALE_DLL_ENTRY
    check(image.read(va, len(new)) == new and before.read(va, len(old)) == old
          and [text for _, _, text in ndisasm(image.read(va, 6), va)] == [f"call {entry:#x}", "nop"]
          and [text for _, _, text in ndisasm(Image(original).read(va, 6), va)]
          == ["mov [edi+0x8],eax", "mov [edi+0xc],dl"],
          f"{va:#x}: `mov [edi+0x8],eax` / `mov [edi+0xc],dl` are `call {entry:#x}` / `nop`; "
          "the build without the late join keeps them")
    under = [v for kind, v in Image(original).relocations().values() if kind == 3 and va - 3 < v < va + len(old)]
    start, end = SIGN_IN_ON_CONNECTED
    found = aimed_inside(original, va, len(old), 0)
    found += [address for address, _, text in ndisasm(Image(original).read(start, end - start), start)
              if (target := branch_target(text)) is not None and va < target < va + len(old)]
    check(not under and not found,
          "no base relocation under the two instructions, no branch or address aims inside them"
          + "".join(f"\n          {a:#x}" for a in found))
    code = open(P.stale_dll_bin(), "rb").read()
    check(image.read(entry, len(code)) == code, f"stale_dll at {entry:#x} is stale_dll_026g.section.bin")
    strings = code[code.index(b"kernel32.dll\0"):]
    listing = ndisasm(code[:len(code) - len(strings)], entry)
    texts = [text for _, _, text in listing]
    here = listing[2][0]
    slots = {0x87003C, 0x870040, 0x870048, 0x870084}
    calls = [text for text in texts if text.startswith("call")]
    indirect = {int(m.group(1), 16) for text in calls
                if (m := re.match(r"^call (?:dword (?:near )?)?\[ebp\+0x([0-9a-f]+)\]$", text))}
    other = [text for text in calls if not re.match(r"^call (?:dword (?:near )?)?\[ebp\+0x", text)]
    jumps = [branch_target(text) for text in texts if text.startswith("j")]
    check(texts[:2] == ["pusha", f"call {here:#x}"]
          and texts[-4:] == ["popa", "mov [edi+0x8],eax", "mov [edi+0xc],dl", "ret"]
          and indirect == slots and other == [f"call {here:#x}", "call eax"]
          and all(entry <= target < entry + len(code) for target in jumps),
          "it saves every register (pushad / popad), calls itself once for its address, GetModuleHandleA, "
          "GetProcAddress, CreateFileA and CloseHandle through the import table and GetTempPathA, "
          "branches only inside itself, then does the two writes it replaced and returns")
    check(strings == b"kernel32.dll\0GetTempPathA\0shell_32.dll\0",
          "the names it holds are kernel32.dll, GetTempPathA and shell_32.dll, and nothing else")


def test_replay_reattach(original, image, before):
    """A replay's camera follows its player into another kit (patch
    revision 11, REPLAY_REATTACH): replay_network_client's slot +0x54 is
    a thunk to is_player_current 0x242A40 in that function's int3 padding;
    the build without the late join keeps both."""
    stock = Image(original)
    va, old, new = P.REPLAY_REATTACH_THUNK
    slot, slot_old, slot_new = P.REPLAY_REATTACH_SLOT
    check(stock.read(va, len(old)) == old and before.read(va, len(old)) == old and image.read(va, len(old)) == new,
          f"{va:#x}: seven int3 in the original and the build without the late join, the thunk in this one")
    rows = [text for _, _, text in ndisasm(image.read(va, len(new)), va)]
    check(rows == ["mov eax,ecx", "jmp 0x242a40"], f"{va:#x}: `mov eax,ecx` / `jmp 0x242a40`: {rows}")
    target = [text for _, _, text in ndisasm(stock.read(0x242A40, 0x29), 0x242A40)]
    check(target == ["mov eax,[eax+0x4]", "test eax,eax", "jz 0x242a64", "mov ecx,0x119190", "test ecx,ecx",
                     "jz 0x242a64", "mov dl,[eax+0x114]", "cmp dl,[esp+0x4]", "jnz 0x242a64", "mov eax,0x1",
                     "ret 0x4", "xor eax,eax", "ret 0x4"],
          "is_player_current 0x242A40: the client in eax, the attached player at +4, its place (+0x114) against "
          "the one pushed, `ret 4`; the padding after it is the thunk's")
    # Nothing aims into the padding: rel8 and rel32 branch bytes of .text,
    # and 4-byte values anywhere in the file.
    text = original[P.TEXT_FILE:P.TEXT_FILE + P.TEXT_VSIZE]
    inside = lambda target: va <= target < va + len(old)
    hits = []
    for at in range(va - P.TEXT_VA - 0x90, va - P.TEXT_VA + len(old) + 0x90):
        op = text[at]
        if (op == 0xEB or 0x70 <= op <= 0x7F or 0xE0 <= op <= 0xE3) and inside(P.TEXT_VA + at + 2 + struct.unpack_from("<b", text, at + 1)[0]):
            hits.append(f"{P.TEXT_VA + at:#x} rel8")
    for opcode, size in ((bytes([0xE8]), 5), (bytes([0xE9]), 5), (bytes([0x0F]), 6)):
        at = text.find(opcode)
        while at != -1:
            if at + size <= len(text) and (size == 5 or 0x80 <= text[at + 1] <= 0x8F):
                target = (P.TEXT_VA + at + size + struct.unpack_from("<i", text, at + size - 4)[0]) & 0xFFFFFFFF
                if inside(target):
                    hits.append(f"{P.TEXT_VA + at:#x} rel32")
            at = text.find(opcode, at + 1)
    for target in range(va, va + len(old)):
        if original.find(struct.pack("<I", target)) != -1:
            hits.append(f"{target:#x} as a value")
    check(not hits, "no branch bytes or pointer values aim into the padding" + "".join(", " + h for h in hits))
    check(stock.read(slot, 4) == slot_old and before.read(slot, 4) == slot_old and image.read(slot, 4) == slot_new,
          f"{slot:#x}: 0x54e10 (`xor al,al` / `ret 4`) in the original and the build without, {va:#x} in this one")
    check([text for _, _, text in ndisasm(stock.read(0x54E10, 5), 0x54E10)] == ["xor al,al", "ret 0x4"],
          "0x54E10 is `xor al,al` / `ret 4`")
    check(rtti_name(stock, 0x9FF344)[0] == ".?AVreplay_network_client@survarium@@",
          "the vtable at 0x9FF344 is replay_network_client's")
    relocated = lambda img: [rva for kind, rva in img.relocations().values() if kind == 3 and rva == slot]
    check(relocated(stock) and relocated(image), f"the slot {slot:#x} keeps its base relocation")
    under = [rva for kind, rva in stock.relocations().values() if kind == 3 and rva + 4 > va and rva < va + len(old)]
    check(not under, "no base relocation under the thunk")


def branches_into(original, lo, hi):
    """Where .text's rel8/rel32 branch bytes, or 4-byte values anywhere in
    the file, aim into lo..hi-1."""
    text = original[P.TEXT_FILE:P.TEXT_FILE + P.TEXT_VSIZE]
    inside = lambda target: lo <= target < hi
    hits = []
    for at in range(lo - P.TEXT_VA - 0x90, hi - P.TEXT_VA + 0x90):
        op = text[at]
        if (op == 0xEB or 0x70 <= op <= 0x7F or 0xE0 <= op <= 0xE3) and inside(P.TEXT_VA + at + 2 + struct.unpack_from("<b", text, at + 1)[0]):
            hits.append(f"{P.TEXT_VA + at:#x} rel8")
    for opcode, size in ((bytes([0xE8]), 5), (bytes([0xE9]), 5), (bytes([0x0F]), 6)):
        at = text.find(opcode)
        while at != -1:
            if at + size <= len(text) and (size == 5 or 0x80 <= text[at + 1] <= 0x8F):
                target = (P.TEXT_VA + at + size + struct.unpack_from("<i", text, at + size - 4)[0]) & 0xFFFFFFFF
                if inside(target):
                    hits.append(f"{P.TEXT_VA + at:#x} rel32")
            at = text.find(opcode, at + 1)
    for target in range(lo, hi):
        if original.find(struct.pack("<I", target)) != -1:
            hits.append(f"{target:#x} as a value")
    return hits


def test_sound_queue(original, image, before):
    """A weapon's full sound queue drops the new sound (patch revision 11,
    SOUND_QUEUE): delayed_callbacks_queue::add 0x2454A0 jumps after its
    drain to a cave in int3 padding, which goes to the game's own drop exit
    when the queue is full; the build without the late join keeps both."""
    stock = Image(original)
    rows = lambda img, va, size: [text for _, _, text in ndisasm(img.read(va, size), va)]
    for va, old, new in (P.SOUND_QUEUE_HOOK, P.SOUND_QUEUE_CAVE_SITE):
        check(stock.read(va, len(old)) == old and before.read(va, len(old)) == old and image.read(va, len(new)) == new,
              f"{va:#x}: the original's bytes in the original and the build without the late join, the new ones in this one")
    check(rows(image, P.SOUND_QUEUE_HOOK_VA, 5) == ["jmp 0x835a20"], "0x2454C1: `jmp 0x835a20`")
    check(rows(image, P.SOUND_QUEUE_CAVE_VA, len(P.SOUND_QUEUE_CAVE)) ==
          ["mov ecx,[esi+0x4]", "cmp ecx,[esi+0x8]", "jnc 0x835a2f", "mov eax,[esi]", "jmp 0x2454c6", "jmp 0x24569a"],
          "the cave: the queue's end against its capacity end; full -> 0x24569A, else the two instructions and back")
    start, size = P.SOUND_QUEUE_PADDING
    run = stock.read(start - 1, size + 2)
    check(run[0] == 0xC3 and run[1:-1] == bytes.fromhex("cc") * size and run[-1] == 0x55
          and start <= P.SOUND_QUEUE_CAVE_VA and P.SOUND_QUEUE_CAVE_VA + len(P.SOUND_QUEUE_CAVE) <= start + size,
          "the cave lies in 42 int3 between a `ret` and a `push ebp`")
    check(rows(stock, 0x2454B0, 0x18) == ["mov esi,[esp+0x19c]", "push edi", "push ebx", "push esi", "mov edi,eax",
                                          "call 0x245860", "mov eax,[esi]", "mov ecx,[esi+0x4]", "xor edx,edx"],
          "add: esi = the queue, its drain (which takes its two arguments off), then the queue's begin and end")
    between = rows(stock, 0x2454C6, 0x2454F7 - 0x2454C6)
    check(between[-1] == "jnz near 0x245691" and not [r for r in between if ("push" in r or "pop" in r or "esp," in r) and r != "lea esp,[esp+0x0]"],
          "no push, pop or esp change (but a `lea esp,[esp+0]` filler) from 0x2454C6 to the chain branch: 0x245691 and 0x24569A have the hook's esp")
    check(rows(stock, 0x245691, 0x9) == ["cmp edx,[esp+0x1a4]", "jna 0x2456e1"],
          "0x245691: chain length against the cap; past it falls into 0x24569A")
    check(rows(stock, 0x24569A, 0xC) == ["mov edi,[esp+0x1ac]", "cmp dword [edi],byte +0x0", "jnz 0x2456c0"]
          and rows(stock, 0x2456C0, 0x14) == ["mov eax,[edi]", "push byte +0x0", "and eax,byte -0x2",
                                              "mov ecx,[eax+0x4]", "push ebx", "add edi,byte +0x8", "push edi",
                                              "call ecx", "add esp,byte +0xc"],
          "0x24569A: the functor from the frame, called with flag 0")
    check(rows(stock, 0x2456D4, 0xD) == ["pop edi", "pop esi", "pop ebp", "pop ebx", "add esp,0x18c", "ret 0x18"],
          "then add's own epilogue, `ret 18h`")
    check(rows(stock, 0x297E20, 0xD) == ["push ebp", "mov ebp,esp", "and esp,byte -0x8", "sub esp,byte +0x74",
                                         "cmp byte [ebp+0x14],0x0"],
          "the sound player 0x297E20 looks at its flag first")
    check(rows(stock, 0x245AD5, 0x9) == ["mov eax,[ebx+0x4]", "push esi", "cmp eax,[ebx+0x8]", "jc 0x245b33"],
          "push_back 0x245AD0 asserts on the same two fields, end against capacity end")
    hits = branches_into(original, P.SOUND_QUEUE_HOOK_VA + 1, P.SOUND_QUEUE_HOOK_VA + 5) + \
        branches_into(original, start, start + size)
    check(not hits, "no branch bytes or pointer values aim inside the hook or into the padding" + "".join(", " + h for h in hits))
    for va, old, _ in (P.SOUND_QUEUE_HOOK, P.SOUND_QUEUE_CAVE_SITE):
        under = [rva for kind, rva in stock.relocations().values() if kind == 3 and rva + 4 > va and rva < va + len(old)]
        check(not under, f"no base relocation under {va:#x}")


def test_seek_clear(original, image, before):
    """A replay's user seek empties every client weapon's delayed sounds
    (patch revision 11, SEEK_CLEAR_HOOK): the site, that only the replay
    reaches it, and seek_clear as the image has it. check_late_join_026g.py
    runs it on made-up places, kits and weapons."""
    stock = Image(original)
    rows = lambda img, va, size: [text for _, _, text in ndisasm(img.read(va, size), va)]
    va, old, offset, _ = P.SEEK_CLEAR_HOOK
    cave = P.SEEK_CLEAR_VA + offset
    check(stock.read(va, len(old)) == old and before.read(va, len(old)) == old
          and rows(image, va, 6) == [f"jmp {cave:#x}", "nop"],
          f"{va:#x}: `lea eax,[edi-0x3e8]` in the original and the build without the late join, `jmp {cave:#x}` here")
    check(rows(stock, 0x1EE2E0, 6) == ["mov eax,[esi+0x53c8]"],
          "the seek 0x1EE2E0 starts from the replay reader [game+53C8h]")
    check(rows(stock, 0x1EE38B, 0x2A) == ["call 0x1ee4a0", "push edi", "push esi", "call 0x1edef0", "push edi",
                                          "mov eax,esi", "call 0x1ee4a0", "test bl,bl", "jnz 0x1ee3b5",
                                          "lea eax,[edi-0x3e8]", "mov [esi+0x4ec],eax", "mov [esi+0x57c],eax"],
          "set_time_in_ms(key frame), the re-simulation, set_time_in_ms(target), then a user seek's own part")
    text = original[P.TEXT_FILE:P.TEXT_FILE + P.TEXT_VSIZE]
    callers = []
    for opcode in (0xE8, 0xE9):
        at = text.find(bytes([opcode]))
        while at != -1:
            if at + 5 <= len(text) and (P.TEXT_VA + at + 5 + struct.unpack_from("<i", text, at + 1)[0]) & 0xFFFFFFFF == 0x1EE2E0:
                callers.append(P.TEXT_VA + at)
            at = text.find(bytes([opcode]), at + 1)
    check(sorted(callers) == [0x1EF8C0, 0x233E2F],
          "the seek's only callers: the replay seek functor 0x1EF8C0 and the replay movie's slider 0x233E2F")
    check(original.count(struct.pack("<I", 0xA0422C)) == 2 and rows(stock, 0x24365C, 7) == ["mov dword [ebx+0x10],0xa0422c"],
          "0xA0422C, the client weapon's item vtable, is written by its constructor (and destructor) alone")
    check(rows(stock, 0x2438BC, 0x1A)[:6] == ["mov eax,[esi+0x9fc]", "lea edi,[esi+0xa00]", "push edi",
                                              "call 0x245c50", "mov edx,[esi+0x9fc]", "mov [edi],edx"],
          "~weapon empties the queue as seek_clear does: 0x245C50 on begin..&end, then end = begin")
    hits = branches_into(original, va + 1, va + len(old))
    check(not hits, "no branch bytes or pointer values aim inside the hook" + "".join(", " + h for h in hits))
    under = [rva for kind, rva in stock.relocations().values() if kind == 3 and rva + 4 > va and rva < va + len(old)]
    check(not under, f"no base relocation under {va:#x}")
    code = rows(image, cave, 0x97)
    check(code[:3] == ["pusha", f"call {cave + 6:#x}", "pop ebp"]
          and code[3] == f"add ebp,{(0xA0422C - (cave + 6)) & 0xFFFFFFFF:#x}"
          and "call 0x245c50" in code and code[-3:] == ["popa", "lea eax,[edi-0x3e8]", "jmp 0x1ee3a9"],
          "seek_clear: the vtable found relative to itself, 0x245C50 for the functors, the lea and back to 0x1EE3A9")


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
    pickup = open(P.pickup_bin(), "rb").read()
    seek = open(P.seek_clear_bin(), "rb").read()
    stale_dll = open(P.stale_dll_bin(), "rb").read()
    # The first page's pieces, in order, none overlapping the next.
    pieces = [(0, len(first_page), "the build without the late join's first page"),
              (P.PICKUP_OFFSET, len(pickup), "the pickup code"),
              (P.SEEK_CLEAR_OFFSET, len(seek), "the seek code"),
              (P.ARCHIVE_PATH_OFFSET, len(P.ARCHIVE_PATH), "the archive's name"),
              (P.STALE_DLL_OFFSET, len(stale_dll), "the shell_32.dll check"),
              (P.LATE_JOIN_OFFSET, len(code), "the late-join code")]
    overlaps = [f"{a[2]} ends at {a[0] + a[1]:#x}, past {b[2]} at {b[0]:#x}"
                for a, b in zip(pieces, pieces[1:]) if a[0] + a[1] > b[0]]
    check(not overlaps, "the first page's code does not overlap: " + ", ".join(
        f"{name} [{start:#x}, {start + size:#x})" for start, size, name in pieces) + "".join("; " + o for o in overlaps))
    check(body[:len(first_page)] == first_page
          and set(body[len(first_page):P.PICKUP_OFFSET]) <= {0xCC}
          and body[P.PICKUP_OFFSET:P.PICKUP_OFFSET + len(pickup)] == pickup
          and set(body[P.PICKUP_OFFSET + len(pickup):P.SEEK_CLEAR_OFFSET]) <= {0xCC}
          and body[P.SEEK_CLEAR_OFFSET:P.SEEK_CLEAR_OFFSET + len(seek)] == seek
          and set(body[P.SEEK_CLEAR_OFFSET + len(seek):P.ARCHIVE_PATH_OFFSET]) <= {0xCC}
          and body[P.ARCHIVE_PATH_OFFSET:P.ARCHIVE_PATH_OFFSET + len(P.ARCHIVE_PATH)] == P.ARCHIVE_PATH
          and set(body[P.ARCHIVE_PATH_OFFSET + len(P.ARCHIVE_PATH):P.STALE_DLL_OFFSET]) <= {0xCC}
          and body[P.STALE_DLL_OFFSET:P.STALE_DLL_OFFSET + len(stale_dll)] == stale_dll
          and set(body[P.STALE_DLL_OFFSET + len(stale_dll):P.LATE_JOIN_OFFSET]) <= {0xCC}
          and body[P.LATE_JOIN_OFFSET:P.LATE_JOIN_OFFSET + len(code)] == code
          and set(body[P.LATE_JOIN_OFFSET + len(code):]) <= {0xCC},
          f"its first page is the build's without the late join ({len(first_page)} bytes, then int3) with the "
          f"pickup code at +{P.PICKUP_OFFSET:#x}, the seek code at +{P.SEEK_CLEAR_OFFSET:#x}, the archive's "
          f"name at +{P.ARCHIVE_PATH_OFFSET:#x} and the "
          f"shell_32.dll check at "
          f"+{P.STALE_DLL_OFFSET:#x}, the code follows, then int3 padding")
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
    ranges.append((image.offset(P.REPLAY_REATTACH_SLOT[0]), 4))
    ranges += [(P.file_offset(P.REVISION_VA) + 4, 1), (P.file_offset(P.LATE_JOIN_READY_VA) + 1, 4),
               (P.file_offset(P.DB_SIGNATURE_VA), len(P.DB_SIGNATURE[1])), (P.file_offset(P.STALE_DLL_VA), len(P.STALE_DLL[1])),
               (P.file_offset(P.ARCHIVE_PATH_VA) + 1, 4),
               (headers.count_at, 2), (headers.size_of_code_at, 4), (headers.optional + 12, 4),
               (headers.size_of_image_at, 4), (headers.checksum_at, 4), (headers.security_at, 4),
               (headers.table + 40 * 7 + 8, 4), (headers.table + 40 * 7 + 16, 4), (headers.table + 40 * 8, 40)]
    # The entries of the relocation table that are disabled (scene::remove_model's
    # branch, uro_fov's least, the pickup's distance).
    disabled = {P.REMOVE_MODEL_VA + 2, P.FOV_MINIMUM_RELOC, P.PICKUP_RELOC}
    ranges += [(at, 2) for at, (kind, va) in before.relocations().items()
               if kind == 3 and va in disabled]
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
    # The sites with an address in them: scene::remove_model's branch, the
    # field-of-view setting's two in place and the pickup's distance.
    fov_sites = {va for va, _, _ in P.FOV_PATCHES}
    with_address = {P.REMOVE_MODEL_VA, P.PICKUP_HOOKS[0][0]} | fov_sites
    kept = [(lo, hi) for lo, hi in sites if lo not in with_address]
    under_kept = [va for kind, va in Image(original).relocations().values()
                  if kind == 3 and any(va + 4 > lo and va < hi for lo, hi in kept)]
    under_fov = sorted(va for kind, va in Image(original).relocations().values()
                       if kind == 3 and any(va + 4 > lo and va < hi for lo, hi in sites if lo in fov_sites))
    option = P.FOV_OPTION_NAME[0] + 1
    now, was = image.relocations(), before.relocations()
    differing = sorted(key for key in set(now) | set(was) if now.get(key) != was.get(key))
    check(not under_kept and sorted(was[key][1] for key in differing) == sorted(disabled)
          and all(was[key][0] == 3 and now[key][0] == 0 for key in differing)
          and under_fov == sorted({option, P.FOV_MINIMUM_RELOC})
          and any(kind == 3 and va == option for kind, va in now.values()),
          "no base relocation is under a late-join site but those of scene::remove_model's branch "
          f"({P.REMOVE_MODEL_VA + 2:#x}), of uro_fov's least ({P.FOV_MINIMUM_RELOC:#x}) and of the pickup's "
          f"distance ({P.PICKUP_RELOC:#x}), which are disabled, "
          f"and of the field-of-view option's name "
          f"({option:#x}, an address for an address), which stays; the rest of the table is the build's without it")

    test_clock_factor(original, image, before)
    test_statistics_count(original, image, before)
    test_pregame_progress(original, image, before)
    test_fov(original, image, before)
    test_pickup(original, image, before)
    test_weapon_random(original, image, before)
    test_kit_state(original, image, before)
    test_first_load(original, image, before)
    test_tdm_deaths(original, image, before)
    test_replay_reattach(original, image, before)
    test_sound_queue(original, image, before)
    test_seek_clear(original, image, before)
    test_db_signature(original, image, before)
    test_archive_path(original, image, before)
    test_stale_dll(original, image, before)
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
