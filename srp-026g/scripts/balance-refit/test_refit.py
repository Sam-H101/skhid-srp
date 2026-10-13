"""Run: python -I scripts/balance-refit/test_refit.py   (no packages needed)"""
import math
import random
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import refit  # noqa: E402


def line(t, body):
    return f"{t}Z [lobby] [INFO] {body}"


def player(name, p, g, l, rating, m):
    v = p + g + l * 2 + 4 * m / (m + 5) * rating
    return f"{name} {v:.1f} (gear {g}, level {l}, rating {rating:+.2f} of {m} matches)"


class Refit(unittest.TestCase):
    def test_parse_pair_and_check(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "srp-2026-10-13.log"
            log.write_text(
                line("12:00:00.000", "Match 0xa sides: team 0 counts 30.0: "
                     + player("A, B (x)", 10, 3, 2, 0.5, 4) + " | team 1 counts 20.0: "
                     + player("C", 10, 1, 1, 0.0, 0)) + "\n"
                + "12:30:00.000Z [srp] [INFO] Match 0xa settled (TimeLimit, winner Some(1), 2 players): x\n"
                + line("12:40:00.000", "Match 0xb sides: team 0 counts 1.0: " + player("D", 10, 0, 0, 0, 0)
                       + " | team 1 counts 1.0: " + player("E", 10, 0, 0, 0, 0)) + "\n"
                + line("12:50:00.000", "Match 0xb late join: X 12.0 to team 1, with them team 0 counts 1.0") + "\n"
                + "13:00:00.000Z [srp] [INFO] Match 0xb settled (ModeRule, winner Some(2), 2 players): x\n",
                encoding="utf-8")
            ms = list(refit.read_matches([d], "", ""))
            self.assertEqual([m["id"] for m in ms], ["0xa", "0xb"])
            a = ms[0]
            self.assertEqual(a["winner"], 1)
            self.assertEqual((len(a["sides"][0]), len(a["sides"][1])), (1, 1))  # a name with ", " and "(" stays one player
            self.assertTrue(refit.explained_by(a, refit.IN_FORCE, 5.0))
            self.assertFalse(refit.explained_by(a, (12.0, 1.0, 1.5, 4.0), 5.0))
            self.assertEqual(ms[1]["winner"], 2)
            self.assertEqual([m["late"] for m in ms], [False, True])
            # since filter
            self.assertEqual(len(list(refit.read_matches([d], "2026-10-13T12:35", ""))), 1)

    def test_features_and_trust(self):
        side = [dict(value=0, gear=3, level=2, rating=1.0, matches=5), dict(value=0, gear=1, level=1, rating=-1.0, matches=0)]
        self.assertEqual(refit.side_features(side, 5.0), (2, 4, 3, 0.5))

    def test_fit_recovers_known_weights(self):
        rnd = random.Random(7)
        true = [0.0, 1.0, 0.8, 1.5]  # per unit gear = 1.0
        rows = []
        for _ in range(1500):
            x = [rnd.choice([-1, 0, 0, 1]), rnd.randint(-6, 6), rnd.randint(-5, 5), rnd.uniform(-2, 2)]
            d = 0.4 * sum(t * v for t, v in zip(true, x))
            rows.append((x, 1 if rnd.random() < refit.sigmoid(d) else 0))
        w = refit.fit_logistic(rows)
        scaled = [v / w[1] for v in w]
        self.assertAlmostEqual(scaled[2], 0.8, delta=0.25)
        self.assertAlmostEqual(scaled[3], 1.5, delta=0.5)

    def test_scoring_prefers_true_direction(self):
        rnd = random.Random(3)
        feats, ys = [], []
        for _ in range(800):
            x = [0, rnd.randint(-6, 6), rnd.randint(-5, 5), 0.0]
            feats.append(x)
            ys.append(1 if rnd.random() < refit.sigmoid(0.5 * x[2]) else 0)
        good, _, _ = refit.score_weights(feats, ys, (0, 0, 1, 0))
        bad, _, _ = refit.score_weights(feats, ys, (0, 1, 0, 0))
        self.assertLess(good, bad)
        self.assertLess(good, math.log(2))


if __name__ == "__main__":
    unittest.main()
