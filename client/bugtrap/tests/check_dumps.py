"""Exercise crash_probe.exe under Wine and inspect the resulting minidumps.

Usage: python3 check_dumps.py /absolute/path/to/build [case ...]
The build directory must contain crash_probe.exe and BugTrap.dll.
Set WINEPREFIX to an isolated prefix, and DISPLAY to an X server (or Xvfb).
"""
import hashlib
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile


def exception_stream(path):
    data = path.read_bytes()
    assert data[:4] == b"MDMP", path
    count, directory = struct.unpack_from("<II", data, 8)
    exception = None
    modules = []
    comment = None
    for index in range(count):
        kind, size, offset = struct.unpack_from("<III", data, directory + index * 12)
        assert offset + size <= len(data), "truncated stream"
        if kind == 6:
            thread, _, code = struct.unpack_from("<III", data, offset)
            exception = thread, code
        if kind == 4:
            module_count = struct.unpack_from("<I", data, offset)[0]
            assert 4 + module_count * 108 <= size
            for index in range(module_count):
                name_rva = struct.unpack_from("<I", data, offset + 4 + index * 108 + 20)[0]
                length = struct.unpack_from("<I", data, name_rva)[0]
                modules.append(data[name_rva+4:name_rva+4+length].decode("utf-16le").lower())
        if kind == 10:
            comment = data[offset:offset+size]
    assert exception is not None, "dump has no exception stream"
    assert any(m.endswith("crash_probe.exe") for m in modules), "missing EXE module"
    assert any(m.endswith("bugtrap.dll") for m in modules), "missing BugTrap module"
    assert comment == b"SRP probe: preserved assertion message\0", "missing game error message"
    return exception


def main():
    build = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="srp-crash-test-") as temporary:
        root = Path(temporary)
        exe = root / "game-Żółw"
        exe.mkdir()
        cwd = root / "different-working-directory"
        cwd.mkdir()
        fallback = root / "fallback"
        fallback.mkdir()
        for name in ("crash_probe.exe", "BugTrap.dll"):
            shutil.copy2(build / name, exe / name)
        env = os.environ.copy()
        previous = {}
        modes = sys.argv[2:] or ("silent", "cpp", "thread", "pre-handler", "stack", "cancel", "silent",
                     "continue-cpp", "continue-seh", "continue-net",
                     "execute-cpp", "execute-seh", "execute-net", "recursive", "concurrent",
                     "fallback", "failure")
        for mode in modes:
            folder = fallback if mode == "fallback" else exe
            before = set(folder.glob("*.dmp"))
            run_env = env.copy()
            if mode == "concurrent":
                shutil.copy2(build / "delayed_dbghelp.dll", exe / "dbghelp.dll")
                run_env["WINEDLLOVERRIDES"] = env.get("WINEDLLOVERRIDES", "") + ";dbghelp=n,b"
            if mode in ("fallback", "failure"):
                exe.chmod(0o555)
            if mode == "failure":
                fallback.chmod(0o555)
            try:
                result = subprocess.run(
                    ["wine", str(exe / "crash_probe.exe"),
                     "silent" if mode == "fallback" else mode, "Z:" + str(fallback)],
                    cwd=cwd, env=run_env, capture_output=True, timeout=45,
                )
            finally:
                exe.chmod(0o755)
                fallback.chmod(0o755)
                (exe / "dbghelp.dll").unlink(missing_ok=True)
            output = result.stdout.decode(errors="replace")
            new = set(folder.glob("*.dmp")) - before
            if mode == "failure":
                assert not new and "failed_dump_dialog" in output, (mode, output, result.returncode)
                print("PASS failure: explicit failure dialog when neither folder is writable", flush=True)
                continue
            assert len(new) == 1, (mode, result.returncode, output, result.stderr.decode(errors="replace"), new)
            dump = new.pop()
            thread = int(re.search(r"fault_thread=(\d+)", output)[1])
            code = 0xC00000FD if mode == "stack" else 0xC0000005
            assert exception_stream(dump) == (thread, code), mode
            if mode == "pre-handler":
                assert result.returncode == 23, result.returncode
            if mode == "cancel":
                assert "closed_dialog" in output, (output, result.returncode)
            expected = ("outer_handler_reached" if mode.startswith("continue-") else
                        "synthetic_returned" if mode.startswith("execute-") else
                        "recursive_capture_rejected" if mode == "recursive" else
                        "concurrent_crash_waited_for_dump_only" if mode == "concurrent" else None)
            if expected:
                assert expected in output and result.returncode == 0, (mode, output, result.returncode)
            assert not list(cwd.glob("*.dmp")), "dump used CWD instead of EXE directory"
            for old, digest in previous.items():
                assert hashlib.sha256(old.read_bytes()).digest() == digest, "old dump changed or deleted"
            previous[dump] = hashlib.sha256(dump.read_bytes()).digest()
            print(f"PASS {mode}: valid dump, faulting thread {thread}, previous dumps retained", flush=True)


if __name__ == "__main__":
    main()
