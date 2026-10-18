# list SymbolClass entries, fonts (DefineFont2/3 + DefineFontName) and, for given ids, tag info
import sys, struct, zlib
d=open(sys.argv[1],'rb').read()
if d[:3] in (b'CWS',b'CFX'): d=d[:8]+zlib.decompress(d[8:])
nb=d[8]>>3; rb=(5+4*nb+7)//8; p=8+rb+4
def tags(buf,p,end):
    while p<end:
        h=struct.unpack_from('<H',buf,p)[0]; p+=2; code=h>>6; ln=h&63
        if ln==63: ln=struct.unpack_from('<I',buf,p)[0]; p+=4
        yield code,p,ln; p+=ln
        if code==0: break
mode=sys.argv[2]
for code,q,ln in tags(d,p,len(d)):
    if code==76 and mode=='sym':
        n=struct.unpack_from('<H',d,q)[0]; r=q+2
        for i in range(n):
            cid=struct.unpack_from('<H',d,r)[0]; r+=2; e=d.index(b'\0',r); print(cid,d[r:e].decode('latin1')); r=e+1
    if code in (48,75) and mode=='fonts':
        fid=struct.unpack_from('<H',d,q)[0]; flags=d[q+2]; nl=d[q+4]; name=d[q+5:q+5+nl].decode('latin1')
        ng=struct.unpack_from('<H',d,q+5+nl)[0]
        print('font',fid,'DefineFont%d'%(2 if code==48 else 3),repr(name),'bold' if flags&1 else '','italic' if flags&2 else '','glyphs',ng,'layout' if flags&0x80 else '')
    if code==88 and mode=='fonts':
        fid=struct.unpack_from('<H',d,q)[0]; e=d.index(b'\0',q+2); print('  fontname',fid,d[q+2:e].decode('latin1'))
