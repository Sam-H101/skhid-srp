import sys,struct
sys.path.insert(0,'.')
from sdis import *
text=[s for s in secs if s[0]==0x11000][0]
tb=raw[text[2]:text[2]+text[1]]
for t in sys.argv[1:]:
    t=int(t,16); out=[]
    for i in range(len(tb)-5):
        if tb[i] in (0xe8,0xe9):
            r=struct.unpack_from('<i',tb,i+1)[0]
            if text[0]+i+5+r==t: out.append(hex(text[0]+i))
    print(hex(t),out)
