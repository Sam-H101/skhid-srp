import sys,struct,re
sys.path.insert(0,'.')
from sdis import *
# find string(s) and code refs to them
text=[s for s in secs if s[0]==0x11000][0]
tb=raw[text[2]:text[2]+text[1]]
def find_str(s):
    out=[]
    b=s.encode()+b'\0'
    for s0,rs,off in secs:
        if s0==0x11000: continue
        data=raw[off:off+rs]
        for m in re.finditer(re.escape(b),data):
            va=s0+m.start()
            if m.start()>0 and data[m.start()-1]!=0: continue
            out.append(va)
    return out
def refs(va):
    p=struct.pack('<I',va); out=[]
    i=tb.find(p)
    while i>=0:
        out.append(text[0]+i); i=tb.find(p,i+1)
    return out
for s in sys.argv[1:]:
    if s.startswith('0x'):
        print(s, [hex(r) for r in refs(int(s,16))]); continue
    for va in find_str(s):
        print(repr(s), hex(va), [hex(r) for r in refs(va)])
