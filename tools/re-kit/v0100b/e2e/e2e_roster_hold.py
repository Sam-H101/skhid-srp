"""Against an srp started with SRP_QUEUE_MIN_PLAYERS=2 on the test ports
(login 21234, browser 21280, lobby 21235, match 21236): two clients ready up
through the queue; the second is placed at once and connects to the match
straight away, the first hears of it 0.7 s later at its next status request,
as the stock client does. Both must get a list of two players. Passwords are
made up."""
import socket, ssl, struct, sys, threading, time, urllib.request

LOGIN = ("127.0.0.1", 21234)
BROWSER = ("127.0.0.1", 21280)
LOGIN_CERT = r"E:\sur-res-pro\Survarium\resources\ssl\survarium_login_server.crt"
LOBBY_QUERIES = [bytes([4, 0, 0, 0]), bytes([5, 0, 0, 0]), bytes([9, 0, 0, 0]),
                 bytes([10, 0, 0, 0]), bytes([6, 1]), bytes([6, 2]), bytes([6, 3]),
                 bytes([6, 4]), bytes([0, 0, 0, 0]), bytes([3, 0, 0, 0]),
                 bytes([7, 0, 0, 0]), bytes([8, 0, 0, 0]), bytes([11, 0, 0, 0])]
CLIENT_STATE = bytes([0, 0, 0, 0])


class LinkSocket:
    """The match socket, acknowledging and ordering like the stock client
    (from e2e_dummy.py)."""

    def __init__(self):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.newest = 0xFFFF
        self.bits = 0
        self.delivered = 0xFFFF
        self.held = {}
        self.ready = []

    def sendto(self, data, addr):
        word = struct.unpack("<H", data[4:6])[0]
        header = struct.pack("<HHH", struct.unpack("<H", data[:2])[0], self.newest,
                             (self.bits << 1) & 0xFFFF | word & 1)
        return self.sock.sendto(header + data[6:], addr)

    def receive(self, deadline):
        """The next game message in order (type, payload), or None once
        `deadline` (time.monotonic()) passes."""
        while not self.ready:
            if time.monotonic() > deadline:
                return None
            try:
                data, _ = self.sock.recvfrom(4096)
            except OSError:
                return None
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
                self.ready.append((message[0], message[3:]))
        return self.ready.pop(0)


def exactly(sock, size):
    data = b""
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise ConnectionError("closed")
        data += chunk
    return data


def send(sock, kind, body=b""):
    message = bytes([kind]) + body
    sock.sendall(bytes([len(message)]) + message)


def receive(sock):
    size = exactly(sock, 1)[0]
    if size == 0:
        size = struct.unpack("<H", exactly(sock, 2))[0]
    return exactly(sock, size)


def sign_in(email):
    sock = socket.create_connection(LOGIN)
    sock.sendall(b"\x01")
    time.sleep(0.1)
    sock.sendall(bytes([len(email)]) + email.encode() + b"0.100b")
    assert sock.recv(1) == b"\x0b"
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    context.check_hostname = False
    context.set_ciphers("DEFAULT:@SECLEVEL=0")
    context.maximum_version = ssl.TLSVersion.TLSv1_2
    context.load_verify_locations(LOGIN_CERT)
    tls = context.wrap_socket(sock)
    tls.sendall(b"\x05dummy")
    reply = tls.recv(256)
    tls.close()
    session = struct.unpack("<I", reply[-4:])[0]
    url = f"http://{BROWSER[0]}:{BROWSER[1]}/hello?unused=1&type=2&local_ip=1&login_ip=1"
    with urllib.request.urlopen(url, timeout=5) as answer:
        host, port = answer.read().decode().rsplit(":", 1)
    lobby = socket.create_connection((host, int(port)))
    send(lobby, 0x26, struct.pack("<I", session))
    assert receive(lobby)[:1] == b"\x30"
    for query in LOBBY_QUERIES:
        send(lobby, 0x21, query)
        receive(lobby)
    send(lobby, 0x21, bytes([1, 0, 0, 0]))
    profiles = receive(lobby)
    profile_id = struct.unpack("<I", profiles[3:7])[0]
    return session, lobby, profile_id


def match_address(answer):
    size = answer[1]
    host = answer[2:2 + size].decode()
    port = struct.unpack("<H", answer[2 + size:4 + size])[0]
    return host, port


class Player:
    def __init__(self, name, session, address):
        self.name, self.session, self.address = name, session, address
        self.link = LinkSocket()
        self.link.sock.bind(("127.0.0.1", 0))
        self.link.sock.settimeout(0.05)
        self.seq = self.order = 0
        self.got = []  # (seconds since start, type, payload)

    def send(self, kind, payload=b""):
        header = struct.pack("<HHH", self.seq, 0, 0)
        self.link.sendto(header + struct.pack("<BH", kind, self.order) + payload, self.address)
        self.seq = (self.seq + 1) & 0xFFFF
        self.order = (self.order + 1) & 0xFFFF

    def keepalive(self):
        header = struct.pack("<HHH", self.seq, 0, 1)
        self.link.sendto(header + bytes([1, 2]), self.address)
        self.seq = (self.seq + 1) & 0xFFFF

    def connect(self):
        self.send(0x40, struct.pack("<I", self.session))  # connection request
        self.send(0x41)  # get startup info

    def pump(self, start):
        deadline = time.monotonic() + 0.02
        while (message := self.link.receive(deadline)) is not None:
            self.got.append((time.monotonic() - start, *message))
        self.keepalive()

    def roster(self):
        """The players in its list: profiles after the match options."""
        types = [kind for _, kind, _ in self.got]
        if 0x81 not in types:
            return None
        options_at = types.index(0x81)
        return sum(1 for kind in types[options_at + 1:] if kind == 0x92)

    def options_at(self):
        return next(at for at, kind, _ in self.got if kind == 0x81)


failed = False


def check(label, ok, detail=""):
    global failed
    failed |= not ok
    print(("PASS " if ok else "FAIL ") + label + ("" if ok else f": {detail}"))


first_session, first_lobby, first_profile = sign_in("roster-first@e2e.invalid")
second_session, second_lobby, second_profile = sign_in("roster-second@e2e.invalid")

send(first_lobby, 0x20, struct.pack("<I", first_profile))
answer = receive(first_lobby)
check("the first waits in the queue", answer[0] == 0x36 and answer[2] == 2, answer.hex())

send(second_lobby, 0x20, struct.pack("<I", second_profile))
answer = receive(second_lobby)
check("the second is placed at once", answer[0] == 0x33, answer.hex())
second = Player("second", second_session, match_address(answer))
start = time.monotonic()
second.connect()
while time.monotonic() - start < 0.7:
    second.pump(start)
check("the second's list waits for the first", second.roster() is None,
      [hex(kind) for _, kind, _ in second.got])

send(first_lobby, 0x21, CLIENT_STATE)
answer = receive(first_lobby)
check("the first hears of its match at its next status request", answer[0] == 0x33, answer.hex())
first = Player("first", first_session, match_address(answer))
first.connect()
while time.monotonic() - start < 3:
    first.pump(start)
    second.pump(start)

check("the first's list has both", first.roster() == 2, first.roster())
check("the second's list has both", second.roster() == 2, second.roster())
if second.roster():
    print(f"  the second's list came {second.options_at():.2f} s after it asked")
print("FAILED" if failed else "ALL PASSED")
sys.exit(1 if failed else 0)
