"""dis.py VERSION VA LEN : disassemble LEN bytes at virtual address VA of that
client build with nasm's ndisasm. Function names: grep ../../vVERSION/functions.txt."""
import os
import subprocess
import sys
import tempfile

from exes import Pe, exe_path

NDISASM = "E:/sur-res-pro/tools/nasm-2.16.03/ndisasm.exe"
version, va, length = sys.argv[1], int(sys.argv[2], 0), int(sys.argv[3], 0)
pe = Pe(exe_path(version))
start = pe.offset(va)
with tempfile.NamedTemporaryFile(delete=False, suffix=".bin") as chunk:
    chunk.write(pe.data[start:start + length])
try:
    out = subprocess.run([NDISASM, "-b", "32", "-o", str(va), chunk.name],
                         capture_output=True, text=True, check=True).stdout
finally:
    os.unlink(chunk.name)
for line in out.splitlines():
    parts = line.split(None, 2)
    print(parts[0], parts[2] if len(parts) > 2 else "")
