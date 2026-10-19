import sys,re
lines=open(sys.argv[1],encoding='utf-8').read().splitlines()
start=[i for i,l in enumerate(lines) if 'type="SymbolClassTag"' in l][0]
i=start; tags=[]; names=[]; cur=None
while True:
    l=lines[i].strip()
    if l=='<tags>': cur=tags
    elif l=='<names>': cur=names
    elif l in('</tags>','</names>'): cur=None
    elif l=='</item>' and i>start: break
    elif cur is not None:
        m=re.match(r'<item>(.*)</item>',l)
        if m: cur.append(m.group(1))
    i+=1
for t,n in zip(tags,names): print(t,n)
