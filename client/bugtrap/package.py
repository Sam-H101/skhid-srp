# SPDX-License-Identifier: GPL-3.0-or-later
"""Package the Rust DLL with matching symbols, source and licence notices."""
from pathlib import Path
import hashlib
import shutil
import zipfile

root = Path(__file__).resolve().parent
build = root / "target/i686-pc-windows-msvc/release"
out = root / "target/package"
out.mkdir(parents=True, exist_ok=True)
files = {}
for name in ("BugTrap.dll", "BugTrap.pdb"):
    source = build / name
    assert source.is_file(), f"Build first: missing {source}"
    files[name] = source
    shutil.copy2(source, out / name)
files["README.md"] = root / "README.md"
license_path = root / "LICENSE"
if not license_path.exists():
    license_path = root.parents[1] / "LICENSE"
files["LICENSE"] = license_path
for directory in ("src", "tests", "licenses", ".cargo"):
    for source in sorted((root / directory).rglob("*")):
        if source.is_file() and "__pycache__" not in source.parts:
            files["source/" + str(source.relative_to(root))] = source
for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.sh", "package.py", "README.md"):
    files["source/" + name] = root / name
files["source/LICENSE"] = license_path
checksums = "".join(hashlib.sha256(files[name].read_bytes()).hexdigest() + "  " + name + "\n" for name in ("BugTrap.dll", "BugTrap.pdb"))
(out / "SHA256SUMS").write_text(checksums)
files["SHA256SUMS"] = out / "SHA256SUMS"
with zipfile.ZipFile(out / "BugTrap-rust-win32.zip", "w", zipfile.ZIP_DEFLATED) as archive:
    for name, source in sorted(files.items()):
        archive.write(source, name)
print(checksums, end="")
print(out / "BugTrap-rust-win32.zip")
