# Walks a remote ISO9660 image's directory tree (Joliet names when present) with HTTP range requests.
import struct, sys, urllib.request
url = sys.argv[1]
def get(start, n):
    req = urllib.request.Request(url, headers={"Range": f"bytes={start}-{start+n-1}"})
    with urllib.request.urlopen(req) as r: return r.read()
root, joliet = None, False
for s in range(16, 32):
    d = get(s * 2048, 2048)
    if d[1:6] != b"CD001": break
    if d[0] == 1 and root is None: root = d[156:190]
    if d[0] == 2 and d[88:90] == b"%/": root, joliet = d[156:190], True
    if d[0] == 255: break
print("# joliet" if joliet else "# iso9660", file=sys.stderr)
def walk(rec, path):
    lba, size = struct.unpack_from("<I", rec, 2)[0], struct.unpack_from("<I", rec, 10)[0]
    data = get(lba * 2048, size); i = 0
    while i < len(data):
        n = data[i]
        if n == 0: i = (i // 2048 + 1) * 2048; continue
        r = data[i:i+n]; i += n
        nl = r[32]; raw = r[33:33+nl]
        if raw in (b"\x00", b"\x01"): continue
        name = raw.decode("utf-16-be") if joliet else raw.decode("latin-1")
        name = name.split(";")[0]
        fsize = struct.unpack_from("<I", r, 10)[0]
        if r[25] & 2: walk(r, path + name + "/")
        else: print(path + name, fsize)
walk(root, "")
