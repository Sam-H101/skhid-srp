"""callers.py VA... : direct call/jmp rel32 sites targeting each VA in the 0.26g launcher (-v 021 for 0.21d)."""
import os, struct, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lpe import Pe, L026, L021
a = sys.argv[1:]
path = L026
if a[0] == "-v":
    path = L021 if a[1] == "021" else L026; a = a[2:]
pe = Pe(path)
_, tva, tvs, tro, trs = pe.sec(".text")
t = pe.data[tro:tro + tvs]
for x in a:
    target = int(x, 16)
    out = []
    for i in range(len(t) - 5):
        if t[i] in (0xE8, 0xE9) and tva + i + 5 + struct.unpack_from("<i", t, i + 1)[0] == target:
            out.append("%X%s" % (tva + i, "j" if t[i] == 0xE9 else ""))
    # absolute references (push imm32 / mov imm32 / tables)
    needle = struct.pack("<I", target)
    absr = []
    p = 0
    while (p := pe.data.find(needle, p)) != -1:
        v = pe.va(p)
        absr.append("%X" % v if v else "file:%X" % p)
        p += 1
    print("%X calls=%s abs=%s" % (target, out, absr))
