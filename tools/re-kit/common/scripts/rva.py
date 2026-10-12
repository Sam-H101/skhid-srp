import sys,struct
d=open(sys.argv[1],'rb').read()
pe=struct.unpack_from('<I',d,0x3c)[0]
n=struct.unpack_from('<H',d,pe+6)[0]; osz=struct.unpack_from('<H',d,pe+20)[0]
base=struct.unpack_from('<I',d,pe+24+28)[0]
secs=[]
o=pe+24+osz
for i in range(n):
    name,vs,va,rs,ro=struct.unpack_from('<8sIIII',d,o+i*40)[:5]
    secs.append((va,vs,rs,ro))
def rd(v,sz):
    r=v-base
    for va,vs,rs,ro in secs:
        if va<=r<va+max(vs,rs): return d[ro+r-va:ro+r-va+sz]
for a in sys.argv[2:]:
    t,v=a.split(':'); v=int(v,16)
    b=rd(v,8)
    if t=='f': print(hex(v),struct.unpack('<f',b[:4])[0])
    elif t=='d': print(hex(v),struct.unpack('<d',b)[0])
    else: print(hex(v),struct.unpack('<I',b[:4])[0])
