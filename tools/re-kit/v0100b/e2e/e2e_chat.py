"""A scripted second player for the live chat test against a running srp.

Signs in like the stock client (verifying the login certificate against the
game's resources/ssl copy), joins the chat on the lobby port, checks the
replies to the client's opening queries, says one general line, then prints
every chat line it receives for LISTEN seconds (type in the game's chat).

usage: py -I e2e_chat.py <game resources/ssl/survarium_login_server.crt> [LISTEN]
"""
import socket, ssl, sys, time

cafile = sys.argv[1]
listen = float(sys.argv[2]) if len(sys.argv) > 2 else 60
EMAIL = b"chatbot@test.invalid"


def frame(payload):
    return (bytes([len(payload)]) if len(payload) < 256
            else b"\x00" + len(payload).to_bytes(2, "little")) + payload


def read_frame(sock):
    def exact(n):
        data = b""
        while len(data) < n:
            chunk = sock.recv(n - len(data))
            if not chunk:
                raise ConnectionError("closed")
            data += chunk
        return data
    n = exact(1)[0]
    if n == 0:
        n = int.from_bytes(exact(2), "little")
    return exact(n)


def string(data, at):
    n = data[at]
    return data[at + 1:at + 1 + n], at + 1 + n


# 1. Sign in, verifying the certificate on the first attempt.
sock = socket.create_connection(("127.0.0.1", 1234))
sock.sendall(b"\x01")
time.sleep(0.1)
sock.sendall(bytes([len(EMAIL)]) + EMAIL + b"0.100b")
assert sock.recv(1) == b"\x0b", "no valid_user_name"
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_REQUIRED
ctx.set_ciphers("DEFAULT:@SECLEVEL=0")
ctx.load_verify_locations(cafile)
tls = ctx.wrap_socket(sock)
tls.sendall(b"\x05dummy")
reply = tls.recv(256)
session = int.from_bytes(reply[-4:], "little")
print(f"login: certificate verified, session {session:#x}", flush=True)
tls.close()

# 2. Join the chat as the client does.
chat = socket.create_connection(("127.0.0.1", 1235))
chat.sendall(frame(b"\xC3" + session.to_bytes(4, "little") + b"\x05"))
signed_in = read_frame(chat)
assert signed_in[0] == 0xCB, signed_in
print("chat: signed in as", string(signed_in, 1)[0].decode(errors="replace"), flush=True)
subscriptions = b"".join(c.to_bytes(4, "little") for c in [0, 0xFFFFFFFF, 0xFFFFFFFF, 0, 0, 0, 0, 0, 0])
chat.sendall(frame(b"\xC5" + subscriptions) + frame(b"\xC4\x05") + frame(b"\xC4\x06"))
for expected in (5, 6):
    answer = read_frame(chat)
    assert answer == bytes([0xCC, expected, 0, 0]), answer
print("chat: friend and ignore lists answered (empty)", flush=True)

# 3. Say one general line, then show what arrives.
body = b"chat relay test line"
chat.sendall(frame(b"\xC1" + (0).to_bytes(4, "little") + b"\x00" + b"\x01" + bytes([len(body)]) + body))
print(f"chat: said {body.decode()!r} on general; listening {listen:.0f}s", flush=True)
chat.settimeout(1)
deadline = time.monotonic() + listen
while time.monotonic() < deadline:
    try:
        data = read_frame(chat)
    except socket.timeout:
        continue
    if data[0] == 0xC9:
        sender_type, account = data[1], int.from_bytes(data[2:6], "little")
        name, at = string(data, 6)
        channel = data[at]
        text, _ = string(data, at + 1)
        print(f"chat: received on channel {channel} from {name.decode(errors='replace')} "
              f"(type {sender_type}, account {account:#x}): {text.decode('cp1251', errors='replace')}",
              flush=True)
        # Answer privately, so the sender sees a line come back to them.
        echo = (b"echo: " + text)[:200]
        chat.sendall(frame(b"\xC1" + (0).to_bytes(4, "little") + bytes([len(name)]) + name
                           + b"\x04" + bytes([len(echo)]) + echo))
        print(f"chat: echoed privately to {name.decode(errors='replace')}", flush=True)
    else:
        print("chat: other message", data.hex(), flush=True)
chat.close()
print("done", flush=True)
