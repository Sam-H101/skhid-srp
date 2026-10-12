"""Reads the game's binary configs (the format of crates/binary-config-parser):
a tree of 24-byte values `u64 data, u64 name offset, u32 crc, u16 type,
u16 count`, the root first. Tables become dicts (named) or lists (indexed)."""
import struct

BOOLEAN, INTEGER, FLOAT, NAMED, INDEXED, STRING, FLOAT2, FLOAT3, FLOAT4 = range(9)


def parse(data):
    def cstr(at):
        return data[at:data.index(b"\0", at)].decode("utf-8", "replace")

    def value(at):
        d, name, _crc, kind, count = struct.unpack_from("<QQIHH", data, at)
        if kind == BOOLEAN:
            return d != 0
        if kind == INTEGER:
            return struct.unpack("<i", struct.pack("<I", d & 0xFFFFFFFF))[0]
        if kind == FLOAT:
            return struct.unpack("<f", struct.pack("<I", d & 0xFFFFFFFF))[0]
        if kind == STRING:
            return data[d:d + max(count, 1) - 1].decode("utf-8", "replace")
        if kind in (FLOAT2, FLOAT3, FLOAT4):
            return tuple(struct.unpack_from("<%df" % (kind - FLOAT2 + 2), data, d))
        if kind == INDEXED:
            return [value(d + 24 * i) for i in range(count)]
        if kind == NAMED:
            out = {}
            for i in range(count):
                at_child = d + 24 * i
                out[cstr(struct.unpack_from("<Q", data, at_child + 8)[0])] = value(at_child)
            return out
        raise ValueError("unknown value type %d" % kind)

    return value(0)


def load(path):
    with open(path, "rb") as file:
        return parse(file.read())
