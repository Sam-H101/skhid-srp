#!/usr/bin/env python3
"""Execute the original and patch 21's camera projections and compare them.

    python check_fov_026g.py <original survarium.exe>

Requires Python `unicorn`. The real matrix builder (0xEEE30, including its
tangent call) runs on both images. Original with its 65-degree constant
replaced by r radians must equal the default build of patch_026g.py (patch
revision 11) with uro_fov = r * 180 / pi, for several zoom factors, viewports
and angles. Also checks the option name, the uro_fov lower bound and the
relocations.
"""
import math
import struct
import sys

from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_PROT_ALL
from unicorn.x86_const import UC_X86_REG_ECX, UC_X86_REG_EIP, UC_X86_REG_ESP

import patch_026g as base

CONSTANT = 0xA2D9EC
STACK, THIS, RETURN = 0x20000000, 0x20100000, 0x20200000  # above the image
VIEW, OUT = THIS + 0x100, THIS + 0x200


def image(data):
    """Map the PE sections at the preferred base (the exe is never rebased)."""
    emu = Uc(UC_ARCH_X86, UC_MODE_32)
    headers = base.Headers(data)
    for _, rva, vsize, raw, rsize, _ in headers.sections(data):
        start = base.IMAGE_BASE + rva
        size = (max(vsize, rsize) + 0xFFF) & ~0xFFF
        emu.mem_map(start, size, UC_PROT_ALL)
        emu.mem_write(start, bytes(data[raw:raw + min(rsize, vsize or rsize)]))
    emu.mem_map(STACK, 0x10000, UC_PROT_ALL)
    emu.mem_map(THIS, 0x1000, UC_PROT_ALL)
    emu.mem_map(RETURN, 0x1000, UC_PROT_ALL)
    return emu


def projection(emu, zoom, width, height, near=0.1, far=1000.0):
    emu.mem_write(THIS + 0x88, struct.pack("<fff", near, far, zoom))
    emu.mem_write(VIEW, struct.pack("<ff", width, height))
    emu.mem_write(OUT, bytes(64))
    esp = STACK + 0x8000
    emu.mem_write(esp, struct.pack("<III", RETURN, OUT, VIEW))
    emu.reg_write(UC_X86_REG_ESP, esp)
    emu.reg_write(UC_X86_REG_ECX, THIS)
    emu.emu_start(base.FOV_PROJECTION_VA, RETURN, count=100000)
    if emu.reg_read(UC_X86_REG_EIP) != RETURN or emu.reg_read(UC_X86_REG_ESP) != esp + 12:
        raise AssertionError("projection did not return with ret 8")
    return struct.unpack("<16f", emu.mem_read(OUT, 64))


def relocations(data):
    headers = base.Headers(data)
    rva, size = struct.unpack_from("<II", data, headers.reloc_at)
    offset = next(rva - start + raw for _, start, vsize, raw, rsize, _ in headers.sections(data)
                  if start <= rva < start + max(vsize, rsize))
    found, end = {}, offset + size
    while offset < end:
        page, block = struct.unpack_from("<II", data, offset)
        if block < 8:
            break
        for at in range(offset + 8, offset + block, 2):
            entry = struct.unpack_from("<H", data, at)[0]
            if entry:
                found.setdefault(base.IMAGE_BASE + page + (entry & 0xFFF), entry >> 12)
        offset += block
    return found


def main():
    original = open(sys.argv[1], "rb").read()
    code = open(base.default_bin("section"), "rb").read()
    late_join = open(base.late_join_bin(), "rb").read()
    patched, _ = base.build(original, "section", code, None, late_join)
    stock, new = image(original), image(patched)
    checks = 0
    for degrees in (50.0, 65.0, 74.5, 90.0, 100.0):
        radians = struct.unpack("<f", struct.pack("<f", degrees * math.pi / 180))[0]
        stock.mem_write(CONSTANT, struct.pack("<f", radians))
        new.mem_write(base.FOV_VALUE, struct.pack("<f", degrees))
        for zoom in (1.0, 0.5, 0.25):
            for width, height in ((1920.0, 1080.0), (1280.0, 1024.0), (800.0, 600.0)):
                expected = projection(stock, zoom, width, height)
                actual = projection(new, zoom, width, height)
                # The angle is rounded once to float in both; allow one ulp from x87 vs SSE.
                if any(abs(a - b) > 1e-6 * max(1.0, abs(b)) for a, b in zip(actual, expected)):
                    raise AssertionError(f"{degrees} deg, zoom {zoom}, {width}x{height}: {actual} != {expected}")
                checks += 1
    # Untouched defaults: uro_fov 65.0 reproduces the stock camera.
    default = image(patched)
    assert struct.unpack("<f", default.mem_read(base.FOV_VALUE, 4))[0] == 65.0
    stock_default = image(original)
    for a, b in zip(projection(default, 1.0, 1920.0, 1080.0), projection(stock_default, 1.0, 1920.0, 1080.0)):
        assert abs(a - b) <= 1e-6 * max(1.0, abs(b)), (a, b)

    name = struct.unpack("<I", base.FOV_OPTION_NAME[2][1:])[0]
    assert bytes(default.mem_read(name, 8)) == b"uro_fov\0"
    assert bytes(default.mem_read(base.FOV_OPTION_NAME[0], 5)) == base.FOV_OPTION_NAME[2]
    least = struct.unpack("<I", base.FOV_MINIMUM[1][7:11])[0]
    assert bytes(default.mem_read(base.FOV_MINIMUM[0] + 2, 8)) == struct.pack("<If", least, base.FOV_MIN)
    # The option's name keeps its relocation; the other two sites' are disabled.
    before, after = relocations(original), relocations(patched)
    option = base.FOV_OPTION_NAME[0] + 1
    assert before.get(option) == after.get(option) == 3
    for reloc in (base.FOV_PROJECTION_RELOC, base.FOV_MINIMUM_RELOC):
        assert before.get(reloc) == 3 and after.get(reloc) == 0, hex(reloc)
    print(f"patch 21: {checks} projections match; option name, minimum and relocations checked")


if __name__ == "__main__":
    main()
