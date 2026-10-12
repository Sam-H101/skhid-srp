import struct, sys
d = open(sys.argv[1], 'rb').read()
pe = struct.unpack_from('<I', d, 0x3c)[0]
nsec = struct.unpack_from('<H', d, pe + 6)[0]
optsz = struct.unpack_from('<H', d, pe + 20)[0]
base = struct.unpack_from('<I', d, pe + 24 + 28)[0]
print('image base %#x' % base)
for i in range(nsec):
    o = pe + 24 + optsz + 40 * i
    name = d[o:o+8].rstrip(b'\0').decode()
    vsz, va, rsz, raw = struct.unpack_from('<IIII', d, o + 8)
    print(name, 'va %#x vsz %#x raw %#x rsz %#x' % (va, vsz, raw, rsz))
