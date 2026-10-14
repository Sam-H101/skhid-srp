#!/usr/bin/env python3
"""Execute build 2777's roster-capacity paths with Unicorn (2.1.4).

Usage: check_late_join_026g.py ORIGINAL --nasm /path/to/nasm
The original instructions run against small fixture objects. Per-player
statistics codecs and the animation bind method are intercepted. Both a
synthetic binder and the native binder with weapon inventories are exercised;
the bounded profile wrapper executes the native constructor and decoder with
only CRT memcpy intercepted. These tests establish profile decoding, capacity,
calling convention and ID allocation, not a playable late join.
"""
import argparse
import hashlib
from pathlib import Path
import struct

from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import (
    UC_X86_REG_EAX, UC_X86_REG_EBX, UC_X86_REG_ECX, UC_X86_REG_EDI,
    UC_X86_REG_ESI, UC_X86_REG_EIP, UC_X86_REG_ESP, UC_X86_REG_EBP,
)

import late_join_026g as patch

BASE, SLOT, COUNTER, STACK, STOP = 0x1000000, 0x1030000, 0x1031000, 0x2008000, 0x3000000


class Machine:
    def __init__(self, data):
        self.uc = Uc(UC_ARCH_X86, UC_MODE_32)
        # Map only executable sections; unrelated runtime globals are not used.
        for name, rva, size, raw, raw_size, flags in patch.base.Headers(data).sections(data):
            if flags & 0x20000000 or name.startswith(b".srpd"):
                self.uc.mem_map(patch.base.IMAGE_BASE + rva, (max(size, raw_size) + 4095) & ~4095)
                if raw_size:
                    self.uc.mem_write(patch.base.IMAGE_BASE + rva, bytes(data[raw:raw + raw_size]))
        self.uc.mem_map(BASE, 0x40000)
        self.uc.mem_map(0x2000000, 0x10000)
        self.uc.mem_map(STOP, 4096)

    def read(self, address):
        return struct.unpack("<I", self.uc.mem_read(address, 4))[0]

    def write(self, address, value):
        self.uc.mem_write(address, struct.pack("<I", value))

    def start(self, address, end=STOP, args=(), count=10000):
        self.uc.reg_write(UC_X86_REG_ESP, STACK)
        self.uc.mem_write(STACK, struct.pack("<" + "I" * (len(args) + 1), STOP, *args))
        self.uc.emu_start(address, end, count=count)
        assert self.uc.reg_read(UC_X86_REG_EIP) == end, "instruction budget exhausted"


def statistics(data, roster, reading=False):
    machine = Machine(data)
    uc = machine.uc
    uc.mem_write(BASE + 0x31E0, bytes([roster]))
    uc.reg_write(UC_X86_REG_ESI, BASE)
    uc.reg_write(UC_X86_REG_EAX, BASE + 0x3500)
    seen = []
    codec = 0x3358B0 if reading else 0x335750

    def visit(uc, address, size, _):
        if address != codec:
            return
        esp = uc.reg_read(UC_X86_REG_ESP)
        seen.append((machine.read(esp + 4) - BASE) // 0x1F8)
        # Both native callers pass the statistics record on the stack.
        uc.reg_write(UC_X86_REG_EIP, machine.read(esp))
        uc.reg_write(UC_X86_REG_ESP, esp + (8 if reading else 12))

    uc.hook_add(UC_HOOK_CODE, visit)
    machine.start(0x2F5BE0 if reading else 0x2F5AC0,
                  0x2F5C0B if reading else 0x2F5AFC)
    assert uc.mem_read(BASE + 0x31E0, 1)[0] == roster
    return seen


def active_objects(machine, core, count=0):
    begin = core + 0x18B58
    machine.write(core + 0x18B4C, begin)
    machine.write(core + 0x18B50, begin + count * 4)
    machine.write(core + 0x18B54, begin + 255 * 4)
    return begin


def binding(data, place, consumed, old_counter):
    machine = Machine(data)
    uc = machine.uc
    active_objects(machine, BASE)
    machine.write(BASE + 0x18FB0, COUNTER)
    machine.write(COUNTER, old_counter)
    uc.mem_write(SLOT + 0x2C, bytes([place]))
    uc.reg_write(UC_X86_REG_EAX, BASE)
    uc.reg_write(UC_X86_REG_EBX, 0x12345678)
    uc.reg_write(UC_X86_REG_ESI, 0x23456789)
    ids = []

    def visit(uc, address, size, _):
        if address != 0x321820:
            return
        esp = uc.reg_read(UC_X86_REG_ESP)
        assert uc.reg_read(UC_X86_REG_EAX) == BASE
        assert machine.read(esp + 4) == SLOT
        counter = machine.read(BASE + 0x18FB0)
        ids.extend(range(machine.read(counter), machine.read(counter) + consumed))
        machine.write(counter, machine.read(counter) + consumed)
        uc.reg_write(UC_X86_REG_EAX, 0xDEADBEEF)  # native volatile register
        uc.reg_write(UC_X86_REG_EIP, machine.read(esp))
        uc.reg_write(UC_X86_REG_ESP, esp + 8)

    uc.hook_add(UC_HOOK_CODE, visit)
    displacement = struct.unpack_from("<i", data, patch.base.file_offset(0x2D3880) + 1)[0]
    machine.start(0x2D3885 + displacement, args=(SLOT,))
    assert machine.read(BASE + 0x18FB0) == COUNTER
    assert machine.read(COUNTER) == old_counter
    assert uc.reg_read(UC_X86_REG_ESP) == STACK + 8
    assert uc.reg_read(UC_X86_REG_EBX) == 0x12345678
    assert uc.reg_read(UC_X86_REG_ESI) == 0x23456789
    assert ids == list(range((place + 1) << 16, ((place + 1) << 16) + consumed))


def native_binding(data, place, kits, weapons=False, world_objects=0):
    """Run the actual slot/player/inventory/weapon binders.

    Only the animation object's virtual bind method is replaced by a ret 4.
    In particular the native code, not the test, assigns the callback IDs.
    """
    machine = Machine(data)
    uc = machine.uc
    begin = active_objects(machine, BASE, world_objects)
    uc.mem_write(begin + 120 * 4, b"\xa5" * (135 * 4))
    machine.write(BASE + 0x18FB0, COUNTER)
    machine.write(COUNTER, 73)
    uc.mem_write(SLOT + 0x2C, bytes([place]))
    vtable, animation_bind = BASE + 0x32000, BASE + 0x32100
    machine.write(vtable, animation_bind)
    uc.mem_map(0x5000000, 0x6000)
    weapon_vtable = BASE + 0x33000
    machine.write(weapon_vtable + 0x70, 0x2E30F0)
    uc.mem_write(animation_bind, b"\xc2\x04\x00")
    for kit in kits:
        player = BASE + 0x34000 + kit * 0x1000
        inventory = player + 0x800
        machine.write(SLOT + kit * 4, player)
        machine.write(player + 0x10C, inventory)
        machine.write(player + 0x3A4, vtable)
        if weapons:
            for index, slot in enumerate((7, 10)):
                weapon = 0x5000000 + kit * 0x2000 + index * 0x1000
                machine.write(weapon + 0x10, weapon_vtable)
                machine.write(inventory + 0x10C + slot * 4, weapon + 0x10)
    uc.reg_write(UC_X86_REG_EAX, BASE)
    displacement = struct.unpack_from("<i", data, patch.base.file_offset(0x2D3880) + 1)[0]
    machine.start(0x2D3885 + displacement, args=(SLOT,))
    first = (place + 1) << 16
    assert machine.read(SLOT + 0x28) == BASE
    assert list(uc.mem_read(SLOT + 0xC, 2)) == [min(kits)] * 2
    for ordinal, kit in enumerate(kits):
        player = machine.read(SLOT + kit * 4)
        assert machine.read(player + 0x120) == BASE
        assert machine.read(player + 0x800 + 0x108) == BASE
        assert machine.read(player + 0x698) == first + ordinal * 2
        assert machine.read(player + 0x6B0) == first + ordinal * 2 + 1
        if weapons:
            for index in range(2):
                weapon = 0x5000000 + kit * 0x2000 + index * 0x1000
                object_id = place * 6 + ordinal * 2 + index
                assert uc.mem_read(weapon + 0xC, 1)[0] == object_id
                assert machine.read(begin + object_id * 4) == weapon
                assert machine.read(weapon + 4) == BASE
    assert machine.read(BASE + 0x18B50) == begin + max(120, world_objects) * 4
    assert uc.mem_read(begin + 120 * 4, 135 * 4) == b"\xa5" * (135 * 4)
    assert machine.read(SLOT + 0x20) == first + len(kits) * 2
    assert machine.read(BASE + 0x18FB0) == COUNTER
    assert machine.read(COUNTER) == 73
    assert uc.reg_read(UC_X86_REG_ESP) == STACK + 8


def saved_list(machine, core):
    """The list the world saves its game objects by (core+0x18914 the first,
    +0x18918 the last), walked from the first with every link checked."""
    nodes, before, node = [], 0, machine.read(core + 0x18914)
    while node:
        assert machine.read(node + 4) == before, "the link back of a saved object"
        assert len(nodes) < 1000, "the list of saved objects does not end"
        nodes.append(node)
        before, node = node, machine.read(node + 8)
    assert machine.read(core + 0x18918) == before, "the last of the saved objects"
    return nodes


def stack_items(machine, inventory, slot, bind, objects, vector):
    """An inventory item that is a stack of world objects (a grenade or a
    trap item: the objects' pointers at +0x268..+0x26C) whose vtable+0x70,
    the bind the inventory calls, is the game's `bind`."""
    item, vtable = vector + 0x100, vector + 0x400
    machine.write(vtable + 0x70, bind)
    machine.write(item, vtable)
    for index, each in enumerate(objects):
        machine.write(vector + index * 4, each)
    machine.write(item + 0x268, vector)
    machine.write(item + 0x26C, vector + len(objects) * 4)
    machine.write(inventory + 0x10C + slot * 4, item)
    return item


def saved_objects(data):
    """The game's own binders (slot 0x321820, kit 0x2D75B0, inventory
    0x2FC7D0, grenade item 0x309EC0 and grenade 0x303820, trap item
    0x3067A0) on made-up kits, through the hooked call: where each slot's
    grenades and traps are in the list the world is saved by, whenever the
    slot is bound. Only the animation object's bind is a `ret 4`."""
    machine = Machine(data)
    uc = machine.uc
    active_objects(machine, BASE)
    machine.write(BASE + 0x18FB0, COUNTER)
    machine.write(COUNTER, 73)
    uc.mem_map(0x5000000, 0x80000)
    free = [0x5000000]

    def room(size):
        at = free[0]
        free[0] += (size + 0xFFF) & ~0xFFF
        return at

    vtable = room(0x200)
    machine.write(vtable, vtable + 0x100)
    uc.mem_write(vtable + 0x100, b"\xc2\x04\x00")
    displacement = struct.unpack_from("<i", data, patch.base.file_offset(0x2D3880) + 1)[0]

    def bind(place, kits):
        """kits: {kit: (grenades, traps)}. Returns the nodes the slot's
        objects have in the list, in the order the game binds them."""
        slot = room(0x40)
        uc.mem_write(slot + 0x2C, bytes([place]))
        expected, ids, next_id = [], [], (place + 1) << 16
        for kit in sorted(kits):
            grenades, traps = kits[kit]
            player = room(0x1000)
            inventory = player + 0x800
            machine.write(slot + kit * 4, player)
            machine.write(player + 0x10C, inventory)
            machine.write(player + 0x3A4, vtable)
            # The inventory binds its slots in their order: the grenades'
            # (14) before the traps' (16).
            objects = [room(0x200) for _ in range(grenades)]
            if objects:
                stack_items(machine, inventory, 14, 0x309EC0, objects, room(0x800))
            for each in objects:
                expected.append(each + 0x10)
                ids.append((each + 0x168, next_id))
                ids.append((each + 0x180, next_id + 1))
                ids.append((each + 0x188, BASE))
                next_id += 2
            objects = [room(0x300) for _ in range(traps)]
            if objects:
                stack_items(machine, inventory, 16, 0x3067A0, objects, room(0x800))
            for each in objects:
                expected.append(each + 0x74)
                ids.append((each + 0x234, next_id))
                ids.append((each + 0x19C, BASE))
                next_id += 1
            ids.append((player + 0x698, next_id))
            ids.append((player + 0x6B0, next_id + 1))
            next_id += 2
        ids.append((slot + 0x20, next_id))
        saved = {UC_X86_REG_EBX: 0x12345678, UC_X86_REG_ESI: 0x23456789,
                 UC_X86_REG_EDI: 0x3456789A, UC_X86_REG_EBP: 0x456789AB}
        for register, value in saved.items():
            uc.reg_write(register, value)
        uc.reg_write(UC_X86_REG_EAX, BASE)
        machine.start(0x2D3885 + displacement, args=(slot,), count=100000)
        assert uc.reg_read(UC_X86_REG_ESP) == STACK + 8
        assert all(uc.reg_read(register) == value for register, value in saved.items())
        assert machine.read(BASE + 0x18FB0) == COUNTER and machine.read(COUNTER) == 73
        for address, value in ids:
            assert machine.read(address) == value, hex(address)
        return expected

    def map_object():
        """As the map's objects come into the list (0x285864..0x28588F):
        at its end."""
        node, last = room(0x40), machine.read(BASE + 0x18918)
        machine.write(node + 4, last)
        machine.write(last + 8 if last else BASE + 0x18914, node)
        machine.write(BASE + 0x18918, node)
        return node

    # A slot with nothing to save, into a list with nothing in it.
    assert bind(0, {0: (0, 0), 1: (0, 0)}) == [] and saved_list(machine, BASE) == []
    # The start of a match: its slots, then the map's objects.
    first = bind(1, {0: (2, 1), 1: (0, 2), 2: (3, 0)})
    assert len(first) == 8 and saved_list(machine, BASE) == first
    second = bind(2, {1: (1, 0)})
    assert saved_list(machine, BASE) == second + first
    of_map = [map_object() for _ in range(3)]
    assert saved_list(machine, BASE) == second + first + of_map
    # Slots bound while the match is played: in front of all of it, as a
    # game that starts with them has them.
    assert bind(3, {0: (0, 0), 2: (0, 0)}) == []
    assert saved_list(machine, BASE) == second + first + of_map
    fourth = bind(4, {0: (2, 0), 2: (2, 2)})
    assert len(fourth) == 6 and saved_list(machine, BASE) == fourth + second + first + of_map
    fifth = bind(19, {1: (0, 1)})
    assert saved_list(machine, BASE) == fifth + fourth + second + first + of_map
    # A list of the map's objects alone, and a slot with grenades after it.
    machine = Machine(data)
    uc = machine.uc
    active_objects(machine, BASE)
    machine.write(BASE + 0x18FB0, COUNTER)
    machine.write(COUNTER, 73)
    uc.mem_map(0x5000000, 0x80000)
    free[0] = 0x5000000
    vtable = room(0x200)
    machine.write(vtable, vtable + 0x100)
    uc.mem_write(vtable + 0x100, b"\xc2\x04\x00")
    of_map = [map_object() for _ in range(2)]
    late = bind(5, {0: (2, 0)})
    assert saved_list(machine, BASE) == late + of_map


def untouched_objects(data):
    """What the install puts into the kept states for each node of a late
    slot is one zero byte. Here the game's own code says so: a grenade and
    a trap as their constructors leave them (grenade_core 0x303640; the
    trap's 0x3043C0 as far as 0x3044E3, where its own fields are set) save
    themselves as that byte (0x303DA0, 0x3052E0) and load it back
    (0x303F80, 0x3053C0) reading nothing more, changed in nothing and
    calling nothing but the grenade's look for its owner. The two
    constructors of the base every game object has (0xF70F0, 0xF17E0)
    are a `ret`; a virtual call would read a vtable that is not mapped."""
    m = Machine(data)
    uc = m.uc
    uc.mem_map(0x1F1D000, 4096)      # the writers' debug flags: off
    uc.mem_map(0x5000000, 0x10000)
    grenade, trap, owner = 0x5000000, 0x5001000, 0x5004000
    stream, buffer, saved = 0x5002000, 0x5002100, 0x5003000
    owner_asked = []

    def visit(uc, address, size, _):
        if address not in (0xF70F0, 0xF17E0, STOP + 0x300):
            return
        esp = uc.reg_read(UC_X86_REG_ESP)
        if address == STOP + 0x300:
            owner_asked.append(uc.reg_read(UC_X86_REG_ECX))
            uc.reg_write(UC_X86_REG_EAX, 0)
        uc.reg_write(UC_X86_REG_EIP, m.read(esp))
        uc.reg_write(UC_X86_REG_ESP, esp + (12 if address == 0xF70F0 else 4))

    uc.hook_add(UC_HOOK_CODE, visit)

    def save(node, function):
        """The bytes the node writes: into a stream as a state has it
        (+0x14 its buffer: an allocator, the bytes, the room, the size)."""
        uc.mem_write(saved, b"\xEE" * 0x100)
        for address, value in ((stream + 0x14, buffer), (buffer, 0), (buffer + 4, saved),
                               (buffer + 8, 0x100), (buffer + 12, 0)):
            m.write(address, value)
        uc.reg_write(UC_X86_REG_ECX, node)
        m.start(function, args=(stream, 0))
        assert uc.reg_read(UC_X86_REG_ESP) == STACK + 12
        written = bytes(uc.mem_read(saved, m.read(buffer + 12)))
        assert bytes(uc.mem_read(saved + len(written), 4)) == b"\xEE" * 4
        return written

    def load(node, function, byte):
        """How many bytes the node reads of `byte` and 0xEE after it."""
        uc.mem_write(saved, bytes([byte]) + b"\xEE" * 0xFF)
        for address, value in ((stream, saved), (stream + 4, saved), (stream + 8, 0x100)):
            m.write(address, value)
        uc.reg_write(UC_X86_REG_ECX, node)
        m.start(function, args=(stream, 0, 0))
        assert uc.reg_read(UC_X86_REG_ESP) == STACK + 16
        return m.read(stream + 4) - saved

    # The grenade: its owner is asked for (the item, +0x250, +0x174, the
    # vtable's +0xC) and has no player.
    uc.reg_write(UC_X86_REG_EDI, grenade)
    m.start(0x303640)
    assert uc.reg_read(UC_X86_REG_EAX) == grenade
    for address, value in ((grenade + 0xC, owner), (owner + 0x250, owner + 0x300),
                           (owner + 0x300 + 0x174, owner + 0x600), (owner + 0x600, owner + 0x700),
                           (owner + 0x700 + 0xC, STOP + 0x300)):
        m.write(address, value)
    whole = bytes(uc.mem_read(grenade, 0x220))
    assert save(grenade + 0x10, 0x303DA0) == b"\0"
    assert load(grenade + 0x10, 0x303F80, 0) == 1 and owner_asked == [owner + 0x600]
    assert bytes(uc.mem_read(grenade, 0x220)) == whole
    # The trap.
    m.start(0x3043C0, end=0x3044E3, args=(trap, 0x1234))
    whole = bytes(uc.mem_read(trap, 0x270))
    assert save(trap + 0x74, 0x3052E0) == b"\0"
    assert load(trap + 0x74, 0x3053C0, 0) == 1 and len(owner_asked) == 1
    assert bytes(uc.mem_read(trap, 0x270)) == whole


def native_histories(data):
    """Construct all four vectors through the original engine functions."""
    machine = Machine(data)
    uc = machine.uc
    # Native pool constructors inspect their initially-zero diagnostic flags.
    uc.mem_map(0x2260000, 4096)
    vectors = []
    for offset, stride, constructor in ((0x2FCC, 0x5A0C, 0x2D41B0),
                                        (0xE3E4, 0x1F4C, 0x2D4330)):
        for bank in range(2):
            vector = BASE + offset + bank * stride
            uc.reg_write(UC_X86_REG_ECX, vector)
            machine.start(constructor)
            assert machine.read(vector) == vector + 12
            assert machine.read(vector + 8) == vector + stride
            vectors.append(vector)
    uc.reg_write(UC_X86_REG_EBP, BASE)
    machine.start(0x2D3906, end=0x2D395F, count=100000)
    for vector in vectors:
        assert machine.read(vector + 4) == machine.read(vector + 8)


def native_profile(data):
    """Validate, then execute the original profile constructor and reader.

    Only CRT memcpy is intercepted; dictionary lookup and inventory vector
    construction/readers run natively, including their inline-pointer setup.
    The wire fixture ends at an unmapped page to detect even one-byte overreads.
    """
    m = Machine(data)
    uc = m.uc
    dictionary, rows, mods, defects = BASE + 0x8000, BASE + 0x9000, BASE + 0xB000, BASE + 0xB100
    m.write(dictionary + 0x130, rows)
    m.write(dictionary + 0x134, 2)
    uc.mem_write(rows, struct.pack("<H", 101))
    uc.mem_write(rows + 0x3A0, struct.pack("<H", 102))
    uc.mem_write(rows + 0x3A0 + 0x22A, b"\x01")
    m.write(dictionary + 0x140, mods)
    m.write(dictionary + 0x144, 1)
    m.write(mods, 1230)
    m.write(dictionary + 0x148, defects)
    m.write(dictionary + 0x14C, 1)
    m.write(defects, 7)
    uc.mem_map(0x2260000, 4096)
    wire_page = 0x4000000
    uc.mem_map(wire_page, 0x4000)
    constructed = []

    def visit(uc, address, size, _):
        if address == 0x2F7C50:
            constructed.append(uc.reg_read(UC_X86_REG_ECX))
        if address == 0x424580:  # memcpy, cdecl(dest, source, size)
            esp = uc.reg_read(UC_X86_REG_ESP)
            dest, source, length = (m.read(esp + i) for i in (4, 8, 12))
            if length:
                uc.mem_write(dest, bytes(uc.mem_read(source, length)))
            uc.reg_write(UC_X86_REG_EAX, dest)
            uc.reg_write(UC_X86_REG_EIP, m.read(esp))
            uc.reg_write(UC_X86_REG_ESP, esp + 4)

    uc.hook_add(UC_HOOK_CODE, visit)
    cave = patch.base.LATE_JOIN_VA
    sentinel = b"\xa5" * 0x45FC

    def run(wire, valid):
        source = wire_page + 0x4000 - len(wire)
        if wire:
            uc.mem_write(source, wire)
        uc.mem_write(BASE, sentinel)
        constructed.clear()
        saved = [(UC_X86_REG_EBX, 0x1234), (UC_X86_REG_ESI, 0x2345),
                 (UC_X86_REG_EDI, 0x3456), (UC_X86_REG_EBP, 0x4567)]
        for reg, value in saved:
            uc.reg_write(reg, value)
        m.start(cave + 192, args=(source, len(wire), BASE, dictionary), count=200000)
        assert uc.reg_read(UC_X86_REG_EAX) == int(valid), (len(wire), wire.hex())
        assert uc.reg_read(UC_X86_REG_ESP) == STACK + 4
        assert all(uc.reg_read(reg) == value for reg, value in saved)
        assert constructed == ([BASE] if valid else [])
        if not valid:
            assert bytes(uc.mem_read(BASE, len(sentinel))) == sentinel

    # Two kits, both item layouts, modification/defect tables and an upgrade.
    wire = bytearray(b"\x01\x03new\x02SR" + struct.pack("<I", 10) + b"\x02\x02")
    kit_offsets = []
    for kit in range(2):
        kit_offsets.append(len(wire))
        wire += bytes([0]) + struct.pack("<I", kit + 1) + b"\x01K"
        wire += struct.pack("<IHHHH", 3, 0, 1, 1, 9)
        wire += struct.pack("<IIIIf", 8, 0, 0, 0, 1.25)  # modifier 67
    item_offsets = []
    for index in range(2):
        item_offsets.append(len(wire))
        wire += struct.pack("<IHI", index + 1, 101 + index, 99)
        if not index:
            wire += struct.pack("<II", 2, 3)
        wire += b"\x02" + struct.pack("<IBIB", 1230, 0, 7, 1)
        wire += b"\x01" + struct.pack("<IIQB", 31, 32, 33, 3)
    wire = bytes(wire)
    run(wire, True)
    assert m.read(BASE + 0x45EC) == 1
    assert bytes(uc.mem_read(BASE + 8, 4)) == b"new\0"
    assert bytes(uc.mem_read(BASE + 0x48, 3)) == b"SR\0"
    assert uc.mem_read(BASE + 0x57, 1)[0] == 2
    for kit in range(2):
        assert m.read(BASE + 0xF4 + kit * 0x704 + 67 * 4) == 0x3FA00000
    for index in range(69):
        entry = BASE + 0x1568 + index * 0xB4
        assert m.read(entry + 0x14) == entry + 0x20
        assert m.read(entry + 0x18) == entry + 0x20 + (16 if index < 2 else 0)
        assert m.read(entry + 0x60) == entry + 0x6C
        assert m.read(entry + 0x64) == entry + 0x6C + (24 if index < 2 else 0)
    for length in range(len(wire)):
        run(wire[:length], False)
    run(wire + b"\0", False)
    for offset, replacement in (
        (0, b"\x02"), (1, b"\x40"), (5, b"\x0f"),
        (12, b"\x04"), (13, b"\x46"),
        (kit_offsets[0] + 7, struct.pack("<I", 1 << 23)),
        (kit_offsets[0] + 11, b"\x02\0"),
        (kit_offsets[0] + 19, struct.pack("<I", 16)),
        (kit_offsets[0] + 23, struct.pack("<I", 1)),
        (kit_offsets[0] + 35, struct.pack("<I", 0x7F800000)),
        (item_offsets[0] + 4, b"\xff\xff"),
        (item_offsets[0] + 18, b"\x09"),
        (item_offsets[0] + 19, struct.pack("<I", 9999)),
        (item_offsets[0] + 23, b"\x02"),
        (item_offsets[0] + 29, b"\x04"),
    ):
        invalid = bytearray(wire)
        invalid[offset:offset + len(replacement)] = replacement
        run(bytes(invalid), False)
    # No usable kit, or an occupied slot referring to an empty item.
    invalid = bytearray(wire)
    for offset in kit_offsets:
        invalid[offset] = 2
    run(bytes(invalid), False)
    invalid = wire[:item_offsets[0] + 4] + b"\0\0" + wire[item_offsets[1]:]
    run(invalid, False)

    maximum = bytearray(b"\0\x3f" + b"N" * 63 + b"\x0e" + b"C" * 14)
    maximum += struct.pack("<I", 10) + b"\x03\x45"
    for kit in range(3):
        maximum += b"\0" + struct.pack("<I", kit + 1) + b"\x3f" + b"K" * 63
        maximum += struct.pack("<I", (1 << 23) - 1)
        for slot in range(23):
            maximum += struct.pack("<HH", kit * 23 + slot, 1)
        maximum += struct.pack("<IIII", 15, 0, 0xFFFFFFFF, 0xFFFFFFFF)
        maximum += struct.pack("<68f", *([1.0] * 68))
    for index in range(69):
        maximum += struct.pack("<IHI", index + 1, 101 + index % 2, 99)
        if index % 2 == 0:
            maximum += struct.pack("<II", 2, 3)
        maximum += b"\x08" + struct.pack("<IBIB", 1230, 0, 7, 1) * 4
        maximum += b"\x03" + struct.pack("<IIQB", 31, 32, 33, 3) * 3
    run(bytes(maximum), True)
    last = BASE + 0x1568 + 68 * 0xB4
    assert m.read(last + 0x18) == last + 0x60
    assert m.read(last + 0x64) == last + 0xB4
    for cut in (1, 17, 52):
        run(bytes(maximum[:-cut]), False)

    # The C validator is independently position-independent. Exercise a
    # copied entry, distant from both its linked address and the native exe.
    moved = 0x5000000
    section = next(s for s in patch.base.Headers(data).sections(data)
                   if patch.base.IMAGE_BASE + s[1] == patch.base.SECTION_VA)
    _, _, _, raw, raw_size, _ = section
    offset = patch.base.LATE_JOIN_OFFSET + patch.ASM_SIZE
    code = bytes(data[raw + offset:raw + raw_size])
    uc.mem_map(moved, (len(code) + 4095) & ~4095)
    uc.mem_write(moved, code)
    for body, valid in ((wire, True), (wire[:-1], False), (bytes(maximum), True)):
        source = wire_page + 0x4000 - len(body)
        uc.mem_write(source, body)
        m.start(moved, args=(source, len(body), dictionary), count=200000)
        assert uc.reg_read(UC_X86_REG_EAX) == int(valid)


def native_query(data):
    """Execute native callback construction and our resource-query adapter.

    Query scheduling and C completion are intercepted at their ABI boundary;
    this checks marshaling, not that the resource engine loads real assets.
    """
    cave = patch.base.LATE_JOIN_VA
    for mask in range(1, 8):
        m = Machine(data)
        uc = m.uc
        uc.mem_map(0x2260000, 4096)
        m.write(0x2260A58, 7)
        uc.mem_map(0x1F1D000, 4096)
        m.write(0x1F1D9C4, 0x12340000)
        m.write(BASE + 44, mask)
        m.write(BASE + 24, 19)
        callbacks = []
        completions = []
        callback = cave + 768
        code = bytes(uc.mem_read(callback, 128))
        call_at = callback + code.index(b"\x50\x56\xe8") + 2
        complete = call_at + 5 + struct.unpack("<i", uc.mem_read(call_at + 1, 4))[0]
        expected_players = [BASE + 0x10000 + i * 0x1000 if mask & (1 << i) else 0 for i in range(3)]

        def visit(uc, address, size, _):
            esp = uc.reg_read(UC_X86_REG_ESP)
            if address == 0xE5B60:
                requests = uc.reg_read(UC_X86_REG_EAX)
                count = uc.reg_read(UC_X86_REG_ECX)
                assert count == mask.bit_count()
                cb, allocator, variants, parent, asynchronous = (m.read(esp + i) for i in (4, 8, 12, 16, 20))
                assert (allocator, parent, asynchronous) == (0x12340000, 0, 1)
                assert m.read(cb) == 0xA09375
                assert m.read(cb + 8) == callback
                assert m.read(cb + 12) == 0
                assert m.read(cb + 16) == BASE
                assert m.read(cb + 20) == 0
                uc.mem_write(BASE + 0x30000, bytes(uc.mem_read(cb + 8, 16)))
                for n, kit in enumerate(i for i in range(3) if mask & (1 << i)):
                    assert m.read(requests + n * 8) == 0xA09F48
                    assert m.read(requests + n * 8 + 4) == 0x54
                    variant = m.read(variants + n * 4)
                    assert m.read(variant) == 0xA09F90
                    assert m.read(variant + 8) == BASE + 60
                    assert m.read(variant + 12) == 19
                    assert m.read(variant + 16) == 0x11110000
                    assert m.read(variant + 20) == 0x22220000
                    assert m.read(variant + 24) == 0x33330000
                    assert m.read(variant + 28) == kit
                    assert m.read(variant + 40) == variant
                    assert m.read(variant + 44) == 7
                callbacks.append(callback)
                uc.reg_write(UC_X86_REG_EAX, 77)
                uc.reg_write(UC_X86_REG_EIP, m.read(esp))
                uc.reg_write(UC_X86_REG_ESP, esp + 4)
            elif address == complete:
                assert m.read(esp + 4) == BASE
                players = m.read(esp + 8)
                assert [m.read(players + 4 * i) for i in range(3)] == expected_players
                completions.append(1)
                uc.reg_write(UC_X86_REG_EIP, m.read(esp))
                uc.reg_write(UC_X86_REG_ESP, esp + 4)

        uc.hook_add(UC_HOOK_CODE, visit)
        saved = [(UC_X86_REG_EBX, 0x1234), (UC_X86_REG_ESI, 0x2345),
                 (UC_X86_REG_EDI, 0x3456), (UC_X86_REG_EBP, 0x4567)]
        for reg, value in saved:
            uc.reg_write(reg, value)
        m.start(cave + 320, args=(BASE, 0x11110000, 0x22220000, 0x33330000))
        assert callbacks == [callback] and uc.reg_read(UC_X86_REG_EAX) == 77
        assert uc.reg_read(UC_X86_REG_ESP) == STACK + 4
        assert all(uc.reg_read(reg) == value for reg, value in saved)
        results = BASE + 0x20000
        for n, player in enumerate(p for p in expected_players if p):
            m.write(results + 0x12C + 0x2E0 * n, player)
        # The original boost invoker selected by vtable A09374, including
        # its member-this adjustment and stdcall callback cleanup.
        m.start(0x202300, args=(BASE + 0x30000, results))
        assert completions == [1]
        assert uc.reg_read(UC_X86_REG_ESP) == STACK + 4
        assert all(uc.reg_read(reg) == value for reg, value in saved)


def animation_table(m, table):
    head = table + 0xE034
    for at, value in ((head, 0), (head + 4, head), (head + 8, head),
                      (table + 0xE044, table + 0xE050),
                      (table + 0xE048, table + 0xE050),
                      (table + 0xE04C, table + 0x10050)):
        m.write(at, value)


def native_animations(data):
    """Native name comparison/append with incremental and repeated kit loads."""
    for order in ((0, 1, 2, 3, 4, 5), (3, 5, 1, 4, 0, 2)):
        m = Machine(data)
        uc = m.uc
        animation_table(m, BASE)
        entries = [BASE + 0x20000 + i * 128 for i in order]
        names = [(b"zeta", b"a"), (b"alpha", b"z"), (b"aardvark", b"a"),
                 (b"beta", b"a"), (b"alpha", b"b"), (b"alpha", b"a")]
        for entry, name in zip(entries, names):
            resource = entry + 32
            m.write(entry, resource)
            m.write(entry + 4, resource + 32)
            m.write(entry + 8, 0xCCCCCCCC)
            m.write(resource, resource + 4)
            uc.mem_write(resource + 4, name[0] + b"\0")
            m.write(resource + 32, resource + 36)
            uc.mem_write(resource + 36, name[1] + b"\0")
        def visit(uc, address, size, _):
            esp = uc.reg_read(UC_X86_REG_ESP)
            if address == 0x2D8100:
                uc.reg_write(UC_X86_REG_EIP, m.read(esp))
                uc.reg_write(UC_X86_REG_ESP, esp + 8)
            elif address == 0xF7200:
                result = uc.reg_read(UC_X86_REG_ECX)
                m.write(result, m.read(uc.reg_read(UC_X86_REG_EAX)))
                uc.reg_write(UC_X86_REG_EAX, result)
                uc.reg_write(UC_X86_REG_EIP, m.read(esp))
                uc.reg_write(UC_X86_REG_ESP, esp + 4)
        uc.hook_add(UC_HOOK_CODE, visit)
        for count, expected in ((2, [1, 0]), (4, [1, 0, 2, 3]), (6, [1, 0, 2, 3, 5, 4]),
                                (6, [1, 0, 2, 3, 5, 4])):
            head = BASE + 0xE034
            nodes = sorted(entry + 12 for entry in entries[:count])
            m.write(head, nodes[0])
            m.write(head + 4, nodes[0])
            m.write(head + 8, nodes[-1])
            for i, node in enumerate(nodes):
                m.write(node, head if i == 0 else nodes[i - 1])
                m.write(node + 4, 0)
                m.write(node + 8, nodes[i + 1] if i + 1 < count else 0)
            cave = patch.base.LATE_JOIN_VA
            m.start(cave + 3072, args=(BASE,), count=100000)
            assert uc.reg_read(UC_X86_REG_ESP) == STACK + 8
            assert m.read(BASE + 0xE048) == BASE + 0xE050 + count * 4
            assert [m.read(BASE + 0xE050 + i * 4) for i in range(count)] == [entries[i] for i in expected]
            assert [m.read(entries[index] + 8) for index in expected] == list(range(count))


def native_preparation(data, grenades=0, traps=0):
    """Exercise the composed ready -> E4 -> query -> install -> ACK -> teardown.

    Native profile reading, callback invocation, slot/player/inventory binding
    and the compiled C lifecycle run. Resource scheduling, allocation, packet
    transport, animation/physics/UI setup and player destruction are intercepted.
    This is an integration fixture, not a live resource-engine test.

    The newcomer's kit has `grenades` grenades and `traps` traps, bound by
    the game's own code; the world has two saved objects already and four
    states to roll back to.
    """
    m = Machine(data)
    uc = m.uc
    cave = patch.base.LATE_JOIN_VA
    state = patch.base.IMAGE_BASE + patch.base.Headers(data).sections(data)[-1][1]
    nc, game, mc, options = 0x7000000, 0x8000000, 0x9000000, 0x6000000
    for address, size in ((nc, 0x1F3000), (game, 0x6000), (mc, 0x59000), (options, 0x60000),
                          (0x2260000, 4096), (0x1F1D000, 4096)):
        uc.mem_map(address, size)
    animations = 0xA000000
    uc.mem_map(animations, 0x11000)
    animation_table(m, animations)
    pvp, core = BASE, BASE + 0x1F8
    active_objects(m, core, 130)
    context, statistics, dictionary = BASE + 0x20000, BASE + 0x25000, BASE + 0x2C000
    reader, wire_at, results = BASE + 0x2D000, BASE + 0x2E000, BASE + 0x2F000
    player, inventory = BASE + 0x34000, BASE + 0x34800
    vtable, animation_bind, packet, pool = BASE + 0x38000, BASE + 0x38100, BASE + 0x3A000, BASE + 0x3B000
    transport_vtable, functor = BASE + 0x3C000, BASE + 0x3D000
    seed = 0x80000009
    begin = core + 0x18464
    slot = begin + 2 * 48
    for address, value in (
        (nc + 0x14, game), (nc + 0x1F1C40, pvp), (nc + 0x1F1C48, mc),
        (nc + 0x1F1C64, 2), (game + 0x53A4, dictionary), (pvp + 0x124, options),
        (pvp + 0xD0, 1), (core + 0x18458, begin), (core + 0x1845C, slot),
        (core + 0x18460, begin + 20 * 48), (core + 0x18FA8, statistics),
        (core + 0x18FB0, COUNTER), (COUNTER, 73), (core + 0x19034, 0xBAD00000),
        (pvp + 0x130, 0x11223344), (pvp + 0x19238, animations),
        (options + 0x577B0, seed), (mc + 0x577BC, seed),
        (mc, transport_vtable), (mc + 0x578B0, pool), (pool + 0x34, 256),
        (transport_vtable + 0x18, STOP + 0x100), (transport_vtable + 0x1C, STOP + 0x200),
        (0x2260A58, 7), (0x1F1D9C4, 0x12340000),
        (player + 0x10C, inventory), (player + 0x3A4, vtable),
        (vtable, animation_bind), (player + 0xD0, 1),
    ):
        m.write(address, value)
    uc.mem_write(animation_bind, b"\xc2\x04\x00")
    # The newcomer's grenades and traps, as stacks in its inventory.
    carried = [BASE + 0x1A000 + index * 0x400 for index in range(grenades + traps)]
    if grenades:
        stack_items(m, inventory, 14, 0x309EC0, carried[:grenades], BASE + 0x35000)
    if traps:
        stack_items(m, inventory, 16, 0x3067A0, carried[grenades:], BASE + 0x36000)
    new_nodes = [each + 0x10 for each in carried[:grenades]] + [each + 0x74 for each in carried[grenades:]]
    # What the list of saved objects holds: an older slot's and the map's.
    older, of_map = BASE + 0x1F000, BASE + 0x1F100
    for address, value in ((core + 0x18914, older), (core + 0x18918, of_map),
                           (older + 8, of_map), (of_map + 4, older)):
        m.write(address, value)
    # The states the game can roll back to, oldest first: the game-objects
    # bytes of each, in a buffer of 0x2000 filled with a pattern. The third
    # is all but full: no late slot's bytes go in.
    states = 0xB000000
    uc.mem_map(states, 0x400000)
    saved = (bytes(range(1, 8)), b"", bytes(index % 251 + 1 for index in range(0x1FFE)),
             bytes([0x44, 0x04, 0x99]))
    items = [states + index * 0x100000 for index in range(len(saved))]
    filler = bytes(0xC0 + index % 61 for index in range(0x2000))
    for index, (item, blob) in enumerate(zip(items, saved)):
        uc.mem_write(item + 0xBBF30, blob + filler[len(blob):])
        for offset, value in ((0xBDF30, 0), (0xBDF34, item + 0xBBF30), (0xBDF38, 0x2000),
                              (0xBDF3C, len(blob)), (0xC0BA8, items[index - 1] if index else 0),
                              (0xC0BAC, items[index + 1] if index + 1 < len(items) else 0)):
            m.write(item + offset, value)
    m.write(core + 0x2FB8, len(items))
    m.write(core + 0x2FBC, items[0])
    m.write(core + 0x2FC0, items[-1])
    uc.mem_write(options + 0x577B5, b"\x02")
    uc.mem_write(statistics + 8 + 0x31E0, b"\x02")
    uc.mem_write(statistics + 0x31E0, b"\x5a")
    histories_at = core + 0x2FCC
    histories = b"\x7d" * (0x18458 - 0x2FCC)
    uc.mem_write(histories_at, histories)
    body = b"\x01\x03new\0" + struct.pack("<I", 10) + b"\x01\0"
    body += b"\0" + struct.pack("<I", 1) + b"\0" + bytes(20)
    payload = b"\xffLJ\x01" + struct.pack("<II", seed, 123) + b"\x02\x07" + body
    uc.mem_write(wire_at, payload)
    for offset, value in ((0, wire_at), (4, wire_at), (8, len(payload))):
        m.write(reader + offset, value)
    calls, messages = [], []

    def finish(pop=0, value=None):
        esp = uc.reg_read(UC_X86_REG_ESP)
        if value is not None:
            uc.reg_write(UC_X86_REG_EAX, value)
        uc.reg_write(UC_X86_REG_EIP, m.read(esp))
        uc.reg_write(UC_X86_REG_ESP, esp + 4 + pop)

    def visit(uc, address, size, _):
        esp = uc.reg_read(UC_X86_REG_ESP)
        if address == 0xE4F90:
            assert uc.reg_read(UC_X86_REG_ESI) == 0x12340000
            assert uc.reg_read(UC_X86_REG_EAX) == 0x463C + len(body)
            calls.append("allocate")
            finish(4, context)
        elif address == 0x1539D0:
            assert uc.reg_read(UC_X86_REG_EAX) == context
            assert uc.reg_read(UC_X86_REG_ECX) == 0x12340000
            m.write(m.read(esp + 4), 0)
            calls.append("free")
            finish()
        elif address == 0x424580:
            dest, source, count = (m.read(esp + i) for i in (4, 8, 12))
            if count:
                uc.mem_write(dest, bytes(uc.mem_read(source, count)))
            finish(value=dest)
        elif address == 0xE5B60:
            assert uc.reg_read(UC_X86_REG_ECX) == 1
            cb, variants = m.read(esp + 4), m.read(esp + 12)
            variant = m.read(variants)
            assert m.read(variant + 8) == context + 60
            assert m.read(variant + 16) == game + 0x18
            assert m.read(variant + 20) == 0x11223344
            assert m.read(variant + 24) == dictionary
            uc.mem_write(functor, bytes(uc.mem_read(cb + 8, 16)))
            calls.append("query")
            finish(value=77)
        elif address == 0x2D8100:
            assert uc.reg_read(UC_X86_REG_EDI) == player
            assert m.read(esp + 4) == animations
            calls.append("animations")
            finish(4)
        elif address == 0x3218A0:
            assert uc.reg_read(UC_X86_REG_EAX) == pvp + 0x138
            assert m.read(esp + 4) == slot
            calls.append("physics")
            finish(4)
        elif address == 0x1F29D0:
            assert uc.reg_read(UC_X86_REG_ESI) == player
            calls.append("ready")
            finish()
        elif address == STOP + 0x100:
            assert uc.reg_read(UC_X86_REG_ECX) == mc and m.read(esp + 4) == 0xE0
            uc.mem_write(packet + 0x98, b"\x60")
            calls.append("new_packet")
            finish(4, packet)
        elif address == 0x7AC10:
            assert uc.reg_read(UC_X86_REG_EDI) == 20
            assert m.read(esp + 4) == packet + 0x2A8
            messages.append(bytes(uc.mem_read(m.read(esp + 8), 20)))
            finish(8)
        elif address == STOP + 0x200:
            assert uc.reg_read(UC_X86_REG_ECX) == mc and m.read(esp + 4) == packet
            assert uc.mem_read(packet + 0x98, 1)[0] == 0x20
            calls.append("enqueue")
            finish(4)
        elif address == 0x1F2100:
            assert uc.reg_read(UC_X86_REG_ECX) == player
            assert bytes(uc.mem_read(context + 60 + 8, 4)) == b"new\0"
            calls.append("destroy")
            finish()

    uc.hook_add(UC_HOOK_CODE, visit)
    uc.reg_write(UC_X86_REG_EBX, nc)
    m.start(0x27AEA1, end=patch.base.SECTION_VA, count=200000)
    assert m.read(state + 40) == nc and m.read(state + 56) == 2
    uc.reg_write(UC_X86_REG_ESI, nc)
    uc.reg_write(UC_X86_REG_EBX, reader)
    m.start(0x27A378, end=0x27A5AB, count=200000)
    assert calls == ["allocate", "query"] and not messages
    assert m.read(pvp + 0xD0) == 2 and m.read(core + 0x1845C) == slot
    assert bytes(uc.mem_read(histories_at, len(histories))) == histories
    m.write(player + 0x6B8, context + 60)
    m.write(results + 0x12C, player)
    m.start(0x202300, args=(functor, results), count=200000)
    assert calls == ["allocate", "query", "animations", "physics", "ready", "new_packet", "enqueue"]
    assert m.read(pvp + 0xD0) == 1 and m.read(player + 0xD0) == 2
    assert m.read(core + 0x1845C) == slot + 48 and m.read(slot) == player
    taken = grenades * 2 + traps
    assert m.read(slot + 0x28) == core and m.read(slot + 0x20) == 0x30002 + taken
    assert m.read(player + 0x698) == 0x30000 + taken and m.read(player + 0x6B0) == 0x30001 + taken
    # The newcomer's objects are in front of what the list held, in the
    # order the game bound them, and each took its ids of the place's.
    assert saved_list(m, core) == new_nodes + [older, of_map]
    for index, each in enumerate(carried[:grenades]):
        assert [m.read(each + offset) for offset in (0x168, 0x180, 0x188)] == [
            0x30000 + index * 2, 0x30001 + index * 2, core]
    for index, each in enumerate(carried[grenades:]):
        assert [m.read(each + offset) for offset in (0x234, 0x19C)] == [0x30000 + grenades * 2 + index, core]
    # Every state it can roll back to has one zero byte for each of them in
    # front of its game-objects bytes; the one with no room is as it was.
    added = len(new_nodes)
    for item, blob in zip(items, saved):
        fits = len(blob) + added <= 0x2000
        now = bytes(added) + blob if fits else blob
        assert m.read(item + 0xBDF3C) == len(now), (hex(item), m.read(item + 0xBDF3C))
        assert bytes(uc.mem_read(item + 0xBBF30, 0x2000)) == now + filler[len(now):]
        assert [m.read(item + offset) for offset in (0xBDF30, 0xBDF34, 0xBDF38)] == [0, item + 0xBBF30, 0x2000]
    assert [m.read(core + offset) for offset in (0x2FB8, 0x2FBC, 0x2FC0)] == [len(items), items[0], items[-1]]
    assert m.read(core + 0x18FB0) == COUNTER and m.read(COUNTER) == 73
    assert bytes(uc.mem_read(histories_at, len(histories))) == histories
    assert m.read(state + 56) == 3 and uc.mem_read(options + 0x577B5, 1)[0] == 3
    assert uc.mem_read(statistics + 8 + 0x31E0, 1)[0] == 3
    assert uc.mem_read(statistics + 0x31E0, 1)[0] == 0x5A
    assert uc.mem_read(options + 0x5782C + 2, 1)[0] == 7
    assert bytes(uc.mem_read(options + 2 * 0x45FC + 8, 4)) == b"new\0"
    expected_ack = b"\xff" * 6 + b"\0\0" + struct.pack("<II", seed, 123) + b"\x02\0\x01\0"
    assert messages == [expected_ack]
    uc.reg_write(UC_X86_REG_ESI, nc)
    uc.reg_write(UC_X86_REG_EBX, reader)
    m.start(0x27A378, end=0x27A5AB, count=200000)
    assert messages == [expected_ack] * 2 and calls.count("query") == 1
    uc.reg_write(UC_X86_REG_EDI, nc)
    m.start(0x27B517, end=0x27B51D, count=200000)
    assert m.read(state + 40) == 0 and "free" not in calls
    uc.reg_write(UC_X86_REG_ECX, player)
    m.start(0x1F2013, end=0x1F2018, count=200000)
    assert calls[-2:] == ["destroy", "free"] and m.read(state + 4) == 0


def native_direct_match(data, session):
    for spectator in (0, 1):
        m = Machine(data)
        m.start(0x27a1d1, end=0x27a1d8)
        nc = BASE + 0x10000
        m.uc.mem_map((nc + 0x1ee7ec) & ~4095, 4096)
        m.write(BASE + 0x53ac, nc)
        saved = {UC_X86_REG_EDI: BASE, UC_X86_REG_EAX: nc,
                 UC_X86_REG_EBX: 0x12345678, UC_X86_REG_EBP: 0x87654321}
        for reg, value in saved.items():
            m.uc.reg_write(reg, value)
        calls = []

        def visit(uc, address, size, _):
            if address not in (0x1ed6e0, 0x21beb0):
                return
            esp = uc.reg_read(UC_X86_REG_ESP)
            count = 3 if address == 0x1ed6e0 else 4
            args = [m.read(esp + 4 + 4 * i) for i in range(count)]
            if count == 3:
                assert args == [BASE + 0x8000, 2249, spectator]
                assert uc.reg_read(UC_X86_REG_EDI) == BASE
                assert uc.reg_read(UC_X86_REG_EAX) == nc
            else:
                assert args == [BASE + 0x8000, 2249, 1, session]
                assert uc.reg_read(UC_X86_REG_ECX) == nc
                assert m.read(nc + 0x1ee7ec) == session
            calls.append(address)
            uc.reg_write(UC_X86_REG_EAX, 0xdeadbeef)
            uc.reg_write(UC_X86_REG_EIP, m.read(esp))
            uc.reg_write(UC_X86_REG_ESP, esp + 4 * (count + 1))

        m.uc.hook_add(UC_HOOK_CODE, visit)
        address = 0x1ec0e1
        assert m.uc.mem_read(address, 1) == b"\xe8"
        target = address + 5 + struct.unpack("<i", m.uc.mem_read(address + 1, 4))[0]
        m.start(target, args=(BASE + 0x8000, 2249, spectator))
        assert calls == ([0x1ed6e0] if spectator else [0x1ed6e0, 0x21beb0])
        assert m.uc.reg_read(UC_X86_REG_ESP) == STACK + 16
        for reg, value in saved.items():
            assert m.uc.reg_read(reg) == value


def check(original, nasm):
    assert hashlib.sha256(original).hexdigest() == patch.base.EXE_SHA256
    changed = patch.build(original, nasm)
    assert Machine(changed).uc.mem_read(0x380866, 1) == bytes([patch.PATCH_REVISION])
    assert Machine(changed).uc.mem_read(0x27a1d1, 2) == bytes.fromhex("33c0")
    native_direct_match(patch.build(original, nasm, test_session=0x1002), 0x1002)
    native_animations(changed)
    native_preparation(changed)
    for grenades, traps in ((2, 0), (0, 1), (6, 3), (1, 1)):
        native_preparation(changed, grenades, traps)
    saved_objects(changed)
    untouched_objects(changed)
    native_query(changed)
    native_profile(changed)
    native_histories(changed)
    for debug in (0, 0xff):
        m = Machine(changed)
        m.uc.reg_write(UC_X86_REG_EAX, BASE)
        m.uc.reg_write(UC_X86_REG_EDI, 4)
        m.uc.reg_write(UC_X86_REG_EBX, 0x12345600 | debug)
        m.write(STACK + 0x1c, 0x98765432)
        m.start(0x27A23D, end=0x27A244)
        assert m.uc.mem_read(BASE + 4, 1)[0] == (0x80 | (debug & 1))
        assert m.uc.reg_read(UC_X86_REG_EAX) == 0x98765432
        assert m.uc.reg_read(UC_X86_REG_EBX) == (0x12345680 | (debug & 1))
        assert m.uc.reg_read(UC_X86_REG_ESP) == STACK
    for roster in (0, 1, 2, 8, 9, 16, 20):
        for reading in (False, True):
            assert statistics(original, roster, reading) == list(range(roster))
            assert statistics(changed, roster, reading) == list(range(20))
        for setter, field in ((0x279610, 0x23D0E8), (0x27A2E0, 0x23CE80)):
            m = Machine(changed)
            m.uc.reg_write(UC_X86_REG_ECX, BASE)
            # Live and journaling match-client fields lie outside the core fixture.
            m.uc.mem_map(BASE + (field & ~4095), 4096)
            m.start(setter, args=(roster,))
            assert m.uc.mem_read(BASE + field, 1)[0] == 3
            assert m.uc.reg_read(UC_X86_REG_ESP) == STACK + 8
    for place in (0, 1, 8, 19):
        for kits in ((0,), (1,), (2,), (0, 2), (0, 1, 2)):
            native_binding(changed, place, kits)
            for world_objects in (0, 130, 255):
                native_binding(changed, place, kits, weapons=True, world_objects=world_objects)
        for consumed in (1, 17, 200):
            for old_counter in (0, 53, 4096):
                binding(changed, place, consumed, old_counter)
    h = patch.base.Headers(changed)
    assert struct.unpack_from("<I", changed, h.checksum_at)[0] == patch.base.pe_checksum(changed, h.checksum_at)
    old_security, old_size = struct.unpack_from("<II", original, patch.base.Headers(original).security_at)
    new_security, new_size = struct.unpack_from("<II", changed, h.security_at)
    assert old_size == new_size and original[old_security:] == changed[new_security:]
    _, rva, size, _, _, flags = h.sections(changed)[-2]
    assert flags == patch.base.SECTION_CHARACTERISTICS
    assert struct.unpack_from("<I", changed, h.size_of_image_at)[0] >= rva + size
    name, rva, size, _, raw_size, flags = h.sections(changed)[-1]
    assert name.startswith(b".srpd") and size == 4096 and raw_size == 0 and flags == 0xC0000080
    print("PASS: native preparation/install/ACK/cleanup fixture; query/callback ABI; bounded profile decoding; history capacity; statistics loops; fixed masks; capability; native weapon binding; stable animation IDs; slot and weapon IDs; a slot's grenades and traps in front of the saved objects, and in the states to roll back to; an untouched grenade and trap save and load as one zero byte; ABI; PE checksum and certificate")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("original", type=Path)
    parser.add_argument("--nasm", default="nasm")
    args = parser.parse_args()
    check(args.original.read_bytes(), args.nasm)


if __name__ == "__main__":
    main()
