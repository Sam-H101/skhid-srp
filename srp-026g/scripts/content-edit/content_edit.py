#!/usr/bin/env python3
"""content_edit.py: make a resources.db patch folder by editing game configs in place.

usage: python3 -I content_edit.py <edits.json> <live folder> <out folder> [--check]

<live folder>  the live game files, at their game paths (gameplay/items/...) or flattened as
               `dbgrab <server-game-data.db> get gameplay/ <dir>` writes them (gameplay__items__...).
               Take them from the live server game data, which holds the files the players' game
               mounts (stock resources.db with every offered patch over it).
<out folder>   gets every edited file at its game path, for `release/build-patch <out> <rev>`.
               Must not exist yet.
--check        edit in memory and report, write nothing.

A Vostok binary config has no header: the root value is at 0, every value is 24 bytes
(+0 data or the offset of its children, +8 offset of its name, +0x10 CRC32 of the name, +0x14 u16 type,
+0x16 u16 count), names and strings after the values. The game's lookup (0xF6B10 -> 0xF6D50)
binary-searches a table's children by name CRC and has no end check (srp-026g/docs/notes/replay_sur.md),
so the files are never re-made: every edit changes the fewest bytes possible and a renamed key's
table is re-sorted by CRC.

Edits (edits.json):
  "set":            [{"file", "path", "from", "to"}]   a number (float or int, its type kept); `from` must
                                                       be the live value (a stale base is refused)
  "rename_key":     [{"files" (glob on the game path), "table", "from", "to"}]
                    renames a key of the named table in every matching file that has it; the new name
                    is appended after the file's last byte, the table's children re-sorted by CRC
  "set_prop":       [{"file", "prop_id", "from", "to"}] the lobby card's text of a stat (ui_desc/props_list:
                                                       1 damage x100, 3 armour piercing x100, 4 rate of fire)
  "replace_string": [{"files", "from", "to"}]          every string value equal to `from`, appended
  "edit_text":      [{"file", "path", "from", "to"}]   in one text (UTF-8, as the localization tables
                                                       keep it), its one `from` becomes `to`

Every edited file is read back with an independent walk: only the intended values, names and order
may differ from the live file (the check prints each difference).
"""
import fnmatch
import json
import os
import struct
import sys
import zlib

T_BOOL, T_INT, T_FLOAT, T_NAMED, T_INDEXED, T_STR = 0, 1, 2, 3, 4, 5
REC = struct.Struct('<QQIHH')


def rec(buf, off):
    return REC.unpack_from(buf, off)


def name_at(buf, off):
    return bytes(buf[off:buf.index(b'\0', off)])


def children(buf, off):
    data, _, _, t, cnt = rec(buf, off)
    assert t in (T_NAMED, T_INDEXED), 'not a table'
    return [data + 24 * i for i in range(cnt)]


def find(buf, path):
    """Offset of the value at a/b/c (named tables only)."""
    off = 0
    for part in path.split('/'):
        want = part.encode()
        for c in children(buf, off):
            if name_at(buf, rec(buf, c)[1]) == want:
                off = c
                break
        else:
            raise KeyError(path)
    return off


def walk(buf, off=0, path=''):
    """Every value as (path, type, value) in file order, names resolved; floats as bits."""
    data, idoff, crc, t, cnt = rec(buf, off)
    out = []
    if t in (T_NAMED, T_INDEXED):
        out.append((path, t, cnt))
        for i, c in enumerate(children(buf, off)):
            cd, cid, ccrc, ct, ccnt = rec(buf, c)
            if t == T_NAMED:
                nm = name_at(buf, cid)
                assert zlib.crc32(nm) & 0xffffffff == ccrc, (path, nm)
                out += walk(buf, c, path + '/' + nm.decode())
            else:
                out += walk(buf, c, '%s[%d]' % (path, i))
        if t == T_NAMED:
            crcs = [rec(buf, c)[2] for c in children(buf, off)]
            assert crcs == sorted(crcs), 'children not sorted by CRC at ' + (path or '/')
    elif t == T_STR:
        out.append((path, t, bytes(buf[data:data + cnt - 1])))
    elif t in (T_BOOL, T_INT, T_FLOAT):
        out.append((path, t, data & 0xffffffff))
    else:
        out.append((path, t, bytes(buf[data:data + 4 * (t - 4)]) if t > 5 else data))
    return out


def as_number(t, raw):
    if t == T_FLOAT:
        return struct.unpack('<f', struct.pack('<I', raw))[0]
    if t == T_INT:
        return struct.unpack('<i', struct.pack('<I', raw))[0]
    raise TypeError('not a number')


def set_number(buf, path, frm, to, as_float=False):
    """An int value keeps its type unless `as_float` (the edit's "as_float": true) makes it a float:
    the game's reader takes either for a number (the same key is an int in some configs, a float in
    others), but a fraction is never silently cut to an int."""
    off = find(buf, path)
    data, idoff, crc, t, cnt = rec(buf, off)
    old = as_number(t, data & 0xffffffff)
    if abs(old - frm) > 1e-6:
        raise ValueError('%s is %r, not %r: the base is not the live data' % (path, old, frm))
    if t == T_INT and as_float:
        t = T_FLOAT
    if t == T_INT and to != int(to):
        raise ValueError('%s is an int; %r needs "as_float": true' % (path, to))
    raw = struct.unpack('<I', struct.pack('<f' if t == T_FLOAT else '<i', to if t == T_FLOAT else int(to)))[0]
    REC.pack_into(buf, off, (data & ~0xffffffff) | raw, idoff, crc, t, cnt)


def append(buf, blob):
    off = len(buf)
    buf.extend(blob)
    return off


def rename_key(buf, table, frm, to):
    try:
        toff = find(buf, table)
    except KeyError:
        return False
    kids = children(buf, toff)
    hit = [c for c in kids if name_at(buf, rec(buf, c)[1]) == frm.encode()]
    if not hit:
        return False
    assert not any(name_at(buf, rec(buf, c)[1]) == to.encode() for c in kids), 'both names in ' + table
    noff = append(buf, to.encode() + b'\0')
    d, _, _, t, n = rec(buf, hit[0])
    REC.pack_into(buf, hit[0], d, noff, zlib.crc32(to.encode()) & 0xffffffff, t, n)
    recs = sorted((bytes(buf[c:c + 24]) for c in kids), key=lambda r: REC.unpack(r)[2])
    for c, r in zip(kids, recs):
        buf[c:c + 24] = r
    return True


def value_offsets(buf, off=0):
    """Every value's offset, the root first."""
    out = [off]
    t = rec(buf, off)[3]
    if t in (T_NAMED, T_INDEXED):
        for c in children(buf, off):
            out += value_offsets(buf, c)
    return out


def replace_string(buf, frm, to):
    n = 0
    noff = None
    for off in value_offsets(buf):
        d, i, c, t, cnt = rec(buf, off)
        if t == T_STR and bytes(buf[d:d + cnt - 1]) == frm.encode():
            if noff is None:
                noff = append(buf, to.encode() + b'\0')
            REC.pack_into(buf, off, noff, i, c, t, len(to.encode()) + 1)
            n += 1
    return n


def edit_text(buf, path, frm, to):
    """A text (a description in a localization table): its one `frm` becomes `to`, the new text
    appended. A text without `frm`, or with it twice, is refused."""
    off = find(buf, path)
    d, i, c, t, cnt = rec(buf, off)
    if t != T_STR:
        raise ValueError('%s is not a text' % path)
    old = bytes(buf[d:d + cnt - 1])
    if old.count(frm.encode()) != 1:
        raise ValueError('%s has %r %d times, not once: the base is not the live data'
                         % (path, frm, old.count(frm.encode())))
    new = old.replace(frm.encode(), to.encode())
    REC.pack_into(buf, off, append(buf, new + b'\0'), i, c, t, len(new) + 1)


def set_prop(buf, prop_id, frm, to):
    """The lobby card's text of a stat: `ui_desc/props_list` entry `prop_id` (1 damage x100, 3 armour
    piercing x100, 4 rate of fire, ...), its `prop_value` string; the card shows this text, it does not
    compute it from `parameters`. Returns the changed value's path."""
    lst = find(buf, 'ui_desc/props_list')
    for i, entry in enumerate(children(buf, lst)):
        kids = {name_at(buf, rec(buf, c)[1]): c for c in children(buf, entry)}
        d, _, _, t, _ = rec(buf, kids[b'prop_id'])
        if as_number(t, d & 0xffffffff) != prop_id:
            continue
        off = kids[b'prop_value']
        d, i2, c, t, cnt = rec(buf, off)
        old = bytes(buf[d:d + cnt - 1]).decode()
        if old != frm:
            raise ValueError('prop %d says %r, not %r: the base is not the live data' % (prop_id, old, frm))
        noff = append(buf, to.encode() + bytes(1))
        REC.pack_into(buf, off, noff, i2, c, t, len(to.encode()) + 1)
        return '/ui_desc/props_list[%d]/prop_value' % i
    raise KeyError('prop %d' % prop_id)


def game_path(name):
    return name.replace('__', '/')


def main():
    args = [a for a in sys.argv[1:] if not a.startswith('--')]
    check = '--check' in sys.argv
    if len(args) != 3:
        sys.exit(__doc__)
    edits_file, live, out = args
    edits = json.load(open(edits_file, encoding='utf-8'))
    if not check and os.path.exists(out):
        sys.exit('%s: already there' % out)
    files = {}
    for root, _, names in os.walk(live):
        for n in names:
            rel = os.path.relpath(os.path.join(root, n), live).replace(os.sep, '/')
            files[game_path(rel)] = os.path.join(root, n)
    bufs = {}

    def buf(p):
        if p not in bufs:
            if p not in files:
                sys.exit('%s: not in the live folder' % p)
            bufs[p] = bytearray(open(files[p], 'rb').read())
        return bufs[p]

    allowed = {}
    for e in edits.get('set', []):
        set_number(buf(e['file']), e['path'], e['from'], e['to'], e.get('as_float', False))
        allowed.setdefault(e['file'], set()).add('/' + e['path'])
    for e in edits.get('set_prop', []):
        q = set_prop(buf(e['file']), e['prop_id'], e['from'], e['to'])
        allowed.setdefault(e['file'], set()).add(q)
    for e in edits.get('edit_text', []):
        edit_text(buf(e['file']), e['path'], e['from'], e['to'])
        allowed.setdefault(e['file'], set()).add('/' + e['path'])
    for e in edits.get('rename_key', []):
        n = 0
        for p in sorted(fnmatch.filter(files, e['files'])):
            b = bytearray(open(files[p], 'rb').read()) if p not in bufs else bufs[p]
            if rename_key(b, e['table'], e['from'], e['to']):
                bufs[p] = b
                n += 1
        print('rename %s -> %s: %d files' % (e['from'], e['to'], n))
    for e in edits.get('replace_string', []):
        n = 0
        for p in sorted(fnmatch.filter(files, e['files'])):
            b = bytearray(open(files[p], 'rb').read()) if p not in bufs else bufs[p]
            k = replace_string(b, e['from'], e['to'])
            if k:
                bufs[p] = b
                n += k
        print('string %s -> %s: %d values' % (e['from'], e['to'], n))

    # read back: compare every value with the live file, by path
    renames = [('/%s/%s' % (e['table'], e['from']), '/%s/%s' % (e['table'], e['to']), e['from'], e['to'])
               for e in edits.get('rename_key', [])]
    strings = [(e['from'].encode(), e['to'].encode()) for e in edits.get('replace_string', [])]
    bad = 0
    for p, b in sorted(bufs.items()):
        old = {q: (t, v) for q, t, v in walk(bytearray(open(files[p], 'rb').read()))}
        new = {q: (t, v) for q, t, v in walk(b)}
        for ro, rn, _, _ in renames:
            if ro in old and rn in new and rn not in old:
                old[rn] = old.pop(ro)
        for q in sorted(set(old) | set(new)):
            o, n = old.get(q), new.get(q)
            if o == n:
                continue
            ok = (q in allowed.get(p, ()) and o and n and n[0] in (o[0], T_FLOAT)) or \
                (o and n and o[0] == n[0] == T_STR and (o[1], n[1]) in strings)
            bad += not ok
            print('%s%s %s: %s -> %s' % ('' if ok else 'UNEXPECTED ', p, q, show(o), show(n)))
    print('%d files edited, %d unexpected differences' % (len(bufs), bad))
    if bad:
        sys.exit(1)
    if check:
        return
    for p, b in bufs.items():
        dst = os.path.join(out, *p.split('/'))
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        with open(dst, 'wb') as f:
            f.write(b)
    print('written to', out)


def show(v):
    if v is None:
        return None
    return round(as_number(v[0], v[1]), 6) if v[0] in (T_INT, T_FLOAT) else v[1]


if __name__ == '__main__':
    main()
