"""End-to-end check of the time sync exchange: two scripted players join; no
sync_response may arrive before a player asks, each time_synchronization_request
(0x45) gets one back carrying the connected bitmask, and the confirmation
(0x46) is accepted.

Usage: E2E_MATCH_PORT=11236 python -I e2e_timesync.py
"""

import os
import socket
import struct
import sys
import threading
import time

MATCH = ("127.0.0.1", int(os.environ.get("E2E_MATCH_PORT", "11236")))


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

    def keep_alive(self, seconds):
        end = time.monotonic() + seconds
        frame = 0
        while time.monotonic() < end:
            payload = struct.pack("<ffffI", 0, 0, 0, 0, 0)
            payload += struct.pack("<fffff", -31.2, 0.504, -66.7, 0.0, 0.0)
            payload += struct.pack("<I", frame * 16)
            self.send(0x43, payload)
            frame += 1
            time.sleep(1 / 30)

    def syncs(self):
        return [(t, struct.unpack("<I", p[:4])[0]) for t, k, p in self.received if k == 0x8B]


failures = []
first = Player(0xEF01)
second = Player(0xEF02)
first.join()
second.join()
first.keep_alive(1.0)

for player, name in ((first, "first"), (second, "second")):
    if player.syncs():
        failures.append(f"{name} got a sync_response it never asked for: {player.syncs()}")
    kinds = sorted({k for _, k, _ in player.received})
    print(f"{name}: after joining, got message types {[hex(k) for k in kinds]}")

for round_ in range(2):
    asked = time.monotonic()
    first.send(0x45, struct.pack("<I", 185_628 + round_ * 4000))
    first.keep_alive(0.5)
    answers = [(t, mask) for t, mask in first.syncs() if t >= asked]
    if len(answers) != 1:
        failures.append(f"round {round_}: {len(answers)} answers to one request")
        continue
    answered, mask = answers[0]
    print(f"round {round_}: sync_response after {1000 * (answered - asked):.1f} ms, "
          f"connected bitmask {mask:#06b}")
    if mask & 0b11 != 0b11:
        failures.append(f"round {round_}: bitmask {mask:#b} lacks the two players")
    first.send(0x46)
if second.syncs():
    failures.append(f"second got the first's answers: {second.syncs()}")

for player in (first, second):
    with player.lock:
        header = struct.pack("<HHH", player.seq, player.newest, (player.bits << 1) & 0xFFFF | 1)
        player.sock.sendto(header + bytes([1, 0]), MATCH)
    player.stop = True

print("\n".join(failures) or "PASS")
sys.exit(1 if failures else 0)
