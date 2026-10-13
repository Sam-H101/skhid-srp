"""lpe.py : tiny PE helper for the launcher notes (stdlib only, read-only).
Usage as a module: from lpe import Pe ; pe = Pe(path)"""
import struct

L026 = r"E:\sur-res-pro\downloads\v026g\extracted\Survarium\game\binaries\x86\survarium_launcher.exe"
L021 = r"E:\sur-res-pro\downloads\v021d\extracted\game\binaries\x86\survarium_launcher.exe"


class Pe:
    def __init__(self, path):
        self.data = d = open(path, "rb").read()
        pe = struct.unpack_from("<I", d, 0x3C)[0]
        n = struct.unpack_from("<H", d, pe + 6)[0]
        osz = struct.unpack_from("<H", d, pe + 20)[0]
        self.base = struct.unpack_from("<I", d, pe + 24 + 28)[0]
        self.secs = []
        o = pe + 24 + osz
        for i in range(n):
            name, vs, va, rs, ro = struct.unpack_from("<8sIIII", d, o + i * 40)
            self.secs.append((name.rstrip(b"\0").decode(), self.base + va, vs, ro, rs))
        self.imports = {}
        imp_rva = struct.unpack_from("<I", d, pe + 24 + 96 + 8)[0]
        if imp_rva:
            o = self.off(self.base + imp_rva)
            while True:
                oft, _, _, name_rva, ft = struct.unpack_from("<IIIII", d, o)
                if not name_rva:
                    break
                dll = self.cstr(self.base + name_rva)
                i = 0
                while True:
                    t = struct.unpack_from("<I", d, self.off(self.base + (oft or ft)) + 4 * i)[0]
                    if not t:
                        break
                    if t & 0x80000000:
                        nm = "%s!#%d" % (dll, t & 0xFFFF)
                    else:
                        nm = "%s!%s" % (dll, self.cstr(self.base + t + 2))
                    self.imports[self.base + ft + 4 * i] = nm
                    i += 1
                o += 20

    def off(self, va):
        for name, sva, vs, ro, rs in self.secs:
            if sva <= va < sva + max(vs, rs):
                return ro + va - sva
        return None

    def va(self, off):
        for name, sva, vs, ro, rs in self.secs:
            if ro <= off < ro + rs:
                return sva + off - ro
        return None

    def sec(self, name):
        for s in self.secs:
            if s[0] == name:
                return s
    def cstr(self, va):
        o = self.off(va)
        e = self.data.index(b"\0", o)
        return self.data[o:e].decode("latin-1")

    def wstr(self, va):
        o = self.off(va)
        e = o
        while self.data[e:e + 2] != b"\0\0":
            e += 2
        return self.data[o:e].decode("utf-16-le", "replace")

    def rd(self, va, n):
        o = self.off(va)
        return self.data[o:o + n]
