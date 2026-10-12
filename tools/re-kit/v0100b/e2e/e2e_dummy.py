"""A scripted target for a live test: one player that joins the local srp
match, stands on open ground near team_1's spawn and logs every hit, affect
and kill the server sends. It reports the same spot after each respawn.

Usage: python -I e2e_dummy.py [minutes] [count] [--no-lobby]

Each dummy signs in (dummy0, dummy1, ...; password "dummy") and readies up in
the lobby like the stock client, so the lobby's matchmaker picks its match and
team. --no-lobby joins with made-up sessions instead (match 0x123, team_1).
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

LOGIN = ("127.0.0.1", 1234)
BROWSER = ("127.0.0.1", 80)
LOGIN_CERT = r"E:\sur-res-pro\Survarium\resources\ssl\survarium_login_server.crt"
# The stock client's lobby queries after signing in, in its order (srp's
# lobby log of a real session): restrictions, compatibility, skills tree,
# service prices, the four price lists, then the account's own state.
LOBBY_QUERIES = [bytes([4, 0, 0, 0]), bytes([5, 0, 0, 0]), bytes([9, 0, 0, 0]),
                 bytes([10, 0, 0, 0]), bytes([6, 1]), bytes([6, 2]), bytes([6, 3]),
                 bytes([6, 4]), bytes([0, 0, 0, 0]), bytes([3, 0, 0, 0]),
                 bytes([7, 0, 0, 0]), bytes([8, 0, 0, 0]), bytes([11, 0, 0, 0])]


def receive_exactly(sock, size):
    data = b""
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise ConnectionError("lobby closed the connection")
        data += chunk
    return data


def lobby_send(sock, kind, body=b""):
    message = bytes([kind]) + body
    sock.sendall(bytes([len(message)]) + message)


def lobby_receive(sock):
    """One lobby message: a u8 length, or 0 and a u16 length."""
    size = receive_exactly(sock, 1)[0]
    if size == 0:
        size = struct.unpack("<H", receive_exactly(sock, 2))[0]
    return receive_exactly(sock, size)


def sign_in(email):
    """Sign in, find the lobby and ready up like the stock client. Returns the
    session, the match server address, the match and team the lobby assigned,
    and the profile the dummy plays."""
    import ssl
    import urllib.request

    sock = socket.create_connection(LOGIN)
    sock.sendall(b"\x01")
    time.sleep(0.1)
    sock.sendall(bytes([len(email)]) + email.encode() + b"0.100b")
    if sock.recv(1) != b"\x0b":
        raise ConnectionError(f"{email}: login refused the user name")
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    context.check_hostname = False
    context.verify_mode = ssl.CERT_REQUIRED
    context.set_ciphers("DEFAULT:@SECLEVEL=0")
    context.load_verify_locations(LOGIN_CERT)
    tls = context.wrap_socket(sock)
    tls.sendall(b"\x05dummy")  # a made-up password: srp logs it in plain text
    reply = tls.recv(256)
    session = struct.unpack("<I", reply[-4:])[0]
    tls.close()

    url = (f"http://{BROWSER[0]}:{BROWSER[1]}/hello?unused=1&type=2"
           f"&local_ip=127.0.0.1&login_ip={LOGIN[0]}")
    with urllib.request.urlopen(url, timeout=5) as answer:
        host, port = answer.read().decode().rsplit(":", 1)

    lobby = socket.create_connection((host, int(port)))
    lobby_send(lobby, 0x26, struct.pack("<I", session))
    if lobby_receive(lobby)[:1] != b"\x30":
        raise ConnectionError(f"{email}: lobby refused session {session:#x}")
    for query in LOBBY_QUERIES:
        lobby_send(lobby, 0x21, query)
        lobby_receive(lobby)
    lobby_send(lobby, 0x21, bytes([1, 0, 0, 0]))
    profiles = lobby_receive(lobby)
    count, rest, profile_ids = profiles[2], profiles[3:], []
    for _ in range(count):
        profile_ids.append(struct.unpack("<I", rest[:4])[0])
        rest = rest[5 + rest[4]:]
    for profile_id in profile_ids:
        lobby_send(lobby, 0x21, bytes([2]) + struct.pack("<I", profile_id))
        lobby_receive(lobby)
    lobby_send(lobby, 0x20, struct.pack("<I", profile_ids[0]))
    return session, lobby, profile_ids[0], lobby_receive(lobby)


def await_match(email, lobby, answer):
    """Poll the queue like the stock client until the lobby sends a match;
    returns the match server address, the match and the team."""
    while answer[:1] == b"\x36":
        time.sleep(0.7)
        lobby_send(lobby, 0x21, bytes([0, 0, 0, 0]))
        answer = lobby_receive(lobby)
    if answer[:1] != b"\x33":
        raise ConnectionError(f"{email}: lobby did not send a match: {answer.hex()}")
    size = answer[1]
    match_host = answer[2:2 + size].decode()
    match_port, match_id, team = struct.unpack("<HIB", answer[2 + size:9 + size])
    # The stock client drops the lobby connection while it plays.
    lobby.close()
    return (match_host, match_port), match_id, team


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



SPOT = (-31.20, 0.504, -66.70)
arguments = [argument for argument in sys.argv[1:] if not argument.startswith("--")]
use_lobby = "--no-lobby" not in sys.argv
minutes = float(arguments[0]) if len(arguments) > 0 else 15.0
count = int(arguments[1]) if len(arguments) > 1 else 1
# A row along x, 1.5 m apart, alternating either side of the first spot.
spots = []
for i in range(count):
    offset = ((i + 1) // 2) * 1.5 * (1 if i % 2 else -1)
    spots.append((SPOT[0] + offset, SPOT[1], SPOT[2]))
dummies = []
queued = []
for i in range(count):
    name = f"dummy{i}"
    if use_lobby:
        queued.append((name, *sign_in(name)))
    else:
        dummies.append(Player(name, 0xDE21 + i))
for name, session, lobby, profile_id, answer in queued:
    MATCH, match_id, team = await_match(name, lobby, answer)
    print(f"{stamp()} {name}: signed in as session {session:#x}; the lobby put "
          f"profile {profile_id} in match {match_id:#x} on team {team}", flush=True)
    dummies.append(Player(name, session))
for dummy, spot in zip(dummies, spots):
    dummy.join()
    print(f"{stamp()} {dummy.name} joined; standing at {spot} for {minutes} min", flush=True)
end = time.monotonic() + minutes * 60
try:
    while time.monotonic() < end:
        for dummy, spot in zip(dummies, spots):
            dummy.update(spot, 0.0, 0)
        time.sleep(0.016)
finally:
    for dummy in dummies:
        dummy.send_low(0)
    time.sleep(0.3)
    stop.set()
    print(f"{stamp()} counts: {counts}", flush=True)
