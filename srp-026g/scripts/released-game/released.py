"""The game as Skhid's players have it, for the site's builds (the Maps tab,
the calculator): the stock resources.db with exactly the patches of
release/patches.tsv mounted over it, in their order, each checked by its size
and sha256; and the record of what a build read (built-from.tsv), which
release/release.sh and a web-server test compare with the offered patches.

    rows = offered(list_path, with_patch)        # [(name, sha256, size, path)]
    extract(dbgrab, stock, rows, patches_dir, wanted, raw_dir)
    write_built_from(out_dir / "built-from.tsv", rows, "srp-026g/scripts/...")
"""
import hashlib
import subprocess
import sys
from pathlib import Path

LIST = Path(__file__).resolve().parents[3] / "release" / "patches.tsv"


def die(message):
    sys.exit(f"{Path(sys.argv[0]).name}: {message}")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def offered(list_path=LIST, with_patch=None):
    """The list's patches as (name, sha256, size, path or None), in order;
    with_patch: the next release's own patch file, not listed yet."""
    rows = []
    for line in Path(list_path).read_text().splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        name, sha, size, *_ = line.split("\t")
        rows.append((name, sha, int(size), None))
    if with_patch:
        extra = Path(with_patch)
        if any(name == extra.name for name, *_ in rows):
            die(f"--with {extra.name}: in {list_path} already")
        rows.append((extra.name, sha256_file(extra), extra.stat().st_size, extra))
    return rows


def revision(rows):
    """The newest patch's number: the revision the build shows."""
    return max((int(name.rsplit("_", 1)[1]) for name, *_ in rows), default=0)


def archives(stock, rows, patches_dir):
    """The archives in the game's order, each patch checked against its row."""
    out = [Path(stock)]
    for name, sha, size, path in rows:
        path = Path(path or Path(patches_dir) / name)
        if not path.is_file():
            die(f"{path}: no such file (resources.db.patch rows of {LIST})")
        if path.stat().st_size != size or sha256_file(path) != sha:
            die(f"{path}: not the file the list names (size or sha256)")
        out.append(path)
    return out


def extract(dbgrab, stock, rows, patches_dir, wanted, raw):
    """Every file under each of `wanted` (path prefixes) into `raw` ('/' as
    '__'), the stock db first and each patch over it: the newest wins."""
    Path(raw).mkdir(parents=True, exist_ok=True)
    for archive in archives(stock, rows, patches_dir):
        for what in wanted:
            subprocess.run([dbgrab, str(archive), "get", what, str(raw)], check=True, capture_output=True)


def write_built_from(path, rows, tool):
    Path(path).write_text(f"# The patches {tool} was built from: name, sha256.\n"
                          + "".join(f"{name}\t{sha}\n" for name, sha, *_ in rows))
