"""Verify the exact player payload and safe output behavior.

Usage: python selftest_overlay_026g.py <original game folder> [Rust BugTrap.dll]
Reads the supplied game and DLL; all outputs go into a temporary directory.
"""
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

import make_client_026g as overlay


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    game = Path(overlay.game_root(sys.argv[1]))
    dll = Path(sys.argv[2] if len(sys.argv) == 3 else overlay.BUGTRAP_DLL).resolve()
    source_files = (game / overlay.EXE_DIR / "survarium.exe", game / overlay.CERT_REL, dll)
    original = {p: hashlib.sha256(p.read_bytes()).digest() for p in source_files}
    script = Path(__file__).with_name("make_client_026g.py")

    def run(output, *options, success=True):
        result = subprocess.run(
            [sys.executable, str(script), str(game), str(output),
             "--server=026g.skhid.link:1234", "--localization=english",
             "--bugtrap=" + str(dll), *options], capture_output=True, text=True)
        assert (result.returncode == 0) == success, result.stdout + result.stderr
        return result

    with tempfile.TemporaryDirectory(prefix="srp-overlay-test-") as temporary:
        root = Path(temporary)
        archive, folder = root / "players.zip", root / "players"
        run(archive)
        run(folder)
        expected = {"game/binaries/x86/survarium_skhid.exe",
                    "game/binaries/x86/bugtrap.dll", "game/binaries/x86/play_skhid.bat",
                    "game/resources/ssl/survarium_login_server.crt", "README.txt"}
        with zipfile.ZipFile(archive) as z:
            assert z.testzip() is None
            assert len(z.namelist()) == 5 and set(z.namelist()) == expected
            assert {p.relative_to(folder).as_posix() for p in folder.rglob("*") if p.is_file()} == expected
            for name in expected:
                assert z.read(name) == (folder / name).read_bytes(), name
            assert z.read("game/binaries/x86/bugtrap.dll") == dll.read_bytes()
            server = b"026g.skhid.link:1234"
            batch = z.read("game/binaries/x86/play_skhid.bat")
            assert (server + b"\0" in z.read("game/binaries/x86/survarium_skhid.exe")
                    or (b"set SERVER=" + server in batch and b"-server=%SERVER%" in batch))
            assert z.read("game/binaries/x86/survarium_skhid.exe") != source_files[0].read_bytes()
        print("PASS: folder and ZIP contain exactly the five player files; built DLL and server configuration verified")

        for name in ("check.zip", "check-folder"):
            output = root / name
            run(output, "--check")
            assert not output.exists()
        print("PASS: --check creates no folder or ZIP")

        archive_bytes = archive.read_bytes()
        run(archive, success=False)
        run(folder, success=False)
        assert archive.read_bytes() == archive_bytes
        assert (folder / "game/binaries/x86/bugtrap.dll").read_bytes() == dll.read_bytes()
        print("PASS: existing outputs are preserved")

        bad = root / "bad.dll"
        bad.write_bytes(b"not a Windows DLL")
        for path in (root / "missing.dll", bad):
            output = root / "invalid.zip"
            run(output, "--bugtrap=" + str(path), success=False)
            assert not output.exists()
        print("PASS: missing or invalid DLL fails before creating the ZIP")

    for path, digest in original.items():
        assert hashlib.sha256(path.read_bytes()).digest() == digest, path
    print("PASS: original game EXE, certificate and built DLL unchanged")


if __name__ == "__main__":
    main()
