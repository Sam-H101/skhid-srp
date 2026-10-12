import struct, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))
from snapdiff import split
for f in sys.argv[2:]:
    d = Path(f).read_bytes()
    _, secs, _, _ = split(d)
    s = [x for x in secs if x[0] == sys.argv[1]][0]
    st = d[s[1]:s[2]]
    print(Path(f).name[10:40], "hex 0-40:", st[:40].hex(" "))
    print("   @15..27 f32:", [round(struct.unpack_from("<f", st, o)[0], 5) for o in (15, 19, 23)], "| @296..312:", [round(struct.unpack_from("<f", st, o)[0], 5) for o in range(292, 316, 4)])
