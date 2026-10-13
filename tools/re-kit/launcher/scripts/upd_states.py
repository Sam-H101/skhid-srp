"""upd_states.py : every updater::set_state call (0.26g 0002305E) with the state value
loaded into edx, the last log text before it and the __FUNCTION__ strings around it.
Also every direct write/compare of m_state (this+00201628h) and m_target_state (+0020162Ch)."""
import os
import re
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from upd_lib import Listing  # noqa: E402

L = Listing("026")
T, A = L.text, L.addrs
funcs = {va: s for va, enc, s in L.strings() if "survarium_updater::" in s}
logs = {va: s for va, enc, s in L.strings() if enc == "A"}


def ctx(i):
    fn = msg = None
    for k in range(i, max(i - 400, 0), -1):
        for m in re.finditer(r"\b([0-9A-F]{6})h\b", T[k]):
            v = int(m.group(1), 16)
            if v in funcs and fn is None:
                fn = re.sub(r"^.*updater::(\w+)\(.*$", r"\1", funcs[v])
            elif v in logs and msg is None and v not in funcs and "updater" not in logs[v][:20]:
                msg = logs[v]
        if fn:
            break
    return fn, msg


for i, t in enumerate(T):
    if t.startswith("call") and t.endswith("0002305E"):
        val = "?"
        for k in range(i - 1, i - 8, -1):
            if T[k].startswith("pop         edx"):
                m = re.match(r"push\s+([0-9A-F]+)h?$", T[k - 1])
                val = m.group(1) if m else "? (%s)" % T[k - 1]
                break
            m = re.match(r"mov\s+edx,(.+)$", T[k])
            if m:
                val = m.group(1)
                break
            if T[k].startswith("xor         edx,edx"):
                val = "0" + (" +inc" if any(T[j].startswith("inc         edx") for j in range(k, i)) else "")
                break
        fn, msg = ctx(i)
        print("%08X set_state(%s)  after-fn-string=%s  last-log=%r" % (A[i], val, fn, msg))
print()
for i, t in enumerate(T):
    if re.search(r"\+0020162[8C]h\]", t) and not t.startswith("push"):
        fn, msg = ctx(i)
        print("%08X %-55s %s" % (A[i], t, fn))
