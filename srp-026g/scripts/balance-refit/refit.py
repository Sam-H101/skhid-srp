#!/usr/bin/env python3
"""Refit the team-balance weights (matchmaking.md section 8) from server logs.

Reads the "Match 0x.. sides: .." lines (the roster each side started with,
what every player counted for) and the "Match 0x.. settled (.., winner
Some(N), ..)" lines (who won), pairs them by match id, and answers:

  1. Which weights would have predicted the winners best (logistic fit)?
  2. How do the weights in force (and any you name) score on the same matches?

A side's value is  P*players + G*gear + L*level + R*trust*rating, the same
sum the server makes (crates/session/src/v026g/balance.rs), with
trust = matches / (matches + trust_matches). The server's logged per-player
numbers are not used, only their parts, so any weights can be tried.

Usage:
  refit.py LOG [LOG ...]                       # files or directories of srp-*.log
  refit.py logs/ --since 2026-10-13T21:31      # only matches made after that UTC time
  refit.py logs/ --weights 12,1,1.5,4 --weights 10,1,2,4
  refit.py logs/ --strict                      # only matches whose roster did not change

Needs only Python 3. Prints no player names.

Reading the result: with few matches the fit is noisy; the bootstrap range
says how much. Do not change a setting unless the range for it excludes the
value in force AND the log loss of the fit beats the weights in force by a
clear margin over several dozen matches.
"""
import argparse
import math
import random
import re
import sys
from pathlib import Path

LINE = re.compile(r"^(\d\d:\d\d:\d\d(?:\.\d+)?)Z ")
SIDES = re.compile(r"Match (0x[0-9a-f]+) sides: team 0 counts [\d.]+: (.*) \| team 1 counts [\d.]+: (.*)$")
SETTLED = re.compile(r"Match (0x[0-9a-f]+) settled \((\w+), winner (Some\((\d+)\)|None), (\d+) players\)")
PLAYER = re.compile(
    r"(?:^|, )(.*?) (-?\d+\.\d+) \(gear (\d+), level (\d+), rating ([+-]\d+\.\d+) of (\d+) matches\)"
)

NAMES = ("players", "gear", "level", "rating")
# The weights in force on the players' server since 2026-10-13 21:31 UTC.
IN_FORCE = (10.0, 1.0, 2.0, 4.0)


def parse_side(text):
    return [
        dict(value=float(v), gear=int(g), level=int(l), rating=float(r), matches=int(m))
        for _, v, g, l, r, m in PLAYER.findall(text)
    ]


def read_matches(paths, since, until):
    """Yield dicts: id, sides=[[player..],[player..]], winner (0/1/None), settled_players."""
    files = []
    for p in paths:
        p = Path(p)
        files += sorted(p.glob("srp-*.log")) if p.is_dir() else [p]
    sides, settled = {}, {}
    for f in files:
        day = re.search(r"(\d{4}-\d\d-\d\d)", f.name)
        day = day.group(1) if day else ""
        for line in f.read_text(encoding="utf-8", errors="replace").splitlines():
            t = LINE.match(line)
            stamp = f"{day}T{t.group(1)}" if t else ""
            m = SIDES.search(line)
            if m:
                if (since and stamp < since) or (until and stamp > until):
                    continue
                sides.setdefault(m.group(1), [parse_side(m.group(2)), parse_side(m.group(3))])
                continue
            m = SETTLED.search(line)
            if m:
                win = int(m.group(4)) if m.group(4) is not None else None
                settled[m.group(1)] = (win, int(m.group(5)))
    for mid, s in sides.items():
        if mid in settled:
            yield dict(id=mid, sides=s, winner=settled[mid][0], settled_players=settled[mid][1])


def explained_by(match, weights, trust_matches, tol=0.08):
    """Does the logged per-player value equal P + G*gear + L*level + R*trust*rating?
    (tol covers the log's rounding: values to 0.1, ratings to 0.01.)"""
    p, g, l, r = weights
    return all(
        abs(p + g * q["gear"] + l * q["level"]
            + r * q["matches"] / (q["matches"] + trust_matches) * q["rating"] - q["value"]) <= tol
        for side in match["sides"] for q in side
    )


def side_features(players, trust_matches):
    """(players, gear, level, trust*rating) summed over a side."""
    n = len(players)
    g = sum(p["gear"] for p in players)
    l = sum(p["level"] for p in players)
    r = sum(p["matches"] / (p["matches"] + trust_matches) * p["rating"] for p in players)
    return (n, g, l, r)


def sigmoid(x):
    return 1 / (1 + math.exp(-x)) if x >= 0 else math.exp(x) / (1 + math.exp(x))


def solve(a, b):
    """Gaussian elimination for a small dense system a x = b."""
    n = len(b)
    m = [row[:] + [b[i]] for i, row in enumerate(a)]
    for i in range(n):
        piv = max(range(i, n), key=lambda r: abs(m[r][i]))
        m[i], m[piv] = m[piv], m[i]
        if abs(m[i][i]) < 1e-12:
            return None
        for r in range(i + 1, n):
            f = m[r][i] / m[i][i]
            for c in range(i, n + 1):
                m[r][c] -= f * m[i][c]
    x = [0.0] * n
    for i in range(n - 1, -1, -1):
        x[i] = (m[i][n] - sum(m[i][c] * x[c] for c in range(i + 1, n))) / m[i][i]
    return x


def fit_logistic(rows, ridge=0.05):
    """rows: (features diff [k], y in {0,1}). No intercept (sides are symmetric).
    Maximises the log likelihood with a small ridge; Newton's method."""
    k = len(rows[0][0])
    w = [0.0] * k
    for _ in range(60):
        grad = [-ridge * wi for wi in w]
        hess = [[ridge if i == j else 0.0 for j in range(k)] for i in range(k)]
        for x, y in rows:
            p = sigmoid(sum(wi * xi for wi, xi in zip(w, x)))
            for i in range(k):
                grad[i] += (y - p) * x[i]
                for j in range(k):
                    hess[i][j] += p * (1 - p) * x[i] * x[j]
        step = solve(hess, grad)
        if step is None:
            return None
        w = [wi + si for wi, si in zip(w, step)]
        if max(abs(s) for s in step) < 1e-9:
            break
    return w


def log_loss(ps, ys):
    return -sum(math.log(max(min(p, 1 - 1e-12), 1e-12)) if y else math.log(max(min(1 - p, 1 - 1e-12), 1e-12)) for p, y in zip(ps, ys)) / len(ys)


def fit_scale(ds, ys):
    """Best k for p = sigmoid(k*d), one weight, Newton."""
    k = 0.05
    for _ in range(60):
        g = h = 0.0
        for d, y in zip(ds, ys):
            p = sigmoid(k * d)
            g += (y - p) * d
            h += p * (1 - p) * d * d
        if h < 1e-12:
            break
        step = g / h
        k += step
        if abs(step) < 1e-10:
            break
    return k


def score_weights(feats, ys, w):
    """feats: per match diff vectors (players, gear, level, rating). Returns loss, accuracy, scale."""
    ds = [sum(wi * xi for wi, xi in zip(w, x)) for x in feats]
    k = fit_scale(ds, ys)
    ps = [sigmoid(k * d) for d in ds]
    decided = [(d, y) for d, y in zip(ds, ys) if abs(d) > 1e-9]
    acc = sum(1 for d, y in decided if (d > 0) == (y == 1)) / len(decided) if decided else float("nan")
    return log_loss(ps, ys), acc, k


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("logs", nargs="+")
    ap.add_argument("--since", default="", help="UTC, e.g. 2026-10-13T21:31 (matches made at or after)")
    ap.add_argument("--until", default="", help="UTC, matches made at or before")
    ap.add_argument("--trust-matches", type=float, default=5.0, help="SRP_BALANCE_TRUST_MATCHES (5)")
    ap.add_argument("--weights", action="append", default=[], help="P,G,L,R to score, e.g. 12,1,1.5,4 (repeatable)")
    ap.add_argument("--strict", action="store_true", help="only matches whose settled player count equals the roster")
    ap.add_argument("--bootstrap", type=int, default=300, help="resamples for the ranges (0 = off)")
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()

    ms = list(read_matches(args.logs, args.since, args.until))
    total = len(ms)
    ms = [m for m in ms if m["winner"] in (0, 1)]
    draws = total - len(ms)
    if args.strict:
        ms = [m for m in ms if m["settled_players"] == len(m["sides"][0]) + len(m["sides"][1])]
    if len(ms) < 8:
        sys.exit(f"{len(ms)} usable matches ({total} paired, {draws} without a winner 0/1): too few to fit.")

    feats, ys = [], []
    for m in ms:
        a = side_features(m["sides"][0], args.trust_matches)
        b = side_features(m["sides"][1], args.trust_matches)
        feats.append([x - y for x, y in zip(a, b)])
        ys.append(1 if m["winner"] == 0 else 0)

    print(f"{len(ms)} matches used ({total} paired by id, {draws} dropped for no winner 0/1"
          f"{', strict roster' if args.strict else ''}); team 0 won {sum(ys)}.")
    known = [IN_FORCE, (12.0, 1.0, 1.5, 4.0)]
    odd = sum(1 for m in ms if not any(explained_by(m, w, args.trust_matches) for w in known))
    if odd:
        print(f"NOTE: {odd} of {len(ms)} matches were made with weights other than the two known sets "
              "(in force / 12,1,1.5,4) or another trust constant; their parts are still used as logged.")
    print(f"Null model (every match a coin): log loss {math.log(2):.4f}\n")

    # 1. Free fit.
    if len(ms) < 40:
        print(f"WARNING: {len(ms)} matches. The free fit below is not usable yet (4 weights from "
              "so few results swing wildly); read only the scored weights at the end.\n")
    w = fit_logistic([(x, y) for x, y in zip(feats, ys)])
    if w is None or w[1] <= 0:
        print("Free fit: gear has no positive pull in these matches; weights below are not meaningful.")
    if w is not None:
        ds = [sum(wi * xi for wi, xi in zip(w, x)) for x in feats]
        loss = log_loss([sigmoid(d) for d in ds], ys)
        print(f"Free fit (log loss {loss:.4f}, in sample):")
        if w[1] > 0:
            scaled = [wi / w[1] for wi in w]  # gear = 1, the server's unit
            print("  in server units (gear = 1): " + ", ".join(f"{n} {v:+.2f}" for n, v in zip(NAMES, scaled)))
            print("  in force:                   " + ", ".join(f"{n} {v:+.2f}" for n, v in zip(NAMES, IN_FORCE)))
        else:
            print("  raw: " + ", ".join(f"{n} {v:+.3f}" for n, v in zip(NAMES, w)))
        if args.bootstrap and w[1] > 0:
            rnd = random.Random(args.seed)
            boots = [[] for _ in NAMES]
            n = len(feats)
            for _ in range(args.bootstrap):
                idx = [rnd.randrange(n) for _ in range(n)]
                wb = fit_logistic([(feats[i], ys[i]) for i in idx])
                if wb is None or wb[1] <= 0.02:
                    continue
                for j, v in enumerate(wb):
                    boots[j].append(v / wb[1])
            print(f"  90% range over {len(boots[0])} resamples (gear = 1; wide = not enough matches):")
            for nme, vals, cur in zip(NAMES, boots, IN_FORCE):
                if nme == "gear" or len(vals) < 20:
                    continue
                vals.sort()
                lo, hi = vals[int(0.05 * len(vals))], vals[int(0.95 * len(vals)) - 1]
                note = "" if lo <= cur <= hi else "   <- in force is outside"
                print(f"    {nme:8s} {lo:+.2f} .. {hi:+.2f}   (in force {cur:g}){note}")
        print()

    # 2. Named weights. Each gets its own best scale k, so only the *direction* of
    #    the weights is compared, not how confident they are.
    sets = [("in force", IN_FORCE), ("before 2026-10-13 21:31", (12.0, 1.0, 1.5, 4.0))]
    for s in args.weights:
        vals = tuple(float(v) for v in s.split(","))
        if len(vals) != 4:
            sys.exit("--weights wants four numbers P,G,L,R")
        sets.append((s, vals))
    sets += [("gear only", (0.0, 1.0, 0.0, 0.0)), ("level only", (0.0, 0.0, 1.0, 0.0)),
             ("rating only", (0.0, 0.0, 0.0, 1.0)), ("players only", (1.0, 0.0, 0.0, 0.0))]
    print("Weights scored on the same matches (lower log loss is better; accuracy skips ties):")
    print(f"  {'weights':28s} {'log loss':>9s} {'accuracy':>9s}")
    for label, v in sets:
        loss, acc, _ = score_weights(feats, ys, v)
        tag = label if label.startswith(("in force", "before", "gear", "level", "rating", "players")) else "P,G,L,R = " + label
        print(f"  {tag:28s} {loss:9.4f} {acc:9.1%}")
    print("\nA difference under about 0.01 in log loss over fewer than ~60 matches is noise.")
    print("Matches that were made close to even have little to teach: if every split was\n"
          "within a point or two, the fit mostly sees luck. Look at the bootstrap range first.")


if __name__ == "__main__":
    main()
