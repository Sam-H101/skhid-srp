#!/usr/bin/env python3
"""Run the original and patch 30's respawn rule tick (0x316820) and the
hud's update of the local player's state (0x23B7B3) on made-up objects.

    python check_kit_change_026g.py <original survarium.exe>

Requires Python `unicorn`. The tick's start (up to 0x316826) and the game's
own spawn (0x316DA0, with patches 18 and 20 in the build) run as machine
code; the player's calls (out of the world, initialize, into the world, its
damage model), the client's view, the statistics' kit event and the hud's
calls are stubs. The default build of patch_026g.py must respawn a place
whose next kit is another, where it stands and facing as it faces, with
what was left of its invulnerability, while the match waits or counts
down, and only then; nothing else may change: not a dead player, not one in
play, not one with any statistics, not a place the tick spawns itself. The
original never does. The hud of the build shows the kits to a player who
becomes alive before the start and hides them from a live one once play
began; the original only hides them at a respawn and shows them at a death.
Says nothing about what the real initialize, physics and hud movie do.
"""
import struct
import sys

from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_PROT_ALL, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EIP, UC_X86_REG_ESP, \
    UC_X86_REG_EBX, UC_X86_REG_ESI, UC_X86_REG_EDI, UC_X86_REG_EBP, UC_X86_REG_FPSW

import patch_026g as base

TICK, TICK_ON = base.KIT_CHANGE_TICK_VA, base.KIT_CHANGE_TICK_VA + 6
SPAWN = 0x316DA0
HEAP = 0x20000000
RULE, RVT, CORE, PLACES, SVT, PVT, DMI = (HEAP + i * 0x1000 for i in range(7))
STATS = HEAP + 0x30000                             # 20 records of 1F8h
CORE_SIZE = 0x1A000                                # the core's fields up to +18FE4h
PLAYERS = HEAP + 0x40000                           # 20 places x 3 kits, 0x1000 each
STACK, STOP, STUB = 0x2F008000, 0x2F100000, 0x2F100100
INIT, PUT, REMOVE, VIEW, STAT_EVENT, DAMAGE = (STUB + 0x10 * i for i in range(6))
NOW = 21_750
KEPT = (UC_X86_REG_EBX, UC_X86_REG_ESI, UC_X86_REG_EDI)


def player_at(place, kit):
    return PLAYERS + (place * 3 + kit) * 0x1000


class Emu:
    def __init__(self, data):
        self.emu = emu = Uc(UC_ARCH_X86, UC_MODE_32)
        headers = base.Headers(data)
        for _, rva, vsize, raw, rsize, _ in headers.sections(data):
            start = base.IMAGE_BASE + rva
            emu.mem_map(start, (max(vsize, rsize) + 0xFFF) & ~0xFFF, UC_PROT_ALL)
            emu.mem_write(start, bytes(data[raw:raw + min(rsize, vsize or rsize)]))
        emu.mem_map(HEAP, 0x40000 + 60 * 0x1000 + CORE_SIZE, UC_PROT_ALL)
        emu.mem_map(0x2F000000, 0x10000, UC_PROT_ALL)
        emu.mem_map(STOP, 0x10000, UC_PROT_ALL)
        self.calls = []
        emu.hook_add(UC_HOOK_CODE, self.stub)

    def word(self, at, value):
        self.emu.mem_write(at, struct.pack("<I", value & 0xFFFFFFFF))

    def read(self, at):
        return struct.unpack("<I", self.emu.mem_read(at, 4))[0]

    def byte(self, at, value=None):
        if value is None:
            return self.emu.mem_read(at, 1)[0]
        self.emu.mem_write(at, bytes([value]))

    def returns(self, value, pop):
        sp = self.emu.reg_read(UC_X86_REG_ESP)
        self.emu.reg_write(UC_X86_REG_EAX, value)
        self.emu.reg_write(UC_X86_REG_EIP, self.read(sp))
        self.emu.reg_write(UC_X86_REG_ESP, sp + 4 + pop)

    def stub(self, emu, address, size, _):
        pass


class Tick(Emu):
    """A rule, a core with 20 places of three kits, statistics."""

    def __init__(self, data):
        super().__init__(data)
        core = CORE + 0x10000                      # leaves room below for the core's fields
        self.core = core
        for at, value in ((RULE, RVT), (RULE + 0x134, core), (RVT + 0x5C, SPAWN), (RVT + 0x60, VIEW),
                          (core + 0x18458, PLACES), (core + 0x18FA8, STATS), (STATS, SVT),
                          (SVT + 0x2C, STAT_EVENT), (PVT + 0x1C, INIT), (PVT + 0x20, PUT),
                          (PVT + 0x24, REMOVE), (DMI + 8, DAMAGE)):
            self.word(at, value)
        self.state(2)
        for place in range(20):
            self.word(RULE + 0x138 + place * 4, -1)
            for kit in range(3):
                p = player_at(place, kit)
                self.word(PLACES + place * 0x30 + kit * 4, p)
                self.word(p, PVT)
                self.word(p + 0x108, DMI)
                self.word(p + 0x10C, p + 0x800)    # an inventory of 23 empty slots
                self.byte(p + 0x114, place)
                self.byte(p + 0x115, kit)
                self.word(p + 0x700, p + 0x900)    # its damage model, [player+108h] -> +8 -> here
                self.word(p + 0x900 + 0x32C, -1)
            self.byte(PLACES + place * 0x30 + 0xC, 0)
            self.byte(PLACES + place * 0x30 + 0xD, 0)

    def state(self, value):
        self.word(self.core + 0x18FE4, value)

    def alive(self, place, kit=0, feet=(1.0, 2.0, 3.0), yaw=0.5, protected_until=None):
        p = player_at(place, kit)
        self.byte(p + 0x33C, 1)
        self.byte(p + 0x33F, 1)
        self.emu.mem_write(p + 0x43C, struct.pack("<3f", *feet))
        self.emu.mem_write(p + 0x490, struct.pack("<f", yaw))
        if protected_until is not None:
            self.word(p + 0x900 + 0x32C, protected_until)
        self.byte(PLACES + place * 0x30 + 0xC, kit)
        self.byte(PLACES + place * 0x30 + 0xD, kit)

    def choose(self, place, kit):
        self.byte(PLACES + place * 0x30 + 0xD, kit)

    def record(self, place):
        return STATS + 8 + place * 0x1F8

    def stub(self, emu, address, size, _):
        sp, this = emu.reg_read(UC_X86_REG_ESP), emu.reg_read(UC_X86_REG_ECX)
        if address == REMOVE:
            assert self.read(sp + 4) == 1
            self.byte(this + 0x33F, 0)
            self.calls.append(("remove", this))
            self.returns(0, 4)
        elif address == INIT:                      # initialize(time, &position, yaw, pitch)
            assert emu.reg_read(UC_X86_REG_FPSW) & 0x3800 == 0, "x87 stack not empty"
            time, position = self.read(sp + 4), self.read(sp + 8)
            feet = struct.unpack("<3f", emu.mem_read(position, 12))
            yaw, pitch = struct.unpack("<2f", emu.mem_read(sp + 12, 8))
            self.calls.append(("initialize", this, time, feet, yaw, pitch))
            self.returns(0, 16)
        elif address == PUT:
            assert self.read(sp + 4) == 1
            self.byte(this + 0x33F, 1)
            self.calls.append(("put", this))
            self.returns(0, 4)
        elif address == VIEW:
            self.calls.append(("view", self.read(sp + 4) & 0xFF))
            self.returns(0, 4)
        elif address == STAT_EVENT:                # (time, place, kit before, kit after)
            self.calls.append(("kit event", self.read(sp + 4), self.read(sp + 8) & 0xFF,
                               self.read(sp + 12) & 0xFF, self.read(sp + 16) & 0xFF))
            self.returns(0, 16)
        elif address == DAMAGE:                    # the damage model's holder, as 0x1F1790
            self.returns(this - 0x108 + 0x700, 0)

    def run(self):
        emu = self.emu
        for at, value in ((STACK, STOP), (STACK + 4, 0), (STACK + 8, NOW), (STACK + 12, 0)):
            self.word(at, value)
        emu.reg_write(UC_X86_REG_ESP, STACK)
        emu.reg_write(UC_X86_REG_ECX, RULE)
        emu.reg_write(UC_X86_REG_EBP, 0x0BADF00D)
        for register in KEPT:
            emu.reg_write(register, 0x12345678)
        emu.emu_start(TICK, TICK_ON, count=200000)
        # Where the original is after `push ebp` / `mov ebp,esp` / `sub esp,34h`.
        assert emu.reg_read(UC_X86_REG_EIP) == TICK_ON
        assert emu.reg_read(UC_X86_REG_ESP) == STACK - 4 - 0x34
        assert emu.reg_read(UC_X86_REG_EBP) == STACK - 4 and self.read(STACK - 4) == 0x0BADF00D
        assert emu.reg_read(UC_X86_REG_ECX) == RULE
        assert all(emu.reg_read(register) == 0x12345678 for register in KEPT)
        assert emu.reg_read(UC_X86_REG_FPSW) & 0x3800 == 0, "x87 stack left non-empty"
        return self.calls

    def kits(self, place):
        entry = PLACES + place * 0x30
        return self.byte(entry + 0xC), self.byte(entry + 0xD)


def switched(place, before, after, feet=(1.0, 2.0, 3.0), yaw=0.5, protection=0):
    """The calls of one respawn in place."""
    old, new = player_at(place, before), player_at(place, after)
    feet = struct.unpack("<3f", struct.pack("<3f", *feet))
    yaw = struct.unpack("<f", struct.pack("<f", yaw))[0]
    calls = [("remove", old), ("view", place), ("kit event", NOW, place, before, after),
             ("initialize", new, NOW, feet, yaw, 0.0)]
    return calls + [("put", new)], protection


def tick_cases():
    """(name, set-up, the build's calls)."""
    def plain(t):
        t.alive(3)
        t.choose(3, 1)
    yield "waits for the countdown", lambda t: (plain(t), t.state(1)), [switched(3, 0, 1)]
    yield "counts down", plain, [switched(3, 0, 1)]
    yield "in process", lambda t: (plain(t), t.state(3)), []
    yield "ended", lambda t: (plain(t), t.state(4)), []
    yield "no state yet", lambda t: (plain(t), t.state(0)), []
    yield "the same kit", lambda t: t.alive(3), []
    yield "kit 2 to kit 0", lambda t: (t.alive(7, kit=2, feet=(-5.5, 0.25, 100.0), yaw=-3.0), t.choose(7, 0)), \
        [switched(7, 2, 0, feet=(-5.5, 0.25, 100.0), yaw=-3.0)]
    yield "a kit with no player", lambda t: (plain(t), t.word(PLACES + 3 * 0x30 + 4, 0)), []
    yield "kit 3", lambda t: (t.alive(3), t.choose(3, 3)), []
    yield "dead", lambda t: (plain(t), t.byte(player_at(3, 0) + 0x33C, 0)), []
    yield "not in the world", lambda t: (plain(t), t.byte(player_at(3, 0) + 0x33F, 0)), []
    yield "due to spawn now", lambda t: (plain(t), t.word(RULE + 0x138 + 12, NOW)), []
    yield "due to spawn later", lambda t: (plain(t), t.word(RULE + 0x138 + 12, NOW + 50)), [switched(3, 0, 1)]
    yield "a shot fired", lambda t: (plain(t), t.emu.mem_write(t.record(3) + (22 * 3 + 0) * 2, b"\1\0")), []
    yield "a shot in kit 2", lambda t: (plain(t), t.emu.mem_write(t.record(3) + (22 * 3 + 2) * 2, b"\1\0")), []
    yield "a kill", lambda t: (plain(t), t.emu.mem_write(t.record(3), b"\1\0")), []
    yield "damage taken", lambda t: (plain(t), t.emu.mem_write(t.record(3) + 0xA8 + 19 * 12 + 8,
                                                               struct.pack("<f", 0.25))), []
    yield "another place's statistics", lambda t: (plain(t), t.emu.mem_write(t.record(4), b"\1\0")), \
        [switched(3, 0, 1)]
    yield "no statistics", lambda t: (plain(t), t.word(t.core + 0x18FA8, 0)), []
    yield "invulnerable 2.5 s more", lambda t: (t.alive(3, protected_until=NOW + 2500), t.choose(3, 1)), \
        [switched(3, 0, 1, protection=2500)]
    yield "invulnerable until now", lambda t: (t.alive(3, protected_until=NOW), t.choose(3, 1)), [switched(3, 0, 1)]
    yield "invulnerable until before", lambda t: (t.alive(3, protected_until=NOW - 1), t.choose(3, 1)), \
        [switched(3, 0, 1)]
    yield "places 0 and 19", lambda t: (t.alive(0), t.choose(0, 2), t.alive(19, kit=1), t.choose(19, 0)), \
        [switched(0, 0, 2), switched(19, 1, 0)]


def check_tick(data, patched):
    checks = 0
    for name, setup, expected in tick_cases():
        t = Tick(data)
        setup(t)
        before = [t.kits(place) for place in range(20)]
        calls = t.run()
        wanted = expected if patched else []
        assert calls == [call for calls_, _ in wanted for call in calls_], (patched, name, calls)
        for place in range(20):
            kits = t.kits(place)
            moved = [w for w in wanted if w[0][0][1] == player_at(place, before[place][0])]
            if moved:
                assert kits == (before[place][1],) * 2, (name, place, kits)
                assert t.read(RULE + 0x138 + place * 4) == 0xFFFFFFFF, name
                new = player_at(place, before[place][1])
                protection = moved[0][1]
                until = t.read(new + 0x900 + 0x32C)
                assert until == (NOW + protection if protection else 0xFFFFFFFF), (name, until)
            else:
                assert kits == before[place], (patched, name, place)
        checks += 1
    return checks


# The hud: game_world_ui (esi) and the local player (ebp), from 0x23B7B3
# (the alive flag compared with the one the hud saw) to the player's
# release (0x23B814).
UI_FROM, UI_TO = 0x23B7B3, 0x23B814
UI, GAME, WORLD, LOCAL = (HEAP + 0x10000 + i * 0x1000 for i in range(4))
SHOWN, STATE = 0x2DF, 0x1F8 + 0x18FE4


class Hud(Emu):
    def __init__(self, data, alive, seen_alive, shown, state):
        super().__init__(data)
        self.word(UI + 0x18, GAME)
        self.word(GAME + 0x4D4C, WORLD)
        self.word(WORLD + STATE, state)
        self.byte(UI + 0x2DE, seen_alive)
        self.byte(UI + SHOWN, shown)
        self.byte(LOCAL + 0x33C, alive)
        self.word(LOCAL + 0xD0, 5)                 # a reference besides the hud's

    def stub(self, emu, address, size, _):
        if address in (0x239C60, 0x23A1A0):        # the hud's other elements; al, edx
            assert emu.reg_read(UC_X86_REG_EAX) & 0xFF in (0, 1)
            self.calls.append((address, emu.reg_read(UC_X86_REG_EAX) & 0xFF))
            self.returns(emu.reg_read(UC_X86_REG_EAX), 0)
        elif address == 0x2383B0:                  # show or hide the kits: eax = the hud, [esp+4]
            assert emu.reg_read(UC_X86_REG_EAX) == UI
            show = self.read(emu.reg_read(UC_X86_REG_ESP) + 4) & 0xFF
            self.calls.append(("kits", show))
            self.byte(UI + SHOWN, show)
            self.returns(0, 4)

    def run(self):
        emu = self.emu
        emu.reg_write(UC_X86_REG_ESP, STACK)
        emu.reg_write(UC_X86_REG_ESI, UI)
        emu.reg_write(UC_X86_REG_EBP, LOCAL)
        emu.reg_write(UC_X86_REG_EBX, 0x12345678)
        emu.emu_start(UI_FROM, UI_TO, count=1000)
        assert emu.reg_read(UC_X86_REG_EIP) == UI_TO and emu.reg_read(UC_X86_REG_ESP) == STACK
        assert (emu.reg_read(UC_X86_REG_ESI), emu.reg_read(UC_X86_REG_EBP), emu.reg_read(UC_X86_REG_EBX)) \
            == (UI, LOCAL, 0x12345678)
        assert self.read(LOCAL + 0xD0) == 4
        return [call for call in self.calls if call[0] == "kits"]


def hud_cases():
    """(name, alive, alive as the hud saw it, kits shown, state, the original's, the build's)."""
    for state in (1, 2):
        yield f"alive in state {state}", 1, 0, 1, state, [("kits", 0)], [("kits", 1)]
        yield f"kits shown, alive in state {state}", 1, 1, 1, state, [], []
    yield "alive in play", 1, 0, 1, 3, [("kits", 0)], [("kits", 0)]
    yield "dead before the start", 0, 1, 0, 2, [("kits", 1)], [("kits", 1)]
    yield "dead in play", 0, 1, 0, 3, [("kits", 1)], [("kits", 1)]
    yield "kits shown, play begins", 1, 1, 1, 3, [], [("kits", 0)]
    yield "kits shown, dead in play", 0, 0, 1, 3, [], []
    yield "kits hidden, alive in play", 1, 1, 0, 3, [], []
    yield "kits shown, the match ended", 1, 1, 1, 4, [], [("kits", 0)]


def check_hud(data, patched):
    checks = 0
    for name, alive, seen, shown, state, original, build in hud_cases():
        hud = Hud(data, alive, seen, shown, state)
        assert hud.run() == (build if patched else original), (patched, name, hud.calls)
        checks += 1
    return checks


def main():
    original = open(sys.argv[1], "rb").read()
    code = open(base.default_bin("section"), "rb").read()
    late_join = open(base.late_join_bin(), "rb").read()
    patched, _ = base.build(original, "section", code, None, late_join)
    checks = 0
    for data, is_patched in ((original, False), (patched, True)):
        checks += check_tick(data, is_patched) + check_hud(data, is_patched)
    # A build that does not look at the statistics, or at the state, fails.
    for old, new in ((bytes.fromhex("837c8afc00"), bytes.fromhex("39c0909090")),   # cmp -> cmp eax,eax
                     (bytes.fromhex("48d1e8750d"), bytes.fromhex("48d1e8eb00"))):  # jnz .done -> on
        broken = bytearray(patched)
        at = broken.rfind(old)                     # in `.srp`, the last section with bytes
        assert at > base.Headers(broken).sections(broken)[-2][3]
        broken[at:at + len(old)] = new
        try:
            check_tick(bytes(broken), True)
        except AssertionError:
            checks += 1
        else:
            sys.exit(f"a broken build ({new.hex()}) passed")
    print(f"patch 30: {checks} runs as expected (the original, the build and two broken builds)")


if __name__ == "__main__":
    main()
