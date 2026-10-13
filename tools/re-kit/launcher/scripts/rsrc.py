"""rsrc.py [026|021] [strings|list] : walk the launcher's .rsrc tree.
list    -> every resource: type, name/id, language, RVA-based VA, file offset, size
strings -> every RT_STRING entry per language: id (decimal and hex) and text"""
import os, struct, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lpe import Pe, L026, L021
a = sys.argv[1:]
path = L026
if a and a[0] in ("026", "021"):
    path = L021 if a[0] == "021" else L026
    a = a[1:]
mode = a[0] if a else "list"
pe = Pe(path)
d = pe.data
_, rva, vs, ro, rs = pe.sec(".rsrc")
TYPES = {1: "CURSOR", 2: "BITMAP", 3: "ICON", 4: "MENU", 5: "DIALOG", 6: "STRING", 9: "ACCEL", 10: "RCDATA",
         12: "GROUP_CURSOR", 14: "GROUP_ICON", 16: "VERSION", 24: "MANIFEST"}

def name(v):
    if v & 0x80000000:
        o = ro + (v & 0x7FFFFFFF)
        n = struct.unpack_from("<H", d, o)[0]
        return d[o + 2:o + 2 + n * 2].decode("utf-16-le")
    return v

def walk(off, pathv):
    nn, ni = struct.unpack_from("<HH", d, ro + off + 12)
    for i in range(nn + ni):
        nm, o = struct.unpack_from("<II", d, ro + off + 16 + i * 8)
        p = pathv + [name(nm)]
        if o & 0x80000000:
            yield from walk(o & 0x7FFFFFFF, p)
        else:
            drva, size, cp, _ = struct.unpack_from("<IIII", d, ro + o)
            yield p, pe.base + drva, size

res = list(walk(0, []))
if mode == "list":
    for p, va, size in res:
        t = TYPES.get(p[0], p[0])
        print("%-12s %-8s lang=%04X va=%08X file=%08X size=%d" % (t, p[1], p[2], va, pe.off(va), size))
else:
    by = {}
    for p, va, size in res:
        if p[0] != 6:
            continue
        o = pe.off(va)
        for i in range(16):
            n = struct.unpack_from("<H", d, o)[0]
            o += 2
            if n:
                by.setdefault(p[2], {})[(p[1] - 1) * 16 + i] = d[o:o + n * 2].decode("utf-16-le")
            o += n * 2
    want = [int(x, 16) for x in a[1:]] or sorted(by)
    for lang in want:
        print("=== language %04X" % lang)
        for sid in sorted(by[lang]):
            print("%d (0x%X): %r" % (sid, sid, by[lang][sid]))
