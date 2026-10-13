"""annotate.py [026|021] : write vNNN/survarium_launcher.annot.txt = the dumpbin
listing with (a) string literals, (b) import names, (c) names from
scripts/names_026g.txt (lines `VA name`), (d) `==== sub_VA` separators at every
direct call target / data-referenced code pointer."""
import os, re, struct, sys
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from lpe import Pe, L026, L021

ver = "021" if len(sys.argv) > 1 and sys.argv[1] == "021" else "026"
pe = Pe(L021 if ver == "021" else L026)
folder = os.path.join(HERE, "..", "v021d" if ver == "021" else "v026g")
d = pe.data
names = {}
nf = os.path.join(HERE, "names_%sg.txt" % ver if ver == "026" else "names_021d.txt")
if os.path.exists(nf):
    for line in open(nf, encoding="utf-8"):
        line = line.split("#")[0].strip()
        if line:
            a, n = line.split(None, 1)
            names[int(a, 16)] = n
# strings
strs = {}
for line in open(os.path.join(folder, "strings_xref.txt"), encoding="utf-8"):
    va = int(line[:8], 16)
    enc = line[9:12]
    s = line[line.index("] ") + 2:].rstrip("\n")
    strs[va] = ("L" if enc == "U16" else "") + s
_, tva, tvs, tro, trs = pe.sec(".text")
tend = tva + tvs
# function starts: call targets + code pointers in data sections
starts = set()
text = d[tro:tro + tvs]
for i in range(len(text) - 5):
    if text[i] == 0xE8:
        t = tva + i + 5 + struct.unpack_from("<i", text, i + 1)[0]
        if tva <= t < tend:
            starts.add(t)
dataptr = set()
for nm in (".rdata", ".data"):
    _, sva, vs, ro, rs = pe.sec(nm)
    blob = d[ro:ro + min(vs, rs)]
    for i in range(0, len(blob) - 3, 4):
        v = struct.unpack_from("<I", blob, i)[0]
        if tva <= v < tend:
            dataptr.add(v)
rx = re.compile(r"\b([0-9A-F]{5,8})h?\b")
out = []
src = os.path.join(folder, "survarium_launcher.disasm.txt")
for line in open(src, encoding="latin-1"):
    line = line.rstrip("\n")
    m = re.match(r"  ([0-9A-F]{8}): ((?:[0-9A-F]{2} )+) *(.*)", line)
    if not m:
        out.append(line)
        continue
    va = int(m.group(1), 16)
    if va in starts or va in dataptr:
        tag = "sub" if va in starts else "ptr"
        out.append("")
        out.append("==== %s_%X %s" % (tag, va, names.get(va, "")))
    ops = m.group(3)
    notes = []
    for mm in rx.finditer(ops):
        v = int(mm.group(1), 16)
        if v in pe.imports:
            notes.append(pe.imports[v])
        elif v in strs:
            notes.append(strs[v])
        elif v in names:
            notes.append(names[v])
        elif tva <= v < tend and not ops.startswith(("j", "call")) and (v in starts or v in dataptr):
            notes.append("code sub_%X" % v)
    out.append(line + ("    ; " + " | ".join(notes) if notes else ""))
open(os.path.join(folder, "survarium_launcher.annot.txt"), "w", encoding="utf-8").write("\n".join(out))
print(len(out), "lines;", len(starts), "call targets;", len(dataptr), "data code ptrs")
