"""fn.py VERSION ADDR|NAME : the function at an address, or the functions
whose name contains NAME, from ../../vVERSION/functions.txt (made from the PDB
by ../pdb-procs; a build without a PDB has none until functions are matched)."""
import bisect
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


class Functions:
    def __init__(self, rows):
        self.rows = sorted(rows)
        self.starts = [r[0] for r in self.rows]

    @classmethod
    def load(cls, version):
        path = os.path.join(HERE, "..", "..", f"v{version}", "functions.txt")
        if not os.path.exists(path):
            return None
        rows = []
        for line in open(path, encoding="utf-8", errors="replace"):
            address, length, name = line.rstrip("\n").split(" ", 2)
            rows.append((int(address, 16), int(length, 16), name))
        return cls(rows)

    def at(self, va):
        i = bisect.bisect_right(self.starts, va) - 1
        if i < 0:
            return ""
        start, length, name = self.rows[i]
        inside = "" if va < start + length else " (past its end)"
        return f"{name} +{va - start:#x}{inside}"

    def named(self, part):
        return [r for r in self.rows if part in r[2]]


if __name__ == "__main__":
    functions = Functions.load(sys.argv[1])
    if functions is None:
        sys.exit(f"no functions.txt for {sys.argv[1]}")
    query = sys.argv[2]
    try:
        print(functions.at(int(query, 16)))
    except ValueError:
        for start, length, name in functions.named(query):
            print(f"{start:08x} {length:x} {name}")
