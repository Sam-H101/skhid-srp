"""End-to-end check of the srp match lifecycle and lobby discard, speaking the
stock client's wire protocol to a running srp on localhost.

Match: connect -> get_startup_info -> team_bases_initialize_info -> join_match,
then log every lifecycle message with its arrival time until match_finished,
then disconnect like the client does. Lobby: sign in, send
discard_playing_order, close.
"""

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
LOBBY = ("127.0.0.1", 1235)
SESSION = 0xDD00

NAMES = {
    0x80: "connection_successful", 0x81: "match_options", 0x82: "server_player_input",
    0x83: "kill_player", 0x84: "spawn_player", 0x86: "match_time_changed",
    0x87: "respawn_time_changed", 0x88: "player_kd_stats_changed", 0x8B: "sync_response",
    0x8C: "match_finished", 0x91: "player_visibility_changed", 0x92: "player_profile",
    0x9A: "game_status_changed", 0x9B: "match_wait_time_changed",
}
STATUS = {0: "inactive", 1: "waiting_for_first_player", 2: "waiting_for_players",
          3: "final_countdown", 4: "inprocess"}

start = time.monotonic()
events = []
finished = threading.Event()
state = {"server_seq": 0xFFFF, "seq": 0, "order": 0}


def stamp():
    return f"{time.monotonic() - start:7.2f}s"


def describe(kind, payload):
    if kind == 0x9A:
        return f"status={STATUS.get(struct.unpack('<I', payload)[0])}"
    if kind in (0x9B, 0x87):
        return f"seconds={struct.unpack('<I', payload)[0]}"
    if kind == 0x86:
        return f"ms={struct.unpack('<I', payload)[0]}"
    if kind == 0x88:
        player, kills, deaths = struct.unpack("<BII", payload)
        return f"player={player} kills={kills} deaths={deaths}"
    if kind == 0x81:
        name_len = payload[1]
        name = payload[2:2 + name_len].decode()
        mode, players, items, respawn = payload[2 + name_len:6 + name_len]
        match_time = struct.unpack("<H", payload[6 + name_len:8 + name_len])[0]
        return (f"map={name} mode={mode} players={players} batteries={items} "
                f"respawn_time={respawn}s match_time={match_time}s")
    return f"{len(payload)} bytes"


def receive(sock):
    while not finished.is_set():
        try:
            data, _ = sock.recvfrom(4096)
        except socket.timeout:
            continue
        except OSError:
            continue
        server_seq, _, word = struct.unpack("<HHH", data[:6])
        state["server_seq"] = server_seq
        if word & 1:
            continue  # low-level keepalive / disconnect confirmation
        kind = data[6]
        payload = data[9:]
        if kind in (0x82,):
            continue
        line = f"{stamp()} {NAMES.get(kind, hex(kind))} {describe(kind, payload)}"
        events.append(line)
        print(line, flush=True)
        if kind == 0x8C:
            finished.set()


def send(sock, kind, payload=b""):
    header = struct.pack("<HHH", state["seq"], state["server_seq"], 0)
    message = struct.pack("<BH", kind, state["order"]) + payload
    sock.sendto(header + message, MATCH)
    state["seq"] += 1
    state["order"] += 1


def send_low(sock, low_type):
    header = struct.pack("<HHH", state["seq"], state["server_seq"], 1)
    sock.sendto(header + bytes([1, low_type]), MATCH)
    state["seq"] += 1


def match_check(timeout):
    sock = LinkSocket()
    sock.bind(("127.0.0.1", 0))
    sock.settimeout(0.2)
    threading.Thread(target=receive, args=(sock,), daemon=True).start()

    send(sock, 0x40, struct.pack("<I", SESSION))
    time.sleep(0.3)
    send(sock, 0x41)  # get_startup_info
    time.sleep(0.3)
    send(sock, 0x48)  # team_bases_initialize_info
    send(sock, 0x42)  # join_match

    # Keep the connection alive like the client's ~33 ms input cadence would.
    deadline = time.monotonic() + timeout
    while not finished.is_set() and time.monotonic() < deadline:
        send_low(sock, 2)  # continuous_flow
        time.sleep(1.0)

    ok = finished.is_set()
    print(f"{stamp()} match_finished received: {ok}", flush=True)
    send_low(sock, 0)  # initiate_disconnection, as close_current_match does
    time.sleep(0.5)
    sock.close()
    return ok


def lobby_check():
    with socket.create_connection(LOBBY, timeout=3) as tcp:
        tcp.sendall(bytes([5, 0x26]) + struct.pack("<I", SESSION))
        reply = tcp.recv(64)
        print(f"{stamp()} lobby sign-in reply: {reply.hex()}", flush=True)
        tcp.sendall(bytes([5, 0x27]) + struct.pack("<I", 0xFFFFFFFF))
        time.sleep(0.5)
    print(f"{stamp()} lobby connection closed by the client", flush=True)


if __name__ == "__main__":
    timeout = float(sys.argv[1]) if len(sys.argv) > 1 else 100
    ok = match_check(timeout)
    lobby_check()
    sys.exit(0 if ok else 1)
