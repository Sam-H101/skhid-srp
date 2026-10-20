#!/usr/bin/env python3
"""Put a changed srp class (SrpRooms) into the newest live 0.26g lobby movie.

    python class_update_026g.py <lobby_menu.swf> <new lobby_menu.swf> --ffdec <ffdec.jar> [--class SrpRooms]

rooms_026g.py built the rooms window into the lobby movie of revision 23; the
lobby patches after it (Get Gold, the exchange update, Buy on owned items, the
craft window, Optics) were built on top of that. A later change of
rooms/SrpRooms.as must not rebuild from revision 23's movie, which would drop
them: this compiles the class anew over the one the newest movie has
(class_update/ReplaceClass.java), and nothing else changes. Only the class's
ABC block changes; every method of every other class and every other script's
initializer is checked byte for byte, and every constant keeps its value.

The input is the newest movie of the chain (`flash_movies/lobby_menu.swf`);
the output is a new movie for a new resources.db.patch_<n> (`srp-game-data
pack`). Requires a JDK and JPEXS FFDec (tested with 25.1.3). No game assets
are stored in this repository.
"""
import argparse
import collections
import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile

import login_hint_026g as hint

# The live movies a class is replaced in, by the sha256 of the file.
MOVIE_SHA256S = {
    "2e4cb7d2a0caba5efadaa64b9398d9065125d9e8cdad9ef59813c05c9723c9da":
        "revision 24's with Get Gold, the exchange update, Buy on owned items, the craft window and Optics",
}
RETAIL_SHA256 = "ab6f22d80d32fa4928403d19e3e054bd5316b2370e2550cefec0c9d22b959850"
HERE = Path(__file__).resolve().parent
# The classes this updates, and their sources.
SOURCES = {"SrpRooms": HERE / "rooms" / "SrpRooms.as"}


def check_source(source, name):
    """FFDec's compiler takes two methods of one name and keeps the last: a
    call to the first then runs the second. Refuse that."""
    names = collections.Counter(re.findall(r"\bfunction\s+(?!get\b|set\b)(\w+)\s*\(", source))
    twice = sorted(n for n, count in names.items() if count > 1)
    if twice:
        raise ValueError(f"{name}.as defines these more than once: " + ", ".join(twice))


def build_movie(original, ffdec, java, javac, work, cls="SrpRooms", known=True):
    source = SOURCES[cls]
    check_source(source.read_text(encoding="utf-8"), cls)
    digest = hashlib.sha256(original).hexdigest()
    if digest == RETAIL_SHA256:
        raise ValueError("This is the retail lobby movie; use the newest live one")
    if known and digest not in MOVIE_SHA256S:
        raise ValueError(f"Not a known live lobby movie (sha256 {digest}); --any-movie to try it")
    version, header, tags = hint.unpack(original)
    # Keep Scaleform graphics out of the editor: transplant only the edited ABC.
    code = work / "code.swf"
    code.write_bytes(hint.pack(version, header, [t for t in tags if t[0] in (69, 82, 0)], False))
    subprocess.run([javac, "-encoding", "UTF-8", "-cp", str(ffdec), "-d", str(work),
                    str(HERE / "class_update" / "ReplaceClass.java")], check=True)
    edited = work / "edited.swf"
    subprocess.run([java, "-Xmx4g", "-Djava.awt.headless=true", "-Duser.home=" + str(work),
                    "-cp", str(work) + os.pathsep + str(ffdec), "ReplaceClass",
                    str(code), str(source), str(edited), cls], check=True)
    replacements = [t for t in hint.unpack(edited.read_bytes())[2] if t[0] == 82]
    before = [t for t in tags if t[0] == 82]
    changed = sum(a != b for a, b in zip(before, replacements))
    if len(before) != len(replacements) or changed != 1:
        raise ValueError(f"Expected the class's ABC block alone to change, got {changed}")
    replacement_iter = iter(replacements)
    updated = [next(replacement_iter) if t[0] == 82 else t for t in tags]
    result = hint.pack(version, header, updated)
    _, saved_header, saved_tags = hint.unpack(result)
    if saved_header != header or [t for t in tags if t[0] != 82] != [t for t in saved_tags if t[0] != 82]:
        raise ValueError("Non-script content changed")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", type=Path, help="the newest live lobby_menu.swf")
    parser.add_argument("output", type=Path, help="the new lobby_menu.swf, a new file")
    parser.add_argument("--ffdec", required=True, type=Path, help="path to ffdec.jar")
    parser.add_argument("--class", dest="cls", default="SrpRooms", choices=sorted(SOURCES))
    parser.add_argument("--java", default="java")
    parser.add_argument("--javac", default="javac")
    parser.add_argument("--any-movie", action="store_true",
                        help="build on a lobby movie whose hash is not listed (development)")
    args = parser.parse_args()
    if args.output.exists() or args.output.is_symlink():
        parser.error("Output already exists; choose a new path")
    original = args.source.read_bytes()
    with tempfile.TemporaryDirectory(prefix="srp-class-update-") as temporary:
        movie = build_movie(original, args.ffdec.resolve(), args.java, args.javac, Path(temporary),
                            cls=args.cls, known=not args.any_movie)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("xb") as output:
        output.write(movie)
    print(f"Created {args.output}: lobby movie with the current {args.cls} ({len(movie)} bytes, "
          f"sha256 {hashlib.sha256(movie).hexdigest()})")


if __name__ == "__main__":
    main()
