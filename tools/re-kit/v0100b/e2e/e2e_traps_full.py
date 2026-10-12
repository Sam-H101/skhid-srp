"""End-to-end check of a trap's whole life against a standalone match server:

1. `first` places a trap on team 1's base floor (0x96 to everyone).
2. `third` joins late and gets it as game_world_object_state (0x9C, armed).
3. `second` walks onto it: everyone gets three hit_player (0x89) from
   `first` on `second` (right_foot, left_foot, pain), then trap_fired (0x98);
   3 s later trap_removed (0x97).
4. `first` places another and crouches over it holding use: trap_disarmed
   (0x99) after 5 s of use, trap_removed 3 s after that.

Usage: E2E_MATCH_PORT=11236 python -I e2e_traps_full.py
"""

import math
import os
import socket
import struct
import sys
import threading
import time

MATCH = ("127.0.0.1", int(os.environ.get("E2E_MATCH_PORT", "11236")))
FEET = (-31.2, 0.504, -66.7)
YAW = math.pi / 2  # facing -x
PRESS, RELEASE = 0x100000, 0x200000  # quick slot 4
CROUCH, USE = 0x100, 0x10000000


class Player:
    def __init__(self, session):
        self.session = session
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.settimeout(0.1)
        self.seq = 0
        self.order = 0
        self.newest = 0xFFFF
        self.bits = 0
        self.delivered = 0xFFFF
        self.held = {}
        self.lock = threading.Lock()
        self.received = []  # (time, kind, payload), in delivery order
        self.frame = 0
        self.feet = FEET
        self.stop = False
        threading.Thread(target=self.receive, daemon=True).start()

    def receive(self):
        while not self.stop:
            try:
                data, _ = self.sock.recvfrom(4096)
            except OSError:
                continue
            sequence, _, word = struct.unpack("<HHH", data[:6])
            with self.lock:
                ahead = (sequence - self.newest) & 0xFFFF
                if ahead and ahead < 0x8000:
                    self.bits = (self.bits >> ahead if ahead < 16 else 0) | 0x8000
                    self.newest = sequence
            if word & 1 == 0:
                messages = [data[6:]]
            else:
                messages, rest = [], data[6:]
                while rest:
                    messages.append(rest[1:1 + rest[0]])
                    rest = rest[1 + rest[0]:]
                if len(messages) == 1:
                    continue  # low level
            for message in messages:
                order = struct.unpack("<H", message[1:3])[0]
                if (order - self.delivered) & 0xFFFF >= 0x8000 or order == self.delivered:
                    continue
                self.held[order] = message
            while (self.delivered + 1) & 0xFFFF in self.held:
                self.delivered = (self.delivered + 1) & 0xFFFF
                message = self.held.pop(self.delivered)
                self.received.append((time.monotonic(), message[0], message[3:]))

    def send(self, kind, payload=b""):
        with self.lock:
            header = struct.pack("<HHH", self.seq, self.newest, (self.bits << 1) & 0xFFFF)
            self.sock.sendto(header + struct.pack("<BH", kind, self.order) + payload, MATCH)
            self.seq = (self.seq + 1) & 0xFFFF
            self.order = (self.order + 1) & 0xFFFF

    def join(self):
        self.send(0x40, struct.pack("<I", self.session))
        time.sleep(0.3)
        self.send(0x41)
        time.sleep(0.3)
        self.send(0x48)
        self.send(0x42)

    def update(self, mask=0, pitch=0.0, yaw=YAW):
        payload = struct.pack("<ffffI", 0, 0, 0, 0, mask)
        payload += struct.pack("<fffff", *self.feet, yaw, pitch)
        payload += struct.pack("<I", self.frame * 16)
        self.send(0x43, payload)
        self.frame += 1

    def hold(self, seconds, mask=0, pitch=0.0, yaw=YAW):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self.update(mask, pitch, yaw)
            time.sleep(1 / 60)

    def tap(self, pitch, yaw=YAW):
        """Quick slot 4 down for a few frames, then up, looking at `pitch`."""
        self.hold(0.1, 0, pitch, yaw)
        self.update(PRESS, pitch, yaw)
        self.hold(0.2, 0, pitch, yaw)
        self.update(RELEASE, pitch, yaw)
        self.hold(0.3, 0, pitch, yaw)

    def of_kind(self, kind, since=0.0):
        return [(t, p) for t, k, p in self.received if k == kind and t >= since]

    def spawned_id(self):
        for _, kind, payload in self.received:
            if kind == 0x84:
                return payload[0]
        return None


def trap_ids(payload):
    return payload[0], payload[1], payload[2]


def main():
    failures = []
    first, second = Player(0xEC31), Player(0xEC32)
    first.join()
    second.join()
    first.hold(1.0)
    second.hold(0.1)
    first_id = first.spawned_id()
    # first sees second's spawn after its own.
    second_id = next(p[0] for _, p in first.of_kind(0x84) if p[0] != first_id)
    print(f"first is player {first_id}, second {second_id}")
    # Keep second far off while first sets the trap.
    second.feet = (FEET[0] + 4.0, FEET[1], FEET[2])
    second.hold(0.2)

    began = time.monotonic()
    first.tap(-0.9)
    placed = first.of_kind(0x96, began)
    if len(placed) != 1:
        print(f"no trap placed: {placed}")
        sys.exit(1)
    payload = placed[0][1]
    ids = trap_ids(payload)
    position = struct.unpack("<fff", payload[3:15])
    print(f"placed {ids} at {position}")
    if ids != (first_id, 0x10, 0):
        failures.append(f"placed ids {ids}")

    # 2. A late joiner gets it as a world object.
    third = Player(0xEC33)
    third.join()
    third.hold(1.0)
    states = third.of_kind(0x9C)
    print(f"third got 0x9C: {[(trap_ids(p), p[3]) for _, p in states]}")
    if [(trap_ids(p), p[3]) for _, p in states] != [((first_id, 0x10, 0), 1)]:
        failures.append(f"late joiner's world objects: {states}")
    elif struct.unpack("<fff", states[0][1][4:16]) != position:
        failures.append("late joiner's trap position differs")

    # 3. second steps on it.
    began = time.monotonic()
    second.feet = (position[0], FEET[1], position[2])
    second.hold(0.5)
    for player, name in ((first, "first"), (second, "second"), (third, "third")):
        hits = [(p[0], p[1]) for _, p in player.of_kind(0x89, began)]
        fired = [trap_ids(p) for _, p in player.of_kind(0x98, began)]
        print(f"{name}: hits {hits}, fired {fired}")
        if hits != [(first_id, second_id)] * 3:
            failures.append(f"{name}: hits {hits}")
        if fired != [(first_id, 0x10, 0)]:
            failures.append(f"{name}: fired {fired}")
    second.feet = (FEET[0] + 4.0, FEET[1], FEET[2])
    second.hold(3.0)
    removed = [trap_ids(p) for _, p in first.of_kind(0x97, began)]
    print(f"removed after 3 s: {removed}")
    if removed != [(first_id, 0x10, 0)]:
        failures.append(f"removed {removed}")

    # 4. first sets another and defuses it, crouched over it.
    began = time.monotonic()
    first.tap(-0.9)
    placed = first.of_kind(0x96, began)
    if len(placed) != 1:
        failures.append(f"second trap: {placed}")
    else:
        index = placed[0][1][2]
        position = struct.unpack("<fff", placed[0][1][3:15])
        first.feet = (position[0] + 0.65, FEET[1], position[2])
        first.hold(1.0, CROUCH, -1.0)
        use_began = time.monotonic()
        first.hold(4.5, CROUCH | USE, -1.0)
        if first.of_kind(0x99, began):
            failures.append("disarmed before 5 s of use")
        first.hold(1.0, CROUCH | USE, -1.0)
        disarmed = first.of_kind(0x99, began)
        print(f"disarmed: {[(trap_ids(p), round(t - use_began, 2)) for t, p in disarmed]}")
        if [trap_ids(p) for _, p in disarmed] != [(first_id, 0x10, index)]:
            failures.append(f"disarmed {disarmed}")
        first.hold(3.2, CROUCH, -1.0)
        removed = [trap_ids(p) for _, p in first.of_kind(0x97, began)]
        if removed != [(first_id, 0x10, index)]:
            failures.append(f"disarmed trap removed: {removed}")

    for player in (first, second, third):
        with player.lock:
            header = struct.pack("<HHH", player.seq, player.newest, (player.bits << 1) & 0xFFFF | 1)
            player.sock.sendto(header + bytes([1, 0]), MATCH)
        player.stop = True

    print("\n".join(failures) or "PASS")
    sys.exit(1 if failures else 0)


main()
