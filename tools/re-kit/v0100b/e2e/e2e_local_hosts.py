"""Against an srp started with SRP_BIND_HOST=0.0.0.0 SRP_PUBLIC_HOST=203.0.113.67
and the test ports (login 21234, browser 21280, lobby 21235, match 21236):
a client that reaches srp at HOST (argv[1]) must be sent on to HOST at every
step: login -> browser host, browser -> lobby address, lobby -> match host.
Passwords are made up."""
import socket, ssl, struct, sys, time, urllib.request

HOST = sys.argv[1]
LOGIN_CERT = r"E:\sur-res-pro\Survarium\resources\ssl\survarium_login_server.crt"
LOBBY_QUERIES = [bytes([4, 0, 0, 0]), bytes([5, 0, 0, 0]), bytes([9, 0, 0, 0]),
                 bytes([10, 0, 0, 0]), bytes([6, 1]), bytes([6, 2]), bytes([6, 3]),
                 bytes([6, 4]), bytes([0, 0, 0, 0]), bytes([3, 0, 0, 0]),
                 bytes([7, 0, 0, 0]), bytes([8, 0, 0, 0]), bytes([11, 0, 0, 0])]


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


failed = False


def check(label, got, want):
    global failed
    ok = got == want
    failed |= not ok
    print(("PASS " if ok else "FAIL ") + f"{label}: {got!r}" + ("" if ok else f" (want {want!r})"))


# Login: the reply names the browser host, then the URL prefix and session.
sock = socket.create_connection((HOST, 21234))
sock.sendall(b"\x01")
time.sleep(0.1)
email = f"hosts-{HOST}@e2e.invalid"
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
check("login names the browser host", reply[2:2 + reply[1]].decode(), HOST)
session = struct.unpack("<I", reply[-4:])[0]

# Browser: the lobby address.
url = f"http://{HOST}:21280/hello?unused=1&type=2&local_ip=1&login_ip=1"
with urllib.request.urlopen(url, timeout=5) as answer:
    lobby_addr = answer.read().decode()
check("browser names the lobby", lobby_addr, f"{HOST}:21235")

# Lobby: sign in, the stock client's queries, ready up; 0x33 names the match.
lobby = socket.create_connection((HOST, 21235))
send(lobby, 0x26, struct.pack("<I", session))
assert receive(lobby)[:1] == b"\x30"
for query in LOBBY_QUERIES:
    send(lobby, 0x21, query)
    receive(lobby)
send(lobby, 0x21, bytes([1, 0, 0, 0]))
profiles = receive(lobby)
profile_id = struct.unpack("<I", profiles[3:7])[0]
send(lobby, 0x20, struct.pack("<I", profile_id))
answer = receive(lobby)
while answer[:1] != b"\x33":
    answer = receive(lobby)
check("lobby names the match server", answer[2:2 + answer[1]].decode(), HOST)
port = struct.unpack("<H", answer[2 + answer[1]:4 + answer[1]])[0]
check("at the match port", port, 21236)
lobby.close()

print("FAILED" if failed else "ALL PASSED")
sys.exit(1 if failed else 0)
