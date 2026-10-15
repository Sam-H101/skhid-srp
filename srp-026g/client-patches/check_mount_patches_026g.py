#!/usr/bin/env python3
"""Run patch 27, the mount of resources.db's patches (mount_patches_026g.asm),
in the default build on made-up game folders.

    python check_mount_patches_026g.py <original survarium.exe>

Requires Python `unicorn`. The exe's sections are loaded as they are and
again 1 MiB higher with its base relocations applied (a rebased image).
What runs as machine code: query_db's call at 0xE7E3E with the code it
calls, and on_mounted_resources' first six bytes with the code they jump
to. Stubs: CreateFileA and CloseHandle (a folder's files), the path
formatting 0x18190 (as its code reads: the text from the string's start,
within its buffer, a '/' or '\\' made the string's separator; the C library
under it needs a started process), query_mount_archive
0xE7EF0 (it records what it was asked to mount), the fatal error 0x113580
(it records the message) and on_mounted_resources after its first six
bytes (it records the mount the game would keep and goes on as the game
does). Each folder must give: the mounts queued in order (resources.db as
the game asked, then patch 1, 2, ... until the first that is not there,
at most 64) with the game's arguments but the two paths; every register
the game had and the stack as it was; no handle left open, and no access
asked for a file; the callbacks of those mounts held but the last, which
goes on into the game with resources.db's mount; a failed patch stopped
with its number. Says nothing about what the real file system does.
"""
import struct
import sys

from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_PROT_ALL, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EDX, UC_X86_REG_EBX, \
    UC_X86_REG_ESP, UC_X86_REG_EBP, UC_X86_REG_ESI, UC_X86_REG_EDI, UC_X86_REG_EIP

import patch_026g as base

IMPORTS = {0x870040: "CloseHandle", 0x870048: "CreateFileA"}
ARGUMENTS = {"CloseHandle": 1, "CreateFileA": 7}
FATAL = 0x113580
FORMAT_PATH = 0x18190
STUBS, HEAP, STACK = 0x30000000, 0x31000000, 0x32000000
RETURN = STUBS + 0x800                  # where the made-up callers continue
ALLOCATOR, HELPER = 0x0A7B688, 0x31000F00
GAME = {UC_X86_REG_EBX: 0x22222222, UC_X86_REG_EBP: 0x0019FF00, UC_X86_REG_ESI: ALLOCATOR,
        UC_X86_REG_EDI: HELPER}
STOP = "stop"


def path_string(at, text, separator):
    """A native/virtual path string as query_db makes them: begin, end of
    the text, end of the buffer, 260 characters, the separator."""
    raw = text.encode("ascii")
    return struct.pack("<III", at + 12, at + 12 + len(raw), at + 0x110) + raw.ljust(0x104, b"\0") \
        + separator.encode("ascii")


class Run:
    def __init__(self, data, delta, files):
        self.emu = emu = Uc(UC_ARCH_X86, UC_MODE_32)
        self.delta, self.files = delta, set(files)
        headers = base.Headers(data)
        image = bytearray()
        for _, rva, vsize, raw, rsize, _ in headers.sections(data):
            start = base.IMAGE_BASE + rva + delta
            size = (max(vsize, rsize) + 0xFFF) & ~0xFFF
            emu.mem_map(start, size, UC_PROT_ALL)
            emu.mem_write(start, bytes(data[raw:raw + min(rsize, vsize or rsize)]))
        if delta:
            reloc_rva, reloc_size = struct.unpack_from("<II", data, headers.reloc_at)
            table = next(raw + reloc_rva - rva for _, rva, vsize, raw, rsize, _ in headers.sections(data)
                         if rva <= reloc_rva < rva + vsize)
            at = table
            while at < table + reloc_size:
                page, block = struct.unpack_from("<II", data, at)
                if block < 8:
                    break
                for i in range((block - 8) // 2):
                    entry = struct.unpack_from("<H", data, at + 8 + 2 * i)[0]
                    if entry >> 12 == 3:
                        where = base.IMAGE_BASE + page + (entry & 0xFFF) + delta
                        self.word(where, self.read(where) + delta)
                at += block
        for at in (STUBS, HEAP, STACK):
            emu.mem_map(at, 0x100000, UC_PROT_ALL)
        self.stubs = {STUBS + 0x10 * i: name for i, name in enumerate(ARGUMENTS)}
        for slot, name in IMPORTS.items():
            self.word(slot + delta, next(at for at, n in self.stubs.items() if n == name))
        self.stubs[base.MOUNT_QUERY_MOUNT_ARCHIVE + delta] = "query_mount_archive"
        self.stubs[FATAL + delta] = "fatal"
        self.stubs[FORMAT_PATH + delta] = "format_path"
        self.stubs[base.MOUNTED_VA + 6 + delta] = "game"
        self.stubs[RETURN] = STOP
        self.opened, self.closed, self.mounts, self.kept, self.fatal = [], [], [], [], None
        for at in self.stubs:
            emu.hook_add(UC_HOOK_CODE, self.stub, begin=at, end=at)
        # The query's own strings, as query_db has them.
        self.virtual, self.archive, self.descriptor, self.callback = HEAP, HEAP + 0x200, HEAP + 0x400, HEAP + 0x500
        emu.mem_write(self.virtual, path_string(self.virtual, "resources", "/"))
        emu.mem_write(self.archive, path_string(self.archive, "..\\..\\resources.db", "\\"))
        emu.mem_write(self.descriptor, b"resources\0")
        self.next_mount = HEAP + 0x10000

    def word(self, at, value):
        self.emu.mem_write(at, struct.pack("<I", value & 0xFFFFFFFF))

    def read(self, at):
        return struct.unpack("<I", self.emu.mem_read(at, 4))[0]

    def text(self, at):
        out = bytearray()
        while (byte := self.emu.mem_read(at + len(out), 1)[0]) != 0:
            out.append(byte)
        return out.decode("ascii")

    def string(self, at):
        """A path string's text, which must be within its buffer and end
        with its zero."""
        begin, end, buffer_end = struct.unpack("<III", self.emu.mem_read(at, 12))
        assert begin == at + 12 and buffer_end == at + 0x110 and begin <= end < buffer_end
        raw = bytes(self.emu.mem_read(begin, end - begin + 1))
        assert raw[-1] == 0 and 0 not in raw[:-1]
        return raw[:-1].decode("ascii"), chr(self.emu.mem_read(at + 0x110, 1)[0])

    def stub(self, emu, address, size, _):
        name = self.stubs[address]
        if name == STOP:
            emu.emu_stop()
            return
        sp = emu.reg_read(UC_X86_REG_ESP)
        if name == "fatal":
            self.fatal = self.text(self.read(sp + 4)).replace("%u", str(self.read(sp + 8)))
            emu.emu_stop()
            return
        if name == "game":
            # on_mounted_resources from 0x147E6: ebp is its frame. What it
            # would keep, then its own end (`mov esp,ebp` / `pop ebp` / `ret`).
            frame = emu.reg_read(UC_X86_REG_EBP)
            assert sp & 7 == 0 and sp <= frame, "the game's frame is as its own six bytes make it"
            self.kept.append(self.read(self.read(frame + 0x10)))
            emu.reg_write(UC_X86_REG_ESP, frame + 8)
            emu.reg_write(UC_X86_REG_EBP, self.read(frame))
            emu.reg_write(UC_X86_REG_EIP, self.read(frame + 4))
            return
        if name == "format_path":
            # esi = the string; cdecl (format, ...); returns the string.
            string = emu.reg_read(UC_X86_REG_ESI)
            begin, _, buffer_end = struct.unpack("<III", emu.mem_read(string, 12))
            separator = emu.mem_read(string + 0x110, 1)[0]
            pieces, args = self.text(self.read(sp + 4)).split("%"), sp + 8
            out = pieces[0]
            for piece in pieces[1:]:
                kind, piece = piece[0], piece[1:]
                out += self.text(self.read(args)) if kind == "s" else str(self.read(args))
                assert kind in "su"
                out += piece
                args += 4
            raw = out.encode("ascii")[:buffer_end - begin - 1]
            raw = raw.replace(b"\\" if separator == 0x2F else b"/", bytes([separator]))
            emu.mem_write(begin, raw + b"\0")
            self.word(string + 4, begin + len(raw))
            result = string
        elif name == "query_mount_archive":
            assert emu.reg_read(UC_X86_REG_ESI) == ALLOCATOR + self.delta, "esi is the allocator"
            args = [self.read(sp + 4 + 4 * i) for i in range(5)]
            virtual, fat, archive, descriptor, callback = args
            assert fat == archive, "the file table is the archive's"
            self.mounts.append((self.string(virtual), self.string(archive), self.text(descriptor), callback))
            result = 0
        else:
            args = [self.read(sp + 4 + 4 * i) for i in range(ARGUMENTS[name])]
            result = getattr(self, name)(*args)
            sp += 4 * len(args)                 # stdcall
        # Every register but esp may be another after a call of the game's.
        emu.reg_write(UC_X86_REG_EAX, result)
        for register in (UC_X86_REG_ECX, UC_X86_REG_EDX, UC_X86_REG_EBX, UC_X86_REG_ESI,
                         UC_X86_REG_EDI, UC_X86_REG_EBP):
            emu.reg_write(register, 0xDEAD0000 | register)
        emu.reg_write(UC_X86_REG_EIP, self.read(emu.reg_read(UC_X86_REG_ESP)))
        emu.reg_write(UC_X86_REG_ESP, sp + 4)

    def CreateFileA(self, path, access, share, security, disposition, attributes, template):
        assert (access, disposition) == (0, 3), "no access, an existing file only"
        if self.text(path) not in self.files:
            return 0xFFFFFFFF
        self.opened.append(0x100 + 4 * len(self.opened))
        return self.opened[-1]

    def CloseHandle(self, handle):
        assert handle in self.opened and handle not in self.closed
        self.closed.append(handle)
        return 1

    def query(self):
        """query_db's call at 0xE7E3E with query_db's arguments."""
        emu = self.emu
        sp = STACK + 0x80000
        for value in (HELPER, self.descriptor, self.archive, self.archive, self.virtual, RETURN):
            sp -= 4
            self.word(sp, value)
        for register, value in GAME.items():
            emu.reg_write(register, value + (self.delta if register == UC_X86_REG_ESI else 0))
        # The patched call at 0xE7E3E, with a made-up return address.
        call = bytes(emu.mem_read(base.MOUNT_QUERY_VA + self.delta, 5))
        assert call[0] == 0xE8
        target = base.MOUNT_QUERY_VA + self.delta + 5 + struct.unpack("<i", call[1:])[0]
        emu.reg_write(UC_X86_REG_ESP, sp)
        emu.emu_start(target, RETURN, count=2_000_000)
        kept = {register: emu.reg_read(register) for register in GAME}
        expected = {register: value + (self.delta if register == UC_X86_REG_ESI else 0)
                    for register, value in GAME.items()}
        assert kept == expected, f"registers changed: {kept} != {expected}"
        assert emu.reg_read(UC_X86_REG_ESP) == sp + 4, "the stack as it was (the caller takes the arguments)"
        assert self.closed == self.opened, "a handle left open"
        for (virtual, archive, descriptor, callback) in self.mounts:
            assert virtual == ("resources", "/") and descriptor == "resources" and callback == HELPER
        return [archive for _, (archive, _), _, _ in self.mounts]

    def mounted(self, mount):
        """One mount's callback: on_mounted_resources(where it is kept, the
        start-up's next step, &mount) with a made-up mount whose reference
        count is 1 (the caller's), or a null one. Returns its count after."""
        emu = self.emu
        if mount:
            mount = self.next_mount
            self.next_mount += 0x40
            self.word(mount + 0x1C, 1)
        holder = HEAP + 0x8000
        self.word(holder, mount)
        sp = STACK + 0x60000 - 4
        for value in (holder, HEAP + 0x9000, HEAP + 0x9100, RETURN):
            sp -= 4
            self.word(sp, value)
        before = len(self.kept)
        for register, value in GAME.items():
            emu.reg_write(register, value)
        emu.reg_write(UC_X86_REG_ESP, sp)
        emu.emu_start(base.MOUNTED_VA + self.delta, RETURN, count=100000)
        if self.fatal is None:
            assert emu.reg_read(UC_X86_REG_ESP) == sp + 4, "cdecl: the stack as the caller left it"
            kept = {register: emu.reg_read(register) for register in GAME}
            assert kept == GAME, f"registers changed: {kept} != {GAME}"
        return (mount, self.read(mount + 0x1C) if mount else None, len(self.kept) > before)


def patch(n):
    return f"..\\..\\resources.db.patch_{n}"


def folder(*numbers):
    return {patch(n) for n in numbers}


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    original = open(sys.argv[1], "rb").read()
    upload = open(base.default_bin("section"), "rb").read()
    data, _ = base.build(original, "section", upload, None, open(base.late_join_bin(), "rb").read())
    db = "..\\..\\resources.db"
    folders = [
        ("no patch", folder(), [db]),
        ("patches 1 to 3", folder(1, 2, 3), [db, patch(1), patch(2), patch(3)]),
        ("patches 1 and 3: 2 is missing", folder(1, 3), [db, patch(1)]),
        ("patch 2 alone", folder(2), [db]),
        ("a patch .db", {db + ".patch_1.db", "..\\..\\resources.db.patch_01"}, [db]),
        ("patches 1 to 70: 64 at most", folder(*range(1, 71)), [db] + [patch(n) for n in range(1, 65)]),
    ]
    failed = 0

    def say(ok, text):
        nonlocal failed
        failed += not ok
        print(f"{'ok  ' if ok else 'FAIL'} {text}")

    for delta in (0, 0x100000):
        rebased = " (rebased)" if delta else ""
        for name, files, expected in folders:
            run = Run(data, delta, files)
            got = run.query()
            names = [g.split("\\")[-1] for g in got]
            shown = ", ".join(names if len(names) < 6 else names[:3] + ["..."] + names[-1:])
            say(got == expected, f"{name}{rebased}: mounts {len(got)}: {shown}")
            # Every callback in order: all held but the last, which goes on
            # into the game with resources.db's mount.
            results = [run.mounted(True) for _ in got]
            first = results[0][0]
            ok = all(count == 2 and not went for _, count, went in results[:-1]) \
                and results[-1][1] == 2 and results[-1][2] and run.kept == [first]
            say(ok, f"{name}{rebased}: {len(got) - 1} callbacks held, the last goes on with resources.db's mount")
            # A later call is the game's own, its argument untouched.
            mount, count, went = run.mounted(True)
            say(went and count == 1 and run.kept[-1] == mount, f"{name}{rebased}: a later callback is the game's own")
        # A failed mount.
        run = Run(data, delta, folder(1, 2))
        run.query()
        run.mounted(True)
        run.mounted(True)
        run.mounted(False)
        say(run.fatal == "Cannot mount resources.db.patch_2. Please run the Skhid launcher again." and not run.kept,
            f"patch 2 fails{rebased}: {run.fatal!r}")
        run = Run(data, delta, folder(1))
        run.query()
        _, _, went = run.mounted(False)
        say(run.fatal is None and went and run.kept == [0],
            f"resources.db fails{rebased}: the game's own fatal (it is given the null mount)")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
