"""Map addresses to the function labels of the dumpbin disassembly."""
import bisect, re, sys

path = sys.argv[1]
addrs = [int(a, 16) for a in sys.argv[2:]]
starts, names = [], []
label = None
for line in open(path, encoding='latin-1'):
    if line and not line[0].isspace() and line.rstrip().endswith(':'):
        label = line.rstrip()[:-1]
        continue
    if label is not None:
        m = re.match(r'\s+([0-9A-F]{8}):', line)
        if m:
            starts.append(int(m.group(1), 16)); names.append(label); label = None
for a in addrs:
    i = bisect.bisect_right(starts, a) - 1
    print(f'{a:#08x} {names[i][:150]} +{a - starts[i]:#x}')
