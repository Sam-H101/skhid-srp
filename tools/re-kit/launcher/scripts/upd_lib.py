"""upd_lib.py : helpers for static reading of survarium_updater.exe (stdlib only,
read-only on the game files).

  python -I upd_lib.py [026|021] strs REGEX      strings matching REGEX with the VA of every
                                                 4-byte reference in .text and the function start
  python -I upd_lib.py [026|021] fn VA [MAXLINES] annotated dumpbin listing of the function that
                                                 contains VA (strings, imports, labels resolved)
  python -I upd_lib.py [026|021] range VA1 VA2   annotated listing between two addresses
  python -I upd_lib.py [026|021] crange VA1 VA2  same, logging boilerplate folded into LOG lines
  python -I upd_lib.py [026|021] cfn VA          same for the function containing VA
  python -I upd_lib.py [026|021] callers VA      direct call/jmp sites of VA and data references
  python -I upd_lib.py [026|021] label NAME      (0.21d) address of labels containing NAME

Function starts are recovered from direct call targets, pointers found in .rdata/.data
and (0.21d) dumpbin labels; they are heuristics, not ground truth.
"""
import bisect
import os
import pickle
import re
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from lpe import Pe  # noqa: E402
try:
    from upd_names026 import NAMES
except ImportError:
    NAMES = {}

EXE = {
    "026": r"E:\sur-res-pro\downloads\v026g\extracted\Survarium\game\binaries\x86\survarium_updater.exe",
    "021": r"E:\sur-res-pro\downloads\v021d\extracted\game\binaries\x86\survarium_updater.exe",
}
LST = {
    "026": os.path.join(HERE, "..", "v026g", "survarium_updater.disasm.txt"),
    "021": os.path.join(HERE, "..", "v021d", "survarium_updater.disasm.txt"),
}
CACHE = os.environ.get("UPD_CACHE", os.path.join(os.environ.get("TEMP", "."), "upd_cache"))

LINE = re.compile(r"^  ([0-9A-F]{8}): ((?:[0-9A-F]{2} )+) *(.*)$")
CONT = re.compile(r"^            ((?:[0-9A-F]{2} ?)+)$")


class Listing:
    def __init__(self, ver):
        self.ver = ver
        self.pe = Pe(EXE[ver])
        os.makedirs(CACHE, exist_ok=True)
        cp = os.path.join(CACHE, "lst_%s.pickle" % ver)
        if os.path.exists(cp):
            self.addrs, self.text, self.labels = pickle.load(open(cp, "rb"))
        else:
            self.addrs, self.text, self.labels = [], [], {}
            pending = None
            for raw in open(LST[ver], encoding="latin-1"):
                raw = raw.rstrip("\n")
                m = LINE.match(raw)
                if m:
                    a = int(m.group(1), 16)
                    self.addrs.append(a)
                    self.text.append(m.group(3))
                    if pending:
                        self.labels[a] = pending
                        pending = None
                elif raw.endswith(":") and not raw.startswith(" "):
                    pending = raw[:-1]
            pickle.dump((self.addrs, self.text, self.labels), open(cp, "wb"))
        self.lab_addrs = sorted(self.labels)
        self._starts = None
        self._strs = None

    # ---- function starts
    def starts(self):
        if self._starts is None:
            cp = os.path.join(CACHE, "starts_%s.pickle" % self.ver)
            if os.path.exists(cp):
                self._starts = pickle.load(open(cp, "rb"))
            else:
                s = set(self.labels)
                _, tva, tvs, tro, trs = self.pe.sec(".text")
                for t in self.text:
                    if t.startswith("call "):
                        m = re.match(r"call\s+([0-9A-F]{8})$", t)
                        if m:
                            s.add(int(m.group(1), 16))
                d = self.pe.data
                for name in (".rdata", ".data"):
                    sec = self.pe.sec(name)
                    if not sec:
                        continue
                    _, sva, vs, ro, rs = sec
                    for o in range(ro, ro + min(vs, rs) - 3, 4):
                        v = struct.unpack_from("<I", d, o)[0]
                        if tva <= v < tva + tvs:
                            s.add(v)
                aset = set(self.addrs)
                self._starts = sorted(x for x in s if x in aset)
                pickle.dump(self._starts, open(cp, "wb"))
        return self._starts

    def fn_start(self, va):
        st = self.starts()
        i = bisect.bisect_right(st, va) - 1
        return st[i] if i >= 0 else None

    def fn_end(self, va):
        st = self.starts()
        i = bisect.bisect_right(st, va)
        return st[i] if i < len(st) else self.addrs[-1] + 1

    # ---- annotations
    def cstr(self, va, limit=120):
        o = self.pe.off(va)
        if o is None:
            return None
        d = self.pe.data
        for name, sva, vs, ro, rs in self.pe.secs:
            if sva <= va < sva + max(vs, rs) and name not in (".rdata", ".data"):
                return None
        e = o
        while e < len(d) and 0x20 <= d[e] < 0x7F or d[e] in (9, 10, 13):
            e += 1
        if e - o >= 3 and d[e] == 0:
            return '"%s"' % d[o:e][:limit].decode("latin-1").replace("\n", "\\n").replace("\r", "\\r")
        e = o
        while e + 1 < len(d) and (0x20 <= d[e] < 0x7F or d[e] in (9, 10, 13)) and d[e + 1] == 0:
            e += 2
        if e - o >= 6 and d[e:e + 2] == b"\0\0":
            return 'L"%s"' % d[o:e][:2 * limit].decode("utf-16-le").replace("\n", "\\n")
        return None

    def annotate(self, t):
        notes = []
        if self.ver == "026":
            m = re.match(r"(?:call|jmp)\s+([0-9A-F]{8})$", t)
            if m and int(m.group(1), 16) in NAMES:
                notes.append(NAMES[int(m.group(1), 16)])
        for m in re.finditer(r"\b([0-9A-F]{5,8})h?\b", t):
            v = int(m.group(1), 16)
            if v in self.pe.imports:
                notes.append(self.pe.imports[v])
                continue
            if v in self.labels:
                notes.append(self.labels[v][:110])
                continue
            if self.pe.base + 0x1000 <= v < self.pe.base + 0x400000:
                s = self.cstr(v)
                if s:
                    notes.append(s)
        return ("   ; " + " | ".join(notes)) if notes else ""

    def dump(self, a, b, maxlines=100000):
        i = bisect.bisect_left(self.addrs, a)
        n = 0
        while i < len(self.addrs) and self.addrs[i] < b and n < maxlines:
            x = self.addrs[i]
            if x in self.labels:
                print("%s:" % self.labels[x][:160])
            print("%08X %s%s" % (x, self.text[i], self.annotate(self.text[i])))
            i += 1
            n += 1

    def cdump(self, a, b):
        """Like dump, but folds the logging boilerplate of the 0.26g updater
        (check channel 000B92D0, build record 00013CDB, emit 000B9830, destroy 00013ADC)
        into one LOG line. Lines of the span that carry real work are kept."""
        i = bisect.bisect_left(self.addrs, a)
        n = len(self.addrs)
        while i < n and self.addrs[i] < b:
            t = self.text[i]
            if t.startswith("mov         eax,dword ptr ds:[001249F0h]"):
                j = i
                while j < n and j < i + 60 and not self.text[j].startswith("call        000B9830"):
                    j += 1
                if j < n and self.text[j].startswith("call        000B9830"):
                    strs, keep, nums = [], [], []
                    boiler = re.compile(
                        r"(mov\s+eax,dword ptr ds:\[001249F0h\]|test\s+eax,eax|test\s+al,al|j[a-z]+\s+[0-9A-F]{8}"
                        r"|push\s+e[a-ds][ixp]$|push\s+[0-9A-F]+h?$|call\s+000B92D0|call\s+00013CDB"
                        r"|add\s+esp,0Ch|push\s+dword ptr ds:\[001249F4h\]|lea\s+esi,\[e[bs]p[^\]]*\]"
                        r"|or\s+dword ptr \[e[bs]p[^\]]*\],[0-9A-F]+h?|mov\s+dword ptr \[e[bs]p[^\]]*\],[0-9A-F]+h?$"
                        r"|mov\s+eax,esi|mov\s+e[bd]i,(100B5Ch|101BDCh|124E28h))")
                    for k in range(i, j):
                        tk = self.text[k]
                        an = self.annotate(tk)
                        if an and tk.startswith(("push", "mov")) and "[" not in tk:
                            strs.append(an[5:])
                        elif re.match(r"push\s+[0-9A-F]+h?$", tk):
                            nums.append(tk.split()[-1])
                        elif not boiler.match(tk):
                            keep.append("%08X %s%s" % (self.addrs[k], tk, an))
                    for kline in keep:
                        print("      | " + kline)
                    print("%08X LOG %s   nums=%s" % (self.addrs[i], " ; ".join(strs), ",".join(nums)))
                    i = j + 1
                    # trailer
                    for k in range(i, min(i + 9, n)):
                        if self.text[k].startswith("call        00013ADC"):
                            ok = all(re.match(r"(add\s+esp|test\s+byte ptr|je|and\s+dword ptr|lea\s+esi|or\s+dword ptr|mov\s+e[sd]i,dword ptr \[e[sb]p)", self.text[m]) for m in range(i, k))
                            if ok:
                                i = k + 1
                            break
                    continue
            x = self.addrs[i]
            if x in self.labels:
                print("%s:" % self.labels[x][:160])
            print("%08X %s%s" % (x, t, self.annotate(t)))
            i += 1

    def skel(self, a, b):
        """Skeleton of a range: LOG lines, named calls, imports, strings, field tests,
        set_state arguments. For orientation only; read the full listing before claiming."""
        import io, contextlib
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            self.cdump(a, b)
        lines = buf.getvalue().splitlines()
        for n, ln in enumerate(lines):
            body = ln[9:] if not ln.startswith("      |") else ln[17:]
            keep = False
            if " LOG " in ln[:14] or "   ; " in ln:
                keep = True
            elif re.search(r"\+0020[0-9A-F]{4}h\]|\+000013C8h\]|\+0000030[0-9A-F]h\]", ln):
                keep = True
            elif body.startswith(("ret", "cmp", "pop         edx", "push        ebp")):
                keep = True
            elif re.match(r"j[a-z]+\s", body):
                keep = True
            elif body.startswith("call"):
                keep = True
            elif n + 1 < len(lines) and "0002305E" in lines[n + 1]:
                keep = True
            elif n + 2 < len(lines) and "0002305E" in lines[n + 2] and body.startswith(("push", "mov")):
                keep = True
            if keep:
                print(ln)

    # ---- strings
    def strings(self):
        if self._strs is None:
            out = []
            d = self.pe.data
            for name in (".rdata", ".data"):
                sec = self.pe.sec(name)
                if not sec:
                    continue
                _, sva, vs, ro, rs = sec
                blob = d[ro:ro + min(vs, rs)]
                for m in re.finditer(rb"(?:[\x20-\x7e\t\r\n]\x00){3,}\x00\x00", blob):
                    out.append((sva + m.start(), "U", m.group()[:-2].decode("utf-16-le")))
                for m in re.finditer(rb"[\x20-\x7e\t\r\n]{4,}\x00", blob):
                    out.append((sva + m.start(), "A", m.group()[:-1].decode("latin-1")))
            out.sort()
            self._strs = out
        return self._strs

    def refs(self, va):
        """VAs in .text where the dword `va` appears."""
        _, tva, tvs, tro, trs = self.pe.sec(".text")
        d = self.pe.data
        needle = struct.pack("<I", va)
        res, at = [], tro
        while True:
            at = d.find(needle, at, tro + tvs)
            if at < 0:
                break
            res.append(tva + at - tro)
            at += 1
        return res

    def insn_at(self, va):
        i = bisect.bisect_right(self.addrs, va) - 1
        return self.addrs[i]

    def callers(self, target):
        pat = "%08X" % target
        res = []
        for a, t in zip(self.addrs, self.text):
            if t.endswith(pat) and (t.startswith("call") or t.startswith("j")):
                res.append((a, t))
        return res


def main():
    args = sys.argv[1:]
    ver = "026"
    if args and args[0] in ("026", "021"):
        ver = args.pop(0)
    L = Listing(ver)
    cmd = args.pop(0)
    if cmd == "strs":
        rx = re.compile(args[0])
        for va, enc, s in L.strings():
            if rx.search(s):
                rs = L.refs(va)
                print("%08X %s %r" % (va, enc, s[:200]))
                for r in rs:
                    ia = L.insn_at(r)
                    print("    ref %08X  fn %08X %s" % (ia, L.fn_start(ia) or 0,
                                                       L.labels.get(L.fn_start(ia), "")[:100]))
    elif cmd == "fn":
        va = int(args[0], 16)
        a, b = L.fn_start(va), L.fn_end(va)
        print("; function %08X .. %08X" % (a, b))
        L.dump(a, b, int(args[1]) if len(args) > 1 else 100000)
    elif cmd == "range":
        L.dump(int(args[0], 16), int(args[1], 16))
    elif cmd == "crange":
        L.cdump(int(args[0], 16), int(args[1], 16))
    elif cmd == "skel":
        L.skel(int(args[0], 16), int(args[1], 16))
    elif cmd == "cfn":
        va = int(args[0], 16)
        a, b = L.fn_start(va), L.fn_end(va)
        print("; function %08X .. %08X" % (a, b))
        L.cdump(a, b)
    elif cmd == "callers":
        for x in args:
            t = int(x, 16)
            print("target %08X" % t)
            for a, tx in L.callers(t):
                print("    %08X %-40s fn %08X %s" % (a, tx, L.fn_start(a) or 0,
                                                    L.labels.get(L.fn_start(a), "")[:90]))
            for r in L.refs(t):
                print("    dataref-in-text %08X fn %08X" % (r, L.fn_start(L.insn_at(r)) or 0))
            d = L.pe.data
            needle = struct.pack("<I", t)
            for name in (".rdata", ".data"):
                sec = L.pe.sec(name)
                if not sec:
                    continue
                _, sva, vs, ro, rs = sec
                at = ro
                while True:
                    at = d.find(needle, at, ro + min(vs, rs))
                    if at < 0:
                        break
                    print("    ptr in %s at %08X" % (name, sva + at - ro))
                    at += 1
    elif cmd == "label":
        for a in L.lab_addrs:
            if args[0] in L.labels[a]:
                print("%08X %s" % (a, L.labels[a][:200]))


if __name__ == "__main__":
    main()
