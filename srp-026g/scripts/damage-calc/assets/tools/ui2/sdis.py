import sys, struct
from capstone import *
EXE='/home/sheep/games/Survarium-0.26g-original/Survarium/game/binaries/x86/survarium.exe'
raw=open(EXE,'rb').read()
pe=struct.unpack_from('<I',raw,0x3c)[0]
cnt=struct.unpack_from('<H',raw,pe+6)[0]; osz=struct.unpack_from('<H',raw,pe+20)[0]
secs=[]
for i in range(cnt):
    e=pe+24+osz+i*40
    vs,rva,rs,off=struct.unpack_from('<4I',raw,e+8)
    secs.append((0x10000+rva,rs,off))
def rd(va,n):
    for s,rs,off in secs:
        if s<=va<s+rs: return raw[off+va-s:off+va-s+n]
    return None
def f32(va): return struct.unpack('<f',rd(va,4))[0]
def u32(va): return struct.unpack('<I',rd(va,4))[0]
def cstr(va):
    b=rd(va,200); return b.split(b'\0')[0].decode('latin1')
if __name__=='__main__':
    va=int(sys.argv[1],16); n=int(sys.argv[2]) if len(sys.argv)>2 else 400
    md=Cs(CS_ARCH_X86,CS_MODE_32); md.detail=False
    code=rd(va,n*8)
    k=0
    for ins in md.disasm(code,va):
        note=''
        import re
        for m in re.findall(r'0x[0-9a-f]{6,8}',ins.op_str):
            a=int(m,16)
            if 0x870000<=a<0xa60000:
                try:
                    s=cstr(a)
                    if len(s)>=3 and all(32<=ord(c)<127 for c in s): note+=f' ; "{s[:60]}"'
                    else: note+=f' ; f={f32(a):g} u={u32(a):#x}'
                except Exception: pass
        print(f'{ins.address:08x}: {ins.mnemonic} {ins.op_str}{note}')
        k+=1
        if k>=n: break
        if ins.mnemonic in('ret',) and len(sys.argv)>3: break
