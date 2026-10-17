#!/usr/bin/env python3
"""Run the original and patch 31's respawn rule tick (0x316820), the
client's view of a kit change (0x2678E0) and the hud's update of the local
player's state (0x23B7B3) and the kits' pick (0x233F28) on made-up
objects.

    python check_kit_change_026g.py <original survarium.exe>

Requires Python `unicorn`. The places' tick (0x321790, which takes a
place's kit choice from its list), the respawn rule tick's start (up to
0x316826), the game's own spawn (0x316DA0, with patches 18 and 20 in the
build) and the view of a kit change run as machine code; the player's calls
(out of the world, initialize, into the world, its damage model), the
network client's, the cameras, the statistics' kit event and the hud's
calls are stubs. The rules: ../docs/notes/kit-change-before-start.md.

The build must respawn a place whose next kit is another, where it stands
and facing as it faces, with what was left of its invulnerability, while the
match waits or counts down, and only then: in the last tick before play, in
the tick whose time rule begins play (the places and the respawn rule tick
before the time rule, check 32 of selftest_026g.py), never after; whatever
tick the choice came to a game in, by the time the choice is for; never a
dead player, one out of the world, one with any statistics, a late joiner.
The view keeps the warmup's camera and input before play and gives the
player's in play, by the state the network client set the camera for. The
hud shows the kits to a player who becomes alive before play, keeps them
shown (and its cursor) after a pick before play, and hides them from a live
one in play. The original does none of it: it never changes a
kit of a live player, its view always gives the player camera, and its hud
only hides the kits at a respawn and shows them at a death. Says nothing
about what the real initialize, physics, cameras and hud movie do.
"""
import struct
import sys

from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_PROT_ALL, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EDX, UC_X86_REG_EIP, \
    UC_X86_REG_ESP, UC_X86_REG_EBX, UC_X86_REG_ESI, UC_X86_REG_EDI, UC_X86_REG_EBP, UC_X86_REG_FPSW

import patch_026g as base

TICK, TICK_ON = base.KIT_CHANGE_TICK_VA, base.KIT_CHANGE_TICK_VA + 6
SPAWN, PLACE_TICK, KIT_VIEW = 0x316DA0, 0x321790, 0x2678E0
PLAYER_CAMERA, WARMUP_CAMERA = 0x1F8200, 0x1F8190
HEAP = 0x20000000
RULE, RVT, CORE, PLACES, SVT, PVT, DMI = (HEAP + i * 0x1000 for i in range(7))
STATS = HEAP + 0x30000                             # 20 records of 1F8h
CHOICES = HEAP + 0x38000                           # kit-choice list nodes, 40h each
LISTS = HEAP + 0x7C000                             # the places' kit-choice lists, 480h each
CORE_SIZE = 0x1A000                                # the core's fields up to +18FE4h
PLAYERS = HEAP + 0x40000                           # 20 places x 3 kits, 0x1000 each
STACK, STOP, STUB = 0x2F008000, 0x2F100000, 0x2F100100
CLIENTS = 0x30000000
INIT, PUT, REMOVE, VIEW, STAT_EVENT, DAMAGE = (STUB + 0x10 * i for i in range(6))
NOW = 21_750
START = 20_400                                     # the tick whose time rule begins play
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
        emu.mem_map(CLIENTS, 0x400000, UC_PROT_ALL)    # network clients: +1F1C60h
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
        self.nodes = 0
        for at, value in ((RULE, RVT), (RULE + 0x134, core), (RVT + 0x5C, SPAWN), (RVT + 0x60, VIEW),
                          (core + 0x18458, PLACES), (core + 0x18FA8, STATS), (STATS, SVT),
                          (SVT + 0x2C, STAT_EVENT), (PVT + 0x1C, INIT), (PVT + 0x20, PUT),
                          (PVT + 0x24, REMOVE), (DMI + 8, DAMAGE), (core + 0x89D8, LISTS)):
            self.word(at, value)
        self.state(2)
        for place in range(20):
            self.word(RULE + 0x138 + place * 4, -1)
            entry = PLACES + place * 0x30
            for kit in range(3):
                p = player_at(place, kit)
                self.word(entry + kit * 4, p)
                self.word(p, PVT)
                self.word(p + 0x108, DMI)
                self.word(p + 0x10C, p + 0x800)    # an inventory of 23 empty slots
                self.byte(p + 0x114, place)
                self.byte(p + 0x115, kit)
                self.word(p + 0x700, p + 0x900)    # its damage model, [player+108h] -> +8 -> here
                self.word(p + 0x900 + 0x32C, -1)
            self.byte(entry + 0xC, 0)              # the first kit, as the slot's bind (0x321820)
            self.byte(entry + 0xD, 0)
            self.word(entry + 0x28, core)
            self.byte(entry + 0x2C, place)

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

    def relay(self, place, time, kit):
        """A kit choice in the place's list ([core+89D8h], 480h a place: +8 the
        newest node; a node: +4 the next older, +8 the kit, +20h its time),
        kept in time order as the network's insert keeps it."""
        node = CHOICES + self.nodes * 0x40
        self.nodes += 1
        self.word(node + 0x20, time)
        self.byte(node + 8, kit)
        head = LISTS + place * 0x480 + 8
        newer, older = 0, self.read(head)
        while older and self.read(older + 0x20) > time:
            newer, older = older, self.read(older + 4)
        self.word(node + 4, older)
        if newer:
            self.word(newer + 4, node)
        else:
            self.word(head, node)

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
            self.byte(this + 0x33C, 1)
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

    def call(self, address, this, time):
        """address(this, 0, time, 0) as thiscall, `ret 0Ch`, to its return."""
        emu = self.emu
        for at, value in ((STACK, STOP), (STACK + 4, 0), (STACK + 8, time), (STACK + 12, 0)):
            self.word(at, value)
        emu.reg_write(UC_X86_REG_ESP, STACK)
        emu.reg_write(UC_X86_REG_ECX, this)
        emu.emu_start(address, STOP, count=10000)
        assert emu.reg_read(UC_X86_REG_ESP) == STACK + 16

    def run(self, time=NOW):
        emu = self.emu
        for at, value in ((STACK, STOP), (STACK + 4, 0), (STACK + 8, time), (STACK + 12, 0)):
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

    def world_tick(self, time, start=START):
        """One tick in the order of the builds with the late join: the
        respawn rule (tick list 0, the core's counter for its key: this
        patch's pass, then the spawn of each place due now), the places
        (list 0, keys (place+1) << 16 from the late join's binds), then the
        time rule (list 1), which begins play at `start`."""
        self.run(time)
        for place in range(20):
            if self.read(RULE + 0x138 + place * 4) == time:
                self.spawn(place, time)
        for place in range(20):
            self.call(PLACE_TICK, PLACES + place * 0x30, time)
        if time >= start:
            self.state(3)

    def spawn(self, place, time):
        """The tick's own spawn of a place due now (0x3169B9: the game's
        spawn 0x316DA0 with (place, point, time)), at a point of the map."""
        point = STACK - 0x400
        self.emu.mem_write(point, bytes(0x24))
        self.emu.mem_write(point + 8, struct.pack("<4f", 50.0, 0.0, -20.0, 1.5))
        emu = self.emu
        for at, value in ((STACK, STOP), (STACK + 4, place), (STACK + 8, point), (STACK + 12, time)):
            self.word(at, value)
        emu.reg_write(UC_X86_REG_ESP, STACK)
        emu.reg_write(UC_X86_REG_ECX, RULE)
        emu.emu_start(SPAWN, STOP, count=20000)
        assert emu.reg_read(UC_X86_REG_ESP) == STACK + 16

    def die(self, place, respawn_at):
        """The place's player dead, its respawn time set (0x316AA0)."""
        kit = self.kits(place)[0]
        self.byte(player_at(place, kit) + 0x33C, 0)
        self.word(RULE + 0x138 + place * 4, respawn_at)

    def kits(self, place):
        entry = PLACES + place * 0x30
        return self.byte(entry + 0xC), self.byte(entry + 0xD)


def switched(place, before, after, feet=(1.0, 2.0, 3.0), yaw=0.5, protection=0, time=NOW):
    """The calls of one respawn in place."""
    old, new = player_at(place, before), player_at(place, after)
    feet = struct.unpack("<3f", struct.pack("<3f", *feet))
    yaw = struct.unpack("<f", struct.pack("<f", yaw))[0]
    calls = [("remove", old), ("view", place), ("kit event", time, place, before, after),
             ("initialize", new, time, feet, yaw, 0.0)]
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
    # A late joiner comes into a match in play: its slot is bound with its
    # first kit current and next (0x321820), it spawns in it, and a choice
    # of its is for its next life.
    yield "a late joiner", lambda t: (t.state(3), t.alive(12)), []
    yield "a late joiner's choice", lambda t: (t.state(3), t.alive(12), t.choose(12, 1)), []


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


def boundary_cases():
    """(name, the relayed choices as (time, kit), the ticks played, the
    tick of the build's switch or None, the kits after)."""
    ticks = [START - 150, START - 100, START - 50, START, START + 50, START + 100]
    # A choice for a tick is the place's next kit from that tick's places on,
    # so the respawn rule plays it at the next tick.
    yield "a choice for two ticks before play", [(START - 100, 1)], ticks, START - 50, (1, 1)
    yield "a choice for the last tick before play", [(START - 50, 1)], ticks, START, (1, 1)
    yield "a choice for the tick play begins in", [(START, 1)], ticks, None, (0, 1)
    yield "a choice for the first tick of play", [(START + 50, 1)], ticks, None, (0, 1)
    yield "two choices before play, the last wins", [(START - 150, 2), (START - 50, 1)], ticks, \
        [START - 100, START], (1, 1)
    yield "a choice before, and one in play", [(START - 50, 2), (START + 50, 1)], ticks, START, (2, 1)


def check_boundary(data, patched):
    """The world's ticks around the start in the game's order: a choice
    switches at the tick after the one it is for while that is before play
    or is the one play begins in (before the time rule's change), never
    later. A game that gets the choice late plays those ticks again from a
    world before it (a rollback): a fresh game with the choice in its list
    is that game, and switches at the same tick."""
    checks = 0
    for name, choices, ticks, at, after in boundary_cases():
        t = Tick(data)
        t.alive(5)
        for time, kit in choices:
            t.relay(5, time, kit)
        switches = []
        for time in ticks:
            done = len(t.calls)
            t.world_tick(time)
            switches += [time for call in t.calls[done:] if call[0] == "initialize"]
        wanted = ([] if at is None else at if isinstance(at, list) else [at]) if patched else []
        assert switches == wanted, (patched, name, switches)
        assert t.kits(5) == (after if patched else (0, after[1])), (patched, name, t.kits(5))
        assert t.read(t.core + 0x18FE4) == 3
        checks += 1
    return checks


# The client's view of a spawn into another kit (0x2678E0): ebx = the
# client's respawn rule, [esp+4] the place.
GAME, CVT = HEAP + 0x10000, HEAP + 0x11000
CLIENT = CLIENTS


class View(Emu):
    def __init__(self, data, shown_state, local=True):
        super().__init__(data)
        self.local = local
        self.word(RULE + 0x1A8, GAME)
        self.word(GAME + 0x53AC, CLIENT)
        self.word(CLIENT, CVT)
        for slot, stub in ((0x54, STUB), (0x4C, STUB + 0x10), (0x5C, STUB + 0x20)):
            self.word(CVT + slot, stub)
        self.word(CLIENT + base.KIT_CHANGE_SHOWN_STATE, shown_state)

    def stub(self, emu, address, size, _):
        sp = emu.reg_read(UC_X86_REG_ESP)
        if address == STUB:                        # is the place the local one
            self.returns(1 if self.local else 0, 4)
        elif address == STUB + 0x10:               # the place's player, into [arg]
            self.word(self.read(sp + 4), 0)
            self.calls.append(("bind",))
            self.returns(self.read(sp + 4), 8)
        elif address == STUB + 0x20:               # made the local player
            self.returns(0, 4)
        elif address == PLAYER_CAMERA:             # eax = the game world, [esp+4] first person
            assert emu.reg_read(UC_X86_REG_EAX) == GAME + 0x18
            self.calls.append(("player camera", self.read(sp + 4)))
            self.returns(0, 4)
        elif address == WARMUP_CAMERA:             # edx = the game world
            assert emu.reg_read(UC_X86_REG_EDX) == GAME + 0x18
            self.calls.append(("warmup camera",))
            self.returns(0, 0)

    def run(self):
        emu = self.emu
        for at, value in ((STACK, STOP), (STACK + 4, 6)):
            self.word(at, value)
        emu.reg_write(UC_X86_REG_ESP, STACK)
        emu.reg_write(UC_X86_REG_ECX, RULE)
        for register in (UC_X86_REG_EBX, UC_X86_REG_ESI, UC_X86_REG_EDI, UC_X86_REG_EBP):
            emu.reg_write(register, 0x12345678)
        emu.emu_start(KIT_VIEW, STOP, count=2000)
        assert emu.reg_read(UC_X86_REG_ESP) == STACK + 8
        assert all(emu.reg_read(register) == 0x12345678
                   for register in (UC_X86_REG_EBX, UC_X86_REG_ESI, UC_X86_REG_EDI, UC_X86_REG_EBP))
        return self.calls


def view_cases():
    """(name, the state the camera is set for, local, the original's calls, the build's)."""
    player, warmup = [("bind",), ("player camera", 1)], [("bind",), ("warmup camera",)]
    yield "a switch while waiting", 1, True, player, warmup
    yield "a switch counting down", 2, True, player, warmup
    yield "a switch before any state", 0, True, player, warmup
    # A rollback that plays a switch again after play began: the camera is
    # play's, set by the time rule's change of the tick first played.
    yield "a switch played again in play", 3, True, player, player
    yield "a respawn into another kit in play", 3, True, player, player
    yield "another place's switch", 2, False, [], []


def check_view(data, patched):
    checks = 0
    for name, state, local, original, build in view_cases():
        assert View(data, state, local).run() == (build if patched else original), (patched, name)
        checks += 1
    return checks


# The hud: game_world_ui (esi) and the local player (ebp), from 0x23B7B3
# (the alive flag compared with the one the hud saw) to the player's
# release (0x23B814).
UI_FROM, UI_TO = 0x23B7B3, 0x23B814
UI, WORLD, HGAME, LOCAL = (HEAP + 0x20000 + i * 0x1000 for i in range(4))
NET = CLIENTS + 0x200000
SHOWN = 0x2DF


class Hud(Emu):
    def __init__(self, data, alive, seen_alive, shown, state):
        super().__init__(data)
        self.word(UI + 0x18, WORLD)
        self.word(WORLD + 0x9C, HGAME)
        self.word(HGAME + 0x53AC, NET)
        self.word(NET + base.KIT_CHANGE_SHOWN_STATE, state)
        self.byte(UI + 0x2DE, seen_alive)
        self.byte(UI + SHOWN, shown)
        self.byte(LOCAL + 0x33C, alive)
        self.word(LOCAL + 0xD0, 5)                 # a reference besides the hud's

    def stub(self, emu, address, size, _):
        if address in (0x239C60, 0x23A1A0):        # the hud's other elements; al, edx
            assert emu.reg_read(UC_X86_REG_EAX) & 0xFF in (0, 1)
            self.returns(emu.reg_read(UC_X86_REG_EAX), 0)
        elif address == 0x2383B0:                  # show or hide the kits: eax = the hud, [esp+4]
            assert emu.reg_read(UC_X86_REG_EAX) == UI
            show = self.read(emu.reg_read(UC_X86_REG_ESP) + 4) & 0xFF
            if self.byte(UI + SHOWN) != show:      # the real one returns at once when it is so
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
        return self.calls


def hud_cases():
    """(name, alive, alive as the hud saw it, kits shown, the state the
    camera is set for, the original's, the build's)."""
    for state in (0, 1, 2):
        yield f"alive in state {state}", 1, 0, 1, state, [("kits", 0)], []
        yield f"alive in state {state}, kits hidden", 1, 0, 0, state, [], [("kits", 1)]
        yield f"kits shown, alive in state {state}", 1, 1, 1, state, [], []
        yield f"kits hidden, alive in state {state}", 1, 1, 0, state, [], [("kits", 1)]
    yield "alive in play", 1, 0, 1, 3, [("kits", 0)], [("kits", 0)]
    yield "a late joiner alive", 1, 0, 0, 3, [], []
    yield "dead before the start", 0, 1, 0, 2, [("kits", 1)], [("kits", 1)]
    yield "dead in play", 0, 1, 0, 3, [("kits", 1)], [("kits", 1)]
    yield "kits shown, play begins", 1, 1, 1, 3, [], [("kits", 0)]
    yield "kits shown, dead in play", 0, 0, 1, 3, [], []
    yield "kits hidden, alive in play", 1, 1, 0, 3, [], []


def check_hud(data, patched):
    checks = 0
    for name, alive, seen, shown, state, original, build in hud_cases():
        hud = Hud(data, alive, seen, shown, state)
        assert hud.run() == (build if patched else original), (patched, name, hud.calls)
        checks += 1
    return checks


# The profile_changed callback after it queued the pick (0x1F8070): from
# 0x233F28 (ebx = the hud) to its `pop edi` (0x233F31).
PICK_FROM, PICK_TO = 0x233F28, 0x233F31


class Pick(Hud):
    def run(self):
        emu = self.emu
        emu.reg_write(UC_X86_REG_ESP, STACK)
        emu.reg_write(UC_X86_REG_EBX, UI)
        emu.reg_write(UC_X86_REG_ESI, 0x12345678)
        emu.reg_write(UC_X86_REG_EBP, 0x0BADF00D)
        emu.emu_start(PICK_FROM, PICK_TO, count=1000)
        assert emu.reg_read(UC_X86_REG_EIP) == PICK_TO and emu.reg_read(UC_X86_REG_ESP) == STACK
        assert (emu.reg_read(UC_X86_REG_EBX), emu.reg_read(UC_X86_REG_ESI), emu.reg_read(UC_X86_REG_EBP)) \
            == (UI, 0x12345678, 0x0BADF00D)
        return self.calls


def pick_cases():
    """(name, alive, the state the camera is set for, the original's, the
    build's): the kits are shown; the build leaves them so, and the hud's
    cursor where it is, for another pick."""
    for state in (0, 1, 2):
        yield f"a pick alive in state {state}", 1, state, [("kits", 0)], []
    yield "a pick dead before the start", 0, 2, [("kits", 0)], []
    yield "a pick on the death screen in play", 0, 3, [("kits", 0)], []


def check_pick(data, patched):
    checks = 0
    for name, alive, state, original, build in pick_cases():
        pick = Pick(data, alive, alive, 1, state)
        assert pick.run() == (build if patched else original), (patched, name, pick.calls)
        checks += 1
    return checks


def death_cases():
    """(name, the relayed choices as (time, kit), the kit of the respawn,
    the next kit after it): a dead player in play, due to respawn at R."""
    R = 30_000
    yield "a pick", [(R - 2000, 1)], 1, 1
    yield "a pick and a re-pick: the last wins", [(R - 2000, 1), (R - 1000, 2)], 2, 2
    yield "a pick for the tick before the respawn", [(R - 2000, 1), (R - 50, 2)], 2, 2
    yield "a pick for the tick of the respawn: the next life's", [(R - 2000, 1), (R, 2)], 1, 2
    yield "no pick", [], 0, 0


def check_death(data, patched):
    """The death screen in play: the original's and the build's respawn
    alike, by the last kit choice for a tick before the respawn's; a choice
    for the respawn's tick is the next life's (the respawn rule ticks before
    the places). No switch while alive in play."""
    R = 30_000
    checks = 0
    for name, choices, kit, after in death_cases():
        t = Tick(data)
        t.state(3)
        t.alive(9)
        t.die(9, R)
        for time, choice in choices:
            t.relay(9, time, choice)
        for time in range(R - 2100, R + 150, 50):
            t.world_tick(time, start=0)
        spawned = [call for call in t.calls if call[0] == "initialize"]
        assert spawned == [("initialize", player_at(9, kit), R,
                            struct.unpack("<3f", struct.pack("<3f", 50.0, 0.0, -20.0)), 1.5, 0.0)], \
            (patched, name, spawned)
        assert t.kits(9) == (kit, after), (patched, name, t.kits(9))
        assert not [call for call in t.calls if call[0] == "remove"], (patched, name)
        checks += 1
    return checks


def main():
    original = open(sys.argv[1], "rb").read()
    code = open(base.default_bin("section"), "rb").read()
    late_join = open(base.late_join_bin(), "rb").read()
    patched, _ = base.build(original, "section", code, None, late_join)
    checks = 0
    for data, is_patched in ((original, False), (patched, True)):
        checks += check_tick(data, is_patched) + check_boundary(data, is_patched)
        checks += check_view(data, is_patched) + check_hud(data, is_patched) + check_pick(data, is_patched)
        checks += check_death(data, is_patched)
    # A build that does not look at the statistics, or at the state, fails;
    # so does one whose view always gives the warmup.
    for old, new, check in ((bytes.fromhex("837c8afc00"), bytes.fromhex("39c0909090"), check_tick),
                            (bytes.fromhex("48d1e8750d"), bytes.fromhex("48d1e8eb00"), check_boundary),
                            (bytes.fromhex("83be601c1f000375"), bytes.fromhex("83be601c1f0003eb"), check_view)):
        broken = bytearray(patched)
        at = broken.rfind(old)                     # in `.srp`, the last section with bytes
        assert at > base.Headers(broken).sections(broken)[-2][3]
        broken[at:at + len(old)] = new
        try:
            check(bytes(broken), True)
        except AssertionError:
            checks += 1
        else:
            sys.exit(f"a broken build ({new.hex()}) passed")
    print(f"patch 31: {checks} runs as expected (the original, the build and three broken builds)")


if __name__ == "__main__":
    main()
