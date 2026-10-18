# minimal SWF tag walker: list tags of a sprite (raw), with code and length
import sys, struct, zlib
d=open(sys.argv[1],'rb').read()
sig=d[:3]
if sig==b'CWS' or sig==b'CFX': d=d[:8]+zlib.decompress(d[8:])
nb=d[8]>>3; rb=(5+4*nb+7)//8; p=8+rb+4
def tags(buf,p,end):
    while p<end:
        h=struct.unpack_from('<H',buf,p)[0]; p+=2; code=h>>6; ln=h&63
        if ln==63: ln=struct.unpack_from('<I',buf,p)[0]; p+=4
        yield code,p,ln; p+=ln
        if code==0: break
want=int(sys.argv[2])
for code,q,ln in tags(d,p,len(d)):
    if code==39 and struct.unpack_from('<H',d,q)[0]==want:
        print('sprite',want,'frames',struct.unpack_from('<H',d,q+2)[0])
        for c2,q2,l2 in tags(d,q+4,q+ln):
            print(' tag',c2,'len',l2, d[q2:q2+min(l2,64)].hex())
        break
print('sig',sig)
