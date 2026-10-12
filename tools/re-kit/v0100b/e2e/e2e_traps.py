"""End-to-end check of trap placement: two scripted players join; the first
stands on team 1's base floor looking down toward -x and taps quick slot 4
(the dummy loadout's traps). Both must get trap_placed (0x96) for player
`first`, slot 0x10, index 0, on the floor in front of it. Tapping again at
the same spot places nothing (the first trap's sensor refuses it); turning
toward -z places index 1.

Usage: E2E_MATCH_PORT=11236 python -I e2e_traps.py
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
        self.player_id = None
        self.frame = 0
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
        payload += struct.pack("<fffff", *FEET, yaw, pitch)
        payload += struct.pack("<I", self.frame * 16)
        self.send(0x43, payload)
        self.frame += 1

    def stand(self, seconds, pitch=0.0, yaw=YAW):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self.update(0, pitch, yaw)
            time.sleep(1 / 60)

    def tap(self, pitch, yaw=YAW):
        """Quick slot 4 down for a few frames, then up, looking at `pitch`."""
        self.stand(0.1, pitch, yaw)
        self.update(PRESS, pitch, yaw)
        self.stand(0.2, pitch, yaw)
        self.update(RELEASE, pitch, yaw)
        self.stand(0.3, pitch, yaw)

    def traps_placed(self, since=0.0):
        found = []
        for t, kind, payload in self.received:
            if kind == 0x96 and t >= since:
                player_id, slot, index = payload[0], payload[1], payload[2]
                position = struct.unpack("<fff", payload[3:15])
                angles = struct.unpack("<fff", payload[15:27])
                found.append((player_id, slot, index, position, angles))
        return found

    def spawned_id(self):
        # Our own spawn_player (0x84) is the first one after joining.
        for _, kind, payload in self.received:
            if kind == 0x84:
                return payload[0]
        return None


def main():
    failures = []
    first = Player(0xEA01)
    second = Player(0xEA02)
    first.join()
    second.join()
    first.stand(1.0)
    second.stand(0.1)
    player_id = first.spawned_id()
    print(f"first is player {player_id}")

    # Pitch -0.9 looks about 67 degrees down: the floor some 0.7 m ahead.
    began = time.monotonic()
    first.tap(-0.9)
    second.stand(0.2)
    for player, name in ((first, "first"), (second, "second")):
        placed = player.traps_placed(began)
        print(f"{name} got: {placed}")
        if len(placed) != 1:
            failures.append(f"{name}: {len(placed)} trap_placed after one tap")
            continue
        owner, slot, index, (x, y, z), angles = placed[0]
        if (owner, slot, index) != (player_id, 0x10, 0):
            failures.append(f"{name}: wrong ids {(owner, slot, index)}")
        if abs(y - FEET[1]) > 0.05 or not 0.3 < FEET[0] - x < 1.2 or abs(z - FEET[2]) > 0.05:
            failures.append(f"{name}: trap at {(x, y, z)}, not on the floor ahead")

    began = time.monotonic()
    first.tap(-0.9)
    if first.traps_placed(began):
        failures.append(f"a second trap on the same spot: {first.traps_placed(began)}")

    began = time.monotonic()
    first.tap(-0.9, yaw=math.pi)
    placed = first.traps_placed(began)
    print(f"toward -z: {placed}")
    if [entry[2] for entry in placed] != [1]:
        failures.append(f"toward -z: {placed}")

    for player in (first, second):
        with player.lock:
            header = struct.pack("<HHH", player.seq, player.newest, (player.bits << 1) & 0xFFFF | 1)
            player.sock.sendto(header + bytes([1, 0]), MATCH)
        player.stop = True

    print("\n".join(failures) or "PASS")
    sys.exit(1 if failures else 0)


main()
