"""relocs.py VA:LEN ... : say which bytes of each range are covered by a base relocation
(HIGHLOW) in the 0.26g launcher. A patch must not overwrite a relocated dword with
non-address bytes, and new absolute addresses need a relocation of their own."""
import os, struct, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lpe import Pe, L026
pe = Pe(L026); d = pe.data
_, rva, vs, ro, rs = pe.sec(".reloc")
rel = set(); o = ro
while o < ro + vs:
    page, size = struct.unpack_from("<II", d, o)
    if not size:
        break
    for i in range((size - 8) // 2):
        e = struct.unpack_from("<H", d, o + 8 + i * 2)[0]
        if e >> 12 == 3:
            rel.add(pe.base + page + (e & 0xFFF))
    o += size
print(len(rel), "HIGHLOW relocations")
for a in sys.argv[1:]:
    va, n = a.split(":"); va = int(va, 16); n = int(n, 0)
    hit = sorted(r for r in rel if r + 4 > va and r < va + n)
    print("%X..%X: %s" % (va, va + n - 1, "relocated dword(s) at " + ",".join("%X" % h for h in hit) if hit else "no relocation"))
