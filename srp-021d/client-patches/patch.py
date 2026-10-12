"""Builds the srp copy of the 0.21d client from the original survarium.exe.

    python patch.py <survarium.exe> <survarium_srp.exe> <snapshot_upload.bin> <battery_spawn.bin>

Patches:
- the single-instance check, so several clients run on one PC;
- the option B snapshot upload (docs/notes/option_b.md): code in
  the zero padding after .text, and two hooks into it;
- mode 3 batteries (docs/notes/battery_spawn.md): a new
  match_time_vi_spawn_rule::tick in place of the old one, and a hook from its
  deserialize into the end of the same padding.
Each site is checked against the original bytes first, so a different build
or an already patched file is refused rather than patched twice.
"""

import sys

IMAGE_BASE = 0x10000
TEXT_VA, TEXT_FILE = 0x11000, 0x400


def file_offset(va):
    return va - TEXT_VA + TEXT_FILE


MUTEX = (0x1267A, bytes.fromhex("7522"), bytes.fromhex("eb22"))
CAVE_VA, CAVE_SIZE = 0x6A1E0B, 0x185
HOOKS = [
    (0x1A27A8, bytes.fromhex("8db3087a0000")),  # lea esi,[ebx+7A08h]
    (0x1A16B1, bytes.fromhex("8b3ddc83d001")),  # mov edi,[g_log_filter_tree]
]
HOOK_SIZE = 6

# battery_spawn.bin: the tick, the cave, the hook.
TICK_VA = 0x1FC388
TICK_ORIGINAL = bytes.fromhex(
    "568bf18b86140100002b8610010000c1f8023986680100000f83850000008b8e7001"
    "00008bc12b8660010000578b7c241003c733d2f7b66c01000085d2751e8d14098d8e"
    "400100008b0169c005840808408901f7e203d78996640100003bbe640100005f723d"
    "8b86680100008b8e100100008b9620010000c1e0028b0c088b0410d940108b1151d9"
    "1c2450ff92900000008b866c010000ff86680100000186640100005ec20800"
)
BATTERY_CAVE_VA, BATTERY_CAVE_SIZE = 0x6A1F90, 0x70
BATTERY_HOOK = (0x1FC663, bytes.fromhex("8b7dfc8bb710010000"))


def patch(data, va, expected, new):
    at = file_offset(va)
    found = bytes(data[at:at + len(expected)])
    if found != expected:
        sys.exit(f"{va:#x}: expected {expected.hex()}, found {found.hex()}")
    data[at:at + len(new)] = new


def main():
    source, target, upload, battery = sys.argv[1:5]
    data = bytearray(open(source, "rb").read())
    code = open(upload, "rb").read()
    spawn = open(battery, "rb").read()

    cave, hooks = code[:-HOOK_SIZE * len(HOOKS)], code[-HOOK_SIZE * len(HOOKS):]
    if len(cave) > CAVE_SIZE:
        sys.exit(f"the cave code is {len(cave)} bytes; {CAVE_SIZE} fit")

    patch(data, MUTEX[0], MUTEX[1], MUTEX[2])
    patch(data, CAVE_VA, bytes(len(cave)), cave)
    for i, (va, original) in enumerate(HOOKS):
        hook = hooks[i * HOOK_SIZE:(i + 1) * HOOK_SIZE]
        if hook[0] != 0xE9 or hook[5] != 0x90:
            sys.exit(f"{va:#x}: the hook is not jmp + nop: {hook.hex()}")
        patch(data, va, original, hook)

    hook_va, hook_original = BATTERY_HOOK
    tick_size, hook_size = len(TICK_ORIGINAL), len(hook_original)
    if len(spawn) != tick_size + BATTERY_CAVE_SIZE + hook_size:
        sys.exit(f"battery_spawn.bin is {len(spawn)} bytes; expected "
                 f"{tick_size} + {BATTERY_CAVE_SIZE} + {hook_size}")
    tick = spawn[:tick_size]
    battery_cave = spawn[tick_size:tick_size + BATTERY_CAVE_SIZE]
    battery_hook = spawn[tick_size + BATTERY_CAVE_SIZE:]
    if battery_hook[0] != 0xE9 or set(battery_hook[5:]) != {0x90}:
        sys.exit(f"{hook_va:#x}: the hook is not jmp + nops: {battery_hook.hex()}")
    patch(data, TICK_VA, TICK_ORIGINAL, tick)
    patch(data, BATTERY_CAVE_VA, bytes(BATTERY_CAVE_SIZE), battery_cave)
    patch(data, hook_va, hook_original, battery_hook)

    open(target, "wb").write(data)
    print(f"{target}: mutex, {len(cave)} bytes of upload code, {len(HOOKS)} hooks, "
          "mode 3 batteries")


if __name__ == "__main__":
    main()
