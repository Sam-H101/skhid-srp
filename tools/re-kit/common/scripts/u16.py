import sys
d=open(sys.argv[1],'rb').read()
try: s=d.decode('utf-16')
except Exception: s=d.decode('utf-8','replace')
open(sys.argv[2],'w',encoding='utf-8').write(s.replace('\x00',''))
