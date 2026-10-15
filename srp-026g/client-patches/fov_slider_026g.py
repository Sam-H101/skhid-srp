#!/usr/bin/env python3
"""Add a working field-of-view slider to the 0.26g Settings → Graphics tab.

Two builds, each writing a NEW file; inputs are never modified:

    python fov_slider_026g.py exe <original survarium.exe> <new survarium_skhid.exe> [--server=host:port]
    python fov_slider_026g.py resources <resources.db> <new resources.db> --ffdec <ffdec.jar>

The exe is the full SRP patch (with the late join) plus three changes. The game already lists a
"fov" slider (snap 1) as video option 7, but no console variable has that name,
and the camera uses a fixed 65 degrees rather than the unused `uro_fov`.
- option 7 reads and writes `uro_fov` (vertical degrees, saved in user.cfg);
- the camera projection uses `uro_fov` instead of the 65-degree constant;
- the lower bound of `uro_fov` becomes 50 instead of 0 (100 remains the upper bound).
The slider is absent from the stock timeline. The resources build compiles
SrpFovSlider.as into main_menu.swf, which places it below gamma. Requires a JDK and JPEXS FFDec.
Any resources.db works if its main_menu.swf is the build 2777 original, so
the login hint can be applied before or after. Both builds are needed: the
stock exe has no "fov" variable, and the stock menu has no slider.
"""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

import login_hint_026g as hint
import patch_026g as base

# Retail build 2777: resources-db list -> flash_movies/main_menu.swf.
MOVIE_OFFSET = 17970448
MOVIE_SIZE = 1194596
MOVIE_SHA256 = "0ff118a85b53f69eb65f36b0f5996bdbdc12ec5068440e4d4d4dd6ece49481a4"
HERE = Path(__file__).resolve().parent / "fov_slider"

URO_FOV = 0xA6FA28         # float storage of the `uro_fov` console variable
URO_FOV_NAME = 0xA281D0    # "uro_fov"
FOV_MIN = 50.0

# The video options constructor names option 7 by `mov eax,offset "fov"`
# (0xA0659C). The relocation at 0x262931 stays valid for the new address.
OPTION_NAME = (0x262930, bytes.fromhex("b89c65a000"),
               b"\xb8" + struct.pack("<I", URO_FOV_NAME))

# Camera projection (vtable function shared by every camera). Original:
# far, near and aspect on the stack, then xmm0 = [ecx+90h] (zoom factor)
# * 1.134464 (65 degrees as radians, 0xA2D9EC) for the matrix builder 0xEEE30.
# New: the same arguments and the same callee, xmm0 = [ecx+90h] * uro_fov
# * pi / 180. One absolute address (uro_fov) replaces the constant's.
PROJECTION_VA = 0x242900
PROJECTION_ORIGINAL = bytes.fromhex(
    "d9818c0000008b442408f30f1000f30f5e4004568b74240883ec0cd95c2408"
    "d98188000000d95c2404f30f110424f30f108190000000f30f5905ecd9a200"
    "e8edc4eaff83c40c8bc65ec20800cccccccc")
PROJECTION = b"".join([
    bytes.fromhex("8b442408"),             # mov eax,[esp+8]     viewport size
    bytes.fromhex("56"),                   # push esi
    bytes.fromhex("8b742408"),             # mov esi,[esp+8]     out matrix
    bytes.fromhex("ffb18c000000"),         # push dword [ecx+8Ch]  far
    bytes.fromhex("ffb188000000"),         # push dword [ecx+88h]  near
    bytes.fromhex("6800003443"),           # push 180.0          aspect slot
    bytes.fromhex("d98190000000"),         # fld dword [ecx+90h]
    b"\xd8\x0d" + struct.pack("<I", URO_FOV),  # fmul dword [uro_fov]
    bytes.fromhex("d9eb"),                 # fldpi
    bytes.fromhex("dec9"),                 # fmulp st1
    bytes.fromhex("d83424"),               # fdiv dword [esp]
    bytes.fromhex("d91c24"),               # fstp dword [esp]
    bytes.fromhex("f30f100424"),           # movss xmm0,[esp]
    bytes.fromhex("d900"),                 # fld dword [eax]
    bytes.fromhex("d87004"),               # fdiv dword [eax+4]
    bytes.fromhex("d91c24"),               # fstp dword [esp]    aspect
    b"\xe8" + struct.pack("<i", 0xEEE30 - (PROJECTION_VA + 0x42)),  # call 0xEEE30
    bytes.fromhex("83c40c"),               # add esp,0Ch
    bytes.fromhex("8bc6"),                 # mov eax,esi
    bytes.fromhex("5e"),                   # pop esi
    bytes.fromhex("c20800"),               # ret 8
]).ljust(len(PROJECTION_ORIGINAL), b"\xcc")
PROJECTION_RELOC = (0x24293A, PROJECTION_VA + 0x22)

# uro_fov registration: `xorps xmm0,xmm0; movss [minimum],xmm0` becomes
# `mov dword [minimum],50.0; nop`. xmm0 is reloaded with the maximum next.
MINIMUM = (0x85AE06, bytes.fromhex("0f57c0f30f11055465aa00"),
           bytes.fromhex("c7055465aa00") + struct.pack("<f", FOV_MIN) + b"\x90")
MINIMUM_RELOC = (0x85AE0D, 0x85AE08)


def move_reloc(data, old_va, new_va):
    """Move one HIGHLOW base relocation within its 4 KiB block."""
    if old_va >> 12 != new_va >> 12:
        raise ValueError("relocations can only move within their page")
    headers = base.Headers(data)
    rva, size = struct.unpack_from("<II", data, headers.reloc_at)
    offset = next(rva - start + raw for _, start, vsize, raw, rsize, _ in headers.sections(data)
                  if start <= rva < start + max(vsize, rsize))
    end = offset + size
    while offset < end:
        page, block = struct.unpack_from("<II", data, offset)
        if block < 8:
            break
        if base.IMAGE_BASE + page == old_va & ~0xFFF:
            for at in range(offset + 8, offset + block, 2):
                if struct.unpack_from("<H", data, at)[0] == 0x3000 | old_va & 0xFFF:
                    struct.pack_into("<H", data, at, 0x3000 | new_va & 0xFFF)
                    return
        offset += block
    raise ValueError(f"no relocation at {old_va:#x}")


def srp_exe(original, server=None):
    """The default full SRP build (with the late join), as patch_026g.py makes it."""
    code = Path(base.default_bin("section")).read_bytes()
    late_join = Path(base.late_join_bin()).read_bytes()
    return base.build(original, "section", code, server, late_join)[0]


def build_exe(original, server=None):
    data = srp_exe(original, server)
    base.patch(data, *OPTION_NAME)
    base.patch(data, PROJECTION_VA, PROJECTION_ORIGINAL, PROJECTION)
    base.patch(data, *MINIMUM)
    move_reloc(data, *PROJECTION_RELOC)
    move_reloc(data, *MINIMUM_RELOC)
    headers = base.Headers(data)
    struct.pack_into("<I", data, headers.checksum_at, base.pe_checksum(data, headers.checksum_at))
    return data


def build_movie(original, ffdec, java, javac, work):
    if hashlib.sha256(original).hexdigest() != MOVIE_SHA256:
        raise ValueError("Not the original build 2777 main menu movie (or already patched)")
    version, header, tags = hint.unpack(original)
    code = work / "code.swf"
    code.write_bytes(hint.pack(version, header, [t for t in tags if t[0] in (69, 82, 0)], False))
    subprocess.run([javac, "-encoding", "UTF-8", "-cp", str(ffdec), "-d", str(work),
                    str(HERE / "PatchFovSlider.java")], check=True)
    edited = work / "edited.swf"
    subprocess.run([java, "-Djava.awt.headless=true", "-Duser.home=" + str(work),
                    "-cp", str(work) + os.pathsep + str(ffdec), "PatchFovSlider",
                    str(code), str(HERE / "SrpFovSlider.as"), str(edited)], check=True)
    replacements = [t for t in hint.unpack(edited.read_bytes())[2] if t[0] == 82]
    before = [t for t in tags if t[0] == 82]
    if len(before) != len(replacements) or sum(a != b for a, b in zip(before, replacements)) != 1:
        raise ValueError("Expected exactly one changed ABC block")
    replacement_iter = iter(replacements)
    updated = [next(replacement_iter) if t[0] == 82 else t for t in tags]
    result = hint.pack(version, header, updated)
    _, saved_header, saved_tags = hint.unpack(result)
    if saved_header != header or [t for t in tags if t[0] != 82] != [t for t in saved_tags if t[0] != 82]:
        raise ValueError("Non-script content changed")
    if len(result) > MOVIE_SIZE - 4:
        raise ValueError("Patched movie no longer fits the original archive entry")
    # Same size and CRC as the original entry: the signed file table is unchanged.
    return hint.preserve_crc(result + bytes(MOVIE_SIZE - len(result)), original)


def write_new(path, data):
    with path.open("xb") as stream:
        stream.write(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    exe = commands.add_parser("exe", help="full SRP exe with the FOV option connected")
    exe.add_argument("source", type=Path, help="original build 2777 survarium.exe")
    exe.add_argument("output", type=Path)
    exe.add_argument("--server", help="embedded default login server, as in patch_026g.py")
    resources = commands.add_parser("resources", help="resources.db with the slider in main_menu.swf")
    resources.add_argument("source", type=Path, help="resources.db with the original main_menu.swf")
    resources.add_argument("output", type=Path, help="new resources.db, outside the original installation")
    resources.add_argument("--ffdec", required=True, type=Path, help="path to ffdec.jar")
    resources.add_argument("--java", default="java")
    resources.add_argument("--javac", default="javac")
    args = parser.parse_args()
    if args.output.exists() or args.output.is_symlink():
        parser.error("Output already exists; choose a new path")

    if args.command == "exe":
        data = build_exe(args.source.read_bytes(), args.server)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        write_new(args.output, data)
        print(f"Created {args.output}: FOV option uses uro_fov; SHA-256 {hashlib.sha256(data).hexdigest()}")
        return
    with args.source.open("rb") as source:
        source.seek(MOVIE_OFFSET)
        original = source.read(MOVIE_SIZE)
    with tempfile.TemporaryDirectory(prefix="srp-fov-slider-") as temporary:
        movie = build_movie(original, args.ffdec.resolve(), args.java, args.javac, Path(temporary))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.source.open("rb") as source, args.output.open("xb") as output:
        shutil.copyfileobj(source, output, 8 * 1024 * 1024)
        output.seek(MOVIE_OFFSET)
        output.write(movie)
    print(f"Created {args.output}: FOV slider in Settings → Graphics")


if __name__ == "__main__":
    main()
