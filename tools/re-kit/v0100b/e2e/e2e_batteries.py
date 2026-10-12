"""Two scripted players (both team_1, no lobby assignment) play a 2-battery
match on a running srp (SRP_BATTERIES=2, short countdown):

  A picks up battery 0 and stores it; A picks up battery 1; B kills A, so A
  drops it; B picks the dropped battery up and stores it: team_1 holds both,
  which ends the match (time-up text, then match_finished).

Positions are "teleports": the server trusts the reported position. Logs every
battery, kill and lifecycle message each player receives."""

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
USE = 0x10000000
TEAM_1_BASE = (-48.700, 1.148, -73.263)  # team_1 use-zone centre (map.rs)
start = time.monotonic()
stop = threading.Event()


def stamp():
    return f"{time.monotonic() - start:6.2f}s"


def beside(point):
    x, y, z = point
    return (x + 0.5, y - 0.8, z)


class Player:
    def __init__(self, name, session):
        self.name = name
        self.session = session
        self.seq = 0
        self.order = 0
        self.server_seq = 0xFFFF
        self.in_process = threading.Event()
        self.finished = threading.Event()
        self.batteries = {}
        self.got_state = threading.Event()
        self.events = []
        self.sock = LinkSocket()
        self.sock.bind(("127.0.0.1", 0))
        self.sock.settimeout(0.2)
        threading.Thread(target=self.receive, daemon=True).start()

    def log(self, text):
        line = f"{stamp()} {self.name}: {text}"
        self.events.append(text)
        print(line, flush=True)

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
            if kind == 0x9A:
                status = struct.unpack("<I", payload)[0]
                self.log(f"game_status {status}")
                if status == 4:
                    self.in_process.set()
            elif kind == 0x86:
                self.log(f"match_time {struct.unpack('<I', payload)[0]} ms")
            elif kind == 0x8C:
                self.log("match_finished")
                self.finished.set()
            elif kind == 0x83:
                victim, killer = payload[0], payload[1]
                self.log(f"kill_player victim={victim} killer={killer}")
            elif kind == 0x94:
                t1, t2, count = struct.unpack("<bbB", payload[:3])
                offset = 3
                items = []
                for _ in range(count):
                    slot, item = payload[offset], payload[offset + 1]
                    pos = struct.unpack("<fff", payload[offset + 2:offset + 14])
                    offset += 14
                    items.append((slot, item, pos))
                    self.batteries[item] = pos
                containers = []
                for _ in range(payload[offset]):
                    cid, n = payload[offset + 1], payload[offset + 2]
                    containers.append((cid, list(payload[offset + 3:offset + 3 + n])))
                    offset += 2 + n
                self.log(f"initialize_victory_items points {t1}/{t2} items "
                         f"{[(s, i, tuple(round(c, 2) for c in p)) for s, i, p in items]} "
                         f"containers {containers}")
                self.got_state.set()
            elif kind == 0x95:
                player, item, take, cid = struct.unpack("<BB?B", payload[:4])
                extra = ""
                if not take and cid == 0xFF:
                    extra = f" at {tuple(round(c, 2) for c in struct.unpack('<fff', payload[4:16]))}"
                self.log(f"victory_item_take_or_put player={player} item={item} "
                         f"{'take' if take else 'put'} container={cid:#x}{extra}")

    def send(self, kind, payload=b""):
        header = struct.pack("<HHH", self.seq, self.server_seq, 0)
        self.sock.sendto(header + struct.pack("<BH", kind, self.order) + payload, MATCH)
        self.seq += 1
        self.order = (self.order + 1) & 0xFFFF

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

    def update(self, feet, use):
        payload = struct.pack("<ffffI", 0, 0, 0, 0, USE if use else 0)
        payload += struct.pack("<fffff", *feet, 0.0, 0.0) + struct.pack("<I", 0)
        self.send(0x43, payload)

    def use_at(self, feet):
        """Walk there, press use, release."""
        self.update(feet, False)
        time.sleep(0.1)
        self.update(feet, True)
        time.sleep(0.2)
        self.update(feet, False)
        time.sleep(0.2)


def hit(attacker, target_id):
    def string(text):
        return bytes([len(text)]) + text.encode()

    attacker.send(0x4B, bytes([1, 0, target_id]) + string("torso") + string("injury")
                  + struct.pack("<ff", 30.0, 0.0))


def keepalive(players):
    while not stop.is_set():
        for player in players:
            player.send_low(2)
        time.sleep(1)


a = Player("A", 0xBA00)
b = Player("B", 0xBA01)
a.join()
time.sleep(0.5)
b.join()
threading.Thread(target=keepalive, args=([a, b],), daemon=True).start()
assert a.got_state.wait(5) and b.got_state.wait(5), "no battery state"
print(f"{stamp()} both joined; waiting for the match to start", flush=True)
assert a.in_process.wait(30), "match never started"
time.sleep(0.3)

base = beside(TEAM_1_BASE)
print(f"{stamp()} -- A picks up battery 0 and stores it", flush=True)
a.use_at(beside(a.batteries[0]))
a.use_at(base)
print(f"{stamp()} -- A picks up battery 1", flush=True)
carrying_at = beside(a.batteries[1])
a.use_at(carrying_at)
print(f"{stamp()} -- B kills A (A should drop battery 1)", flush=True)
for _ in range(4):
    hit(b, 0)
    time.sleep(0.1)
time.sleep(0.3)
print(f"{stamp()} -- B picks up the dropped battery and stores it", flush=True)
b.use_at(carrying_at)
b.use_at(base)
ok = a.finished.wait(15) and b.finished.wait(1)
print(f"{stamp()} match_finished on both: {ok}", flush=True)
for player in (a, b):
    player.send_low(0)
time.sleep(0.5)
stop.set()
