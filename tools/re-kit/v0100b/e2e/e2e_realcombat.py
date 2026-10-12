"""Two scripted players in one srp match, driven the way the stock client
drives it: one 0x43 update per 16 ms frame and nothing else.

The shooter crouches 8 m from the target and holds aim + fire (AK-74u). The
server must replay the shots and answer with 0x89 hits and a 0x83 kill, then
respawn the target. Then the respawned target is shot once more and presses
quick slot 3 (a medkit in the dummy loadout); the stack it uses is logged by
the server.

Usage: python -I e2e_realcombat.py [seconds_of_fire]
"""

import math
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


MATCH = ("127.0.0.1", int(__import__("os").environ.get("E2E_MATCH_PORT", "1236")))
FIRE, AIM, CROUCH, WEAPON_2 = 0x20, 0x80, 0x100, 0x2000
QUICK_SLOT_3_DOWN, QUICK_SLOT_3_UP = 0x40000, 0x80000
# An open stretch of level_03_evn (the combat tests' firing range).
SHOOTER_FEET = (38.883, 0.445, 14.502)
TARGET_FEET = (30.883, 0.445, 14.502)
YAW = math.pi / 2  # facing -x, at the target

start = time.monotonic()
stop = threading.Event()
counts = {}


def stamp():
    return f"{time.monotonic() - start:6.2f}s"


def note(key):
    counts[key] = counts.get(key, 0) + 1


class Player:
    def __init__(self, name, session):
        self.name = name
        self.session = session
        self.seq = 0
        self.order = 0
        self.server_seq = 0xFFFF
        self.frame = 0
        self.player_id = None
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
            if kind == 0x83:
                victim, killer, headshot, item = struct.unpack("<BB?I", payload[:7])
                note(f"{self.name}:kill")
                print(f"{stamp()} {self.name}: kill_player victim={victim} killer={killer} "
                      f"headshot={headshot} weapon={item}", flush=True)
            elif kind == 0x89:
                initiator, victim = payload[0], payload[1]
                n = payload[2]
                part = payload[3:3 + n].decode()
                rest = payload[3 + n:]
                m = rest[0]
                damage_type = rest[1:1 + m].decode()
                amount, ap = struct.unpack("<ff", rest[1 + m:9 + m])
                note(f"{self.name}:hit")
                print(f"{stamp()} {self.name}: hit_player {initiator}->{victim} {part} "
                      f"{damage_type} {amount:.3f} ap {ap:.3f}", flush=True)
            elif kind == 0x8A:
                player = payload[0]
                n = payload[1]
                part = payload[2:2 + n].decode()
                affect, event = struct.unpack("<II", payload[2 + n:10 + n])
                print(f"{stamp()} {self.name}: affect_damage_model player={player} {part} "
                      f"affect={affect} event={event}", flush=True)
            elif kind == 0x84:
                note(f"{self.name}:spawn{payload[0]}")
                print(f"{stamp()} {self.name}: spawn_player id={payload[0]}", flush=True)
            elif kind == 0x87:
                print(f"{stamp()} {self.name}: respawn_time_changed "
                      f"{struct.unpack('<I', payload[:4])[0]}", flush=True)

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
        """One client frame: input (angular velocity, acceleration, mask),
        state (feet, yaw, look pitch), the client's time."""
        payload = struct.pack("<ffffI", 0, 0, 0, 0, mask)
        payload += struct.pack("<fffff", *feet, yaw, 0.0)
        payload += struct.pack("<I", self.frame * 16)
        self.frame += 1
        self.send(0x43, payload)


def frames(seconds, shooter_mask, target_mask=lambda frame: 0):
    for frame in range(int(seconds * 1000 / 16)):
        target.update(TARGET_FEET, -YAW, target_mask(frame))
        shooter.update(SHOOTER_FEET, YAW, shooter_mask(frame))
        time.sleep(0.016)


seconds = float(sys.argv[1]) if len(sys.argv) > 1 else 3.0
shooter = Player("shooter", 0xDE10)
target = Player("target", 0xDE11)
shooter.join()
time.sleep(0.3)
target.join()
time.sleep(0.5)
print(f"{stamp()} both joined; firing for {seconds} s", flush=True)

frames(seconds, lambda frame: AIM | CROUCH | FIRE)
print(f"{stamp()} cease fire; waiting for the respawn", flush=True)
waited = time.monotonic() + 12
while time.monotonic() < waited and counts.get("target:spawn1", 0) < 2:
    frames(0.5, lambda frame: AIM | CROUCH)
# A full-auto magazine leaves the view kicked up (as on the client) and this
# shooter never pulls its aim down: switch to the Remington 700 for one shot.
print(f"{stamp()} switching to the Remington 700", flush=True)
# Hiding the AK and showing the Remington take 600 ms each.
frames(1.6, lambda frame: AIM | CROUCH | (WEAPON_2 if frame == 0 else 0),
       lambda frame: QUICK_SLOT_3_DOWN if frame == 5 else QUICK_SLOT_3_UP if frame == 6 else 0)
print(f"{stamp()} one shot at the respawned target, then it uses a medkit", flush=True)
frames(0.12, lambda frame: AIM | CROUCH | FIRE)
frames(0.5, lambda frame: AIM | CROUCH,
       lambda frame: QUICK_SLOT_3_DOWN if frame == 0 else QUICK_SLOT_3_UP if frame == 1 else 0)
frames(3.5, lambda frame: AIM | CROUCH)

for player in (shooter, target):
    player.send_low(0)
time.sleep(0.5)
stop.set()
print(f"{stamp()} counts: {counts}", flush=True)
