#!/usr/bin/env python3
"""Replace the retail 0.26g registration button with a localized account hint.

Builds a NEW resources.db; the input is never modified. Requires a JDK and
JPEXS FFDec (tested with 26.0.0). No game assets are stored in this repository.
"""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import zlib

# Retail build 2777: resources-db list -> flash_movies/login_menu.swf.
MOVIE_OFFSET = 58150813
MOVIE_SIZE = 3508744
MOVIE_SHA256 = "805ea5f872402d0b8c4abceed4f8cabf5ed50b7d62d5c3fc45f58f6d64e982cf"
HERE = Path(__file__).resolve().parent / "login_hint"


def unpack(data):
    if len(data) < 9 or data[:3] not in (b"CWS", b"FWS"):
        raise ValueError("Expected a CWS/FWS movie")
    body = zlib.decompress(data[8:]) if data[:3] == b"CWS" else data[8:]
    if len(body) + 8 != struct.unpack_from("<I", data, 4)[0]:
        raise ValueError("SWF length mismatch")
    header_size = (5 + 4 * (body[0] >> 3) + 7) // 8 + 4
    pos, tags = header_size, []
    while pos < len(body):
        start = pos
        word, = struct.unpack_from("<H", body, pos)
        pos += 2
        kind, size = word >> 6, word & 63
        if size == 63:
            size, = struct.unpack_from("<I", body, pos)
            pos += 4
        if pos + size > len(body):
            raise ValueError("Truncated SWF tag")
        pos += size
        tags.append((kind, body[start:pos]))
        if kind == 0:
            if size or pos != len(body):
                raise ValueError("Unexpected bytes after SWF End")
            break
    if not tags or tags[-1][0] != 0:
        raise ValueError("Missing SWF End")
    return data[3], body[:header_size], tags


def pack(version, header, tags, compressed=True):
    body = header + b"".join(raw for _, raw in tags)
    return ((b"CWS" if compressed else b"FWS") + bytes([version])
            + struct.pack("<I", len(body) + 8)
            + (zlib.compress(body, 9) if compressed else body))


def preserve_crc(data, original):
    """Use four ignored padding bytes to retain the archive entry's CRC.

    The engine uses the same CRC polynomial with different initial/final bits;
    equal-length messages with equal zlib CRCs also have equal engine CRCs.
    """
    prefix = data[:-4]
    initial = zlib.crc32(prefix)
    base = zlib.crc32(bytes(4), initial)
    basis = {}
    for bit in range(32):
        vector = zlib.crc32((1 << bit).to_bytes(4, "little"), initial) ^ base
        mask = 1 << bit
        while vector:
            pivot = vector.bit_length() - 1
            if pivot not in basis:
                basis[pivot] = (vector, mask)
                break
            other, other_mask = basis[pivot]
            vector ^= other
            mask ^= other_mask
    vector, mask = zlib.crc32(original) ^ base, 0
    while vector:
        other, other_mask = basis[vector.bit_length() - 1]
        vector ^= other
        mask ^= other_mask
    result = prefix + mask.to_bytes(4, "little")
    if len(result) != len(original) or zlib.crc32(result) != zlib.crc32(original):
        raise ValueError("Archive entry CRC was not preserved")
    return result


def build_movie(original, ffdec, java, javac, work):
    if hashlib.sha256(original).hexdigest() != MOVIE_SHA256:
        raise ValueError("Not the original build 2777 login movie (or already patched)")
    version, header, tags = unpack(original)
    # Keep Scaleform graphics out of the editor: transplant only the edited ABC.
    code = work / "code.swf"
    code.write_bytes(pack(version, header, [t for t in tags if t[0] in (69, 82, 0)], False))
    subprocess.run([javac, "-encoding", "UTF-8", "-cp", str(ffdec), "-d", str(work),
                    str(HERE / "PatchLoginHint.java")], check=True)
    edited = work / "edited.swf"
    subprocess.run([java, "-Djava.awt.headless=true", "-Duser.home=" + str(work),
                    "-cp", str(work) + os.pathsep + str(ffdec), "PatchLoginHint",
                    str(code), str(HERE / "SrpLoginHint.as"), str(edited)], check=True)
    replacements = [t for t in unpack(edited.read_bytes())[2] if t[0] == 82]
    before = [t for t in tags if t[0] == 82]
    if len(before) != len(replacements) or sum(a != b for a, b in zip(before, replacements)) != 1:
        raise ValueError("Expected exactly one changed ABC block")
    replacement_iter = iter(replacements)
    updated = [next(replacement_iter) if t[0] == 82 else t for t in tags]
    result = pack(version, header, updated)
    _, saved_header, saved_tags = unpack(result)
    if saved_header != header or [t for t in tags if t[0] != 82] != [t for t in saved_tags if t[0] != 82]:
        raise ValueError("Non-script content changed")
    if len(result) > MOVIE_SIZE - 4:
        raise ValueError("Patched movie no longer fits the original archive entry")
    # CWS ends at the zlib end marker. Padding preserves the original entry's
    # size and offsets, so the signed archive file table stays byte-identical.
    return preserve_crc(result + bytes(MOVIE_SIZE - len(result)), original)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="original build 2777 resources.db")
    parser.add_argument("output", type=Path, help="new resources.db, outside the original installation")
    parser.add_argument("--ffdec", required=True, type=Path, help="path to ffdec.jar")
    parser.add_argument("--java", default="java")
    parser.add_argument("--javac", default="javac")
    args = parser.parse_args()
    if args.output.exists() or args.output.is_symlink():
        parser.error("Output already exists; choose a new path")
    with args.source.open("rb") as source:
        source.seek(MOVIE_OFFSET)
        original = source.read(MOVIE_SIZE)
    with tempfile.TemporaryDirectory(prefix="srp-login-hint-") as temporary:
        movie = build_movie(original, args.ffdec.resolve(), args.java, args.javac, Path(temporary))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.source.open("rb") as source, args.output.open("xb") as output:
        shutil.copyfileobj(source, output, 8 * 1024 * 1024)
        output.seek(MOVIE_OFFSET)
        output.write(movie)
    print(f"Created {args.output}: first-sign-in hint in all five languages; registration link removed")


if __name__ == "__main__":
    main()
