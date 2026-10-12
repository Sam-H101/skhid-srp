"""Split 0.21d world snapshots into sections (snapshot_format.md) and compare pairs.

Layout: u32 mask; per set bit: u16 L + player state, bool (0x00/0xFF), anim bitstream
if true; then u16-prefixed bullets, statistics, objects, rules.
"""
import struct
import sys
from pathlib import Path


def W(x):
    if x <= 1:
        return 1
    return max(1, (x - 1).bit_length())


class Bits:
    """MSB-first bit reader over bytes starting at `start`."""

    def __init__(self, data, start):
        self.data, self.start, self.pos = data, start, 0

    def read(self, n):
        v = 0
        for _ in range(n):
            byte = self.data[self.start + self.pos // 8]
            v = (v << 1) | ((byte >> (7 - self.pos % 8)) & 1)
            self.pos += 1
        return v


def parse_anim(data, at):
    """Return (end offset, dict of parts) of the anim bitstream at `at`."""
    b = Bits(data, at)
    h = b.read(16)
    tree_from = b.pos
    b.pos += h
    times_from = b.pos
    n = b.read(10)
    flag = b.read(1)
    b.read(32)
    if n != 1:
        w = b.read(5)
        if flag == 0:
            b.pos += n * w
        else:
            u = b.read(W(n + 1))
            b.pos += (u - 1) * w
            b.pos += n * W(u + 1)
    floats_from = b.pos
    fflag = b.read(1)
    m = b.read(10)
    floats = []
    if fflag == 0:
        for _ in range(m):
            floats.append(b.read(32))
        index = None
    else:
        k = b.read(W(m + 1))
        uniq = [b.read(32) for _ in range(k)]
        index = [b.read(W(k + 1)) for _ in range(m)]
        floats = [uniq[i] if i < len(uniq) else None for i in index]
    end_bits = b.pos
    end = at + (end_bits + 7) // 8
    return end, {
        "tree_bits": (tree_from, tree_from + h),
        "times_bits": (times_from, floats_from),
        "floats_bits": (floats_from, end_bits),
        "floats": floats,
    }


def split(data):
    """Sections as a list of (name, start, end) plus anim details per player."""
    mask = struct.unpack_from("<I", data, 0)[0]
    at = 4
    sections = [("mask", 0, 4)]
    anims = {}
    for i in range(32):
        if not mask >> i & 1:
            continue
        ln = struct.unpack_from("<H", data, at)[0]
        sections.append((f"player{i}.state", at + 2, at + 2 + ln))
        at += 2 + ln
        flag = data[at]
        at += 1
        if flag:
            end, parts = parse_anim(data, at)
            sections.append((f"player{i}.anim", at, end))
            anims[i] = (at, parts)
            at = end
    for name in ("bullets", "statistics", "objects", "rules"):
        ln = struct.unpack_from("<H", data, at)[0]
        sections.append((name, at + 2, at + 2 + ln))
        at += 2 + ln
    return mask, sections, anims, at == len(data)


def f32(b):
    return struct.unpack("<f", b)[0]


def describe_runs(a, b, start, end, limit=6):
    diffs = [i for i in range(start, end) if a[i] != b[i]]
    runs = []
    for i in diffs:
        if runs and i - runs[-1][1] <= 3:
            runs[-1][1] = i
        else:
            runs.append([i, i])
    out = []
    for lo, hi in runs[:limit]:
        # The 4-byte-aligned (relative to the section) floats covering the run.
        rel_lo = (lo - start) // 4 * 4 + start
        vals = []
        for p in range(rel_lo, hi + 1, 4):
            if p + 4 <= end:
                vals.append((p - start, f32(a[p:p + 4]), f32(b[p:p + 4])))
        out.append((lo - start, hi - start, vals))
    return len(diffs), runs, out


def main(folder):
    folder = Path(folder)
    files = sorted(folder.glob("*_diagnostic.bin"))
    for diag in files:
        stem = diag.name.rsplit("_", 2)[0]
        refs = [f for f in folder.glob(stem + "_*.bin") if f != diag]
        for ref in refs:
            a, b = diag.read_bytes(), ref.read_bytes()
            ma, sa, an_a, ok_a = split(a)
            mb, sb, an_b, ok_b = split(b)
            print(f"\n=== {diag.name} vs {ref.name}: {len(a)} vs {len(b)} bytes, "
                  f"parse {'ok' if ok_a and ok_b else 'FAILED'}")
            if not (ok_a and ok_b):
                continue
            if [s[0] for s in sa] != [s[0] for s in sb]:
                print("   different section lists:", [s[0] for s in sa], [s[0] for s in sb])
                continue
            for (name, s1, e1), (_, s2, e2) in zip(sa, sb):
                x, y = a[s1:e1], b[s2:e2]
                if x == y:
                    continue
                if len(x) != len(y):
                    print(f"   {name}: length {len(x)} vs {len(y)}")
                    continue
                n, runs, desc = describe_runs(x, y, 0, len(x))
                print(f"   {name}: {n} of {len(x)} bytes differ, in {len(runs)} runs")
                for lo, hi, vals in desc:
                    v = ", ".join(f"@{o}: {p:.7g} vs {q:.7g}" for o, p, q in vals)
                    print(f"      bytes {lo}-{hi}: {v}")
            # anim floats compared value by value
            for i in an_a:
                if i in an_b:
                    fa, fb = an_a[i][1]["floats"], an_b[i][1]["floats"]
                    if fa != fb and len(fa) == len(fb):
                        d = [(k, struct.unpack('<f', struct.pack('<I', p))[0],
                              struct.unpack('<f', struct.pack('<I', q))[0])
                             for k, (p, q) in enumerate(zip(fa, fb)) if p != q and p is not None and q is not None]
                        print(f"   player{i}.anim floats: {len(d)} of {len(fa)} differ: " +
                              ", ".join(f"#{k} {p:.7g} vs {q:.7g}" for k, p, q in d[:6]))


if __name__ == "__main__":
    main(sys.argv[1])
