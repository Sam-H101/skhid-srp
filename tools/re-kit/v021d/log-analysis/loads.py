import re, sys, statistics
def ts(l):
    m = re.match(r"(?:\S+\s+\|\s+)?(\d\d):(\d\d):(\d\d\.\d+)Z", l)
    return int(m[1])*3600+int(m[2])*60+float(m[3]) if m else None
allloads = []
for path in sys.argv[1:]:
    log = open(path, encoding="utf-8", errors="ignore").read().splitlines()
    pend = {}
    names = {}
    for l in log:
        m = re.search(r"Session (\S+) joined the chat as (\S+)", l)
        if m: names[m[1]] = m[2]
        t = ts(l)
        m = re.search(r"Match (\S+): player (\d) \(session (\S+)\) connected from ([0-9.]+)", l)
        if m: pend[(m[1], m[2])] = (t, m[3], m[4]); continue
        m = re.search(r"Match (\S+): player (\d) has loaded", l)
        if m and (m[1], m[2]) in pend:
            c = pend.pop((m[1], m[2])); allloads.append(t - c[0])
            if t - c[0] > 8: print("SLOW", path[-22:], m[1], m[2], names.get(c[1], c[1]), c[2], "%.1fs" % (t - c[0]))
            continue
        m = re.search(r"Match (\S+): player (\d) (timed out|left;.*)", l)
        if m and (m[1], m[2]) in pend:
            c = pend.pop((m[1], m[2]))
            print("NEVER LOADED", path[-22:], m[1], "slot", m[2], names.get(c[1], c[1]), c[2], "after %.1fs" % (t - c[0]), m[3][:140])
s = sorted(allloads)
print("loads:", len(s), "median %.2f p90 %.2f p99 %.2f max %.2f" % (s[len(s)//2], s[int(len(s)*.9)], s[int(len(s)*.99)], s[-1]))
