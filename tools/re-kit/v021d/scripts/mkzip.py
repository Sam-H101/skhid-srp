import hashlib
import os
import zipfile

root = r'E:\sur-res-pro\client-dist'
top = 'Survarium-0.21d-srp'
out = os.path.join(root, top + '.new.zip')
with zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED, compresslevel=6, allowZip64=True) as z:
    for base, dirs, files in os.walk(os.path.join(root, top)):
        dirs.sort()
        rel = os.path.relpath(base, root).replace(os.sep, '/')
        z.write(base, rel + '/')
        for name in sorted(files):
            z.write(os.path.join(base, name), rel + '/' + name)
h = hashlib.sha256()
with open(out, 'rb') as f:
    while chunk := f.read(1 << 20):
        h.update(chunk)
print('size', os.path.getsize(out))
print('sha256', h.hexdigest())
