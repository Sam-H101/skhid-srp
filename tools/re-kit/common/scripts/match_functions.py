"""match_functions.py : name the functions of a build without a PDB from a
build that has one.

    python match_functions.py [--ref 021d] [--target 026g] [--out PATH]
                              [--ref-exe PATH] [--target-exe PATH] [--stats]

Writes ../../v{target}/functions_matched.txt as `address length name  # evidence`
(the evidence names the confidence, the method and the reference address).
Only stdlib; x86-32 lengths come from the small decoder below.

How it works
1. Both exes are loaded with their relocations, imports and RTTI (type
   descriptors -> complete object locators -> vtables).
2. Reference functions come from ../../v{ref}/functions.txt (PDB). Target
   function starts are found: call targets, pointers to .text from data
   (vtables, callbacks), code pointers in instructions, then tail-jump
   targets outside the current function, the ends of int3 padding; weak
   starts that the code before them runs across are dropped.
3. Every function is decoded into instructions, giving
   - a normalized byte hash (rel32 operands and relocated dwords masked),
   - a shape sequence (prefixes, opcode, modrm; no displacements/immediates),
   - tokens: referenced strings (ASCII/UTF-16), imports, vtables, RTTI
     type descriptors, and later matched callees/globals.
4. Passes, repeated until nothing new:
   a. exact: normalized bytes unique in both builds;
   b. shape: shape sequence unique in both builds (>= 10 instructions);
   c. tokens: reciprocal best weighted-token match (rare tokens weigh most);
   d. aligned propagation: in every matched pair the instruction sequences
      are aligned (difflib); calls and relocated pointers in aligned
      instructions pair callees and globals;
   e. vtables: same class, same slot count -> slot i <-> slot i;
   f. gaps: between two matched neighbours with the same number of
      functions in between, in address order.
   Each candidate from c-f must pass structure(): shape similarity, or (for
   recompiled code: 0.26g omits frame pointers and inlines differently) the
   cosine of the opcode histograms plus the instruction-count ratio. The
   mapping stays one-to-one.
Confidence: high = exact bytes (>= 6 insns) / exact shape (>= 30 insns), or a
strong signal with structure 'high' (shape >= 0.8, or histogram >= 0.9 and
size x0.75-1.33); medium = one signal with structure 'medium' (shape >= 0.6,
or histogram >= 0.75 and size x0.5-2). Weaker candidates are not written.
--holdout hides a share of the exact (or >= 2-unique-string) matches from
their pass and reports how the other passes map them (an error estimate).
The 0.26g starts and the reference exe path are build-specific: see
ORIGINAL_EXES (the srp-patched 0.21d copy in exes.py would skew the hashes).
"""
import argparse
import bisect
import collections
import difflib
import hashlib
import math
import os
import re
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
KIT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, HERE)
from exes import EXES  # noqa: E402

# The 0.21d exe in EXES is the srp-patched copy; matching wants the original.
ORIGINAL_EXES = {
    "021d": "E:/sur-res-pro/downloads/v021d/extracted/game/binaries/x86/survarium.exe",
}

# ---------------------------------------------------------------- x86 lengths
# Immediate kinds: 1 imm8, 2 imm16, 3 imm16/32 (operand size), 4 far ptr,
# 5 moffs, 6 enter, 7 F6 group (imm8 for /0 /1), 8 F7 group (immz for /0 /1).
M1, I1, M2, I2 = [0] * 256, [0] * 256, [0] * 256, [0] * 256


def _tables():
    for b in range(0x40):
        if b in (0x26, 0x2E, 0x36, 0x3E, 0x0F):
            continue
        lo = b & 7
        if lo < 4:
            M1[b] = 1
        elif lo == 4:
            I1[b] = 1
        elif lo == 5:
            I1[b] = 3
    for b, m, i in [(0x62, 1, 0), (0x63, 1, 0), (0x68, 0, 3), (0x69, 1, 3), (0x6A, 0, 1),
                    (0x6B, 1, 1), (0x80, 1, 1), (0x81, 1, 3), (0x82, 1, 1), (0x83, 1, 1),
                    (0x9A, 0, 4), (0xA8, 0, 1), (0xA9, 0, 3), (0xC0, 1, 1), (0xC1, 1, 1),
                    (0xC2, 0, 2), (0xC4, 1, 0), (0xC5, 1, 0), (0xC6, 1, 1), (0xC7, 1, 3),
                    (0xC8, 0, 6), (0xCA, 0, 2), (0xCD, 0, 1), (0xD4, 0, 1), (0xD5, 0, 1),
                    (0xE8, 0, 3), (0xE9, 0, 3), (0xEA, 0, 4), (0xEB, 0, 1), (0xF6, 1, 7),
                    (0xF7, 1, 8), (0xFE, 1, 0), (0xFF, 1, 0)]:
        M1[b], I1[b] = m, i
    for b in range(0x70, 0x80):
        I1[b] = 1
    for b in range(0x84, 0x90):
        M1[b] = 1
    for b in range(0xA0, 0xA4):
        I1[b] = 5
    for b in range(0xB0, 0xB8):
        I1[b] = 1
    for b in range(0xB8, 0xC0):
        I1[b] = 3
    for b in list(range(0xD0, 0xD4)) + list(range(0xD8, 0xE0)):
        M1[b] = 1
    for b in range(0xE0, 0xE8):
        I1[b] = 1
    # 0F xx
    for b in [0, 1, 2, 3, 0x0D] + list(range(0x10, 0x24)) + list(range(0x28, 0x30)) \
            + list(range(0x40, 0x70)) + [0x74, 0x75, 0x76] + list(range(0x78, 0x80)) \
            + list(range(0x90, 0xA0)) + [0xA3, 0xA5, 0xAB, 0xAD, 0xAE, 0xAF] \
            + list(range(0xB0, 0xC0)) + [0xC0, 0xC1, 0xC3, 0xC7] + list(range(0xD0, 0x100)):
        M2[b] = 1
    for b in [0x0F, 0x70, 0x71, 0x72, 0x73, 0xA4, 0xAC, 0xBA, 0xC2, 0xC4, 0xC5, 0xC6]:
        M2[b], I2[b] = 1, 1
    for b in range(0x80, 0x90):
        I2[b] = 3


_tables()
PREFIX = {0x66: 1, 0x67: 2, 0xF2: 4, 0xF3: 8, 0xF0: 16, 0x2E: 32, 0x36: 32, 0x3E: 32,
          0x26: 32, 0x64: 64, 0x65: 64}

# Instruction kinds
K_PLAIN, K_CALL, K_JMP, K_JCC, K_RET, K_JMP8, K_JCC8, K_IJMP, K_ICALL = range(9)


def decode(d, p):
    """-> (length, shape, kind, rel_target_file_offset or -1, rel32 offset in insn or -1)."""
    start = p
    pre = 0
    while True:
        b = d[p]
        f = PREFIX.get(b)
        if f is None:
            break
        pre |= f
        p += 1
        if p - start > 4:
            return 1, 0xFFFFFF, K_PLAIN, -1, -1
    op = b
    p += 1
    two = False
    if op == 0x0F:
        two = True
        op2 = d[p]
        p += 1
        if op2 == 0x38 or op2 == 0x3A:
            op3 = d[p]
            p += 1
            has_m, imm = 1, (1 if op2 == 0x3A else 0)
            opid = 0x10000 | (op2 << 8) | op3
        else:
            has_m, imm = M2[op2], I2[op2]
            opid = 0x100 | op2
    else:
        has_m, imm = M1[op], I1[op]
        opid = op
    modrm = 0
    if has_m:
        m = d[p]
        modrm = m
        mod, rm = m >> 6, m & 7
        p += 1
        if mod != 3:
            if pre & 2:  # 16-bit addressing
                if mod == 0 and rm == 6:
                    p += 2
                elif mod == 1:
                    p += 1
                elif mod == 2:
                    p += 2
            else:
                if rm == 4:
                    sib = d[p]
                    p += 1
                    if mod == 0 and (sib & 7) == 5:
                        p += 4
                elif mod == 0 and rm == 5:
                    p += 4
                if mod == 1:
                    p += 1
                elif mod == 2:
                    p += 4
    kind, target, relat = K_PLAIN, -1, -1
    if imm:
        if imm == 1:
            if not two and (0x70 <= op <= 0x7F or 0xE0 <= op <= 0xE3):
                kind, target = K_JCC8, p + 1 + struct.unpack_from("<b", d, p)[0]
            elif not two and op == 0xEB:
                kind, target = K_JMP8, p + 1 + struct.unpack_from("<b", d, p)[0]
            p += 1
        elif imm == 2:
            p += 2
        elif imm == 3:
            n = 2 if pre & 1 else 4
            if n == 4 and ((not two and op in (0xE8, 0xE9)) or (two and 0x80 <= op2 <= 0x8F)):
                relat = p - start
                target = p + 4 + struct.unpack_from("<i", d, p)[0]
                kind = K_CALL if (not two and op == 0xE8) else (K_JMP if not two else K_JCC)
            p += n
        elif imm == 4:
            p += 4 if pre & 1 else 6
        elif imm == 5:
            p += 2 if pre & 2 else 4
        elif imm == 6:
            p += 3
        elif imm == 7:
            if (modrm >> 3) & 7 < 2:
                p += 1
        elif imm == 8:
            if (modrm >> 3) & 7 < 2:
                p += 2 if pre & 1 else 4
    if not two:
        if op in (0xC3, 0xC2, 0xCB, 0xCA):
            kind = K_RET
        elif op == 0xFF:
            r = (modrm >> 3) & 7
            if r == 4 or r == 5:
                kind = K_IJMP
            elif r == 2 or r == 3:
                kind = K_ICALL
    shape = (pre << 24) | (opid << 8) | modrm if opid < 0x10000 else (pre << 24) ^ (opid << 4) ^ modrm
    return p - start, shape, kind, target, relat


# ------------------------------------------------------------------------ PE
class Image:
    def __init__(self, path):
        self.path = path
        self.data = d = open(path, "rb").read()
        pe = struct.unpack_from("<I", d, 0x3C)[0]
        count = struct.unpack_from("<H", d, pe + 6)[0]
        optional = struct.unpack_from("<H", d, pe + 20)[0]
        self.base = struct.unpack_from("<I", d, pe + 24 + 28)[0]
        dirs = pe + 24 + 96
        self.sections = []
        for i in range(count):
            o = pe + 24 + optional + 40 * i
            name = d[o:o + 8].rstrip(b"\0").decode()
            vsize, rva, rsize, raw = struct.unpack_from("<IIII", d, o + 8)
            self.sections.append((name, self.base + rva, vsize, raw, rsize))
        t = self.section(".text")
        self.text_va, self.text_raw, self.text_size = t[1], t[3], t[2]
        self.text_end = self.text_va + self.text_size
        # relocations: set of VAs holding absolute addresses
        rva, size = struct.unpack_from("<II", d, dirs + 8 * 5)
        self.relocs = set()
        o, end = self.offset(self.base + rva), self.offset(self.base + rva) + size
        while o < end:
            page, block = struct.unpack_from("<II", d, o)
            if block < 8:
                break
            for i in range((block - 8) // 2):
                e = struct.unpack_from("<H", d, o + 8 + 2 * i)[0]
                if e >> 12 == 3:
                    self.relocs.add(self.base + page + (e & 0xFFF))
            o += block
        # imports: IAT slot VA -> "dll!name"
        self.imports = {}
        rva, size = struct.unpack_from("<II", d, dirs + 8 * 1)
        o = self.offset(self.base + rva)
        while True:
            oft, _, _, name, ft = struct.unpack_from("<IIIII", d, o)
            if not ft:
                break
            dll = self.cstr(self.base + name).lower()
            names = self.offset(self.base + (oft or ft))
            i = 0
            while True:
                v = struct.unpack_from("<I", d, names + 4 * i)[0]
                if not v:
                    break
                sym = f"#{v & 0xFFFF}" if v & 0x80000000 else self.cstr(self.base + v + 2)
                self.imports[self.base + ft + 4 * i] = f"{dll}!{sym}"
                i += 1
            o += 20
        self.reloc_sorted = sorted(self.relocs)

    def section(self, name):
        return next(s for s in self.sections if s[0] == name)

    def offset(self, va):
        for _, start, vsize, raw, rsize in self.sections:
            if start <= va < start + max(vsize, rsize):
                if va - start >= rsize:
                    return None
                return va - start + raw
        return None

    def u32(self, va):
        o = self.offset(va)
        return None if o is None else struct.unpack_from("<I", self.data, o)[0]

    def cstr(self, va):
        o = self.offset(va)
        return self.data[o:self.data.index(b"\0", o)].decode("latin-1")

    def in_text(self, va):
        return self.text_va <= va < self.text_end

    def rtti(self):
        """class key ('name' or 'name@offset') -> vtable VA; vtable VA -> key;
        type descriptor VA -> name."""
        d = self.data
        tds = {}
        for m in re.finditer(rb"\.\?A[VU][^\0]{1,400}\0", d):
            va = self.va_of(m.start() - 8)
            if va is not None and va in self.relocs:
                tds[va] = m.group(0)[:-1].decode("latin-1")
        cols = {}
        for site in self.reloc_sorted:
            if site - 12 in self.relocs or True:
                tgt = self.u32(site)
                if tgt in tds and self.u32(site - 12) == 0 and (site + 4) in self.relocs:
                    cols[site - 12] = (tds[tgt], self.u32(site - 8))
        vtables, keys = {}, {}
        for site in self.reloc_sorted:
            tgt = self.u32(site)
            if tgt in cols and (site + 4) in self.relocs and self.in_text(self.u32(site + 4) or 0):
                name, off = cols[tgt]
                key = name if off == 0 else f"{name}@{off:#x}"
                if key in keys and keys[key] != site + 4:
                    key = f"{key}#{site + 4:x}"
                keys[key] = site + 4
                vtables[site + 4] = key
        return keys, vtables, tds

    def va_of(self, offset):
        for _, start, _, raw, rsize in self.sections:
            if raw <= offset < raw + rsize:
                return offset - raw + start
        return None

    def vtable_slots(self, vt):
        slots, a = [], vt
        while a in self.relocs and self.in_text(self.u32(a)):
            slots.append(self.u32(a))
            a += 4
            if a in self.vt_set:
                break
        return slots


def string_at(img, va):
    """A NUL-terminated ASCII or UTF-16LE string starting at va, or None."""
    o = img.offset(va)
    if o is None:
        return None
    d = img.data
    end = d.find(b"\0", o, o + 512)
    if end - o >= 3:
        s = d[o:end]
        if all(32 <= c < 127 or c in (9, 10, 13) for c in s):
            return s.decode()
    if end - o == 1:  # maybe UTF-16
        chars = []
        p = o
        while p + 1 < len(d) and len(chars) < 256:
            c = d[p] | d[p + 1] << 8
            if c == 0:
                break
            if not (32 <= c < 127 or c in (9, 10, 13)):
                return None
            chars.append(chr(c))
            p += 2
        if len(chars) >= 3:
            return "L" + "".join(chars)
    return None


# ----------------------------------------------------------------- functions
class Func:
    __slots__ = ("va", "size", "name", "n", "shape", "bhash", "shash", "calls", "refs",
                 "tokens", "insn", "_hist", "_norm")

    def __init__(self, va, size, name):
        self.va, self.size, self.name = va, size, name


def analyse(img, f):
    """Decode f and fill its hashes, shape, calls and pointer references."""
    d = img.data
    o = img.offset(f.va)
    end = o + f.size
    delta = f.va - o
    relocs = img.relocs
    norm = bytearray()
    shape, calls, refs, insn = [], [], [], []
    p = o
    while p < end:
        va = p + delta
        if va in relocs:  # data dword (jump table) inside code
            norm += b"\xAA\xAA\xAA\xAA"
            shape.append(0xDA7A)
            insn.append((va, K_PLAIN, -1, ()))
            p += 4
            continue
        try:
            n, sh, kind, tgt, relat = decode(d, p)
        except (IndexError, struct.error):
            break
        if p + n > end:
            norm += d[p:end]
            break
        raw = bytearray(d[p:p + n])
        if relat >= 0:
            raw[relat:relat + 4] = b"\xEE\xEE\xEE\xEE"
            tgt_va = tgt + delta
            if kind == K_CALL:
                calls.append((len(shape), tgt_va))
        else:
            tgt_va = tgt + delta if tgt >= 0 else -1
        ptrs = []
        for k in range(1, n - 3):
            if va + k in relocs:
                raw[k:k + 4] = b"\xBB\xBB\xBB\xBB"
                t = struct.unpack_from("<I", d, p + k)[0]
                ptrs.append(t)
                refs.append((len(shape), t))
        norm += raw
        shape.append(sh)
        insn.append((va, kind, tgt_va, ptrs))
        p += n
    # trailing int3 / nop padding is not part of the function
    while norm and norm[-1] in (0xCC, 0x90) and shape and shape[-1] in (0xCC00, 0x9000):
        norm.pop()
        shape.pop()
        insn.pop()
    f.n = len(shape)
    f.shape = shape
    f.bhash = hashlib.blake2b(bytes(norm), digest_size=12).digest()
    f.shash = hashlib.blake2b(struct.pack(f"<{len(shape)}I", *[s & 0xFFFFFFFF for s in shape]),
                              digest_size=12).digest()
    f.calls = calls
    f.refs = refs
    f.insn = insn


def static_tokens(img, f, strings, vt_of, tds):
    toks = collections.Counter()
    for _, t in f.refs:
        if t in img.imports:
            toks["I:" + img.imports[t]] += 1
        elif t in vt_of:
            toks["V:" + vt_of[t]] += 1
        elif t in tds:
            toks["T:" + tds[t]] += 1
        elif not img.in_text(t):
            s = strings.get(t)
            if s is None:
                s = strings[t] = string_at(img, t) or ""
            if s:
                toks["S:" + s] += 1
    for i, (va, kind, tgt, ptrs) in enumerate(f.insn):
        if kind in (K_JMP, K_CALL) and tgt in img.thunks:
            toks["I:" + img.thunks[tgt]] += 1
    return toks


def load_ref_functions(version, img):
    path = os.path.join(KIT, f"v{version}", "functions.txt")
    by_va = {}
    for line in open(path, encoding="utf-8", errors="replace"):
        a, l, name = line.rstrip("\n").split(" ", 2)
        va, size = int(a, 16), int(l, 16)
        if va in by_va:
            if size > by_va[va].size:
                by_va[va].size = size
            by_va[va].name += " | " + name if len(by_va[va].name) < 400 else ""
            continue
        if img.in_text(va) and size:
            by_va[va] = Func(va, size, name)
    return by_va


def find_starts(img, log):
    """Function starts of an exe without symbols. Strong starts: the entry
    point, pointers to .text from data (vtables, callbacks, EH tables), code
    pointers in instructions, targets of >= 2 calls. Weak starts: a single
    call or a tail jump to a plausible boundary, the end of int3 padding.
    A weak start that a linear decode of the code before it runs across
    (an instruction straddles it) is dropped."""
    d = img.data
    strong, weak = set(), set()
    entry_rva = struct.unpack_from("<I", d, struct.unpack_from("<I", d, 0x3C)[0] + 24 + 16)[0]
    strong.add(img.base + entry_rva)
    for site in img.reloc_sorted:
        if not img.in_text(site):
            t = img.u32(site)
            if t is not None and img.in_text(t):
                strong.add(t)
    # linear sweep with data skipping; code pointers in instructions and calls
    delta = img.text_va - img.text_raw
    call_count = collections.Counter()
    jmp_from = []
    p, end = img.text_raw, img.text_raw + img.text_size
    relocs = img.relocs
    while p < end:
        va = p + delta
        if va in relocs:
            p += 4
            continue
        if d[p] == 0xCC:
            p += 1
            continue
        n, sh, kind, tgt, relat = decode(d, p)
        if kind == K_CALL and img.text_raw <= tgt < end:
            call_count[tgt + delta] += 1
        elif kind == K_JMP and img.text_raw <= tgt < end:
            jmp_from.append((va, tgt + delta))
        for k in range(1, n - 3):
            if va + k in relocs:
                t = struct.unpack_from("<I", d, p + k)[0]
                if img.in_text(t) and d[p] not in (0xFF,):  # not jmp [table+reg*4]
                    strong.add(t)
        p += n

    def after_end(o):
        """o follows a ret/jmp, or int3 padding that does."""
        q = o
        while d[q - 1] == 0xCC:
            q -= 1
        if o - q >= 2:
            return True
        return d[q - 1] == 0xC3 or d[q - 3] == 0xC2 or d[q - 5] == 0xE9 or d[q - 2] == 0xEB \
            or (d[q - 2] == 0xFF and 0xE0 <= d[q - 1] <= 0xE7)

    def plausible(t):
        o = t - delta
        return after_end(o) or ((o - img.text_raw) % 16 == 0 and d[o - 1] == 0x90)
    for t, c in call_count.items():
        (strong if c >= 2 else weak if plausible(t) else set()).add(t)
    # builds that pad functions to 16 bytes with int3 (0.26g does, 0.21d not)
    tail = d.find(b"\xCC", img.text_raw)
    while 0 <= tail < end:
        q = tail
        while q < end and d[q] == 0xCC:
            q += 1
        va = q + delta
        if q < end and va % 16 == 0 and va not in relocs and after_end(q):
            weak.add(va)
        tail = d.find(b"\xCC", q, end)
    # tail jumps leaving the function they are in
    for _ in range(2):
        s = sorted(strong | weak)
        added = 0
        for va, t in jmp_from:
            i = bisect.bisect_right(s, va) - 1
            if i < 0:
                continue
            lo, hi = s[i], (s[i + 1] if i + 1 < len(s) else img.text_end)
            if not lo <= t < hi and t not in strong and t not in weak and plausible(t):
                weak.add(t)
                added += 1
        if not added:
            break
    # drop weak starts that the code before them runs across
    starts = sorted(strong | weak)
    keep, dropped = [], 0
    for i, s in enumerate(starts):
        if keep and s in weak and s not in strong:
            prev = keep[-1]
            p, o = img.offset(prev), img.offset(s)
            # decode at most the last 4 KB before s
            if o - p > 4096:
                p = o - 4096
                while p < o and d[p] != 0xCC:
                    p += 1
            while p < o:
                if p + delta in relocs:
                    p += 4
                    continue
                if d[p] == 0xCC:
                    p += 1
                    continue
                p += decode(d, p)[0]
            ok = p == o
            if not ok:
                dropped += 1
                continue
        keep.append(s)
    log(f"  {len(keep)} function starts ({len(strong)} strong, {len(weak)} weak, "
        f"{dropped} weak ones dropped as mid-instruction)")
    return set(keep)


def target_functions(img, log):
    starts = sorted(find_starts(img, log))
    funcs = {}
    for i, s in enumerate(starts):
        e = starts[i + 1] if i + 1 < len(starts) else img.text_end
        funcs[s] = Func(s, e - s, None)
    return funcs


# ------------------------------------------------------------------ matching
def shape_sim(a, b, limit=6000):
    if not a.n or not b.n:
        return 0.0
    if a.shash == b.shash:
        return 1.0
    if max(a.n, b.n) > limit:
        # long functions: compare a sample of the start and the end
        x = a.shape[:limit // 2] + a.shape[-limit // 2:]
        y = b.shape[:limit // 2] + b.shape[-limit // 2:]
    else:
        x, y = a.shape, b.shape
    sm = difflib.SequenceMatcher(None, x, y, autojunk=False)
    if sm.real_quick_ratio() < 0.5:
        return sm.real_quick_ratio()
    return sm.ratio()


def opcode_hist(f):
    h = getattr(f, "_hist", None)
    if h is None:
        h = collections.Counter((s >> 8) & 0xFFFF for s in f.shape)
        f._hist = h
        f._norm = math.sqrt(sum(v * v for v in h.values())) or 1.0
    return h


def hist_cos(a, b):
    """Cosine of the opcode histograms: tolerant of register allocation and
    scheduling changes, which make shape_sim low for unchanged source."""
    x, y = opcode_hist(a), opcode_hist(b)
    if len(x) > len(y):
        x, y = y, x
    return sum(v * y[k] for k, v in x.items()) / (a._norm * b._norm)


def structure(a, b):
    """-> (level, text): 'high', 'medium' or None, from strict shape similarity
    or, for recompiled code, opcode histogram plus instruction-count ratio."""
    sim = shape_sim(a, b)
    ratio = b.n / max(1, a.n)
    if sim >= 0.8:
        return "high", f"shape {sim:.2f}"
    hc = hist_cos(a, b)
    text = f"shape {sim:.2f}, hist {hc:.2f}, size x{ratio:.2f}"
    if hc >= 0.9 and 0.75 <= ratio <= 1.33 and a.n >= 8:
        return "high", text
    if sim >= 0.6 or (hc >= 0.75 and 0.5 <= ratio <= 2.0):
        return "medium", text
    return None, text


class Matcher:
    def __init__(self, ref_img, ref, tgt_img, tgt, log):
        self.ri, self.ti = ref_img, tgt_img
        self.ref, self.tgt = ref, tgt
        self.log = log
        self.r2t, self.t2r = {}, {}
        self.why = {}
        self.data_votes = collections.defaultdict(collections.Counter)
        self.data_map, self.data_inv = {}, {}
        self.rstarts = sorted(ref)
        self.tstarts = sorted(tgt)
        self.blocked = set()        # held out from the hash passes (--holdout)
        self.token_blocked = set()  # held out from the token pass

    def accept(self, r, t, conf, why):
        if r in self.r2t or t in self.t2r:
            return False
        self.r2t[r], self.t2r[t] = t, r
        self.why[r] = (conf, why)
        return True

    # a. exact bytes / b. exact shape
    def unique_hash(self, attr, min_n, label):
        rc, tc = collections.defaultdict(list), collections.defaultdict(list)
        for f in self.ref.values():
            if f.n >= min_n:
                rc[getattr(f, attr)].append(f.va)
        for f in self.tgt.values():
            if f.n >= min_n:
                tc[getattr(f, attr)].append(f.va)
        added = 0
        for h, rs in rc.items():
            ts = tc.get(h)
            if len(rs) == 1 and ts and len(ts) == 1 and rs[0] not in self.blocked:
                f = self.ref[rs[0]]
                if attr == "bhash":
                    conf = "high" if f.n >= 6 else "medium"
                else:
                    conf = "high" if f.n >= 30 else "medium"
                added += self.accept(rs[0], ts[0], conf, f"{label}: {f.n} insns")
        return added

    # c. tokens
    def dynamic_tokens(self, f, side):
        toks = collections.Counter(f.tokens)
        if side == "r":
            for _, c in f.calls:
                if c in self.r2t:
                    toks[f"K:{c:x}"] += 1
            for _, p in f.refs:
                if p in self.r2t:
                    toks[f"K:{p:x}"] += 1
                elif p in self.data_map:
                    toks[f"G:{p:x}"] += 1
        else:
            for _, c in f.calls:
                r = self.t2r.get(c)
                if r is not None:
                    toks[f"K:{r:x}"] += 1
            for _, p in f.refs:
                r = self.t2r.get(p)
                if r is not None:
                    toks[f"K:{r:x}"] += 1
                elif p in self.data_inv:
                    toks[f"G:{self.data_inv[p]:x}"] += 1
        return toks

    def token_pass(self):
        rt = {va: self.dynamic_tokens(f, "r") for va, f in self.ref.items()
              if va not in self.r2t and va not in self.token_blocked}
        tt = {va: self.dynamic_tokens(f, "t") for va, f in self.tgt.items()
              if va not in self.t2r}
        rt = {k: v for k, v in rt.items() if v}
        tt = {k: v for k, v in tt.items() if v}
        df_r, df_t = collections.Counter(), collections.Counter()
        for v in rt.values():
            df_r.update(v.keys())
        for v in tt.values():
            df_t.update(v.keys())
        index = collections.defaultdict(list)
        for va, v in tt.items():
            for k in v:
                if df_t[k] <= 12:
                    index[k].append(va)
        rindex = collections.defaultdict(list)
        for va, v in rt.items():
            for k in v:
                if df_r[k] <= 12:
                    rindex[k].append(va)

        def weight(k):
            return math.log(1 + 64.0 / (df_r[k] + df_t[k]))

        def score(a, b):
            common = sum(weight(k) * min(a[k], b[k]) for k in a if k in b)
            wa = sum(weight(k) * c for k, c in a.items())
            wb = sum(weight(k) * c for k, c in b.items())
            return common / max(wa, wb, 1e-9)

        added = 0
        for va, v in rt.items():
            cands = set()
            for k in v:
                if df_r[k] <= 12 and k in index:
                    cands.update(index[k])
            if not cands:
                continue
            sc = sorted(((score(v, tt[c]), c) for c in cands), reverse=True)
            s1, t = sc[0]
            if s1 < 0.45 or (len(sc) > 1 and sc[1][0] > 0.8 * s1):
                continue
            back = set()
            for k in tt[t]:
                if df_t[k] <= 12 and k in rindex:
                    back.update(rindex[k])
            bs = sorted(((score(rt[c], tt[t]), c) for c in back), reverse=True)
            if bs[0][1] != va or (len(bs) > 1 and bs[1][0] > 0.8 * bs[0][0]):
                continue
            rf, tf = self.ref[va], self.tgt[t]
            rare = [k for k in v if k in tt[t] and df_r[k] == 1 and df_t[k] == 1]
            level, text = structure(rf, tf)
            ratio = tf.n / max(1, rf.n)
            strong = len(rare) >= 2 or (len(rare) == 1 and s1 >= 0.8)
            if level == "high" and (strong or s1 >= 0.7):
                conf = "high"
            elif level and (strong or s1 >= 0.6):
                conf = "medium"
            elif len(rare) >= 2 and s1 >= 0.7 and 0.5 <= ratio <= 2.0:
                conf = "medium"
            else:
                continue
            kinds = sorted({k[0] for k in rare}) or ["-"]
            ex = next((k for k in rare if k[0] in "SIVT"), None)
            exs = f", e.g. {ex[:60]!r}" if ex else ""
            added += self.accept(va, t, conf, f"tokens: score {s1:.2f}, {len(rare)} unique "
                                              f"({''.join(kinds)}){exs}; {text}")
        return added

    # d. propagation through matched pairs
    def propagate(self):
        cand = collections.defaultdict(collections.Counter)
        weak = collections.defaultdict(lambda: collections.defaultdict(set))
        block = collections.Counter()  # longest aligned stretch a pair was seen in
        for r, t in list(self.r2t.items()):
            rf, tf = self.ref[r], self.tgt[t]
            if rf.n > 20000 or tf.n > 20000:
                continue
            # 1. instruction-aligned: calls and pointers in identical stretches
            if rf.shash == tf.shash:
                blocks = [(0, 0, rf.n)]
            else:
                sm = difflib.SequenceMatcher(None, rf.shape, tf.shape, autojunk=False)
                blocks = sm.get_matching_blocks()
            for i, j, n in blocks:
                for k in range(n):
                    ra, rk, rtgt, rptrs = rf.insn[i + k]
                    ta, tk, ttgt, tptrs = tf.insn[j + k]
                    if rk in (K_CALL, K_JMP) and tk == rk:
                        if rtgt in self.ref and ttgt in self.tgt and rtgt != r:
                            cand[rtgt][ttgt] += 1
                            block[(rtgt, ttgt)] = max(block[(rtgt, ttgt)], n)
                    if rptrs and len(rptrs) == len(tptrs):
                        for x, y in zip(rptrs, tptrs):
                            if x in self.ref and y in self.tgt:
                                cand[x][y] += 1
                                block[(x, y)] = max(block[(x, y)], n)
                            elif not self.ri.in_text(x) and not self.ti.in_text(y):
                                self.data_votes[x][y] += 1
            # 2. call sequence: unmatched callees between matched ones, when
            #    both gaps hold the same number of calls (recompiled code)
            cr = [c for _, c in rf.calls if c in self.ref]
            ct = [c for _, c in tf.calls if c in self.tgt]
            a = [self.r2t.get(c, ("r", i)) for i, c in enumerate(cr)]
            b = [c if c in self.t2r else ("t", j) for j, c in enumerate(ct)]
            sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
            pi = pj = 0
            for i, j, n in sm.get_matching_blocks():
                gi, gj = cr[pi:i], ct[pj:j]
                if gi and len(gi) == len(gj) and len(gi) <= 4:
                    for x, y in zip(gi, gj):
                        if x not in self.r2t and y not in self.t2r and x != r:
                            weak[x][y].add(r)
                pi, pj = i + n, j + n
        added = 0
        for x, c in cand.items():
            if x in self.r2t:
                continue
            (y, votes), = c.most_common(1)
            if votes < 0.75 * sum(c.values()) or y in self.t2r:
                continue
            level, text = structure(self.ref[x], self.tgt[y])
            if level is None:
                continue
            # one short aligned stretch is weak evidence (`mov eax,ebx; call`)
            if votes == 1 and level != "high" and block[(x, y)] < 6:
                continue
            exactish = text.startswith("shape 0.9") or text.startswith("shape 1")
            conf = "high" if level == "high" and (votes >= 2 or exactish) else "medium"
            added += self.accept(x, y, conf, f"propagated: callee/pointer at {votes} aligned "
                                            f"sites of matched callers; {text}")
        # call-sequence gaps alone are weaker (inlining shifts them): they need
        # two different matched callers that agree, and a similar body
        for x, c in weak.items():
            if x in self.r2t:
                continue
            ranked = sorted(c.items(), key=lambda kv: -len(kv[1]))
            y, callers = ranked[0]
            total = sum(len(v) for v in c.values())
            if len(callers) < 2 or len(callers) < 0.75 * total or y in self.t2r:
                continue
            level, text = structure(self.ref[x], self.tgt[y])
            if level is None:
                continue
            added += self.accept(x, y, "medium", f"propagated: same call-sequence gap in "
                                                 f"{len(callers)} matched callers; {text}")
        for x, c in self.data_votes.items():
            (y, votes), = c.most_common(1)
            if votes >= 0.75 * sum(c.values()) and x not in self.data_map \
                    and y not in self.data_inv:
                self.data_map[x], self.data_inv[y] = y, x
        return added

    # e. vtables
    def vtable_pass(self):
        added = 0
        for key, rv in self.ri.vt_keys.items():
            tv = self.ti.vt_keys.get(key)
            if tv is None:
                continue
            rs, ts = self.ri.vtable_slots(rv), self.ti.vtable_slots(tv)
            if len(rs) != len(ts):
                continue
            for i, (x, y) in enumerate(zip(rs, ts)):
                if x in self.r2t or y in self.t2r or x not in self.ref or y not in self.tgt:
                    continue
                if rs.count(x) > 1 or ts.count(y) > 1:
                    continue
                level, text = structure(self.ref[x], self.tgt[y])
                if level is None:
                    continue
                conf = "high" if level == "high" and self.ref[x].n >= 8 else "medium"
                added += self.accept(x, y, conf, f"vtable: {key[:60]} slot {i}; {text}")
        return added

    # f. gaps in address order
    def gap_pass(self):
        anchors = sorted(self.r2t.items())
        added = 0
        for (r1, t1), (r2, t2) in zip(anchors, anchors[1:]):
            if t2 <= t1:
                continue
            i1, i2 = bisect.bisect_right(self.rstarts, r1), bisect.bisect_left(self.rstarts, r2)
            j1, j2 = bisect.bisect_right(self.tstarts, t1), bisect.bisect_left(self.tstarts, t2)
            rs, ts = self.rstarts[i1:i2], self.tstarts[j1:j2]
            if not rs or len(rs) != len(ts) or len(rs) > 12:
                continue
            for x, y in zip(rs, ts):
                if x in self.r2t or y in self.t2r:
                    continue
                rf, tf = self.ref[x], self.tgt[y]
                if rf.n < 6:
                    continue
                sim = shape_sim(rf, tf)
                ratio = tf.n / max(1, rf.n)
                if sim >= 0.75:
                    conf = "high" if sim >= 0.95 and rf.n >= 20 else "medium"
                    text = f"shape {sim:.2f}"
                elif len(rs) <= 3 and rf.n >= 10 and 0.75 <= ratio <= 1.33 \
                        and hist_cos(rf, tf) >= 0.9:
                    conf = "medium"
                    text = f"shape {sim:.2f}, hist {hist_cos(rf, tf):.2f}, size x{ratio:.2f}"
                else:
                    continue
                added += self.accept(x, y, conf, f"order: {len(rs)} between matched "
                                                 f"neighbours; {text}")
        return added

    def gold_token_pairs(self):
        """Pairs that share >= 2 strings/imports/vtables found nowhere else and
        have a similar size: the truth for --holdout-from tokens."""
        owner_r, owner_t = {}, {}
        df_r, df_t = collections.Counter(), collections.Counter()
        for f in self.ref.values():
            for k in f.tokens:
                df_r[k] += 1
                owner_r[k] = f.va
        for f in self.tgt.values():
            for k in f.tokens:
                df_t[k] += 1
                owner_t[k] = f.va
        votes = collections.defaultdict(collections.Counter)
        for k, n in df_r.items():
            if n == 1 and df_t.get(k) == 1:
                votes[owner_r[k]][owner_t[k]] += 1
        back = collections.Counter(next(iter(c)) for c in votes.values() if len(c) == 1)
        gold = {}
        for r, c in votes.items():
            if len(c) == 1:
                (t, n), = c.items()
                ratio = self.tgt[t].n / max(1, self.ref[r].n)
                if n >= 2 and back[t] == 1 and 0.67 <= ratio <= 1.5:
                    gold[r] = t
        return gold

    def run(self, holdout=0.0, holdout_from="exact"):
        truth = {}
        if holdout and holdout_from == "tokens":
            import random
            rng = random.Random(1)
            for r, t in sorted(self.gold_token_pairs().items()):
                if rng.random() < holdout:
                    truth[r] = t
            self.blocked |= set(truth)
            self.token_blocked |= set(truth)
            self.log(f"  held out {len(truth)} token pairs (>= 2 unique tokens)")
        n = self.unique_hash("bhash", 3, "exact bytes")
        self.log(f"  exact bytes: {n}")
        if holdout and holdout_from == "exact":
            # Hide a share of the exact-byte matches from every hash pass and
            # see what the other methods make of them: an error-rate estimate.
            import random
            rng = random.Random(1)
            for r in sorted(self.r2t):
                if self.ref[r].n >= 6 and rng.random() < holdout:
                    t = self.r2t.pop(r)
                    del self.t2r[t], self.why[r]
                    truth[r] = t
                    self.blocked.add(r)
            self.log(f"  held out {len(truth)} exact matches")
        n = self.unique_hash("shash", 10, "exact shape")
        self.log(f"  exact shape: {n}")
        for rnd in range(12):
            a = self.token_pass()
            b = self.propagate()
            c = self.vtable_pass()
            d = self.gap_pass()
            self.log(f"  round {rnd + 1}: tokens {a}, propagation {b}, vtables {c}, "
                     f"gaps {d} -> {len(self.r2t)} matched, {len(self.data_map)} globals")
            if not (a or b or c or d):
                break
        if truth:
            res = collections.Counter()
            for r, t in truth.items():
                if r in self.r2t:
                    conf, why = self.why[r]
                    res[(conf, why.split(":")[0], self.r2t[r] == t)] += 1
                else:
                    res[("-", "unmatched", None)] += 1
            for k in sorted(res, key=str):
                self.log(f"  holdout {k[0]:6} {k[1]:12} {'right' if k[2] else 'WRONG' if k[2] is False else ''} {res[k]}")


def build(version, path, is_ref, log):
    log(f"{version}: {path}")
    img = Image(path)
    img.vt_keys, vt_of, tds = img.rtti()
    img.vt_set = set(vt_of)
    log(f"  {len(img.relocs)} relocations, {len(img.imports)} imports, "
        f"{len(img.vt_keys)} vtables, {len(tds)} type descriptors")
    funcs = load_ref_functions(version, img) if is_ref else target_functions(img, log)
    # import thunks: jmp [__imp_x]
    img.thunks = {}
    for va, f in funcs.items():
        o = img.offset(va)
        if img.data[o] == 0xFF and img.data[o + 1] == 0x25:
            slot = struct.unpack_from("<I", img.data, o + 2)[0]
            if slot in img.imports:
                img.thunks[va] = img.imports[slot]
    strings = {}
    for f in funcs.values():
        analyse(img, f)
    for f in funcs.values():
        f.tokens = static_tokens(img, f, strings, vt_of, tds)
    log(f"  {len(funcs)} functions analysed")
    return img, funcs


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--ref", default="021d")
    ap.add_argument("--target", default="026g")
    ap.add_argument("--ref-exe")
    ap.add_argument("--target-exe")
    ap.add_argument("--out")
    ap.add_argument("--stats", action="store_true", help="print a confidence/method summary")
    ap.add_argument("--holdout-from", choices=["exact", "tokens"], default="exact")
    ap.add_argument("--holdout", type=float, default=0.0,
                    help="hide this share of exact matches to estimate the other methods' error")
    a = ap.parse_args()
    log = lambda s: print(s, file=sys.stderr, flush=True)  # noqa: E731
    ref_exe = a.ref_exe or ORIGINAL_EXES.get(a.ref) or EXES[a.ref]
    tgt_exe = a.target_exe or ORIGINAL_EXES.get(a.target) or EXES[a.target]
    ri, ref = build(a.ref, ref_exe, True, log)
    ti, tgt = build(a.target, tgt_exe, False, log)
    m = Matcher(ri, ref, ti, tgt, log)
    m.run(a.holdout, a.holdout_from)
    out = a.out or os.path.join(KIT, f"v{a.target}", "functions_matched.txt")
    rows = []
    for r, t in m.r2t.items():
        conf, why = m.why[r]
        rf, tf = ref[r], tgt[t]
        size = tf.size
        o = ti.offset(t)
        while size > 1 and ti.data[o + size - 1] == 0xCC:
            size -= 1
        rows.append((t, size, rf.name, f"{conf}; {why}; {a.ref} {r:08x} len {rf.size:x}"))
    rows.sort()
    counts = collections.Counter(m.why[r][0] for r in m.r2t)
    methods = collections.Counter((m.why[r][0], m.why[r][1].split(":")[0])
                                  for r in m.r2t)
    with open(out, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(
            f"# {a.target} functions named from {a.ref} by common/scripts/match_functions.py\n"
            f"# reference: {ref_exe}\n# target:    {tgt_exe}\n"
            f"# format: <{a.target} VA, hex> <length, hex> <{a.ref} name(s)>  # <confidence>; "
            f"<method>: <details>; {a.ref} <ref VA> len <ref length>\n"
            f"# matched {len(rows)} of {len(ref)} {a.ref} functions (unique addresses); "
            f"{len(tgt)} function starts found in {a.target}; "
            + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())) + "\n"
            "# by method: " + ", ".join(f"{k[1]} {k[0]} {v}" for k, v in sorted(methods.items()))
            + "\n"
            "# methods: exact bytes (relocations/rel32 masked, unique both sides); exact shape\n"
            "#   (opcode+modrm sequence unique both sides); tokens (reciprocal best on rare\n"
            "#   strings/imports/vtables/RTTI + matched callees/globals); propagated (callee or\n"
            "#   pointer at aligned sites / same call-sequence gap of matched callers); vtable (same\n"
            "#   RTTI class, same slot count, slot i); order (between matched address neighbours).\n"
            "#   shape = difflib ratio of opcode+modrm sequences; hist = cosine of opcode histograms;\n"
            "#   size = target/ref instruction count. Candidates below 'medium' are not written.\n"
            "# lengths are to the next found start, trailing int3 dropped (may include an\n"
            "#   undiscovered neighbour); names joined by ' | ' were folded to one address in the ref.\n"
            "# lines starting with '#' are comments.\n")
        for t, size, name, ev in rows:
            fh.write(f"{t:08x} {size:x} {name}  # {ev}\n")
    log(f"wrote {out}: {len(rows)} functions of {len(ref)} named in {a.ref}, "
        f"{len(tgt)} found in {a.target}; " + ", ".join(f"{k} {v}" for k, v in counts.items()))
    if a.stats:
        for (conf, meth), c in sorted(methods.items()):
            log(f"  {conf:6} {meth:45} {c}")
        # order consistency: matches whose neighbours (by ref address) go the same way
        pairs = sorted(m.r2t.items())
        bad = sum(1 for (r0, t0), (r1, t1), (r2, t2) in zip(pairs, pairs[1:], pairs[2:])
                  if not (t0 < t1 < t2) and t0 < t2)
        log(f"  out-of-order matches between ordered neighbours: {bad}")


if __name__ == "__main__":
    main()
