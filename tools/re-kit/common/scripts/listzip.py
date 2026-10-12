# Lists a remote .zip's members by reading its central directory with HTTP range requests.
import io, sys, urllib.request, zipfile

class HttpFile(io.RawIOBase):
    def __init__(s, url):
        s.url, s.pos = url, 0
        req = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(req) as r:
            s.size = int(r.headers["Content-Length"])
    def seekable(s): return True
    def readable(s): return True
    def tell(s): return s.pos
    def seek(s, off, whence=0):
        s.pos = [off, s.pos + off, s.size + off][whence]; return s.pos
    def readinto(s, b):
        n = min(len(b), s.size - s.pos)
        if n <= 0: return 0
        req = urllib.request.Request(s.url, headers={"Range": f"bytes={s.pos}-{s.pos+n-1}"})
        with urllib.request.urlopen(req) as r: data = r.read()
        b[:len(data)] = data; s.pos += len(data); return len(data)

z = zipfile.ZipFile(io.BufferedReader(HttpFile(sys.argv[1]), 1 << 20))
for i in z.infolist():
    print(i.filename, i.file_size)
