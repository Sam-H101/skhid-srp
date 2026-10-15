"""The database patches' formats, in Python: the twin of
client/updater/logic/src/dbpatch.rs, which defines them (and is what the
updater reads). For the tools that make patches and for make_client_026g.py,
which puts the list into the players' ZIP. It reads and writes text only:
no database is diffed here and nothing is compressed.

A patch is a description and a blob:

    skhid-db-patch 1
    revision 14
    from <sha256 of the whole input file> <size>
    to <sha256 of the whole output file> <size>
    blob <sha256 of the blob> <size>
    write <offset> <length>      bytes put over the input, in place
    append <length>              bytes added at the end
    end

The blob is a zstd-compressed file (level 3 is fine) that holds the bytes of
every write and append, in the order of the lines, and nothing else. Writes
are inside the input, in rising order, not overlapping; `to size` is
`from size` and every append. Applying it: the input is `from`, the writes
and appends are done, the whole file is hashed and must be `to`.

A release's patches live in one folder that only grows (a player who missed
releases needs every patch): each description as `<anything>.skhid-db-patch`,
each blob beside it named by its sha256, as on the server
(`download/resources/<sha256>`).

The list in the ZIP, `game/binaries/x86/updater_skhid.patches`, is the
original and every description, in the order of their revisions:

    skhid-db-patches 1
    original <sha256> <size>
    skhid-db-patch 1
    revision 14
    ...
    end
    end
"""

import hashlib
import os
import re

LIST_NAME = "game/binaries/x86/updater_skhid.patches"
DESCRIPTION_SUFFIX = ".skhid-db-patch"
# The stock resources.db of build 2777 (RESOURCES_DB in
# client/updater/logic/src/game.rs): the input of the first patch.
STOCK = ("c74078be702d17113b9ca23f59aa2b0087238481e7f01fb793620d51d875492c", 6285533906)
MAX_OPS = 4096
_HEX = re.compile(r"[0-9a-fA-F]{64}")


def _number(text):
    if not text.isascii() or not text.isdigit() or len(text) > 18:
        raise ValueError(f"not a number: {text!r}")
    return int(text)


def _file(values):
    if len(values) != 2 or not _HEX.fullmatch(values[0]):
        raise ValueError(f"not <sha256> <size>: {' '.join(values)!r}")
    return (values[0].lower(), _number(values[1]))


def _lines(text):
    return [line[:-1] if line.endswith("\r") else line for line in text.decode("ascii").split("\n")]


def _patch(lines, at):
    """One patch from the line after its first; (patch, index after its end)."""
    patch = {"ops": []}
    while at < len(lines):
        word, _, rest = lines[at].partition(" ")
        values = rest.split(" ") if rest else []
        at += 1
        if word in ("revision", "from", "to", "blob"):
            if word in patch:
                raise ValueError(f"line {at}: {word} twice")
            patch[word] = _number(rest) if word == "revision" else _file(values)
            if word == "revision" and patch[word] == 0:
                raise ValueError(f"line {at}: revision 0")
        elif word == "write":
            if len(values) != 2 or "from" not in patch:
                raise ValueError(f"line {at}: write <offset> <length>, after from")
            offset, length = map(_number, values)
            last = [op for op in patch["ops"] if op[0] == "write"]
            if length == 0 or offset + length > patch["from"][1] or (
                    last and last[-1][1] + last[-1][2] > offset):
                raise ValueError(f"line {at}: a write must be inside the input, in rising order, not overlapping")
            patch["ops"].append(("write", offset, length))
        elif word == "append":
            length = _number(rest)
            if length == 0:
                raise ValueError(f"line {at}: append 0")
            patch["ops"].append(("append", length))
        elif word == "end":
            missing = [name for name in ("revision", "from", "to", "blob") if name not in patch]
            if missing:
                raise ValueError(f"line {at}: no {', '.join(missing)}")
            appended = sum(op[1] for op in patch["ops"] if op[0] == "append")
            if not patch["ops"] or patch["from"] == patch["to"] or patch["blob"][1] == 0 \
                    or patch["from"][1] + appended != patch["to"][1]:
                raise ValueError(f"line {at}: the writes and appends do not make `to` from `from`")
            return patch, at
        if len(patch["ops"]) > MAX_OPS:
            raise ValueError("too many writes and appends")
    raise ValueError("cut short: no end")


def _header(line, magic):
    word, _, version = line.partition(" ")
    if word != magic or version != "1":
        raise ValueError(f"not {magic} 1: {line!r}")


def parse_patch(text):
    """A description: {"revision", "from", "to", "blob": (sha256, size),
    "ops": [("write", offset, length) | ("append", length)]}."""
    lines = _lines(text)
    _header(lines[0], "skhid-db-patch")
    return _patch(lines, 1)[0]


def data_size(patch):
    """The bytes the blob holds once decompressed."""
    return sum(op[-1] for op in patch["ops"])


def write_patch(patch):
    lines = ["skhid-db-patch 1", f"revision {patch['revision']}"]
    lines += [f"{name} {patch[name][0]} {patch[name][1]}" for name in ("from", "to", "blob")]
    lines += [f"write {op[1]} {op[2]}" if op[0] == "write" else f"append {op[1]}" for op in patch["ops"]]
    return "".join(line + "\r\n" for line in [*lines, "end"]).encode("ascii")


def check_chain(original, patches):
    """Rising revisions; the first from the original, each from the one before."""
    before, revision = original, 0
    for patch in patches:
        if patch["revision"] <= revision:
            raise ValueError(f"revision {patch['revision']} after {revision}")
        if patch["from"] != before:
            raise ValueError(f"revision {patch['revision']} starts from {patch['from']}, not {before}")
        before, revision = patch["to"], patch["revision"]


def write_list(original, patches):
    """The list's text (write_manifest in dbpatch.rs)."""
    check_chain(original, patches)
    out = f"skhid-db-patches 1\r\noriginal {original[0]} {original[1]}\r\n".encode("ascii")
    return out + b"".join(write_patch(patch) for patch in patches) + b"end\r\n"


def parse_list(text):
    """(original, patches) of a list."""
    lines = _lines(text)
    _header(lines[0], "skhid-db-patches")
    original, patches, at = None, [], 1
    while at < len(lines):
        word, _, rest = lines[at].partition(" ")
        at += 1
        if word == "original" and original is None:
            original = _file(rest.split(" "))
        elif word == "skhid-db-patch":
            _header(lines[at - 1], "skhid-db-patch")
            patch, at = _patch(lines, at)
            patches.append(patch)
        elif word == "end":
            if original is None:
                raise ValueError("no original")
            check_chain(original, patches)
            return original, patches
    raise ValueError("cut short: no end")


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for piece in iter(lambda: file.read(1 << 20), b""):
            digest.update(piece)
    return digest.hexdigest()


def read_folder(folder, original=None):
    """The patches of a release folder, in order, each with its blob's path;
    every description read, the chain checked from `original`, every blob
    there with its size and sha256."""
    found = []
    for name in sorted(os.listdir(folder)):
        if name.endswith(DESCRIPTION_SUFFIX):
            path = os.path.join(folder, name)
            with open(path, "rb") as file:
                try:
                    patch = parse_patch(file.read())
                except (ValueError, UnicodeDecodeError) as error:
                    raise ValueError(f"{path}: {error}") from None
            found.append(patch)
    found.sort(key=lambda patch: patch["revision"])
    check_chain(original or STOCK, found)
    out = []
    for patch in found:
        sha, size = patch["blob"]
        blob = os.path.join(folder, sha)
        if not os.path.isfile(blob) or os.path.getsize(blob) != size or sha256_file(blob) != sha:
            raise ValueError(f"{blob}: the blob of revision {patch['revision']} ({size} bytes, sha256 {sha}) "
                             "is not there")
        out.append((patch, blob))
    return out
