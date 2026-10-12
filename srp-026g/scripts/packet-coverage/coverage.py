"""What the clients really sent in a test session, from srp's packet log.

USAGE

    python coverage.py <packet log folder> [<srp logs folder>]

    <packet log folder>  what SRP_PACKET_LOG was set to (one file a
                         connection, written by crates/udp-transport
                         packet_log.rs).
    <srp logs folder>    optional: srp's logs/ folder; its lines about
                         messages the server could not read are counted.

It prints, for the lobby and the match server apart, every message id seen
in each direction with its count, and for a lobby request also its first
payload byte (the sub-operation of the requests that have one). Then the
datagrams that did not parse. Compare the "IN" lists with
docs/notes/coverage.md: an id that is not in that note is a
message nobody knew the client sends.

Only reads files. A redacted payload (`**`) is counted without its bytes.
The lobby sign-in (2E) is not in a connection's file: the file starts once
the server knows whose connection it is.
"""

import collections
import os
import re
import sys

RAW = re.compile(r"^\S+ (IN |OUT) \S+ len=\d+((?: [0-9A-F*]{2})*)\s*$")
MSG = re.compile(
    r"^    msg id=([0-9A-F]{2})(?: x=\d+)?(?: part=(\d+)(?:/\d+)?)?"
    r"(?: (low-level))?(?: len=(\d+) at=(\d+))?(?: (redacted))?\s*$"
)


def read_file(path, seen, unparsed):
    direction, data = None, []
    with open(path, encoding="utf-8", errors="replace") as lines:
        for line in lines:
            raw = RAW.match(line)
            if raw:
                direction = raw.group(1).strip()
                data = raw.group(2).split()
                continue
            if line.startswith("    not parsed:"):
                unparsed[(direction, line.strip())] += 1
                continue
            msg = MSG.match(line)
            if not msg or direction is None:
                continue
            ident, part, low, length, at, redacted = msg.groups()
            # A split message is counted once, by its first part.
            if part not in (None, "0"):
                continue
            first = ""
            if low:
                first = "low-level"
            elif length and int(length) > 0 and not redacted:
                byte = data[int(at)] if int(at) < len(data) else "??"
                first = byte
            seen[(direction, ident, first)] += 1


def report(kind, seen, unparsed, by_first_byte):
    print(f"== {kind} ==")
    if not seen and not unparsed:
        print("  nothing logged\n")
        return
    for direction in ("IN", "OUT"):
        title = "client -> server" if direction == "IN" else "server -> client"
        rows = collections.Counter()
        for (way, ident, first), count in seen.items():
            if way != direction:
                continue
            detailed = by_first_byte and direction == "IN" and first != "low-level"
            key = (ident, first if detailed or first == "low-level" else "")
            rows[key] += count
        print(f"  {title}: {len(rows)} kinds, {sum(rows.values())} messages")
        for (ident, first), count in sorted(rows.items()):
            label = f"{ident} {first}".rstrip()
            print(f"    {label:<16}{count:>8}")
    if unparsed:
        print("  datagrams that did not parse:")
        for (direction, reason), count in sorted(unparsed.items()):
            print(f"    {direction or '?':<4}{count:>6}  {reason}")
    print()


def server_complaints(folder):
    words = ("was not read", "was not sent", "not parsed", "unknown message", "Unknown")
    found = collections.Counter()
    for name in sorted(os.listdir(folder)):
        if not name.endswith(".log"):
            continue
        with open(os.path.join(folder, name), encoding="utf-8", errors="replace") as lines:
            for line in lines:
                if any(word in line for word in words):
                    # Drop the time and the numbers, so repeats add up.
                    text = re.sub(r"^\S+\s+", "", line.strip())
                    text = re.sub(r"\d+\.\d+\.\d+\.\d+:\d+", "<peer>", text)
                    found[(name, text[:160])] += 1
    print("== server log lines about messages it could not use ==")
    if not found:
        print("  none\n")
        return
    for (name, text), count in found.most_common(60):
        print(f"  {count:>5}  {name}: {text}")
    print()


def main():
    if len(sys.argv) < 2 or not os.path.isdir(sys.argv[1]):
        print(__doc__)
        return 2
    folder = sys.argv[1]
    kinds = collections.defaultdict(
        lambda: (collections.Counter(), collections.Counter())
    )
    files = 0
    for name in sorted(os.listdir(folder)):
        if not name.endswith(".log"):
            continue
        kind = name.split("-", 1)[0]
        seen, unparsed = kinds[kind]
        read_file(os.path.join(folder, name), seen, unparsed)
        files += 1
    print(f"{files} connection files in {folder}\n")
    for kind in sorted(kinds):
        seen, unparsed = kinds[kind]
        report(kind, seen, unparsed, by_first_byte=(kind == "lobby"))
    if len(sys.argv) > 2 and os.path.isdir(sys.argv[2]):
        server_complaints(sys.argv[2])
    return 0


if __name__ == "__main__":
    sys.exit(main())
