# Lists the file names in a remote .7z by reading only its start header and
# (encoded) end header with HTTP range requests.
import lzma, struct, sys, urllib.request

url = sys.argv[1]

def get(start, length):
    req = urllib.request.Request(url, headers={"Range": f"bytes={start}-{start+length-1}"})
    with urllib.request.urlopen(req) as r:
        data = r.read()
    assert len(data) == length, (len(data), length)
    return data

class R:
    def __init__(s, b): s.b, s.i = b, 0
    def byte(s): v = s.b[s.i]; s.i += 1; return v
    def num(s):
        first = s.byte(); mask = 0x80; value = 0
        for i in range(8):
            if first & mask == 0:
                return value | ((first & (mask - 1)) << (8 * i))
            value |= s.byte() << (8 * i); mask >>= 1
        return value
    def take(s, n): v = s.b[s.i:s.i+n]; s.i += n; return v

start = get(0, 32)
assert start[:6] == b"7z\xbc\xaf\x27\x1c", start[:6]
off, size, _ = struct.unpack("<QQI", start[12:32])
hdr = get(32 + off, size)
r = R(hdr)
kind = r.byte()
if kind == 0x17:  # encoded header: StreamsInfo describing the packed header
    assert r.byte() == 0x06  # PackInfo
    pack_pos = r.num(); n = r.num()
    assert r.byte() == 0x09; pack_sizes = [r.num() for _ in range(n)]
    while r.byte() != 0x00: pass
    assert r.byte() == 0x07 and r.byte() == 0x0B  # UnpackInfo, Folder
    assert r.num() == 1 and r.byte() == 0  # one folder, not external
    assert r.num() == 1  # one coder
    flags = r.byte(); cid = r.take(flags & 0x0F)
    props = r.take(r.num()) if flags & 0x20 else b""
    assert r.byte() == 0x0C; unpack = r.num()
    packed = get(32 + pack_pos, pack_sizes[0])
    if cid == b"\x03\x01\x01":
        d = props[0]; filt = {"id": lzma.FILTER_LZMA1, "lc": d % 9, "lp": (d // 9) % 5, "pb": d // 45,
                              "dict_size": struct.unpack("<I", props[1:5])[0]}
    elif cid == b"\x21":
        filt = {"id": lzma.FILTER_LZMA2, "dict_size": 1 << 26}
    else:
        raise SystemExit(f"coder {cid.hex()}")
    hdr = lzma.LZMADecompressor(lzma.FORMAT_RAW, filters=[filt]).decompress(packed, unpack)[:unpack]
    print(f"# encoded header {size} B -> packed {pack_sizes[0]} B -> {len(hdr)} B", file=sys.stderr)

# Pull the names block (property 0x11 in FilesInfo) out of the plain header.
i = hdr.find(b"\x11")
names = None
pos = 0
while True:
    pos = hdr.find(b"\x11", pos)
    if pos < 0: break
    rr = R(hdr); rr.i = pos + 1
    try:
        ln = rr.num(); ext = rr.byte()
        blob = hdr[rr.i:rr.i + ln - 1]
        if ext == 0 and len(blob) > 100 and blob.endswith(b"\x00\x00"):
            text = blob.decode("utf-16-le")
            if text.count("\x00") > 10:
                names = text.rstrip("\x00").split("\x00"); break
    except Exception:
        pass
    pos += 1
if names is None:
    raise SystemExit("names not found")
for n in names:
    print(n)
