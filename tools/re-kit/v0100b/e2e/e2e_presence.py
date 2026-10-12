"""Steps 1-5 end to end against an srp started from scratchpad/e2e_run with
SRP_LOGIN_PORT=21234 SRP_BROWSER_PORT=21280 SRP_LOBBY_PORT=21235
SRP_MATCH_PORT=21236 SRP_LOGIN_UDP_PORT=35100 RUST_LOG=trace.

Signs in two accounts like the stock client (login TLS with the game's
certificate, browser, lobby, chat), pings presence, makes friends, signs one
out, and checks the refusals. Passwords are made up."""
import socket, ssl, struct, sys, time, urllib.request

LOGIN = ("127.0.0.1", 21234)
BROWSER = ("127.0.0.1", 21280)
PINGS = ("127.0.0.1", 35100)
LOGIN_CERT = r"E:\sur-res-pro\Survarium\resources\ssl\survarium_login_server.crt"


def tls_context():
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    context.check_hostname = False
    context.verify_mode = ssl.CERT_REQUIRED
    context.set_ciphers("DEFAULT:@SECLEVEL=0")
    # The 2013 client speaks TLS 1.2 at most; with 1.3 a client that closes
    # right after writing can reset the server's unfinished handshake.
    context.maximum_version = ssl.TLSVersion.TLSv1_2
    context.load_verify_locations(LOGIN_CERT)
    return context


def recv_exact(sock, n):
    data = b""
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise EOFError
        data += chunk
    return data


def send_frame(sock, body):
    sock.sendall(bytes([len(body)]) + body)


def frame(sock):
    """The next frame, or None once the server closed the connection."""
    try:
        n = sock.recv(1)
    except ConnectionResetError:
        return None
    if not n:
        return None
    n = n[0]
    if n == 0:
        n = struct.unpack("<H", recv_exact(sock, 2))[0]
    return recv_exact(sock, n)


def chat_frame(sock):
    """The next chat frame that isn't an online count."""
    while True:
        f = frame(sock)
        if f is None or not f.startswith(b"\xC9\x08"):
            return f


def sign_in(email):
    sock = socket.create_connection(LOGIN)
    sock.sendall(b"\x01")
    time.sleep(0.1)
    sock.sendall(bytes([len(email)]) + email.encode() + b"0.100b")
    assert sock.recv(1) == b"\x0b"
    tls = tls_context().wrap_socket(sock)
    tls.sendall(b"\x05dummy")
    reply = tls.recv(256)
    tls.close()
    return struct.unpack("<I", reply[-4:])[0]


def sign_out(session):
    sock = socket.create_connection(LOGIN)
    sock.sendall(b"\x02" + struct.pack("<I", session))
    tls = tls_context().wrap_socket(sock)
    tls.sendall(b"\x05dummy")
    tls.close()


def lobby_address():
    url = (f"http://{BROWSER[0]}:{BROWSER[1]}/hello?unused=1&type=2"
           f"&local_ip=127.0.0.1&login_ip={LOGIN[0]}")
    with urllib.request.urlopen(url, timeout=5) as answer:
        host, port = answer.read().decode().rsplit(":", 1)
    return host, int(port)


def lobby(session):
    sock = socket.create_connection(LOBBY)
    sock.settimeout(3)
    send_frame(sock, b"\x26" + struct.pack("<I", session))
    return sock, frame(sock)


def chat(session):
    sock = socket.create_connection(LOBBY)
    sock.settimeout(3)
    send_frame(sock, b"\xC3" + struct.pack("<I", session) + b"\x05")
    return sock, chat_frame(sock)


def check(label, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + label + (f": {detail}" if detail and not ok else ""))
    if not ok:
        global failed
        failed = True


failed = False
LOBBY = lobby_address()
print("lobby at", LOBBY)

alice = sign_in("alice@e2e.invalid")
bob = sign_in("bob@e2e.invalid")
print(f"sessions alice={alice:#x} bob={bob:#x}")

pinger = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
for _ in range(3):
    pinger.sendto(struct.pack("<I", alice), PINGS)
    pinger.sendto(struct.pack("<I", 0xBAD), PINGS)
    time.sleep(0.2)

a_lobby, answer = lobby(alice)
check("alice's lobby sign-in succeeds", answer == b"\x30", answer)
a_chat, answer = chat(alice)
check("alice's chat sign-in names her", answer == b"\xCB\x05alice", answer)
b_chat, answer = chat(bob)
check("bob's chat sign-in names him", answer == b"\xCB\x03bob", answer)

# Search, then add bob as a friend, as the client does.
send_frame(a_chat, b"\xC4\x04\x03BOB")
found = chat_frame(a_chat)
check("a search finds bob", found[:4] == b"\xCC\x04\x01\x00" and found.endswith(b"\x03bob"), found)
bob_id = struct.unpack("<I", found[4:8])[0]
send_frame(a_chat, b"\xC4\x00" + struct.pack("<I", bob_id))
check("adding bob is accepted", chat_frame(a_chat) == b"\xCC\x00\x34")
send_frame(a_chat, b"\xC4\x05")
check("the friend list shows bob online",
      chat_frame(a_chat) == b"\xCC\x05\x01\x00" + struct.pack("<I", bob_id) + b"\x03bob\x01")

# Bob signs out (as the client does when it quits): his open chat stays,
# new connections are refused.
sign_out(bob)
time.sleep(0.3)
b_lobby, answer = lobby(bob)
check("bob's lobby is refused after sign-out", answer == b"\x31", answer)
check("then closed", frame(b_lobby) is None)
b_chat2, answer = chat(bob)
check("bob's new chat is closed after sign-out", answer is None, answer)
b_chat.close()
time.sleep(0.3)
send_frame(a_chat, b"\xC4\x07")
check("bob shows offline once his chat closed",
      chat_frame(a_chat) == b"\xCC\x07\x01\x00" + struct.pack("<I", bob_id) + b"\x00")

# A made-up session is refused.
x_lobby, answer = lobby(0xBAD)
check("an unknown session's lobby is refused", answer == b"\x31", answer)
check("then closed", frame(x_lobby) is None)

# Bob signs in again: same account, his friend lists' owner still knows him.
bob2 = sign_in("bob@e2e.invalid")
b_chat3, answer = chat(bob2)
check("bob signs in again", answer == b"\xCB\x03bob", answer)
time.sleep(0.2)
send_frame(a_chat, b"\xC4\x07")
check("bob shows online again",
      chat_frame(a_chat) == b"\xCC\x07\x01\x00" + struct.pack("<I", bob_id) + b"\x01")

print("FAILED" if failed else "ALL PASSED")
sys.exit(1 if failed else 0)
