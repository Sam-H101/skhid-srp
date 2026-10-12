"""Entries after the match start: first-time late joins vs rejoins of a slot
that left earlier, and whether the player was wrong within 1 s of entering.
Usage: rejoins.py LOG..."""
import collections
import re
import sys

table = collections.Counter()
for path in sys.argv[1:]:
    state = {}
    for line in open(path, errors="ignore"):
        m = re.search(r"Match (0x[0-9a-f]+) formed", line)
        if m:
            state[m[1]] = {"left": set(), "first": None, "open": {}}
            continue
        m = re.search(r"Match (0x[0-9a-f]+): player (\d) (entered at (\d+)|left;|is out of sync, wrong from (\d+))", line)
        if not m or m[1] not in state:
            continue
        s, p = state[m[1]], m[2]
        if m[4]:
            at = int(m[4])
            if s["first"] is None:
                s["first"] = at
            if at > s["first"] + 15000 or p in s["left"]:
                kind = "rejoin" if p in s["left"] else "late first join"
                s["open"][p] = (kind, at, line[:12])
        elif m[3] == "left;":
            s["left"].add(p)
            if p in s["open"]:
                kind, at, t = s["open"].pop(p)
                table[(kind, "no desync before leaving")] += 1
                print(path[-28:], m[1], p, kind, t, at, "left without desync")
        else:
            if p in s["open"]:
                kind, at, t = s["open"].pop(p)
                wrong = int(m[5])
                quick = wrong - at < 1000
                table[(kind, "wrong within 1 s" if quick else "wrong later")] += 1
                print(path[-28:], m[1], p, kind, t, "entered", at, "wrong from", wrong, "+%d" % (wrong - at))
    for mid, s in state.items():
        for p, (kind, at, t) in s["open"].items():
            table[(kind, "no desync seen")] += 1
            print(path[-28:], mid, p, kind, t, at, "no desync seen")
print()
for k, v in sorted(table.items()):
    print(k, v)
