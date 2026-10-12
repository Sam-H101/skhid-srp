"""Fixes two bugs of the 0.21d lobby UI (flash_movies/lobby_menu.swf), in
resources.db.

    python lobby_menu.py <resources.db>            apply
    python lobby_menu.py <resources.db> --restore  put the original back

1. The shop buy slider cap (docs/notes/shop_slider.md).
   MessageAsk.update_slider caps a stack purchase at
   int(int(money / _price) / _step) * _step, but _price is the price of one
   step (clip), so the cap is _step*_step times too low. The fix turns it
   into int(int(money / _price) * _step): the second divide becomes a
   multiply and the trailing multiply becomes a pop.
2. The new items badge (docs/notes/new_items_badge.md).
   StatusPanel.setNewItems puts the count of newly unlocked shop items at
   topMenu.x + (buttonWidth + spacing) * 2 - 35, with buttonWidth 100; the
   buttons are wider, so it lands on the third button (Tasks) instead of
   the second (Items). The fix counts one button: `pushbyte 2` becomes
   `pushbyte 1`.

The SWF is CWS (zlib) and is stored uncompressed in the db. The patched
body is recompressed to exactly the original size: level 8 deflate of all
but the last `tail` bytes, a sync flush, then the tail as one final stored
block and the adler32; `tail` is searched for. The db entry is rewritten in
place after the original bytes are saved next to the db as
lobby_menu_original.swf. Anything but the known original, an earlier
patched version or this script's result is refused.
"""

import hashlib
import os
import sys
import zlib

# flash_movies/lobby_menu.swf in the 0.21d resources.db (FAT pos_in_db, size_in_db).
OFFSET, SIZE = 14268063, 16529440
ORIGINAL_SHA = "1370380ff8029fabbe7f564b44574ccd5aafbd5675868b4f5a140b1b2956c8b9"
# What lobby_slider.py (the slider fix alone) wrote.
EARLIER_SHAS = {"2049e72f4f0b7e231dc63438047e050180c40e03bd8955e803464bf5c4cef2cf"}

PATCHES = [
    (
        "MessageAsk.update_slider",
        # d2 d0 66 _price a3 46 int 01 d0 66 _step a3 46 int 01 d0 66 _step a2 75 75
        bytes.fromhex("d2d066f81fa3460c01d066f71fa3460c01d066f71fa27575"),
        bytes.fromhex("d2d066f81fa3460c01d066f71fa2460c01d066f71f297575"),
    ),
    (
        "StatusPanel.setNewItems",
        # getproperty buttonWidth, getlocal0, getproperty _topMenu, getproperty
        # tabPage, getlex ButtonBar, astypelate, getproperty spacing, add,
        # pushbyte 2, multiply, add, pushbyte 35, subtract
        bytes.fromhex("66c22560538766a206a02402a2a02423a1"),
        bytes.fromhex("66c22560538766a206a02401a2a02423a1"),
    ),
]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def body(swf):
    return zlib.decompress(swf[8:])


def pack(header, plain, tail):
    c = zlib.compressobj(8, zlib.DEFLATED, 15, 8)
    head = c.compress(plain[:-tail]) + c.flush(zlib.Z_SYNC_FLUSH)
    rest = plain[-tail:]
    stored = b"\x01" + len(rest).to_bytes(2, "little") + (len(rest) ^ 0xFFFF).to_bytes(2, "little")
    return header + head + stored + rest + zlib.adler32(plain).to_bytes(4, "big")


def build(original):
    plain = body(original)
    for name, site, fixed in PATCHES:
        at = plain.find(site)
        if at < 0 or plain.find(site, at + 1) >= 0:
            sys.exit(f"{name}: site not found exactly once")
        plain = plain[:at] + fixed + plain[at + len(site):]
    # The size grows with the stored tail (about 0.7 bytes a byte): step
    # towards the original size, then try the neighbours.
    tail, tried = 574, set()
    while tail not in tried and 1 <= tail <= 65535:
        tried.add(tail)
        swf = pack(original[:8], plain, tail)
        missing = len(original) - len(swf)
        if missing == 0:
            if body(swf) != plain:
                sys.exit("repack does not decompress to the patched body")
            return swf
        step = round(missing / 0.7)
        tail += step if step else (1 if missing > 0 else -1)
    for tail in range(min(tried) - 64, max(tried) + 64):
        if 1 <= tail <= 65535 and tail not in tried:
            swf = pack(original[:8], plain, tail)
            if len(swf) == len(original) and body(swf) == plain:
                return swf
    sys.exit("no stored tail gives the original size")


def main():
    db, restore = sys.argv[1], "--restore" in sys.argv[2:]
    backup = os.path.join(os.path.dirname(os.path.abspath(db)), "lobby_menu_original.swf")
    with open(db, "rb") as f:
        f.seek(OFFSET)
        current = f.read(SIZE)

    if sha(current) == ORIGINAL_SHA:
        original = current
    elif os.path.exists(backup) and sha(open(backup, "rb").read()) == ORIGINAL_SHA:
        original = open(backup, "rb").read()
    else:
        sys.exit("lobby_menu.swf in the db is not the known original and no backup exists")
    patched = build(original)
    known = {ORIGINAL_SHA, sha(patched)} | EARLIER_SHAS
    if sha(current) not in known:
        sys.exit("lobby_menu.swf in the db is neither the original nor a patched one")

    new = original if restore else patched
    if current == new:
        print("already the original" if restore else "already patched")
        return
    if not os.path.exists(backup):
        with open(backup, "wb") as f:
            f.write(original)
    with open(db, "r+b") as f:
        f.seek(OFFSET)
        f.write(new)
    with open(db, "rb") as f:
        f.seek(OFFSET)
        if f.read(SIZE) != new:
            sys.exit("verification failed after writing")
    print(f"{'restored' if restore else 'patched'}: {db} @ {OFFSET} ({SIZE} bytes), sha256 {sha(new)}")


if __name__ == "__main__":
    main()
