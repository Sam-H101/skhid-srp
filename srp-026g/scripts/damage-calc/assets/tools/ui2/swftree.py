# Dump a sprite tree from a SWF: placements (matrix, colour transform, name) and leaf tags
# (DefineEditText fields, shape bounds + fill colours, bitmaps).  usage: swftree.py <swf> <id> [maxdepth]
import sys, struct, zlib
d = open(sys.argv[1], 'rb').read()
if d[:3] in (b'CWS', b'CFX'):
    d = d[:8] + zlib.decompress(d[8:])


class Bits:
    def __init__(s, b, p): s.b, s.p, s.bit = b, p, 0
    def ub(s, n):
        v = 0
        for _ in range(n):
            v = (v << 1) | ((s.b[s.p] >> (7 - s.bit)) & 1)
            s.bit += 1
            if s.bit == 8: s.bit, s.p = 0, s.p + 1
        return v
    def sb(s, n):
        v = s.ub(n)
        return v - (1 << n) if n and v >> (n - 1) else v
    def align(s):
        if s.bit: s.bit, s.p = 0, s.p + 1


def rect(b, p):
    bt = Bits(b, p); n = bt.ub(5)
    r = [bt.sb(n) / 20 for _ in range(4)]; bt.align(); return r, bt.p


def matrix(b, p):
    bt = Bits(b, p); sx = sy = 1.0; r0 = r1 = 0.0
    if bt.ub(1): n = bt.ub(5); sx = bt.sb(n) / 65536; sy = bt.sb(n) / 65536
    if bt.ub(1): n = bt.ub(5); r0 = bt.sb(n) / 65536; r1 = bt.sb(n) / 65536
    n = bt.ub(5); tx = bt.sb(n) / 20; ty = bt.sb(n) / 20
    bt.align(); return (sx, r0, r1, sy, tx, ty), bt.p


def cxform(b, p, alpha=True):
    bt = Bits(b, p); ha = bt.ub(1); hm = bt.ub(1); n = bt.ub(4)
    k = 4 if alpha else 3
    mul = [bt.sb(n) for _ in range(k)] if hm else None
    add = [bt.sb(n) for _ in range(k)] if ha else None
    bt.align(); return (mul, add), bt.p


def tags(buf, p, end):
    while p < end:
        h = struct.unpack_from('<H', buf, p)[0]; p += 2; code = h >> 6; ln = h & 63
        if ln == 63: ln = struct.unpack_from('<I', buf, p)[0]; p += 4
        yield code, p, ln; p += ln
        if code == 0: break


nb = d[8] >> 3; rb = (5 + 4 * nb + 7) // 8; p0 = 8 + rb + 4
chars = {}
for code, q, ln in tags(d, p0, len(d)):
    if code in (2, 22, 32, 83, 39, 37, 6, 21, 35, 20, 36, 90, 46, 84, 48, 75, 10, 11, 33, 7, 34):
        chars[struct.unpack_from('<H', d, q)[0]] = (code, q, ln)
sym = {}
for code, q, ln in tags(d, p0, len(d)):
    if code == 76:
        n = struct.unpack_from('<H', d, q)[0]; r = q + 2
        for _ in range(n):
            cid = struct.unpack_from('<H', d, r)[0]; r += 2; e = d.index(b'\0', r); sym[cid] = d[r:e].decode('latin1'); r = e + 1
fontname = {}
for cid, (code, q, ln) in chars.items():
    if code in (48, 75):
        nl = d[q + 4]; fontname[cid] = d[q + 5:q + 5 + nl].rstrip(b'\0').decode('latin1') + (' Bold' if d[q + 2] & 1 else '') + (' Italic' if d[q + 2] & 2 else '')
NAMES = {2: 'Shape1', 22: 'Shape2', 32: 'Shape3', 83: 'Shape4', 39: 'Sprite', 37: 'EditText', 6: 'BitsJPEG',
         21: 'JPEG2', 35: 'JPEG3', 20: 'Lossless', 36: 'Lossless2', 90: 'JPEG4', 46: 'MorphShape', 84: 'MorphShape2',
         11: 'Text', 33: 'Text2', 7: 'Button', 34: 'Button2'}


def edittext(q, ln):
    cid = struct.unpack_from('<H', d, q)[0]; r, p = rect(d, q + 2)
    f1, f2 = d[p], d[p + 1]; p += 2
    info = {'bounds': r}
    hasText, wordWrap, multiline, password, readonly, hasColor, hasMaxLen, hasFont = [(f1 >> (7 - i)) & 1 for i in range(8)]
    hasFontClass, autosize, hasLayout, noSelect, border, wasStatic, html, useOutlines = [(f2 >> (7 - i)) & 1 for i in range(8)]
    if hasFont: fid = struct.unpack_from('<H', d, p)[0]; p += 2; info['font'] = f'{fid} {fontname.get(fid, "?")}'
    if hasFontClass: e = d.index(b'\0', p); info['fontClass'] = d[p:e].decode(); p = e + 1
    if hasFont or hasFontClass: info['size'] = struct.unpack_from('<H', d, p)[0] / 20; p += 2
    if hasColor: c = d[p:p + 4]; info['color'] = '#%02x%02x%02x a%d' % (c[0], c[1], c[2], c[3]); p += 4
    if hasMaxLen: p += 2
    if hasLayout:
        al, lm, rm, ind, lead = d[p], *struct.unpack_from('<HHHh', d, p + 1); p += 9
        info['align'] = ['left', 'right', 'center', 'justify'][al] if al < 4 else al
        info['margins'] = (lm / 20, rm / 20); info['indent'] = ind / 20; info['leading'] = lead / 20
    e = d.index(b'\0', p); info['var'] = d[p:e].decode('latin1'); p = e + 1
    if hasText: e = d.index(b'\0', p); info['text'] = d[p:e].decode('utf8', 'replace')[:200]
    info['flags'] = ''.join(k for k, v in (('html ', html), ('wrap ', wordWrap), ('multi ', multiline), ('autosize ', autosize), ('outlines ', useOutlines), ('ro ', readonly), ('nosel ', noSelect)) if v)
    return info


def shape_fills(code, q, ln):
    r, p = rect(d, q + 2)
    if code == 83: _, p = rect(d, p); p += 1
    n = d[p]; p += 1
    if n == 0xff: n = struct.unpack_from('<H', d, p)[0]; p += 2
    out = []
    for _ in range(n):
        t = d[p]; p += 1
        if t == 0:
            if code in (32, 83): out.append('#%02x%02x%02x a%d' % tuple(d[p:p + 4])); p += 4
            else: out.append('#%02x%02x%02x' % tuple(d[p:p + 3])); p += 3
        elif t in (0x10, 0x12, 0x13):
            _, p = matrix(d, p); ng = d[p] & 15; p += 1; st = []
            for _ in range(ng):
                ratio = d[p]; p += 1
                if code in (32, 83): st.append('%d:#%02x%02x%02x a%d' % (ratio, *d[p:p + 4])); p += 4
                else: st.append('%d:#%02x%02x%02x' % (ratio, *d[p:p + 3])); p += 3
            if t == 0x13: p += 2
            out.append(('linear' if t == 0x10 else 'radial') + '[' + ' '.join(st) + ']')
        elif t in (0x40, 0x41, 0x42, 0x43):
            bid = struct.unpack_from('<H', d, p)[0]; p += 2; m, p = matrix(d, p); out.append(f'bitmap{bid}@({m[4]:g},{m[5]:g}) s{m[0]:g}')
        else:
            out.append(f'type{t:#x}?'); break
    # line styles (first only, rough)
    nl = d[p]; p += 1; ls = []
    if nl == 0xff: nl = struct.unpack_from('<H', d, p)[0]; p += 2
    for _ in range(min(nl, 4)):
        w = struct.unpack_from('<H', d, p)[0] / 20; p += 2
        if code == 83:
            f1, f2 = d[p], d[p + 1]; p += 2
            hasfill = f1 & 8; join = (f1 >> 4) & 3
            if join == 2: p += 2
            if hasfill: ls.append(f'line{w:g} fill'); break
            ls.append(f'line{w:g} #%02x%02x%02x a%d' % tuple(d[p:p + 4])); p += 4
        elif code == 32: ls.append(f'line{w:g} #%02x%02x%02x a%d' % tuple(d[p:p + 4])); p += 4
        else: ls.append(f'line{w:g} #%02x%02x%02x' % tuple(d[p:p + 3])); p += 3
    return r, out, ls


def desc(cid):
    if cid not in chars: return f'#{cid} ?'
    code, q, ln = chars[cid]
    s = f'#{cid} {NAMES.get(code, code)}' + (f' <{sym[cid]}>' if cid in sym else '')
    if code in (2, 22, 32, 83):
        r, f, ls = shape_fills(code, q, ln)
        s += f' bounds x[{r[0]:g},{r[1]:g}] y[{r[2]:g},{r[3]:g}] fills {f} {ls}'
    elif code == 37:
        s += ' ' + str(edittext(q, ln))
    elif code in (20, 36):
        s += f' {struct.unpack_from("<H", d, q + 3)[0]}x{struct.unpack_from("<H", d, q + 5)[0]}'
    return s


def filters(p):
    n = d[p]; p += 1; out = []
    fx = lambda q: struct.unpack_from('<i', d, q)[0] / 65536
    for _ in range(n):
        t = d[p]; p += 1
        if t == 0:
            c = d[p:p+4]; out.append('DropShadow(#%02x%02x%02x a%d blur %g,%g angle %.3g dist %g str %g flags %#x)' % (*c, fx(p+4), fx(p+8), fx(p+12), fx(p+16), struct.unpack_from('<h', d, p+20)[0]/256, d[p+22])); p += 23
        elif t == 1: out.append('Blur(%g,%g)' % (fx(p), fx(p+4))); p += 9
        elif t == 2:
            c = d[p:p+4]; out.append('Glow(#%02x%02x%02x a%d blur %g,%g str %g flags %#x)' % (*c, fx(p+4), fx(p+8), struct.unpack_from('<h', d, p+12)[0]/256, d[p+14])); p += 15
        elif t == 3: out.append('Bevel'); p += 27
        elif t == 6: out.append('ColorMatrix'); p += 80
        else: out.append(f'filter{t}?'); break
    return '[' + '; '.join(out) + ']'


def fmt_m(m):
    sx, r0, r1, sy, tx, ty = m
    s = f'({tx:g},{ty:g})'
    if (sx, sy) != (1, 1): s += f' s({sx:.4g},{sy:.4g})'
    if r0 or r1: s += f' r({r0:.4g},{r1:.4g})'
    return s


def sprite(cid, ind, maxd, seen):
    code, q, ln = chars[cid]
    print('  ' * ind + desc(cid) + f' frames={struct.unpack_from("<H", d, q + 2)[0]}')
    if ind >= maxd: return
    frame = 1; kids = []
    for c2, q2, l2 in tags(d, q + 4, q + ln):
        if c2 == 1: frame += 1; continue
        if c2 == 43: e = d.index(b'\0', q2); print('  ' * (ind + 1) + f'f{frame} label {d[q2:e].decode()}'); continue
        if c2 in (26, 70):
            f1 = d[q2]; p = q2 + 1
            f2 = 0
            if c2 == 70: f2 = d[p]; p += 1
            depth = struct.unpack_from('<H', d, p)[0]; p += 2
            if c2 == 70 and (f2 & 8 or (f2 & 16 and f1 & 2)): e = d.index(b'\0', p); p = e + 1
            ch = None; m = None; cx = None; name = ''
            if f1 & 2: ch = struct.unpack_from('<H', d, p)[0]; p += 2
            if f1 & 4: m, p = matrix(d, p)
            if f1 & 8: cx, p = cxform(d, p)
            if f1 & 16: p += 2
            if f1 & 32: e = d.index(b'\0', p); name = d[p:e].decode('latin1'); p = e + 1
            if f1 & 64: p += 2
            extra = ''
            if c2 == 70 and f2 & 1: extra += ' filters' + filters(p)
            if c2 == 70 and f2 & 2: extra += ' blend'
            line = '  ' * (ind + 1) + f'f{frame} d{depth} ' + ('move ' if f1 & 1 else '') + (f'"{name}" ' if name else '') + (fmt_m(m) if m else '') + (f' cx{cx}' if cx else '') + extra
            if ch is not None:
                code3 = chars.get(ch, (None,))[0]
                if code3 == 39 and ch not in seen:
                    print(line + ' ->')
                    seen.add(ch); sprite(ch, ind + 2, maxd, seen)
                else:
                    print(line + ' ' + desc(ch))
            else:
                print(line)
        elif c2 == 28:
            print('  ' * (ind + 1) + f'f{frame} remove d{struct.unpack_from("<H", d, q2)[0]}')


for a in sys.argv[2:3]:
    sprite(int(a), 0, int(sys.argv[3]) if len(sys.argv) > 3 else 3, set())
