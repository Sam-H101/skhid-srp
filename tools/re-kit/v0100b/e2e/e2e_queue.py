"""Queue + online counter check against a standalone lobby-server.

Run lobby-server.exe with SRP_LOBBY_PORT=11235 SRP_QUEUE_MIN_PLAYERS=2 first.
Unknown sessions get a dummy account (profile 200000)."""
import socket, struct, sys, time

PORT = 11235


def frame(sock):
    n = sock.recv(1)
    if not n:
        raise EOFError
    n = n[0]
    if n == 0:
        n = struct.unpack('<H', recv_exact(sock, 2))[0]
    return recv_exact(sock, n)


def recv_exact(sock, n):
    b = b''
    while len(b) < n:
        c = sock.recv(n - len(b))
        if not c:
            raise EOFError
        b += c
    return b


def send(sock, body):
    sock.sendall(bytes([len(body)]) + body)


def chat(session):
    s = socket.create_connection(('127.0.0.1', PORT))
    s.settimeout(3)
    send(s, b'\xC3' + struct.pack('<I', session) + b'\x05')
    frames = [frame(s), frame(s)]
    return s, frames


def lobby(session):
    s = socket.create_connection(('127.0.0.1', PORT))
    s.settimeout(3)
    send(s, b'\x26' + struct.pack('<I', session))
    assert frame(s)[0] == 0x30, 'connection successful'
    return s


def ready(s):
    send(s, b'\x20' + struct.pack('<I', 200000))
    return frame(s)


def poll(s):
    send(s, b'\x21\x00\x00\x00\x00')
    return frame(s)


ok = True
def check(what, cond):
    global ok
    print(('PASS ' if cond else 'FAIL ') + what)
    ok &= bool(cond)


c1, f1 = chat(0xA001)
print('chat 1:', [f.hex() for f in f1])
check('chat 1 hears the online count', any(b'#pc:[1] #mc:[0]' in f and f[:2] == b'\xC9\x08' for f in f1))

l1 = lobby(0xA001)
a = ready(l1)
print('ready 1:', a.hex())
check('first player is queued (status 2)', a[:3] == bytes([0x36, 0, 2]))
c1.settimeout(3)
q = frame(c1)
print('chat 1 after ready:', q.hex(), q)
check('queue place #q:[1] from the matchmaker', q[:2] == b'\xC9\x07' and q.endswith(b'#q:[1]'))
a = poll(l1)
check('poll keeps it queued', a[:3] == bytes([0x36, 0, 2]))

c2, f2 = chat(0xA002)
check('chat 2 hears 2 players', any(b'#pc:[2]' in f for f in f2))
l2 = lobby(0xA002)
a = ready(l2)
print('ready 2:', a.hex())
check('second player starts the match', a[0] == 0x33)
a = poll(l1)
print('poll 1:', a.hex())
check('first player is sent to the same match', a[0] == 0x33)

# The ticker announces the new match within ~2 s.
deadline = time.time() + 5
seen = False
while time.time() < deadline and not seen:
    try:
        f = frame(c1)
        seen = b'#mc:[1]' in f
    except socket.timeout:
        break
check('everyone hears #mc:[1]', seen)

print('ALL PASS' if ok else 'SOME FAILED')
sys.exit(0 if ok else 1)
