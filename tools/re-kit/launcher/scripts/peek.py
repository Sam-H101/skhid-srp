"""peek.py VA [LEN] [-v 021] : hex + ASCII/UTF-16 view of bytes at a VA of the launcher exe."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lpe import Pe, L026, L021
a = [x for x in sys.argv[1:]]
path = L026
if "-v" in a:
    i = a.index("-v"); path = L021 if a[i + 1] == "021" else L026; del a[i:i + 2]
pe = Pe(path)
va = int(a[0], 16); n = int(a[1], 0) if len(a) > 1 else 64
b = pe.rd(va, n)
print("file offset %#x" % pe.off(va))
for i in range(0, len(b), 16):
    c = b[i:i + 16]
    print("%08X  %-48s %s" % (va + i, c.hex(" "), "".join(chr(x) if 32 <= x < 127 else "." for x in c)))
