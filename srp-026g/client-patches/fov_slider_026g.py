#!/usr/bin/env python3
"""Put the field-of-view slider into the 0.26g Settings → Graphics tab.

Builds a NEW resources.db; the input is never modified:

    python fov_slider_026g.py <resources.db> <new resources.db> --ffdec <ffdec.jar>

The exe side is patch 21 of patch_026g.py (patch revision 11): the game's
video option 7, a "FOV" slider, is bound to `uro_fov`, which the camera
follows. The stock menu has no instance of that slider; this build compiles
fov_slider/SrpFovSlider.as into main_menu.swf, which adds one below gamma.
Requires a JDK and JPEXS FFDec (tested with 25.1.3). Any resources.db works
if its main_menu.swf is the build 2777 original, so the login hint can be
applied before or after. An exe before revision 11 shows the slider without
effect: its option 7 names a console variable that does not exist.
"""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

import login_hint_026g as hint

# Retail build 2777: resources-db list -> flash_movies/main_menu.swf.
MOVIE_OFFSET = 17970448
MOVIE_SIZE = 1194596
MOVIE_SHA256 = "0ff118a85b53f69eb65f36b0f5996bdbdc12ec5068440e4d4d4dd6ece49481a4"
HERE = Path(__file__).resolve().parent / "fov_slider"


def build_movie(original, ffdec, java, javac, work):
    if hashlib.sha256(original).hexdigest() != MOVIE_SHA256:
        raise ValueError("Not the original build 2777 main menu movie (or already patched)")
    version, header, tags = hint.unpack(original)
    code = work / "code.swf"
    code.write_bytes(hint.pack(version, header, [t for t in tags if t[0] in (69, 82, 0)], False))
    subprocess.run([javac, "-encoding", "UTF-8", "-cp", str(ffdec), "-d", str(work),
                    str(HERE / "PatchFovSlider.java")], check=True)
    edited = work / "edited.swf"
    subprocess.run([java, "-Djava.awt.headless=true", "-Duser.home=" + str(work),
                    "-cp", str(work) + os.pathsep + str(ffdec), "PatchFovSlider",
                    str(code), str(HERE / "SrpFovSlider.as"), str(edited)], check=True)
    replacements = [t for t in hint.unpack(edited.read_bytes())[2] if t[0] == 82]
    before = [t for t in tags if t[0] == 82]
    if len(before) != len(replacements) or sum(a != b for a, b in zip(before, replacements)) != 1:
        raise ValueError("Expected exactly one changed ABC block")
    replacement_iter = iter(replacements)
    updated = [next(replacement_iter) if t[0] == 82 else t for t in tags]
    result = hint.pack(version, header, updated)
    _, saved_header, saved_tags = hint.unpack(result)
    if saved_header != header or [t for t in tags if t[0] != 82] != [t for t in saved_tags if t[0] != 82]:
        raise ValueError("Non-script content changed")
    if len(result) > MOVIE_SIZE - 4:
        raise ValueError("Patched movie no longer fits the original archive entry")
    # Same size and CRC as the original entry: the signed file table is unchanged.
    return hint.preserve_crc(result + bytes(MOVIE_SIZE - len(result)), original)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", type=Path, help="resources.db with the original main_menu.swf")
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
    with tempfile.TemporaryDirectory(prefix="srp-fov-slider-") as temporary:
        movie = build_movie(original, args.ffdec.resolve(), args.java, args.javac, Path(temporary))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.source.open("rb") as source, args.output.open("xb") as output:
        shutil.copyfileobj(source, output, 8 * 1024 * 1024)
        output.seek(MOVIE_OFFSET)
        output.write(movie)
    print(f"Created {args.output}: FOV slider in Settings → Graphics")


if __name__ == "__main__":
    main()
