"""Per-match deep dive of a srp log: roster, lag, and every resync with the
evidence around it, classified."""
import collections
import json
import re
import sys

LOG = sys.argv[1]
FIRST = int(sys.argv[2], 16) if len(sys.argv) > 2 else 0
GEO = json.load(open("geo.json")) if len(sys.argv) < 4 else json.load(open(sys.argv[3]))

lines = open(LOG, encoding="utf-8", errors="ignore").read().splitlines()


def ts(l):
    m = re.match(r"(\d\d):(\d\d):(\d\d\.\d+)Z", l)
    return int(m[1]) * 3600 + int(m[2]) * 60 + float(m[3]) if m else None


def hms(t):
    return "%02d:%02d:%02d" % (t // 3600, t % 3600 // 60, t % 60)


names = {}
for l in lines:
    m = re.search(r"Session (\S+) joined the chat as (\S+)", l)
    if m:
        names[m[1]] = m[2]


class M:
    def __init__(s, mid, t, n):
        s.mid, s.formed, s.n = mid, t, n
        s.over = s.result = None
        s.slots = collections.defaultdict(lambda: dict(conns=[], loads=[], left=[], pings=[], late_rel=0, late_dis=0,
                                                       late_lines=[], toolate=0, toolate_lines=[], resyncs=[],
                                                       copies=[], bad_uploads=[], summary=[]))
        s.events = []  # (t, kind, p, data)
        s.trails = {}  # p -> list of (t, text)
        s.disagree = []


matches = {}
order = []
for l in lines:
    t = ts(l)
    if t is None:
        continue
    m = re.search(r"Match (0x[0-9a-f]+) formed for (\d+) players", l)
    if m:
        if int(m[1], 16) >= FIRST:
            matches[m[1]] = M(m[1], t, int(m[2]))
            order.append(m[1])
        continue
    m = re.search(r"Match (0x[0-9a-f]+) is over", l)
    if m and m[1] in matches:
        matches[m[1]].over = t
        continue
    m = re.search(r"Match (0x[0-9a-f]+): (.*)", l)
    if not m or m[1] not in matches:
        continue
    g, r = matches[m[1]], m[2]
    x = re.match(r"player (\d) \(session (\S+)\) connected from ([0-9.]+)", r)
    if x:
        g.slots[x[1]]["conns"].append((t, x[2], x[3]))
        continue
    x = re.match(r"player (\d) has loaded", r)
    if x:
        g.slots[x[1]]["loads"].append(t)
        continue
    x = re.match(r"player (\d) left; (.*)", r)
    if x:
        g.slots[x[1]]["left"].append(t)
        g.slots[x[1]]["summary"].append(x[2])
        continue
    x = re.match(r"player (\d)'s ping over the last 30 s: (\d+) ms on average,\s+(\d+) ms at most", r)
    if x:
        g.slots[x[1]]["pings"].append((int(x[2]), int(x[3])))
        continue
    x = re.match(r"player (\d): (\d+) discards \(0x58\) and (\d+) relays \(0x54\) reached it after its 500 ms horizon in \d+ ms, for ticks (\d+) to (\d+), the latest (\d+) ms past it", r)
    if x:
        s = g.slots[x[1]]
        s["late_dis"] += int(x[2]); s["late_rel"] += int(x[3])
        s["late_lines"].append((t, int(x[2]), int(x[3]), int(x[4]), int(x[5]), int(x[6])))
        continue
    x = re.match(r"player (\d): (\d+) inputs too late to relay in \d+ ms, the latest (\d+) ms past the match clock \(its clock (-?\d+) ms behind(?:, resynced (\d+) ms ago)?", r)
    if x:
        s = g.slots[x[1]]
        s["toolate"] += int(x[2])
        s["toolate_lines"].append((t, int(x[2]), int(x[3]), int(x[4]), int(x[5]) if x[5] else None))
        continue
    x = re.match(r"player (\d) is out of sync, wrong from (\d+) to (\d+) ms; resyncing it", r)
    if x:
        g.slots[x[1]]["resyncs"].append(dict(t=t, W=int(x[2]), W2=int(x[3])))
        continue
    x = re.match(r"player (\d) told to drop its own inputs for ticks (\d+) to (\d+) before the snapshot, but the (\d+) the others have", r)
    if x:
        rs = g.slots[x[1]]["resyncs"]
        if rs:
            rs[-1].update(drop=(int(x[2]), int(x[3])), kept=int(x[4]))
        continue
    x = re.match(r"player (\d) starts from the snapshot at (\d+) ms \((\d+) bytes\) at (\d+) ms, its clock at (\d+) ms", r)
    if x:
        rs = g.slots[x[1]]["resyncs"]
        if rs and "S" not in rs[-1] and t - rs[-1]["t"] < 30:
            rs[-1].update(S=int(x[2]), K=int(x[4]), C0=int(x[5]), ts=t)
        else:
            g.events.append((t, "start", x[1], (int(x[2]), int(x[4]), int(x[5]))))
        continue
    x = re.match(r"player (\d) sent (\d+) inputs while it loaded the snapshot \(ticks (\d+) to (\d+)\), (\d+) of them after", r)
    if x:
        rs = g.slots[x[1]]["resyncs"]
        if rs:
            rs[-1].update(loadU=int(x[2]), loadUafter=int(x[5]))
        continue
    x = re.match(r"player (\d) is in sync \((\d+) inputs since the snapshot\)", r)
    if x:
        rs = g.slots[x[1]]["resyncs"]
        if rs and "insync" not in rs[-1] and "ts" in rs[-1]:
            rs[-1]["insync"] = t
        continue
    x = re.match(r"player (\d) sent another input for (\d+) ms", r)
    if x:
        g.slots[x[1]]["copies"].append((t, int(x[2])))
        continue
    x = re.match(r"player (\d)'s snapshot at (\d+) ms has CRC \S+ the majority", r)
    if x:
        g.slots[x[1]]["bad_uploads"].append((t, int(x[2])))
        continue
    x = re.match(r"inputs for ticks (\d+) to (\d+), around the divergence of player (\d) .*?its own \[(.*?)\]; others: (.*)", r)
    if x:
        g.trails.setdefault(x[3], []).append((t, int(x[1]), int(x[2]), x[4], x[5]))
        continue
    x = re.match(r"over at (\d+) ms; winner (\d+), batteries (\[.*?\]), scores (\[.*?\])", r)
    if x:
        g.result = (int(x[1]), x[2], x[3], x[4])
        continue
    x = re.match(r"the clients disagree at (\d+) ms, no majority: (.*)", r)
    if x:
        g.disagree.append((t, int(x[1]), x[2]))
        continue
    x = re.match(r"player (\d) (entered at|is back in|timed out)(.*)", r)
    if x:
        g.events.append((t, x[2], x[1], x[3][:60]))
        continue


def who(g, p):
    s = g.slots[p]
    if not s["conns"]:
        return "slot %s (never connected)" % p
    _, sess, ip = s["conns"][-1]
    nm = [names.get(c[1], "?") for c in s["conns"]]
    nm = "/".join(sorted(set(nm)))
    return "slot %s %s (%s)" % (p, nm, GEO.get(ip, ip))


def parse_own(text):
    out = []
    for it in text.split():
        x = re.match(r"(\d+):([+-]\d+)([A-Z])", it)
        if x:
            out.append((int(x[1]), int(x[2]), x[3]))
    return out


def classify(g, p, i):
    """Why resync i of player p was needed."""
    s = g.slots[p]
    r = s["resyncs"][i]
    W = r["W"]
    prev = s["resyncs"][i - 1] if i else None
    why = []
    # Was it the previous resync failing: wrong from soon after its start?
    if prev and "C0" in prev and W - prev["C0"] < 15000:
        since = W - prev["C0"]
        why.append("REPEAT: wrong %d ms after the previous resync's start (C0 %d)" % (since, prev["C0"]))
        # pre-resync input that arrived after the sync and was relayed at/after C0
        tr = [x for x in g.trails.get(p, []) if abs(x[0] - r["t"]) < 1]
        if tr:
            own = parse_own(tr[0][3])
            L = prev["K"] - prev["C0"]
            inflight = [(tk, a, k) for tk, a, k in own if k in "R" and tk >= prev["C0"] and a < L and tk <= prev["K"] + 400]
            copies = [(tk, a, k) for tk, a, k in own if k == "C"]
            if inflight:
                why.append("pre-resync input(s) relayed after the sync at/after its new clock: %s" % inflight[:4])
            if copies:
                why.append("copies (merged into a kept own tick): %s" % copies[:4])
        cps = [c for c in s["copies"] if prev["ts"] <= c[0] <= r["t"]]
        if cps and not any("copies" in w for w in why):
            why.append("copy lines: %s" % cps[:3])
    # Late messages to it near the divergence
    late = [x for x in s["late_lines"] if r["t"] - 6 <= x[0] <= r["t"] and x[3] - 600 <= W and W <= x[4] + 1100]
    if late:
        why.append("late to it (0x58/0x54 past its horizon) for ticks %s" % ", ".join("%d-%d (%dD/%dR, +%d)" % (a, b, d, rr, pst) for _, d, rr, a, b, pst in late))
    # Its own inputs dropped as too late near W (0x58 back to it)
    tl = [x for x in s["toolate_lines"] if r["t"] - 4 <= x[0] <= r["t"]]
    if tl:
        why.append("own inputs too late to relay just before: %d (worst +%d, clock %s behind)" % (sum(x[1] for x in tl), max(x[2] for x in tl), tl[-1][3]))
    # Its upload disagreed with the majority
    bu = [x for x in s["bad_uploads"] if r["t"] - 20 <= x[0] <= r["t"] + 1]
    if bu:
        why.append("its upload at %s differed from the majority" % [x[1] for x in bu])
    tr = [x for x in g.trails.get(p, []) if abs(x[0] - r["t"]) < 1]
    if tr:
        own = parse_own(tr[0][3])
        d = [x for x in own if x[2] == "D"]
        lateown = [x for x in own if x[2] == "R" and x[1] > 200]
        others = tr[0][4]
        od = sum(int(y) for y in re.findall(r"(\d+) dropped", others))
        oldest = max([int(y) for y in re.findall(r"oldest \+(\d+)", others)] or [0])
        why.append("trail: own %d (D %d, R>200ms %d); others dropped %d, oldest others' relay +%d" % (len(own), len(d), len(lateown), od, oldest))
    return why


for mid in order:
    g = matches[mid]
    end = g.over or ts(lines[-1])
    print("=" * 100)
    print("Match %s formed %s, %d players, %s, %.1f min%s" % (mid, hms(g.formed), g.n, "over " + hms(g.over) if g.over else "RUNNING", (end - g.formed) / 60,
                                                         ("; result: at %d ms winner %s batteries %s scores %s" % g.result) if g.result else ""))
    for p in sorted(g.slots):
        s = g.slots[p]
        loads = []
        for (ct, sess, ip) in s["conns"]:
            ld = [x for x in s["loads"] if x >= ct]
            loads.append("%.1fs" % (ld[0] - ct) if ld else "NEVER")
        pg = s["pings"]
        ping = "ping avg %d max %d" % (sum(a for a, b in pg) / len(pg), max(b for a, b in pg)) if pg else "no ping"
        print("  %-48s conns %d load %s | %s | resyncs %d | own too late %d | late to it: %d 0x58 %d 0x54 | copies %d | left %d" % (
            who(g, p), len(s["conns"]), ",".join(loads), ping, len(s["resyncs"]), s["toolate"], s["late_dis"], s["late_rel"], len(s["copies"]), len(s["left"])))
    for e in sorted(g.events):
        if e[1] != "entered at":
            print("    event %s %s player %s %s" % (hms(e[0]), e[1], e[2], e[3]))
    for p in sorted(g.slots):
        s = g.slots[p]
        for i, r in enumerate(s["resyncs"]):
            print("  RESYNC %s %-30s wrong %d-%d" % (hms(r["t"]), who(g, p)[:30], r["W"], r["W2"]), end="")
            if "S" in r:
                print(" | S %d K %d C0 %d (lag %d, snap age %d) kept %s U %s" % (r["S"], r["K"], r["C0"], r["K"] - r["C0"], r["K"] - r["S"], r.get("kept"), r.get("loadUafter")), end="")
            print()
            for w in classify(g, p, i):
                print("        -", w)

# ---- summary by class
cls = collections.Counter()
per = collections.defaultdict(list)
alltimes = collections.defaultdict(list)
for mid in order:
    g = matches[mid]
    for p, s in g.slots.items():
        for r in s["resyncs"]:
            alltimes[mid].append((r["t"], p))
for mid in order:
    g = matches[mid]
    for p, s in g.slots.items():
        for i, r in enumerate(s["resyncs"]):
            why = classify(g, p, i)
            txt = " ".join(why)
            prev = s["resyncs"][i - 1] if i else None
            quick = prev and "C0" in prev and r["W"] - prev["C0"] < 1500
            paired = any(abs(t - r["t"]) < 0.6 and q != p for t, q in alltimes[mid])
            if "pre-resync input" in txt:
                c = "I in-flight input relayed after the sync (confirmed)"
            elif quick:
                c = "Q wrong within 1.5 s of the previous resync's start"
            elif "late to it" in txt:
                c = "L 0x54/0x58 reached it past its horizon"
            elif "REPEAT" in txt:
                c = "R wrong again 1.5-15 s after a resync, no late messages seen"
            elif paired:
                c = "P flagged together with another player, no late messages"
            else:
                c = "U first divergence, no evidence in the log"
            cls[c] += 1
            per[c].append("%s p%s %s" % (mid, p, who(g, p).split(" ", 2)[2][:28]))
print()
print("SUMMARY", sum(cls.values()), "resyncs")
for c, n in sorted(cls.items()):
    print("  %3d %s" % (n, c))
    print("        ", "; ".join(per[c][:40]))
