import struct, sys, pickle, bisect, os
HERE = os.path.dirname(os.path.abspath(__file__))
syms = pickle.load(open(os.path.join(HERE, 'syms.pkl'), 'rb'))
saddrs = [s[0] for s in syms]
PREF_BASE = 0x10000

def u32(b, o): return struct.unpack_from('<I', b, o)[0]
def u64(b, o): return struct.unpack_from('<Q', b, o)[0]

def rva_str(b, rva):
    n = u32(b, rva)
    return b[rva + 4: rva + 4 + n].decode('utf-16le', 'replace')

def parse(path):
    b = open(path, 'rb').read()
    assert b[:4] == b'MDMP'
    nstreams = u32(b, 8); dirrva = u32(b, 12)
    streams = {}
    for i in range(nstreams):
        t, sz, rva = struct.unpack_from('<III', b, dirrva + 12 * i)
        streams.setdefault(t, []).append((sz, rva))
    d = {'b': b, 'streams': streams}
    # modules
    mods = []
    if 4 in streams:
        sz, rva = streams[4][0]
        n = u32(b, rva)
        for i in range(n):
            o = rva + 4 + 108 * i
            base = u64(b, o); size = u32(b, o + 8); namerva = u32(b, o + 20)
            mods.append((base, size, rva_str(b, namerva)))
    d['mods'] = mods
    # memory
    mem = []
    if 5 in streams:
        sz, rva = streams[5][0]
        n = u32(b, rva)
        for i in range(n):
            o = rva + 4 + 16 * i
            start = u64(b, o) & 0xffffffff; dsz, drva = struct.unpack_from('<II', b, o + 8)
            mem.append((start, dsz, drva))
    if 9 in streams:
        sz, rva = streams[9][0]
        n = u64(b, rva); baserva = u64(b, rva + 8)
        cur = baserva
        for i in range(n):
            start, dsz = struct.unpack_from('<QQ', b, rva + 16 + 16 * i)
            mem.append((start, dsz, cur)); cur += dsz
    d['mem'] = mem
    # threads
    threads = []
    if 3 in streams:
        sz, rva = streams[3][0]
        n = u32(b, rva)
        for i in range(n):
            o = rva + 4 + 48 * i
            tid = u32(b, o); teb = u64(b, o + 16)
            sstart = u64(b, o + 24) & 0xffffffff; ssz, srva = struct.unpack_from('<II', b, o + 32)
            csz, crva = struct.unpack_from('<II', b, o + 40)
            threads.append(dict(tid=tid, teb=teb, sstart=sstart, ssz=ssz, srva=srva, csz=csz, crva=crva))
    d['threads'] = threads
    if 6 in streams:
        sz, rva = streams[6][0]
        tid = u32(b, rva)
        o = rva + 8
        code, flags = struct.unpack_from('<II', b, o)
        addr = u64(b, o + 16); npar = u32(b, o + 24)
        pars = [u64(b, o + 32 + 8 * k) for k in range(npar)]
        csz, crva = struct.unpack_from('<II', b, rva + 8 + 152)
        d['exc'] = dict(tid=tid, code=code, addr=addr, params=pars, csz=csz, crva=crva)
    return d

def ctx_x86(b, rva):
    # x86 CONTEXT: flags@0, Dr0-7 @4..28, FloatSave 112 bytes @28, then GS@140 FS ES DS EDI@156 ESI EBX EDX ECX EAX EBP@180 EIP@184 CS EFLAGS ESP@196 SS
    names = ['edi', 'esi', 'ebx', 'edx', 'ecx', 'eax', 'ebp', 'eip', 'cs', 'eflags', 'esp', 'ss']
    vals = struct.unpack_from('<12I', b, rva + 156)
    return dict(zip(names, vals))

def read(d, addr, n):
    for start, sz, rva in d['mem']:
        if start <= addr and addr + n <= start + sz:
            return d['b'][rva + addr - start: rva + addr - start + n]
    return None

def modof(d, a):
    for base, size, name in d['mods']:
        if base <= a < base + size:
            return base, os.path.basename(name)
    return None, None

def sym(d, a):
    base, name = modof(d, a)
    if name is None: return '?'
    if name.lower() == 'survarium.exe':
        r = a - base + PREF_BASE
        i = bisect.bisect_right(saddrs, r) - 1
        if i >= 0:
            return 'survarium!%s+0x%x (pref %08x)' % (syms[i][1], r - syms[i][0], r)
    return '%s+0x%x' % (name, a - base)

def is_call_ret(d, a):
    # check if bytes before a look like a call instruction
    pre = read(d, a - 7, 7)
    if pre is None:
        return None
    if pre[2] == 0xE8: return True  # call rel32
    if pre[5] == 0xFF and (pre[6] & 0x38) == 0x10: return True  # call r/m (2 bytes)
    if pre[4] == 0xFF and (pre[5] & 0x38) == 0x10: return True
    if pre[1] == 0xFF and (pre[2] & 0x38) == 0x10: return True  # call [disp32]
    if pre[3] == 0xFF and (pre[4] & 0x38) == 0x10: return True
    return False

if __name__ == '__main__':
    for path in sys.argv[1:]:
        d = parse(path)
        print('=' * 100); print(os.path.basename(path))
        mainmod = [m for m in d['mods'] if m[2].lower().endswith('survarium.exe')]
        print('survarium.exe base %x size %x' % (mainmod[0][0], mainmod[0][1]) if mainmod else 'no main mod')
        e = d.get('exc')
        if e:
            print('exception tid=%d code=%08x addr=%08x params=%s' % (e['tid'], e['code'], e['addr'], [hex(p) for p in e['params']]))
            print('  at', sym(d, e['addr']))
            c = ctx_x86(d['b'], e['crva'])
            print('  ctx', ' '.join('%s=%08x' % kv for kv in c.items()))
            for th in d['threads']:
                if th['tid'] == e['tid']:
                    tthread = th
            esp = c['esp']
            st = read(d, esp, 4)
            # scan stack
            lo = tthread['sstart']; hi = lo + tthread['ssz']
            print('  stack %x-%x esp %x' % (lo, hi, esp))
            a = max(esp, lo); cnt = 0
            while a < hi and cnt < int(os.environ.get('NFR', '60')):
                v = u32(read(d, a, 4), 0)
                base, mn = modof(d, v)
                if mn and is_call_ret(d, v) is not False:
                    print('   [%08x] %08x %s' % (a, v, sym(d, v)))
                    cnt += 1
                a += 4
        print('threads:', len(d['threads']))
