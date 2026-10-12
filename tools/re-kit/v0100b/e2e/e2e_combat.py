"""Two scripted players in one srp match: wait for the match to start, then
player 0 reports four hits on player 1 (srp's synthetic client_player_hit).
Logs the kill, scoreboard and respawn messages each player receives."""

import socket
import struct
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
start = time.monotonic()
stop = threading.Event()


def stamp():
    return f"{time.monotonic() - start:6.2f}s"


class Player:
    def __init__(self, name, session):
        self.name = name
        self.session = session
        self.seq = 0
        self.order = 0
        self.server_seq = 0xFFFF
        self.in_process = threading.Event()
        self.sock = LinkSocket()
        self.sock.bind(("127.0.0.1", 0))
        self.sock.settimeout(0.2)
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
            if kind == 0x9A and struct.unpack("<I", payload)[0] == 4:
                self.in_process.set()
            elif kind == 0x83:
                victim, killer, headshot, item = struct.unpack("<BB?I", payload)
                print(f"{stamp()} {self.name}: kill_player victim={victim} killer={killer} "
                      f"headshot={headshot} weapon={item}", flush=True)
            elif kind == 0x89:
                print(f"{stamp()} {self.name}: hit_player", flush=True)
            elif kind == 0x88:
                player, kills, deaths = struct.unpack("<BII", payload)
                print(f"{stamp()} {self.name}: kd player={player} kills={kills} "
                      f"deaths={deaths}", flush=True)
            elif kind == 0x87:
                print(f"{stamp()} {self.name}: respawn_time_changed "
                      f"{struct.unpack('<I', payload)[0]}", flush=True)
            elif kind == 0x84:
                print(f"{stamp()} {self.name}: spawn_player id={payload[0]}", flush=True)

    def send(self, kind, payload=b""):
        header = struct.pack("<HHH", self.seq, self.server_seq, 0)
        self.sock.sendto(header + struct.pack("<BH", kind, self.order) + payload, MATCH)
        self.seq += 1
        self.order += 1

    def send_low(self, low_type):
        header = struct.pack("<HHH", self.seq, self.server_seq, 1)
        self.sock.sendto(header + bytes([1, low_type]), MATCH)
        self.seq += 1

    def join(self):
        self.send(0x40, struct.pack("<I", self.session))
        time.sleep(0.3)
        self.send(0x41)
        time.sleep(0.3)
        self.send(0x48)
        self.send(0x42)


def hit(attacker, target_id):
    def string(text):
        return bytes([len(text)]) + text.encode()

    payload = (bytes([1, 0, target_id]) + string("torso") + string("injury")
               + struct.pack("<ff", 30.0, 0.0))
    attacker.send(0x4B, payload)


shooter = Player("shooter", 0xDE00)
target = Player("target", 0xDE01)
shooter.join()
time.sleep(0.5)
target.join()
print(f"{stamp()} both joined; waiting for the match to start", flush=True)
if not target.in_process.wait(30):
    print("match never started", flush=True)
for _ in range(4):
    hit(shooter, 1)
    time.sleep(0.1)
deadline = time.monotonic() + 12
while time.monotonic() < deadline:
    for player in (shooter, target):
        player.send_low(2)
    time.sleep(1)
for player in (shooter, target):
    player.send_low(0)
time.sleep(0.5)
stop.set()
