# summarise top-level tags of an ffdec swf2xml dump for given character ids
import sys, xml.etree.ElementTree as ET
path=sys.argv[1]; ids=set()
for a in sys.argv[2:]:
    if '-' in a: lo,hi=map(int,a.split('-')); ids.update(range(lo,hi+1))
    else: ids.add(int(a))
depth=0
def cid(e):
    for k in ('spriteId','shapeId','characterID','characterId','fontId','buttonId','fontID'):
        if k in e.attrib: return int(e.attrib[k])
    return None
def mat(m):
    if m is None: return ''
    a=m.attrib
    s=f"tx={int(a['translateX'])/20:g} ty={int(a['translateY'])/20:g}"
    if a.get('hasScale')=='true': s+=f" sx={a['scaleX']} sy={a['scaleY']}"
    if a.get('hasRotate')=='true': s+=f" r0={a['rotateSkew0']} r1={a['rotateSkew1']}"
    return s
def cx(c):
    if c is None: return ''
    a=c.attrib; s=' cx['
    if a.get('hasMultTerms')=='true': s+=f"mul r{a['redMultTerm']} g{a['greenMultTerm']} b{a['blueMultTerm']} a{a.get('alphaMultTerm')} "
    if a.get('hasAddTerms')=='true': s+=f"add r{a['redAddTerm']} g{a['greenAddTerm']} b{a['blueAddTerm']} a{a.get('alphaAddTerm')}"
    return s+']'
stack=[]
for ev,e in ET.iterparse(path,events=('start','end')):
    if ev=='start': stack.append(e); continue
    stack.pop()
    if len(stack)==2 and e.tag=='item':  # top-level tag item under swf/tags
        i=cid(e); t=e.attrib.get('type')
        if i in ids:
            out=[f"#{i} {t}"]
            for k in ('bitmapWidth','bitmapHeight','frameCount','fontName','className'):
                if k in e.attrib: out.append(f"{k}={e.attrib[k]}")
            b=e.find('shapeBounds') or e.find('bounds')
            if b is not None: out.append("bounds x[%g..%g] y[%g..%g]"%(int(b.attrib['Xmin'])/20,int(b.attrib['Xmax'])/20,int(b.attrib['Ymin'])/20,int(b.attrib['Ymax'])/20))
            print(' '.join(out))
            if t and 'Shape' in t:
                for fs in e.iter('item'):
                    if fs.attrib.get('type')=='FILLSTYLE':
                        print('   fill type',fs.attrib.get('fillStyleType'),'bitmap',fs.attrib.get('bitmapId'),mat(fs.find('bitmapMatrix')), 'color', [ (c.attrib) for c in fs.findall('color')])
            if t=='DefineSpriteTag':
                fr=1
                for st in e.find('subTags'):
                    tt=st.attrib.get('type')
                    if tt=='ShowFrameTag': fr+=1; continue
                    if tt in('PlaceObject2Tag','PlaceObject3Tag'):
                        print(f"   f{fr} d{st.attrib.get('depth')} ch{st.attrib.get('characterId') if st.attrib.get('placeFlagHasCharacter')=='true' else '-'} {st.attrib.get('name','')} {mat(st.find('matrix'))}{cx(st.find('colorTransform'))}", 'move' if st.attrib.get('placeFlagMove')=='true' else '')
                    elif tt=='FrameLabelTag': print(f"   f{fr} label {st.attrib.get('name')}")
                    elif tt=='RemoveObject2Tag': print(f"   f{fr} remove d{st.attrib.get('depth')}")
                    else: print(f"   f{fr} {tt}")
        if len(stack)==2: stack[1].remove(e) if e in list(stack[1]) else None
        e.clear()
