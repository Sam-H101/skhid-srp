"""upd_msgids.py [026|021] : every place the updater builds a pipe packet.
Lists each call of the packet-send routine (0.26g: 0001A3B1) with the function
start, and every `mov byte ptr [..],imm8` that directly precedes a call of the
byte-append routine (0.26g: 0001A1A7) = candidate message id / byte field."""
import os
import re
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from upd_lib import Listing  # noqa: E402

ver = sys.argv[1] if len(sys.argv) > 1 else "026"
SEND = sys.argv[2] if len(sys.argv) > 2 else "0001A3B1"
APPEND = sys.argv[3] if len(sys.argv) > 3 else "0001A1A7"
L = Listing(ver)
T, A = L.text, L.addrs
for i, t in enumerate(T):
    if t.startswith("call") and t.endswith(SEND):
        ids = []
        j = i - 1
        # walk back to the packet initialisation (capacity 1000h)
        while j > i - 120 and not re.search(r"\],1000h$", T[j]):
            j -= 1
        for k in range(j, i):
            if T[k].startswith("call") and T[k].endswith(APPEND):
                for m in range(k - 1, max(k - 8, j - 12), -1):
                    mm = re.match(r"mov\s+byte ptr \[[^\]]+\],([0-9A-F]+)h?$", T[m])
                    if mm:
                        ids.append("%s@%08X" % (mm.group(1), A[m]))
                        break
                else:
                    ids.append("var@%08X" % A[k])
            elif T[k].startswith("call") and not T[k].endswith(("000D93E0",)):
                ids.append("call %s@%08X" % (T[k].split()[-1], A[k]))
        print("send at %08X fn %08X %s : %s" % (A[i], L.fn_start(A[i]), L.labels.get(L.fn_start(A[i]), "")[:70], " ".join(ids)))
