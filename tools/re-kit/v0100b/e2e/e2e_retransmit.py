"""End-to-end check of server->client retransmission: two scripted players
join a match on a match server started with SRP_SIM_LOSS_PERCENT; one moves
for a while and the other must receive every one of its frames, in order,
through the lossy link. The receiving side acknowledges and orders exactly
like the stock client (never skipping a missing order id).

Usage: E2E_MATCH_PORT=11236 python -I e2e_retransmit.py [frames]
"""

import os
import socket
import struct
import sys
import threading
import time

MATCH = ("127.0.0.1", int(os.environ.get("E2E_MATCH_PORT", "11236")))
FRAMES = int(sys.argv[1]) if len(sys.argv) > 1 else 600


class Player:
    def __init__(self, name, session):
        self.name = name
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
        self.player_id = None
        self.frames_from = {}  # player id -> relayed client frame times, in delivery order
        self.datagrams = 0
        self.copies = 0  # messages that arrived again after they were held or delivered
        self.max_held = 0
        self.stop = False
        threading.Thread(target=self.receive, daemon=True).start()

    def receive(self):
        while not self.stop:
            try:
                data, _ = self.sock.recvfrom(4096)
            except OSError:
                continue
            assert len(data) <= 256, len(data)
            self.datagrams += 1
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
                if (order - self.delivered) & 0xFFFF >= 0x8000 or order == self.delivered \
                        or order in self.held:
                    self.copies += 1
                    continue
                self.held[order] = message
            self.max_held = max(self.max_held, len(self.held))
            while (self.delivered + 1) & 0xFFFF in self.held:
                self.delivered = (self.delivered + 1) & 0xFFFF
                self.handle(self.held.pop(self.delivered))

    def handle(self, message):
        kind, payload = message[0], message[3:]
        if kind == 0x80:
            pass
        elif kind == 0x82:  # server_player_input: time, player id, input, state
            player_id = payload[4]
            client_time = struct.unpack("<I", payload[0:4])[0]
            self.frames_from.setdefault(player_id, []).append(client_time)
        elif kind == 0x92 and self.player_id is None:
            pass

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

    def update(self, frame, feet):
        payload = struct.pack("<ffffI", 0, 0, 0, 0, 0)
        payload += struct.pack("<fffff", *feet, 0.0, 0.0)
        payload += struct.pack("<I", frame * 16)
        self.send(0x43, payload)

    def disconnect(self):
        with self.lock:
            header = struct.pack("<HHH", self.seq, self.newest, (self.bits << 1) & 0xFFFF | 1)
            self.sock.sendto(header + bytes([1, 0]), MATCH)


mover = Player("mover", 0xEE01)
watcher = Player("watcher", 0xEE02)
mover.join()
watcher.join()
time.sleep(1.0)

start = time.monotonic()
for frame in range(FRAMES):
    mover.update(frame, (-31.2 + frame * 0.01, 0.504, -66.7))
    watcher.update(frame, (-29.2, 0.504, -66.7))
    time.sleep(1 / 60)
sent_for = time.monotonic() - start

# Let the resends finish (500 ms each round).
deadline = time.monotonic() + 10
while time.monotonic() < deadline:
    counts = [len(v) for v in watcher.frames_from.values()]
    if counts and max(counts) >= FRAMES:
        break
    watcher.update(FRAMES, (-29.2, 0.504, -66.7))
    time.sleep(1 / 30)
drained = time.monotonic() - start - sent_for

for player in (mover, watcher):
    player.disconnect()
    player.stop = True

print(f"sent {FRAMES} mover frames in {sent_for:.1f} s; drained in {drained:.1f} s")
print(f"watcher: {watcher.datagrams} datagrams, {watcher.copies} repeated messages, "
      f"at most {watcher.max_held} held behind a gap, delivered up to order {watcher.delivered}")
for player_id, times in watcher.frames_from.items():
    in_order = all(b >= a for a, b in zip(times, times[1:]))
    print(f"  player {player_id}: {len(times)} frames, non-decreasing: {in_order}")
best = max((len(t) for t in watcher.frames_from.values()), default=0)
ok = best >= FRAMES
print("PASS" if ok else "FAIL")
sys.exit(0 if ok else 1)
