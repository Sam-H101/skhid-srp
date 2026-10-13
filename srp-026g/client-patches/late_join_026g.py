#!/usr/bin/env python3
"""Build the experimental 0.26g late-join client.

Requires a growing-roster server, NASM, an x86 C compiler and GNU ld.
CC/LD override compiler paths. Source and existing outputs are never changed.
"""
import argparse
import hashlib
import os
import re
from pathlib import Path
import struct
import subprocess
import tempfile

import patch_026g as base

ASM_SIZE = 4096
PATCH_REVISION = 0x80

# Keep the actual roster count for resources and the countdown. Only storage
# and wire layouts that must survive roster growth use the fixed capacity.
PATCHES = (
    # The ED/EE player masks have three bytes from the first connection.
    # Both implementations of base_match_client::set_players_count:
    # journaling vtable A083D8+34 and live match_client A08864+34.
    (0x279610, bytes.fromhex("0fb6442404489983e20703c2c1f803fec0"),
     bytes.fromhex("b803000000") + b"\x90" * 12),
    (0x27A2E0, bytes.fromhex("0fb6442404489983e20703c2c1f803fec0"),
     bytes.fromhex("b803000000") + b"\x90" * 12),
    # kd_stats_rule: its fixed twenty-entry array is zeroed by the ctor.
    (0x3237B4, bytes.fromhex("8a91b5770500"), b"\xb2\x14" + b"\x90" * 4),
    # shared_statistics::{serialize,deserialize}: twenty initialized records.
    # Don't alter the live participant count used by rewards/event handlers.
    (0x2F5ACF, bytes.fromhex("389ee0310000"), b"\x80\xfb\x14" + b"\x90" * 3),
    (0x2F5AF4, bytes.fromhex("3a9ee0310000"), b"\x80\xfb\x14" + b"\x90" * 3),
    (0x2F5BE6, bytes.fromhex("389ee0310000"), b"\x80\xfb\x14" + b"\x90" * 3),
    (0x2F5C03, bytes.fromhex("3a9ee0310000"), b"\x80\xfb\x14" + b"\x90" * 3),
)
HOOKS = (
    (0x2CBF9B, bytes.fromhex("e860c10000"), 3072, 0xE8),
    (0x2D3880, bytes.fromhex("e89bdf0400"), 0, 0xE8),
    (0x2D3906, bytes.fromhex("03f28d8de4e30000"), 96, 0xE9),
    (0x27A23D, bytes.fromhex("881c388b44241c"), 128, 0xE9),
    (0x27A378, bytes.fromhex("83be641c1f0001"), 1856, 0xE9),
    (0x1F2013, bytes.fromhex("e8e8000000"), 2048, 0xE8),
    (0x27B517, bytes.fromhex("8b8f401c1f00"), 2336, 0xE9),
    (0x219E60, bytes.fromhex("558bec83e4f8"), 2432, 0xE9),
)


def c_payload(directory):
    """Link position-independent validation and asynchronous ownership code."""
    binary = directory / "runtime.bin"
    link_map = directory / "runtime.map"
    script = directory / "validate.ld"
    script.write_text("""
SECTIONS {
  . = 0;
  _GLOBAL_OFFSET_TABLE_ = .;
  .text : { *(.text.entry) *(.text*) }
  .unexpected : { *(.rodata*) *(.data*) *(.bss*) *(.got*) }
  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) *(.llvm_addrsig) }
  ASSERT(SIZEOF(.unexpected) == 0, "validator must have no static storage")
}
""")
    objects = []
    for name in ("late_join_validate_026g", "late_join_runtime_026g", "late_join_install_026g"):
        source = Path(__file__).with_name(name + ".c")
        obj = directory / (name + ".o")
        subprocess.run([os.environ.get("CC", "cc"), "-m32", "-Os", "-std=c11",
                        "-ffreestanding", "-fPIC", "-fvisibility=hidden", "-fno-builtin",
                        "-fno-stack-protector", "-mno-sse", "-mno-sse2", "-mno-mmx",
                        "-fno-asynchronous-unwind-tables", "-fno-unwind-tables",
                        "-Wall", "-Wextra", "-Werror", "-c", str(source), "-o", str(obj)], check=True)
        objects.append(str(obj))
    subprocess.run([os.environ.get("LD", "ld"), "-m", "elf_i386", "-T", str(script),
                    "--oformat=binary", "-Map", str(link_map), *objects, "-o", str(binary)], check=True)
    symbols = {name: int(address, 16) for address, name in re.findall(
        r"^\s+(0x[0-9a-f]+)\s+(lj_\w+|late_join_validate_profile)\s*$",
        link_map.read_text(), re.MULTILINE)}
    if symbols.get("late_join_validate_profile") != 0 or "lj_complete" not in symbols:
        raise ValueError("missing C payload entry points")
    return binary.read_bytes(), symbols


def build(original, nasm="nasm", test_session=None):
    if test_session is not None and not 1 <= test_session <= 0xffffffff:
        raise ValueError("test session must be a nonzero u32")
    upload, _, _ = base.split_code(Path(base.default_bin("section")).read_bytes())
    cave = base.SECTION_VA + len(upload)
    with tempfile.TemporaryDirectory(prefix="srp-late-join-") as temporary:
        code, symbols = c_payload(Path(temporary))
        state_va = (cave + ASM_SIZE + len(code) + 4095) & ~4095
        binary = Path(temporary) / "late-join.bin"
        test_defines = [] if test_session is None else [f"-DTEST_SESSION={test_session}"]
        subprocess.run([nasm, "-f", "bin", *test_defines, f"-DCAVE={cave}",
                        f"-DC_CODE={cave + ASM_SIZE}",
                        f"-DLJ_COMPLETE={cave + ASM_SIZE + symbols['lj_complete']}",
                        f"-DLJ_PREPARE={cave + ASM_SIZE + symbols['lj_prepare']}",
                        f"-DLJ_ACTIVATE={cave + ASM_SIZE + symbols['lj_activate']}",
                        f"-DLJ_DEACTIVATE={cave + ASM_SIZE + symbols['lj_deactivate']}",
                        f"-DLJ_DESTROYED={cave + ASM_SIZE + symbols['lj_player_destroyed']}",
                        f"-DLJ_INSTALL={cave + ASM_SIZE + symbols['lj_native_install']}",
                        f"-DLJ_ANIMATIONS={cave + ASM_SIZE + symbols['lj_native_animations']}",
                        f"-DSTATE_VA={state_va}", f"-DUPLOAD_READY={base.SECTION_VA}",
                        str(Path(__file__).with_suffix(".asm")), "-o", str(binary)], check=True)
        extension = binary.read_bytes()
        if len(extension) != ASM_SIZE:
            raise ValueError("C payload must follow the NASM prefix")
        extension += code
    data, _ = base.build(original, "section", Path(base.default_bin("section")).read_bytes())
    address, _, released = base.REVISION_SITE
    experimental = bytes([0xC6, 0x44, 0x24, 0x27, PATCH_REVISION, 0x90])
    base.patch(data, address, released, experimental)
    # The released patch deliberately caps its payload at one page. Grow
    # only this experimental section, retaining the certificate blob and
    # the released builder's checks and hooks.
    headers = base.Headers(data)
    sections = headers.sections(data)
    name, rva, _, raw, old_size, flags = sections[-1]
    security, _ = struct.unpack_from("<II", data, headers.security_at)
    if name != base.SECTION_NAME or security != raw + old_size:
        raise ValueError("unexpected experimental code section layout")
    payload = upload + extension
    align = lambda n, a: (n + a - 1) // a * a
    raw_size = align(len(payload), headers.file_alignment)
    data[raw:security] = payload + b"\xcc" * (raw_size - len(payload))
    section_at = headers.table + 40 * (len(sections) - 1)
    struct.pack_into("<I", data, section_at + 8, len(payload))
    struct.pack_into("<I", data, section_at + 16, raw_size)
    struct.pack_into("<I", data, headers.size_of_image_at,
                     align(rva + len(payload), headers.section_alignment))
    struct.pack_into("<I", data, headers.size_of_code_at,
                     struct.unpack_from("<I", data, headers.size_of_code_at)[0] + raw_size - old_size)
    struct.pack_into("<I", data, headers.security_at, raw + raw_size)
    # Separate zero-initialized writable state; executable code remains RX.
    slot = headers.table + 40 * len(sections)
    if slot + 40 > headers.size_of_headers or any(data[slot:slot + 40]):
        raise ValueError("no free section-table entry for late-join state")
    data[slot:slot + 40] = b".srpd\0\0\0" + struct.pack(
        "<IIIIIIHHI", 4096, state_va - base.IMAGE_BASE, 0, 0, 0, 0, 0, 0, 0xC0000080)
    struct.pack_into("<H", data, headers.count_at, len(sections) + 1)
    struct.pack_into("<I", data, headers.size_of_image_at, state_va - base.IMAGE_BASE + 4096)
    uninitialized_at = headers.optional + 12
    struct.pack_into("<I", data, uninitialized_at,
                     struct.unpack_from("<I", data, uninitialized_at)[0] + 4096)
    ready_address = 0x27AEA1
    old_ready = b"\xe9" + struct.pack("<i", base.SECTION_VA - ready_address - 5) + b"\x90"
    new_ready = b"\xe9" + struct.pack("<i", cave + 1664 - ready_address - 5) + b"\x90"
    base.patch(data, ready_address, old_ready, new_ready)
    for address, expected, replacement in PATCHES:
        base.patch(data, address, expected, replacement)
    for address, expected, offset, opcode in HOOKS:
        replacement = bytes([opcode]) + struct.pack("<i", cave + offset - address - 5)
        base.patch(data, address, expected, replacement + b"\x90" * (len(expected) - 5))
    if test_session is not None:
        # Direct connections have no lobby profile whose statistics can reset.
        base.patch(data, 0x27a1d1, bytes.fromhex("33c0"), bytes.fromhex("eb05"))
        address = 0x1ec0e1
        base.patch(data, address, bytes.fromhex("e8fa150000"),
                   b"\xe8" + struct.pack("<i", cave + 2560 - address - 5))
    ranges = [(a, a + len(b)) for a, b, *_ in PATCHES + HOOKS]
    if base.neutralize_relocs(data, ranges):
        raise ValueError("unexpected relocation in growing-roster hooks")
    headers = base.Headers(data)
    struct.pack_into("<I", data, headers.checksum_at, base.pe_checksum(data, headers.checksum_at))
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("original", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--nasm", default="nasm")
    parser.add_argument("--test-session", type=lambda value: int(value, 0),
                        help="test only: bypass login and connect directly to -server with this session")
    args = parser.parse_args()
    if args.output.exists() or args.output.resolve() == args.original.resolve():
        parser.error("choose a new output file; existing executables are never overwritten")
    data = build(args.original.read_bytes(), args.nasm, args.test_session)
    with args.output.open("xb") as stream:
        stream.write(data)
    print(f"Experimental late-join client: {args.output}; SHA-256 {hashlib.sha256(data).hexdigest()}")


if __name__ == "__main__":
    main()
