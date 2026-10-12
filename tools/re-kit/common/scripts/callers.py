"""callers.py VERSION VA... : every direct `call VA` (E8 rel32) in that build's
.text. Indirect calls (vtables, pointers) are not found."""
import struct
import sys

from exes import Pe, exe_path

pe = Pe(exe_path(sys.argv[1]))
_, text_va, _, raw, size = pe.text()
d = pe.data
for target in (int(a, 0) for a in sys.argv[2:]):
    sites = []
    for o in range(raw, raw + size - 5):
        if d[o] == 0xE8:
            va = o - raw + text_va
            if va + 5 + struct.unpack_from("<i", d, o + 1)[0] == target:
                sites.append(hex(va))
    print(hex(target), sites)
