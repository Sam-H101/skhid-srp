"""Lag compensation end to end. The target strafes across the shooter's line
of fire; the shooter aims where the relayed 0x82 updates show the target
(the stock client shows remote players at their latest update, with no
buffer) and taps single AK shots. Run the server with SRP_SIM_LATENCY_MS to
add round trip; compare with SRP_LAG_COMP_MAX_MS=0.

Usage: python -I e2e_lagcomp.py [taps]
"""

import math
import os
import socket
import struct
import sys
import threading
import time

class LinkSocket:
    """The match socket, acknowledging and ordering like the stock client."""

    def __init__(self):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.newest = 0xFFFF
        self.bits = 0
        self.delivered = 0xFFFF
        self.held = {}
        self.ready = []

    def __getattr__(self, name):
        return getattr(self.sock, name)

    def sendto(self, data, addr):
        word = struct.unpack("<H", data[4:6])[0]
        header = struct.pack("<HHH", struct.unpack("<H", data[:2])[0], self.newest,
                             (self.bits << 1) & 0xFFFF | word & 1)
        return self.sock.sendto(header + data[6:], addr)

    def recvfrom(self, size):
        while not self.ready:
            data, addr = self.sock.recvfrom(size)
            sequence = struct.unpack("<H", data[:2])[0]
            ahead = (sequence - self.newest) & 0xFFFF
            if ahead and ahead < 0x8000:
                self.bits = (self.bits >> ahead if ahead < 16 else 0) | 0x8000
                self.newest = sequence
            word = struct.unpack("<H", data[4:6])[0]
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
                self.ready.append((struct.pack("<HHH", self.newest, 0, 0) + message, addr))
        return self.ready.pop(0)


MATCH = ("127.0.0.1", int(os.environ.get("E2E_MATCH_PORT", "1236")))
FIRE, AIM, CROUCH, LEFT, RIGHT = 0x20, 0x80, 0x100, 0x4, 0x8
SHOOTER_FEET = (38.883, 0.445, 14.502)
CENTRE = (30.883, 0.445, 14.502)
SWAY = 1.2  # metres either side of the line of fire
PERIOD = 2.0  # seconds per sway

start = time.monotonic()
stop = threading.Event()
hits = []
kills = []


class Player:
    def __init__(self, session):
        self.session = session
        self.seq = 0
        self.order = 0
        self.server_seq = 0xFFFF
        self.frame = 0
        self.seen = None  # the other player's feet, as relayed
        self.sock = LinkSocket()
        self.sock.bind(("127.0.0.1", 0))
        self.sock.settimeout(0.2)
        self.lock = threading.Lock()
        threading.Thread(target=self.receive, daemon=True).start()

    def receive(self):
        while not stop.is_set():
            try:
                data, _ = self.sock.recvfrom(4096)
            except OSError:
                continue
            self.server_seq, _, word = struct.unpack("<HHH", data[:6])
            if word & 1:
                continue
            kind, payload = data[6], data[9:]
            if kind == 0x82 and len(payload) >= 37:
                self.seen = struct.unpack("<fff", payload[25:37])
            elif kind == 0x89 and self is shooter:
                hits.append(time.monotonic() - start)
            elif kind == 0x83 and self is shooter:
                kills.append(time.monotonic() - start)

    def send(self, kind, payload=b""):
        with self.lock:
            header = struct.pack("<HHH", self.seq, self.server_seq, 0)
            self.sock.sendto(header + struct.pack("<BH", kind, self.order) + payload, MATCH)
            self.seq = (self.seq + 1) & 0xFFFF
            self.order = (self.order + 1) & 0xFFFF

    def send_low(self, low_type):
        with self.lock:
            header = struct.pack("<HHH", self.seq, self.server_seq, 1)
            self.sock.sendto(header + bytes([1, low_type]), MATCH)
            self.seq = (self.seq + 1) & 0xFFFF

    def join(self):
        self.send(0x40, struct.pack("<I", self.session))
        time.sleep(0.3)
        self.send(0x41)
        time.sleep(0.3)
        self.send(0x48)
        self.send(0x42)

    def update(self, feet, yaw, mask):
        payload = struct.pack("<ffffI", 0, 0, 0, 0, mask)
        payload += struct.pack("<fffff", *feet, yaw, 0.0)
        payload += struct.pack("<I", self.frame * 16)
        self.frame += 1
        self.send(0x43, payload)


taps = int(sys.argv[1]) if len(sys.argv) > 1 else 12
shooter = Player(0xDE20)
target = Player(0xDE21)
shooter.join()
time.sleep(0.3)
target.join()
time.sleep(1.0)

shots = 0
frame = 0
t0 = time.monotonic()
while shots < taps and not kills:
    t = time.monotonic() - t0
    z = CENTRE[2] + SWAY * math.sin(2 * math.pi * t / PERIOD)
    velocity = math.cos(2 * math.pi * t / PERIOD)
    # Facing the shooter (+x), +z is to the target's left.
    target.update((CENTRE[0], CENTRE[1], z), -math.pi / 2, LEFT if velocity > 0 else RIGHT)
    seen = shooter.seen or CENTRE
    dx, dz = seen[0] - SHOOTER_FEET[0], seen[2] - SHOOTER_FEET[2]
    yaw = math.atan2(-dx, dz)
    # One tap every 25 frames (400 ms), once the target is moving fast.
    fire = frame % 25 == 0 and frame > 0
    shooter.update(SHOOTER_FEET, yaw, AIM | CROUCH | (FIRE if fire else 0))
    shots += fire
    frame += 1
    time.sleep(0.016)
time.sleep(0.5)

for player in (shooter, target):
    player.send_low(0)
time.sleep(0.3)
stop.set()
print(f"latency {os.environ.get('E2E_LABEL', '?')}: {shots} taps, {len(hits)} hits, "
      f"{'killed' if kills else 'alive'}", flush=True)
