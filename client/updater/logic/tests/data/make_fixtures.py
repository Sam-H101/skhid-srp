"""Writes the test fixtures of this folder again (Python 3, nothing else).

    python make_fixtures.py

The DEFLATE streams and the ZIPs come from Python's zlib and zipfile, which
is what `make_client_026g.py` makes the players' download with. The files
are kept in the repository, so the tests do not depend on the zlib of the
machine they run on.
"""
import os
import struct
import zipfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))


def write(name, data):
    with open(os.path.join(HERE, name), "wb") as out:
        out.write(data)


def raw(data, level=9):
    packer = zlib.compressobj(level, zlib.DEFLATED, -15)
    return packer.compress(data) + packer.flush()


def text():
    words = ("the launcher asks updater server answers with a checksum archive file "
             "survarium skhid game folder player version download install restart").split()
    out, state = [], 12345
    for _ in range(1100):
        state = (state * 1103515245 + 12345) & 0x7FFFFFFF
        out.append(words[(state >> 8) % len(words)])
        if state % 17 == 0:
            out.append("\r\n")
    return " ".join(out).encode("ascii")[:6000]


def deflate_fixtures():
    data = text()
    dynamic = raw(data)
    assert (dynamic[0] >> 1) & 3 == 2, "a dynamic block is wanted"
    write("text.bin", data)
    write("text.dynamic.deflate", dynamic)
    # One stream of several blocks: compressed, an empty stored block (the
    # sync flush), compressed again.
    packer = zlib.compressobj(9, zlib.DEFLATED, -15)
    multi = packer.compress(data[:2000]) + packer.flush(zlib.Z_FULL_FLUSH)
    multi += packer.compress(data[2000:4000]) + packer.flush(zlib.Z_SYNC_FLUSH)
    plain = data[:4000] + b"ab" * 40 + data[4000:]
    multi += packer.compress(b"ab" * 40 + data[4000:]) + packer.flush()
    assert zlib.decompress(multi, -15) == plain
    write("text.multi.bin", plain)
    write("text.multi.deflate", multi)


def make_zip(name, files, patch=None):
    path = os.path.join(HERE, name)
    if os.path.exists(path):
        os.remove(path)
    with zipfile.ZipFile(path, "x") as out:
        for entry_name, data, method in files:
            entry = zipfile.ZipInfo(entry_name, date_time=(2026, 10, 13, 0, 0, 0))
            entry.create_system = 3
            entry.external_attr = 0o100644 << 16
            entry.compress_type = method
            out.writestr(entry, data)
    if patch:
        with open(path, "rb") as source:
            data = bytearray(source.read())
        patch(data)
        with open(path, "wb") as out:
            out.write(data)


def replace_all(old, new):
    assert len(old) == len(new)

    def patch(data):
        count, at = 0, data.find(old)
        while at >= 0:
            data[at:at + len(old)] = new
            count += 1
            at = data.find(old, at + 1)
        assert count == 2, f"{old!r}: local and central name expected, found {count}"
    return patch


def central(data):
    return data.find(b"PK\x01\x02")


def zip_fixtures():
    exe = bytes(i * i % 251 for i in range(70_000))
    deflated, stored = zipfile.ZIP_DEFLATED, zipfile.ZIP_STORED
    make_zip("bundle.zip", [
        ("game/binaries/x86/", b"", stored),
        ("game/binaries/x86/survarium_skhid.exe", exe, deflated),
        ("game/binaries/x86/play_skhid.bat", b"@echo off\r\nstart survarium_skhid.exe\r\n", deflated),
        ("game/resources/ssl/survarium_login_server.crt", b"-----BEGIN CERTIFICATE-----\n", deflated),
        ("README.txt", b"stored, not deflated", stored),
        ("empty.txt", b"", deflated),
    ])
    payload = [("aa/evil.txt", b"x" * 40, deflated)]
    make_zip("evil-parent.zip", payload, replace_all(b"aa/evil.txt", b"../evil.txt"))
    make_zip("evil-absolute.zip", payload, replace_all(b"aa/evil.txt", b"/a/evil.txt"))
    make_zip("evil-backslash.zip", payload, replace_all(b"aa/evil.txt", b"aa" + bytes([0x5C]) + b"evil.txt"))
    make_zip("evil-duplicate.zip", [("a.txt", b"one", stored), ("b.txt", b"two", stored)],
             replace_all(b"b.txt", b"A.TXT"))

    def encrypted(data):
        at = central(data)
        flags = struct.unpack_from("<H", data, at + 8)[0]
        struct.pack_into("<H", data, at + 8, flags | 1)
    make_zip("evil-encrypted-flag.zip", payload, encrypted)

    def size(data):
        struct.pack_into("<I", data, central(data) + 24, 39)
    make_zip("evil-size.zip", payload, size)


if __name__ == "__main__":
    deflate_fixtures()
    zip_fixtures()
    print("fixtures written to", HERE)
