"""Seed the writable per-user game once. Never overwrite an existing installation."""
import fcntl
import os
from pathlib import Path
import shutil
import subprocess
import sys

source = Path(sys.argv[1])
data_home = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
if not data_home.is_absolute():
    sys.exit("XDG_DATA_HOME must be an absolute path")
root = data_home / "skhid/0.26g"
root.mkdir(parents=True, exist_ok=True)
with (root / ".prepare.lock").open("a") as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    game = root / "game"
    if not game.exists():
        staging = root / ".game-staging"
        if staging.exists():
            shutil.rmtree(staging)
        staging.mkdir()
        print("Preparing Survarium game files…", file=sys.stderr)
        subprocess.run(["cp", "-r", "--reflink=auto", str(source) + "/.", str(staging)], check=True)
        subprocess.run(["chmod", "-R", "u+w", str(staging)], check=True)
        staging.rename(game)
    if not (game / "binaries/x86/survarium_launcher.exe").is_file():
        sys.exit(f"Existing game at {game} is incomplete; it has been left unchanged")
    print(game)
