"""upd_fields.py OFFSET_HEX... : every instruction of 0.26g survarium_updater.exe that
touches updater member this+OFFSET (as `+00XXXXXXh]`), with the nearest preceding
__FUNCTION__ string as a hint of the method."""
import os
import re
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from upd_lib import Listing  # noqa: E402

L = Listing("026")
T, A = L.text, L.addrs
funcs = {va: s for va, enc, s in L.strings() if "survarium_updater::" in s}


def fn_hint(i):
    for k in range(i, max(i - 500, 0), -1):
        for m in re.finditer(r"\b([0-9A-F]{6})h\b", T[k]):
            v = int(m.group(1), 16)
            if v in funcs:
                return re.sub(r"^.*updater::(\w+)\(.*$", r"\1", funcs[v])
    return None


for off in sys.argv[1:]:
    pat = "+%08Xh]" % int(off, 16)
    print("== this+%s" % off)
    for i, t in enumerate(T):
        if pat in t:
            print("  %08X %-58s %s" % (A[i], t, fn_hint(i)))
