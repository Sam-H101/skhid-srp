"""The client exes per version, and address <-> file offset from each exe's
own PE headers (image base and section table differ between builds).

    python exes.py VERSION        prints the exe path and its sections
"""
import os
import struct
import sys

# Original (unpatched) code of each client build. Add a version here once its
# exe is unpacked. SURVARIUM_EXE overrides.
EXES = {
    "021d": "E:/sur-res-pro/downloads/v021d/extracted/game/binaries/x86/survarium.exe",
    "026g": "E:/sur-res-pro/downloads/v026g/extracted/Survarium/game/binaries/x86/survarium.exe",
    "0100b": "E:/sur-res-pro/Survarium/binaries/x86/survarium.exe",
}


def exe_path(version):
    return os.environ.get("SURVARIUM_EXE") or EXES[version]


class Pe:
    def __init__(self, path):
        self.data = open(path, "rb").read()
        d = self.data
        pe = struct.unpack_from("<I", d, 0x3C)[0]
        count = struct.unpack_from("<H", d, pe + 6)[0]
        optional = struct.unpack_from("<H", d, pe + 20)[0]
        self.base = struct.unpack_from("<I", d, pe + 24 + 28)[0]
        self.sections = []
        for i in range(count):
            o = pe + 24 + optional + 40 * i
            name = d[o:o + 8].rstrip(b"\0").decode()
            vsize, rva, rsize, raw = struct.unpack_from("<IIII", d, o + 8)
            self.sections.append((name, self.base + rva, vsize, raw, rsize))

    def offset(self, va):
        for _, start, vsize, raw, rsize in self.sections:
            if start <= va < start + max(vsize, rsize):
                return va - start + raw
        raise ValueError(f"{va:#x} is in no section")

    def va(self, offset):
        for _, start, _, raw, rsize in self.sections:
            if raw <= offset < raw + rsize:
                return offset - raw + start
        return None

    def text(self):
        return next(s for s in self.sections if s[0] == ".text")


if __name__ == "__main__":
    path = exe_path(sys.argv[1])
    pe = Pe(path)
    print(path, f"image base {pe.base:#x}")
    for name, start, vsize, raw, rsize in pe.sections:
        print(f"{name:8} va {start:#x} size {vsize:#x} file {raw:#x} size {rsize:#x}")
