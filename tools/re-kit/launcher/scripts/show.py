"""show.py [-v 021] START END : compact view of the annotated listing between two
VAs (hex): no opcode bytes, `VA mnemonic operands ; note`. Reads
vNNN/survarium_launcher.annot.txt (run annotate.py first)."""
import os, re, sys
HERE = os.path.dirname(os.path.abspath(__file__))
args = sys.argv[1:]
ver = "v026g"
if args[0] == "-v":
    ver = "v021d" if args[1] == "021" else "v026g"
    args = args[2:]
lo, hi = int(args[0], 16), int(args[1], 16)
rx = re.compile(r"  ([0-9A-F]{8}): ((?:[0-9A-F]{2} )+) *(.*)")
on = False
for line in open(os.path.join(HERE, "..", ver, "survarium_launcher.annot.txt"), encoding="utf-8"):
    line = line.rstrip("\n")
    m = rx.match(line)
    if m:
        va = int(m.group(1), 16)
        on = lo <= va < hi
        if on and m.group(3):
            t = re.sub(r"\s+", " ", m.group(3))
            t = t.replace("dword ptr ", "d ").replace("word ptr ", "w ").replace("byte ptr ", "b ")
            print("%X %s" % (va, t))
    elif on and line.startswith("===="):
        print(line)
