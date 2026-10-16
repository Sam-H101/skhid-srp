#!/usr/bin/env python3
"""Turn the lobby's gold-for-silver window into an exchange both ways.

    python exchange_026g.py <resources.db | lobby_menu.swf entry> <new file> --ffdec <ffdec.jar>

The gold button ("Get Gold", which asked to open the online store) and the
silver button both open it: Pay above, Receive below, a swap between them and
the rate under them. Silver for gold goes to the server as `47 04` with
SrpExchange.BUY_GOLD added to the gold amount; the server must know it.

The input is the live lobby region (the lobby with the matches list), as a
whole resources.db or as the extracted lobby_menu.swf file; the output is of the same kind and
never replaces the input. Requires a JDK and JPEXS FFDec (tested with 26.0.0).
No game assets are stored in this repository.
"""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

import login_hint_026g as hint

# Retail build 2777: resources-db list -> flash_movies/lobby_menu.swf.
MOVIE_OFFSET = 19165044
MOVIE_SIZE = 38985769
# The lobby regions with the matches list (make_client_026g.py's lobby_menu
# targets), not the retail movie: that one lacks the list. The exchange
# changes other classes than the list's (BottomPanel), so either takes it.
MOVIE_SHA256S = {
    "8a98cf0df2c7c3c22c95c0ae80d310a0cfe234d84d3bca47d681a29f3a51456e": "live since 14 Oct",
    "48c3439f350d3d2aec4e6472d03970d6313026f7fdf35024b5009ce922bde37f": "JOIN reasons and match level",
}
RETAIL_SHA256 = "ab6f22d80d32fa4928403d19e3e054bd5316b2370e2550cefec0c9d22b959850"
HERE = Path(__file__).resolve().parent / "exchange"


def build_movie(original, ffdec, java, javac, work):
    digest = hashlib.sha256(original).hexdigest()
    if digest == RETAIL_SHA256:
        raise ValueError("This is the retail lobby movie; patch the live region (with the matches list)")
    if digest not in MOVIE_SHA256S:
        raise ValueError("Not the live build 2777 lobby movie (or already patched)")
    version, header, tags = hint.unpack(original)
    # Keep Scaleform graphics out of the editor: transplant only the edited ABC.
    code = work / "code.swf"
    code.write_bytes(hint.pack(version, header, [t for t in tags if t[0] in (69, 82, 0)], False))
    subprocess.run([javac, "-encoding", "UTF-8", "-cp", str(ffdec), "-d", str(work),
                    str(HERE / "PatchExchange.java")], check=True)
    edited = work / "edited.swf"
    subprocess.run([java, "-Xmx4g", "-Djava.awt.headless=true", "-Duser.home=" + str(work),
                    "-cp", str(work) + os.pathsep + str(ffdec), "PatchExchange",
                    str(code), str(HERE / "SrpExchange.as"), str(edited)], check=True)
    replacements = [t for t in hint.unpack(edited.read_bytes())[2] if t[0] == 82]
    before = [t for t in tags if t[0] == 82]
    # The window's block (the new class) and the status panel's (its buttons).
    if len(before) != len(replacements) or sum(a != b for a, b in zip(before, replacements)) != 2:
        raise ValueError("Expected exactly two changed ABC blocks")
    replacement_iter = iter(replacements)
    updated = [next(replacement_iter) if t[0] == 82 else t for t in tags]
    result = hint.pack(version, header, updated)
    _, saved_header, saved_tags = hint.unpack(result)
    if saved_header != header or [t for t in tags if t[0] != 82] != [t for t in saved_tags if t[0] != 82]:
        raise ValueError("Non-script content changed")
    if len(result) > MOVIE_SIZE - 4:
        raise ValueError("Patched movie no longer fits the original archive entry")
    # As for the login hint: padding keeps the entry's size, offset and CRC,
    # so the signed file table stays byte-identical.
    return hint.preserve_crc(result + bytes(MOVIE_SIZE - len(result)), original)


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", type=Path,
                        help="resources.db with the live lobby region, or that region's entry file")
    parser.add_argument("output", type=Path, help="new file of the same kind, outside the installation")
    parser.add_argument("--ffdec", required=True, type=Path, help="path to ffdec.jar")
    parser.add_argument("--java", default="java")
    parser.add_argument("--javac", default="javac")
    args = parser.parse_args()
    if args.output.exists() or args.output.is_symlink():
        parser.error("Output already exists; choose a new path")
    entry = args.source.stat().st_size == MOVIE_SIZE
    with args.source.open("rb") as source:
        source.seek(0 if entry else MOVIE_OFFSET)
        original = source.read(MOVIE_SIZE)
    with tempfile.TemporaryDirectory(prefix="srp-exchange-") as temporary:
        movie = build_movie(original, args.ffdec.resolve(), args.java, args.javac, Path(temporary))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.source.open("rb") as source, args.output.open("xb") as output:
        if entry:
            output.write(movie)
        else:
            shutil.copyfileobj(source, output, 8 * 1024 * 1024)
            output.seek(MOVIE_OFFSET)
            output.write(movie)
    print(f"Created {args.output}: lobby exchange both ways "
          f"(entry sha256 {hashlib.sha256(movie).hexdigest()})")


if __name__ == "__main__":
    main()
