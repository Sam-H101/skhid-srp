"""strings_xref.py [026|021] : list every ASCII and UTF-16 string in .rdata/.data
with VA, encoding, length, zero bytes after it, and every 4-byte reference to
its address found in .text (VA of the dword)."""
import re, struct, sys
import os; sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lpe import Pe, L026, L021

pe = Pe(L021 if len(sys.argv) > 1 and sys.argv[1] == "021" else L026)
d = pe.data
tname, tva, tvs, tro, trs = pe.sec(".text")
text = d[tro:tro + tvs]
refs = {}
for m in re.finditer(rb"(?s)....", b""):
    pass
# index all dwords in .text that fall in image range
lo, hi = pe.base, pe.base + 0x500000
for i in range(len(text) - 3):
    v = struct.unpack_from("<I", text, i)[0]
    if lo <= v < hi:
        refs.setdefault(v, []).append(tva + i)
out = []
for name in (".rdata", ".data"):
    _, sva, vs, ro, rs = pe.sec(name)
    blob = d[ro:ro + min(vs, rs)]
    for m in re.finditer(rb"(?:[\x20-\x7e\t\r\n]\x00){3,}\x00\x00", blob):
        if m.start() % 2:
            pass
        s = m.group()[:-2].decode("utf-16-le")
        end = m.end() - 2
        z = 0
        while end + z < len(blob) and blob[end + z] == 0:
            z += 1
        out.append((sva + m.start(), "U16", len(s), z, s))
    for m in re.finditer(rb"[\x20-\x7e\t\r\n]{4,}\x00", blob):
        if m.start() and blob[m.start() - 1:m.start() + 1] and m.start() >= 1 and blob[m.start() + 1] == 0:
            continue
        s = m.group()[:-1].decode("latin-1")
        end = m.end() - 1
        z = 0
        while end + z < len(blob) and blob[end + z] == 0:
            z += 1
        out.append((sva + m.start(), "ASC", len(s), z, s))
out.sort()
for va, enc, n, z, s in out:
    r = refs.get(va, [])
    print("%08X %s len=%d zeros=%d refs=[%s] %r" % (va, enc, n, z, ",".join("%X" % x for x in r), s))
