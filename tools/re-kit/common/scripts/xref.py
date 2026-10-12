"""xref.py VERSION TEXT : where that build's code uses a string. Finds the
NUL-terminated ASCII string, then every 4-byte reference to its address (push
imm32, mov imm32, pointer tables), each with its function when the version
has a functions.txt. Strings are how functions are matched across builds."""
import re
import struct
import sys

from exes import Pe, exe_path
from fn import Functions

version, text = sys.argv[1], sys.argv[2].encode()
pe = Pe(exe_path(version))
functions = Functions.load(version)
d = pe.data
for m in re.finditer(re.escape(text) + b"\0", d):
    if m.start() and d[m.start() - 1] != 0:
        continue  # the tail of a longer string
    va = pe.va(m.start())
    if va is None:
        continue
    print(f"string at {va:#x}")
    needle, at = struct.pack("<I", va), 0
    while (at := d.find(needle, at)) != -1:
        ref = pe.va(at)
        where = functions.at(ref) if functions and ref else ""
        print(f"  ref {ref:#x} {where}" if ref else f"  ref at file {at:#x}")
        at += 1
