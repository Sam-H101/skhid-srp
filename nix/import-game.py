"""Normalize a user-supplied original 0.26g archive/folder in the Nix build."""
import hashlib
from pathlib import Path
import shutil
import sys

source, destination = map(Path, sys.argv[1:])
candidates = list(source.rglob("binaries/x86/survarium_launcher.exe"))
if len(candidates) != 1:
    sys.exit("Expected one original 0.26g game containing binaries/x86/survarium_launcher.exe")
game = candidates[0].parents[2]
if hashlib.sha256(candidates[0].read_bytes()).hexdigest() != "710956d2347864effb353064dd7beaee1dd2773c74756d3f6677df71431c5266":
    sys.exit("The supplied game is not the original 0.26g launcher/build")
if not (game / "resources.db").is_file() or not (game / "binaries/x86/survarium.exe").is_file():
    sys.exit("The game archive is incomplete: resources.db or survarium.exe missing")
# Refuse links so the resulting package contains only the supplied game files.
if any(p.is_symlink() for p in game.rglob("*")):
    sys.exit("Game input contains symlinks; supply the original archive or a plain folder")
shutil.copytree(game, destination)
