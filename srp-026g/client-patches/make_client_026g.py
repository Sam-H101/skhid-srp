"""Makes what a player needs to play Survarium 0.26g on an srp server, as a
small overlay folder. The ORIGINAL game folder is only read; nothing in it
is changed, and nothing is run.

USAGE

    cd srp-026g/client-patches
    python make_client_026g.py <game folder> <output folder> --server=<host>:<port>
        [--localization=ukrainian]   english, russian, ukrainian, german, polish
        [--cert-only | --cave=hash-mismatch]   another build of the exe (see patch_026g.py)
        [--check]                    every check, nothing written

    <game folder>   the original 0.26g game: the folder that holds
                    binaries/x86/survarium.exe (the installer's `game`
                    folder), or its parent. The exe must be the original
                    (patch_026g.py checks its sha256).
    <output folder> a new or empty folder OUTSIDE the game folder.
    --server        where the srp login server is reached: the host players
                    use and SRP_LOGIN_PORT (1234 by default; 2234 with the
                    0.26g docker-compose.yml beside the 0.21d server).

WHAT IT WRITES (the overlay mirrors the game's layout)

    <output>/game/binaries/x86/survarium_skhid.exe
        the patched client (patch_026g.py: the certificate check, and in
        the full builds the single-instance check, the 50 ms hash interval,
        the battery rule and the snapshot upload). The original
        survarium.exe is not in the
        overlay and stays as it is.
    <output>/game/binaries/x86/play_skhid.bat
        starts it from that folder (the client finds its certificate by a
        path relative to the working directory) with the launch line:
        survarium_skhid.exe -localization=<..>
            -server=<host>:<port> -mt_allocator=768
            -minimum_resources_memory_size=768
        (-mt_allocator: the global multithreaded arena, 376 MiB by
        default, which map loading ran out of in upstream's live runs;
        -minimum_resources_memory_size: the resources arenas' floor.
        Two different arenas: docs/notes/README.md.)
    <output>/game/resources/ssl/survarium_login_server.crt
        srp's login certificate (beside this script; the same file as in ../certs): the client accepts the
        login server only with this very file.
    <output>/README.txt
        what to do with it, and where the original game is had.

A player copies the overlay's `game` folder over the game's own `game`
folder (two new files, one replaced) and starts play_skhid.bat. Zip the
overlay to make the download the web page offers (SRP_DOWNLOAD_FILE); the
game itself is had from SRP_GAME_URL.
"""

import hashlib
import os
import re
import sys

import patch_026g as P

HERE = os.path.dirname(os.path.abspath(__file__))
# Where the original game is had: the web archive's copy of build 2777.
GAME_URL = (
    "https://archive.org/download/"
    "vostok_engine_v0.26g0_build_2777_internal_id_1104_jan_14_2015/"
    "survarium_full_026g0.7z"
)
SRP_CERT = os.path.join(HERE, "survarium_login_server.crt")
EXE_DIR = os.path.join("binaries", "x86")
CERT_REL = os.path.join("resources", "ssl", "survarium_login_server.crt")
# The localizations the client knows (login_chat.md: the language byte).
LOCALIZATIONS = ("english", "russian", "ukrainian", "german", "polish")


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def game_root(folder):
    """The folder that holds binaries/x86/survarium.exe: `folder` or its `game`."""
    for root in (folder, os.path.join(folder, "game")):
        if os.path.isfile(os.path.join(root, EXE_DIR, "survarium.exe")):
            return os.path.abspath(root)
    sys.exit(f"{folder}: no {os.path.join(EXE_DIR, 'survarium.exe')} in it or in its `game` folder")


def inside(path, folder):
    path, folder = (os.path.normcase(os.path.realpath(p)) for p in (path, folder))
    return path == folder or path.startswith(folder + os.sep)


def parse_server(text):
    match = re.fullmatch(r"([A-Za-z0-9](?:[A-Za-z0-9.-]{0,251}[A-Za-z0-9])?):(\d{1,5})", text)
    if not match or not 1 <= int(match.group(2)) <= 65535:
        sys.exit(f"--server={text}: it must be <host>:<port>, e.g. 203.0.113.9:1234")
    return f"{match.group(1)}:{int(match.group(2))}"


def launch_line(server, localization):
    return (f"survarium_skhid.exe -localization={localization} "
            f"-server={server} -mt_allocator=768 -minimum_resources_memory_size=768")


def bat(server, localization):
    lines = [
        "@echo off",
        "rem Put the server address (host:1234) here, as given on the server page.",
        f"set SERVER={server}",
        "rem Game language: english, russian, ukrainian, german or polish.",
        f"set LANGUAGE={localization}",
        "rem It must run from this folder (game\\binaries\\x86): the game finds",
        "rem ..\\..\\resources\\ssl\\survarium_login_server.crt from where it is started.",
        'cd /d "%~dp0"',
        'start "" survarium_skhid.exe -localization=%LANGUAGE% -server=%SERVER% '
        "-mt_allocator=768 -minimum_resources_memory_size=768",
        "",
    ]
    return "\r\n".join(lines).encode("ascii")


def readme(server, localization, description, files):
    lines = [
        "SURVARIUM 0.26g - SKHID COMMUNITY SERVER FILES (UNOFFICIAL)",
        "",
        "Not affiliated with or endorsed by Vostok Games. Survarium is a trademark",
        "of its owners. The server is built on the open source SRP project:",
        "https://github.com/srp-survarium/srp",
        "",
        "This archive holds only the few files that go over the original game; the",
        "game itself is not in it.",
        "",
        "This is an early test server. It has bugs and may not work fully. Your",
        "progress, items and accounts can be lost or reset at any time, and the",
        "server may be stopped at any time. By playing you accept this.",
        "",
        "HOW TO PLAY",
        "1. Get the original Survarium 0.26g (build 2777), a .7z archive of about",
        "   2.9 GB (7-Zip opens it), and unpack it:",
        f"   {GAME_URL}",
        "   The game is in its Survarium\\game folder.",
        "2. Copy the `game` folder of this archive over the game's own `game`",
        "   folder. It adds survarium_skhid.exe and play_skhid.bat to",
        "   game\\binaries\\x86 and replaces",
        "   game\\resources\\ssl\\survarium_login_server.crt (keep a copy if you",
        "   want the original back). The game's own survarium.exe is not changed.",
        "3. Double-click game\\binaries\\x86\\play_skhid.bat. It connects to the",
        "   Skhid server. Do not run survarium_skhid.exe by hand without the",
        "   options.",
        "4. Sign in with any user name and password. A new name is registered on its",
        "   first sign-in; remember the password.",
        "",
        "To change the language, open play_skhid.bat in a text editor and set",
        "LANGUAGE.",
        "",
        "OPTIONS (in play_skhid.bat)",
        "  -server=         server to sign in to (host:port)",
        "  -localization=   language: english, russian, ukrainian, german or polish",
        "  -mt_allocator=768, -minimum_resources_memory_size=768",
        "                   memory for the game, MB (maps need more than the default)",
        "",
        "The rules of the server are on its page:",
        f"https://{server.split(':')[0]}",
        "",
        "survarium_skhid.exe is the game's survarium.exe patched to work with this",
        "server. The launcher and the updater of the original game are not needed.",
        "",
    ]
    return "\r\n".join(lines).encode("utf-8")


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    options = [a for a in sys.argv[1:] if a.startswith("--")]
    mode, check, server, localization = "section", False, None, "ukrainian"
    for option in options:
        if option == "--check":
            check = True
        elif option == "--cert-only":
            mode = "cert-only"
        elif option.startswith("--cave=") and option[7:] in P.CAVES:
            mode = option[7:]
        elif option.startswith("--server="):
            server = parse_server(option[9:])
        elif option.startswith("--localization=") and option[15:] in LOCALIZATIONS:
            localization = option[15:]
        else:
            sys.exit(f"unknown option {option}\n\n{__doc__}")
    if len(args) != 2 or server is None:
        sys.exit(__doc__)
    source, output = game_root(args[0]), os.path.abspath(args[1])
    # Never into the game: not its folder, nor the install that holds it.
    if inside(output, source) or inside(output, os.path.dirname(source)):
        sys.exit(f"{output}: the output folder must be outside the game folder {os.path.dirname(source)}")
    if not check and os.path.isdir(output) and os.listdir(output):
        sys.exit(f"{output}: the output folder is not empty")

    # The exe: patch_026g.py refuses anything but the original.
    original = open(os.path.join(source, EXE_DIR, "survarium.exe"), "rb").read()
    code = None if mode == "cert-only" else open(P.default_bin(mode), "rb").read()
    exe, description = P.build(original, mode, code)
    exe = bytes(exe)

    # The certificates.
    if not os.path.isfile(SRP_CERT):
        sys.exit(f"{SRP_CERT}: srp's certificate is missing")
    srp_cert = open(SRP_CERT, "rb").read()
    if b"BEGIN CERTIFICATE" not in srp_cert:
        sys.exit(f"{SRP_CERT}: not a PEM certificate")
    game_cert_path = os.path.join(source, CERT_REL)
    if not os.path.isfile(game_cert_path):
        sys.exit(f"{game_cert_path}: the game folder has no login certificate")

    files = [
        (os.path.join("game", EXE_DIR, "survarium_skhid.exe"), exe),
        (os.path.join("game", EXE_DIR, "play_skhid.bat"), bat(server, localization)),
        (os.path.join("game", CERT_REL), srp_cert),
    ]
    listed = [(name.replace(os.sep, "/"), sha256(data)) for name, data in files]
    files.append(("README.txt", readme(server, localization, description, listed)))

    for name, data in files:
        target = os.path.join(output, name)
        if not check:
            os.makedirs(os.path.dirname(target), exist_ok=True)
            with open(target, "xb") as out:   # never over a file that is there
                out.write(data)
        print(f"{'would write' if check else 'wrote'} {target} ({len(data)} bytes, sha256 {sha256(data)})")
    print(f"exe: {description}")
    print(f"launch line: {launch_line(server, localization)}")
    if check:
        print("--check: nothing was written")


if __name__ == "__main__":
    main()
