"""End-to-end tests of updater_skhid.exe on Windows: the real program, a
game folder made for each test, a web server on 127.0.0.1 in the place of
the srp one, and this script in the place of the launcher (it makes the
launcher's named pipe, starts the updater with the launcher's command line
and reads what the updater says).

    cd client/updater
    ./build.sh                    (the updater and tests/fake_exe)
    python tests/test_updater.py [-v] [TestName ...]

Nothing outside the temporary folder is touched; no game file is needed.
SKHID_TEST_BUNDLE=<zip> also installs a real players' archive and compares
the result with Python's own unpacking of it.
"""

import _winapi
import ctypes
import hashlib
import http.server
import io
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
# The release builder makes the resource plans here, as for the players'
# archive, so its plan is read by the real updater.
sys.path.insert(0, os.path.join(HERE, "..", "..", "..", "srp-026g", "client-patches"))
import make_client_026g as builder  # noqa: E402
from test_release_overlay import forged  # noqa: E402

RELEASE = os.path.join("target", "i686-pc-windows-msvc", "release")
UPDATER = os.path.join(HERE, "..", "exe", RELEASE, "updater_skhid.exe")
FAKE_EXE = os.path.join(HERE, "fake_exe", RELEASE, "fake-exe.exe")
X86 = os.path.join("game", "binaries", "x86")

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
kernel32.CreateMutexW.restype = ctypes.c_void_p
kernel32.CloseHandle.argtypes = [ctypes.c_void_p]

# The launcher's messages and the updater's states (client/updater/logic/src/pipe.rs).
ABORT = b"\x1b"
CHECKING, CHECK_FAILED, DOWNLOADING, DOWNLOAD_FAILED, DOWNLOADED = 0x04, 0x05, 0x09, 0x0A, 0x0C
LATEST, ABORTED, INSTALLING, COPIED, ERROR = 0x0E, 0x0F, 0x10, 0x13, 0x01


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def make_zip(files):
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, data in files.items():
            # A fixed time: the same files are the same archive, byte for byte.
            entry = zipfile.ZipInfo(name, date_time=(2026, 10, 13, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(entry, data)
    return out.getvalue()


class Site:
    """The web server: `/api/client-files` and `/download`, as told."""

    def __init__(self):
        self.archive = b""
        self.manifest = None            # bytes to answer instead of the true manifest
        self.manifest_status = 200
        self.download_statuses = []     # statuses for the next downloads, then 200
        self.cut_downloads = 0          # the next N downloads end half way
        self.slow = 0.0                 # seconds between pieces of a download
        self.blobs = {}                 # /resources/<sha256>: sha256 -> bytes
        self.blob_statuses = {}         # sha256 -> another status than 200
        self.blob_bodies = {}           # sha256 -> other bytes than the blob's
        self.requests = []
        site = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def answer(self, status, body, content_type="text/plain"):
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                site.requests.append(self.path)
                if self.path == "/api/client-files":
                    body = site.manifest if site.manifest is not None else site.true_manifest()
                    self.answer(site.manifest_status, body if site.manifest_status == 200 else b"no\n")
                elif self.path == "/download":
                    status = site.download_statuses.pop(0) if site.download_statuses else 200
                    if status != 200:
                        self.answer(status, b"")
                        return
                    body = site.archive
                    self.send_response(200)
                    self.send_header("Content-Type", "application/zip")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    if site.cut_downloads:
                        site.cut_downloads -= 1
                        body = body[:len(body) // 2]
                    try:
                        for at in range(0, len(body), 4096):
                            self.wfile.write(body[at:at + 4096])
                            self.wfile.flush()
                            if site.slow:
                                time.sleep(site.slow)
                    except OSError:
                        pass
                elif self.path.startswith("/resources/"):
                    name = self.path[len("/resources/"):]
                    status = site.blob_statuses.get(name, 200 if name in site.blobs else 404)
                    if status != 200:
                        self.answer(status, b"")
                    else:
                        self.answer(200, site.blob_bodies.get(name, site.blobs[name]), "application/octet-stream")
                else:
                    self.answer(404, b"not found\n")

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def true_manifest(self):
        return f"skhid-client-files 1\nsha256 {sha256(self.archive)}\nsize {len(self.archive)}\n".encode()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class Launcher:
    """The launcher's end: its pipe, the updater's process, what it said."""

    counter = 0

    def __init__(self, x86, action="update"):
        Launcher.counter += 1
        self.product = f"SkhidTest{os.getpid()}x{Launcher.counter}"
        name = rf"\\.\pipe\vostok_updater_pipe-{self.product}"
        # As pipe::accept of the launcher makes it: duplex, bytes, one instance.
        self.pipe = _winapi.CreateNamedPipe(
            name, _winapi.PIPE_ACCESS_DUPLEX | 0x00080000, 0,   # first instance; bytes, waiting
            1, 0x10000, 0x10000, 0, _winapi.NULL)
        exe = os.path.join(x86, "updater_skhid.exe")
        # run_survarium_executable_as_administrator: every argument in quotes
        # (the fourteen of start_updater; only the first three matter).
        arguments = [exe, action, name, "0.26g0", "torrent", "", "C:\\Start Menu", "C:\\Desktop",
                     "C:\\Documents", "", "a user", "C:\\Temp", "skhid.link", ""]
        self.process = subprocess.Popen(" ".join(f'"{argument}"' for argument in arguments), cwd=x86)
        try:
            _winapi.ConnectNamedPipe(self.pipe)
        except OSError as error:
            if error.winerror != 535:       # ERROR_PIPE_CONNECTED: it was faster
                raise
        self.buffer = b""
        self.frames = []
        self.open = True

    def poll(self):
        while self.open:
            try:
                waiting, _ = _winapi.PeekNamedPipe(self.pipe, 0)
                if not waiting:
                    break
                data, _ = _winapi.ReadFile(self.pipe, waiting)
                self.buffer += data
            except OSError:
                self.open = False
        while len(self.buffer) >= 2:
            (length,) = struct.unpack_from("<H", self.buffer)
            if len(self.buffer) < 2 + length:
                break
            self.frames.append(self.buffer[2:2 + length])
            self.buffer = self.buffer[2 + length:]

    def send(self, payload):
        _winapi.WriteFile(self.pipe, struct.pack("<H", len(payload)) + payload)

    def states(self):
        return [frame[1] for frame in self.frames if frame[0] == 0x02]

    def texts(self, kind=0x05):
        return [frame[2:2 + frame[1]].decode("ascii") for frame in self.frames if frame[0] == kind]

    def wait_for(self, done, seconds=60, what="the expected messages"):
        until = time.time() + seconds
        while time.time() < until:
            self.poll()
            if done(self):
                return
            if not self.open:
                break
            time.sleep(0.02)
        self.poll()
        if not done(self):
            raise AssertionError(f"no {what}; states {[hex(s) for s in self.states()]}, "
                                 f"texts {self.texts()}, pipe open {self.open}")

    def wait_for_state(self, state, seconds=60):
        self.wait_for(lambda launcher: state in launcher.states(), seconds, f"state {state:#x}")

    def finish(self, abort=True):
        """Closes as the launcher does: abort, then the pipe. Returns the
        updater's exit code."""
        if abort and self.open:
            try:
                self.send(ABORT)
            except OSError:
                pass
        deadline = time.time() + 20
        while self.process.poll() is None and time.time() < deadline:
            self.poll()
            time.sleep(0.02)
        self.poll()
        _winapi.CloseHandle(self.pipe)
        self.open = False
        return self.process.wait(20)


class UpdaterTest(unittest.TestCase):
    def setUp(self):
        for needed in (UPDATER, FAKE_EXE):
            if not os.path.isfile(needed):
                self.fail(f"{needed} is not built: run client/updater/build.sh")
        self.root = tempfile.mkdtemp(prefix="skhid-updater-test-")
        self.top = os.path.join(self.root, "Survarium")
        self.x86 = os.path.join(self.top, X86)
        os.makedirs(self.x86)
        shutil.copy(UPDATER, os.path.join(self.x86, "updater_skhid.exe"))
        # These scenarios exercise overlay updates over an existing game.
        self.write("game/resources.db", b"existing game resources")
        self.write(f"{X86}/survarium.exe", b"existing original game")
        self.site = Site()
        self.write(f"{X86}/updater_skhid.url", self.site.url.encode())
        self.launchers = []
        with open(UPDATER, "rb") as file:
            self.updater_bytes = file.read()
        with open(FAKE_EXE, "rb") as file:
            self.fake = file.read()

    def tearDown(self):
        for launcher in self.launchers:
            if launcher.process.poll() is None:
                launcher.process.kill()
                launcher.process.wait(10)
        self.site.close()
        shutil.rmtree(self.root, ignore_errors=True)

    def path(self, name):
        return os.path.join(self.top, *name.split("/"))

    def write(self, name, data):
        os.makedirs(os.path.dirname(self.path(name)), exist_ok=True)
        with open(self.path(name), "wb") as file:
            file.write(data)

    def read(self, name):
        with open(self.path(name), "rb") as file:
            return file.read()

    def exists(self, name):
        return os.path.exists(self.path(name))

    def launcher(self, action="update"):
        launcher = Launcher(self.x86, action)
        self.launchers.append(launcher)
        return launcher

    def bundle(self, **changes):
        """An archive like the players' one. The updater in it is the one
        that runs, so it is not replaced unless a test changes it."""
        files = {
            f"{X86}/survarium_skhid.exe": self.fake + b"game 1",
            f"{X86}/bugtrap.dll": b"MZ a dll " * 500,
            f"{X86}/play_skhid.bat": b"@echo off\r\nset LANGUAGE=english\r\n",
            f"{X86}/updater_skhid.exe": self.updater_bytes,
            f"{X86}/launcher_skhid/index-en.html": b"<html>offline</html>",
            "game/resources/ssl/survarium_login_server.crt": b"-----BEGIN CERTIFICATE-----\n",
            "README.txt": b"read me\r\n",
        }
        for name, data in changes.items():
            name = name.replace("__", "/")
            if data is None:
                files.pop(name)
            else:
                files[name] = data
        return files

    def offer(self, files):
        self.site.archive = make_zip(files)
        return files

    def leftovers(self):
        found = []
        for folder, _, names in os.walk(self.top):
            found += [name for name in names if ".skhid-new" in name or ".skhid-old" in name]
        return found

    def install(self, files):
        """A whole normal run that ends in "latest"."""
        self.offer(files)
        launcher = self.launcher()
        launcher.wait_for_state(LATEST)
        launcher.said = launcher.states()       # before it is told to stop
        self.assertEqual(launcher.finish(), 0)
        return launcher

    def log(self):
        return self.read(f"{X86}/updater_skhid.log").decode("utf-8", "replace")

    # --- the normal runs -------------------------------------------------

    def test_a_first_run_installs_everything_and_the_next_finds_it_up_to_date(self):
        files = self.offer(self.bundle())
        launcher = self.launcher()
        launcher.wait_for_state(LATEST)
        for name, data in files.items():
            self.assertEqual(self.read(name), data, name)
        # What the launcher was told, in order: checking, downloading with
        # progress, downloaded, installing with the file names, then
        # "checking" (which clears the file name line) and "latest".
        self.assertEqual(launcher.states(), [CHECKING, DOWNLOADING, DOWNLOADED, INSTALLING, CHECKING, LATEST])
        self.assertIn("survarium_skhid.exe", launcher.texts(0x07))
        self.assertNotIn("updater_skhid.exe", launcher.texts(0x07))   # already the same bytes
        progress = [struct.unpack("<4Q", frame[1:]) for frame in launcher.frames if frame[0] == 0x01]
        self.assertTrue(any(done == total == len(self.site.archive) for done, total, _, _ in progress))
        self.assertTrue(all(frame[0] in range(1, 24) for frame in launcher.frames), "a message type the launcher would crash on")
        # It stays until the launcher says stop, then says "aborted" and ends.
        time.sleep(0.5)
        self.assertIsNone(launcher.process.poll())
        self.assertEqual(launcher.finish(), 0)
        self.assertEqual(launcher.states()[-1], ABORTED)
        self.assertEqual(self.leftovers(), [])
        state = self.read(f"{X86}/updater_skhid.state").decode()
        self.assertIn(f"archive {sha256(self.site.archive)}", state)
        self.assertIn(f"file {sha256(files['README.txt'])} 9 README.txt", state)

        # The second start: one question, no download.
        self.site.requests.clear()
        launcher = self.launcher()
        launcher.wait_for_state(LATEST)
        self.assertEqual(launcher.states(), [CHECKING, LATEST])
        self.assertEqual(self.site.requests, ["/api/client-files"])
        self.assertEqual(launcher.finish(), 0)

    def test_a_new_archive_replaces_the_files_and_takes_away_those_it_no_longer_has(self):
        self.install(self.bundle())
        self.write(f"{X86}/play_skhid.bat", b"@echo off\r\nset LANGUAGE=ukrainian\r\n")
        self.write(f"{X86}/my_notes.txt", b"the player's own file")
        files = self.bundle(**{
            f"{X86}/survarium_skhid.exe".replace("/", "__"): self.fake + b"game 2",
            f"{X86}/bugtrap.dll".replace("/", "__"): None,
            "LINUX.txt": b"new in this archive",
        })
        launcher = self.install(files)
        self.assertEqual(launcher.said, [CHECKING, DOWNLOADING, DOWNLOADED, INSTALLING, CHECKING, LATEST])
        for name, data in files.items():
            self.assertEqual(self.read(name), data, name)
        # A new archive brings its own start script, the player's edit goes.
        self.assertIn(b"english", self.read(f"{X86}/play_skhid.bat"))
        # Gone from the archive, and still as it was installed: taken away.
        self.assertFalse(self.exists(f"{X86}/bugtrap.dll"))
        # Never installed by the updater: not its to touch.
        self.assertEqual(self.read(f"{X86}/my_notes.txt"), b"the player's own file")
        self.assertEqual(self.leftovers(), [])

    def test_a_damaged_program_is_repaired_and_an_edited_script_is_left_alone(self):
        files = self.bundle()
        self.install(files)
        edited = b"@echo off\r\nset LANGUAGE=ukrainian\r\n"
        self.write(f"{X86}/play_skhid.bat", edited)
        launcher = self.install(files)
        self.assertEqual(launcher.said, [CHECKING, LATEST], "an edited script is no reason to install")

        self.write(f"{X86}/survarium_skhid.exe", b"damaged")
        os.remove(self.path("game/resources/ssl/survarium_login_server.crt"))
        launcher = self.install(files)
        self.assertIn(DOWNLOADING, launcher.states())
        self.assertEqual(self.read(f"{X86}/survarium_skhid.exe"), files[f"{X86}/survarium_skhid.exe"])
        self.assertTrue(self.exists("game/resources/ssl/survarium_login_server.crt"))
        # A repair brings back what was damaged or lost, and nothing else:
        # the script stays the player's.
        self.assertEqual(sorted(launcher.texts(0x07)), ["survarium_login_server.crt", "survarium_skhid.exe"])
        self.assertEqual(self.read(f"{X86}/play_skhid.bat"), edited)
        self.assertEqual(self.install(files).said, [CHECKING, LATEST])

    def test_the_launcher_and_the_updater_are_replaced_while_they_run(self):
        self.install(self.bundle())
        new_updater = self.updater_bytes + b"\0new build"
        files = self.offer(self.bundle(**{
            f"{X86}/updater_skhid.exe".replace("/", "__"): new_updater,
            f"{X86}/launcher_skhid.exe".replace("/", "__"): self.fake,
        }))
        launcher = self.launcher()
        # The launcher holds its one-at-a-time mutex while it runs.
        mutex = kernel32.CreateMutexW(None, False, f"{launcher.product}_or_its_installer_is_already_running")
        self.assertTrue(mutex)
        try:
            launcher.wait_for_state(ABORTED)
            launcher.wait_for(lambda l: not l.open, 10, "closed pipe")
            self.assertEqual(launcher.states(), [CHECKING, DOWNLOADING, DOWNLOADED, INSTALLING, ABORTED])
            self.assertEqual(self.read(f"{X86}/updater_skhid.exe"), new_updater)
            self.assertEqual(self.read(f"{X86}/launcher_skhid.exe"), self.fake)
            # The new launcher is not started while the old one is there.
            started = f"{X86}/launcher_skhid.exe.started.txt"
            time.sleep(1.5)
            self.assertFalse(self.exists(started))
            self.assertIsNone(launcher.process.poll())
        finally:
            kernel32.CloseHandle(mutex)
        self.assertEqual(launcher.process.wait(20), 0)
        _winapi.CloseHandle(launcher.pipe)
        deadline = time.time() + 10
        while not self.exists(started) and time.time() < deadline:
            time.sleep(0.05)
        command, folder = self.read(started).decode("utf-16").strip().splitlines()
        self.assertEqual(command, f'"{self.x86}\\launcher_skhid.exe" -product_name={launcher.product}')
        self.assertEqual(os.path.normcase(folder), os.path.normcase(self.x86))
        # The file of the updater that ran is deleted at the next start.
        self.assertEqual(self.leftovers(), ["updater_skhid.exe.skhid-old"])
        files[f"{X86}/updater_skhid.exe"] = new_updater
        launcher = self.launcher()
        launcher.wait_for_state(LATEST)
        self.assertEqual(launcher.states(), [CHECKING, LATEST])
        self.assertEqual(launcher.finish(), 0)
        self.assertEqual(self.leftovers(), [])

    def test_the_real_players_archive_installs_as_python_unpacks_it(self):
        bundle = os.environ.get("SKHID_TEST_BUNDLE")
        if not bundle:
            self.skipTest("SKHID_TEST_BUNDLE is not set")
        with open(bundle, "rb") as file:
            self.site.archive = file.read()
        # By hand: with a launcher's pipe it would start the archive's real
        # launcher once that is installed.
        exe = os.path.join(self.x86, "updater_skhid.exe")
        self.assertEqual(subprocess.run([exe, "check", "quiet"], timeout=120).returncode, 0, self.log())
        with zipfile.ZipFile(bundle) as archive:
            for info in archive.infolist():
                # (The archive's own .url would send the next test run elsewhere.)
                if not info.is_dir():
                    self.assertEqual(self.read(info.filename), archive.read(info), info.filename)
        # The updater that ran was replaced by the archive's, if that is another build.
        self.assertLessEqual(set(self.leftovers()), {"updater_skhid.exe.skhid-old"})
        self.assertIn("\r\nend\r\n", self.read(f"{X86}/updater_skhid.state").decode())

    # --- what can go wrong -----------------------------------------------

    def test_no_server_at_the_first_start_says_so_and_lets_the_player_try(self):
        # The files were copied by hand and the updater never installed
        # anything: it knows nothing against them.
        self.offer(self.bundle())
        self.site.close()
        launcher = self.launcher()
        launcher.wait_for_state(LATEST)
        self.assertEqual(launcher.states(), [CHECKING, CHECK_FAILED] * 2 + [CHECKING, LATEST])
        self.assertRegex(launcher.texts()[0], r"^No update check: Cannot connect to 127\.0\.0\.1\. .*"
                                              r"The files could not be checked; you can try to play\.$")
        # The box with the reason comes before PLAY is enabled.
        types = [frame[0] for frame in launcher.frames]
        self.assertLess(types.index(0x05), len(types) - 1)
        self.assertEqual(launcher.finish(), 0)
        self.site = Site()

    def test_no_server_but_whole_files_lets_the_game_be_played(self):
        self.install(self.bundle())
        self.site.close()
        launcher = self.launcher()
        launcher.wait_for_state(LATEST)
        self.assertEqual(len(launcher.texts()), 1)
        self.assertRegex(launcher.texts()[0], r"^No update check: Cannot connect to 127\.0\.0\.1\..*you can play\.$")
        self.assertLessEqual(len(launcher.texts()[0]), 255)
        self.assertEqual(launcher.states()[-1], LATEST)
        self.assertEqual(launcher.finish(), 0)
        # But not with a program file missing: that is an error, with the
        # file's name and the reason it cannot be repaired.
        os.remove(self.path(f"{X86}/survarium_skhid.exe"))
        launcher = self.launcher()
        launcher.wait_for(lambda l: l.texts(), 60, "error text")
        self.assertNotIn(LATEST, launcher.states())
        # "Error" in the launcher's line first, then the box with the reason.
        self.assertEqual(launcher.states()[-1], ERROR)
        self.assertEqual([frame[0] for frame in launcher.frames][-2:], [0x02, 0x05])
        self.assertRegex(launcher.texts()[0], r"^x86/survarium_skhid[.]exe is missing or damaged and cannot be "
                                              r"repaired now: Cannot connect to 127\.0\.0\.1\.")
        self.assertEqual(launcher.finish(), 1)
        self.site = Site()

    def test_a_server_without_client_files_says_so(self):
        self.site.manifest_status = 404
        launcher = self.launcher()
        launcher.wait_for_state(LATEST, 30)
        self.assertEqual(launcher.states(), [CHECKING, LATEST])
        self.assertEqual(launcher.texts(), ["No update check: 127.0.0.1 offers no client files now. Try again later. "
                                            "The files could not be checked; you can try to play."])
        self.assertEqual(launcher.finish(), 0)

    def test_an_answer_that_is_not_the_servers_is_told_apart(self):
        self.site.manifest = b"<html><body>Sign in to the Wi-Fi</body></html>"
        launcher = self.launcher()
        launcher.wait_for(lambda l: l.texts(), 60, "the text")
        self.assertIn("is not the update server's", launcher.texts()[0])
        self.assertEqual(launcher.finish(), 0)
        self.site.manifest = b"skhid-client-files 2\nfuture things\n"
        launcher = self.launcher()
        launcher.wait_for(lambda l: l.texts(), 30, "the text")
        self.assertIn("newer form than this updater reads", launcher.texts()[0])
        self.assertEqual(launcher.finish(), 0)

    def test_a_wrong_checksum_installs_nothing(self):
        self.install(self.bundle())
        before = {name: self.read(name) for name in self.bundle()}
        self.offer(self.bundle(README__txt=b"changed"))
        self.site.manifest = f"skhid-client-files 1\nsha256 {'ab' * 32}\nsize {len(self.site.archive)}\n".encode()
        launcher = self.launcher()
        launcher.wait_for(lambda l: l.texts(), 90, "error text")
        self.assertIn("the checksum is wrong", launcher.texts()[0])
        self.assertEqual(launcher.states().count(DOWNLOADING), 4)
        self.assertEqual(launcher.states()[-1], ERROR)
        self.assertEqual(launcher.finish(), 1)
        for name, data in before.items():
            self.assertEqual(self.read(name), data, name)
        self.assertEqual(self.leftovers(), [])

    def test_a_download_that_breaks_off_is_tried_again(self):
        files = self.offer(self.bundle())
        self.site.cut_downloads = 1
        self.site.download_statuses = [200, 500]
        launcher = self.launcher()
        launcher.wait_for_state(LATEST, 90)
        self.assertEqual(launcher.states(), [CHECKING, DOWNLOADING, DOWNLOAD_FAILED, DOWNLOADING, DOWNLOAD_FAILED,
                                             DOWNLOADING, DOWNLOADED, INSTALLING, CHECKING, LATEST])
        self.assertEqual(launcher.texts(), [])
        self.assertEqual(launcher.finish(), 0)
        self.assertEqual(self.read("README.txt"), files["README.txt"])
        log = self.log()
        self.assertIn("broke off", log)
        self.assertIn("answered /download with error 500", log)

    def test_an_archive_that_would_write_outside_the_game_is_refused(self):
        self.site.archive = make_zip({"README.txt": b"fine", "../outside.txt": b"not fine"})
        launcher = self.launcher()
        launcher.wait_for(lambda l: l.texts(), 30, "error text")
        self.assertIn("cannot be used (BadName)", launcher.texts()[0])
        self.assertEqual(launcher.finish(), 1)
        self.assertFalse(os.path.exists(os.path.join(self.root, "outside.txt")))
        self.assertFalse(self.exists("README.txt"), "no file of a refused archive is written")
        self.site.archive = b"this is not a zip" * 10
        launcher = self.launcher()
        launcher.wait_for(lambda l: l.texts(), 30, "error text")
        self.assertIn("cannot be used (NotZip)", launcher.texts()[0])
        self.assertEqual(launcher.finish(), 1)

    def test_a_file_that_cannot_be_replaced_leaves_everything_as_it_was(self):
        self.install(self.bundle())
        before = {name: self.read(name) for name in self.bundle()}
        self.offer(self.bundle(**{
            "README.txt": b"second",
            f"{X86}/bugtrap.dll".replace("/", "__"): b"MZ new dll",
            f"{X86}/survarium_skhid.exe".replace("/", "__"): self.fake + b"game 2",
        }))
        # Another program holds the game exe and lets nobody rename it.
        held = _winapi.CreateFile(self.path(f"{X86}/survarium_skhid.exe"), _winapi.GENERIC_READ, 1,
                                  _winapi.NULL, _winapi.OPEN_EXISTING, 0, _winapi.NULL)
        try:
            launcher = self.launcher()
            launcher.wait_for(lambda l: l.texts(), 60, "error text")
            self.assertEqual(launcher.states()[-2:], [INSTALLING, ERROR])
            self.assertRegex(launcher.texts()[0], r"^Cannot replace x86\\survarium_skhid\.exe: .* \(error 32\)\. "
                                                  r"Nothing was changed\. Another program holds the file")
            self.assertEqual(launcher.finish(), 1)
        finally:
            _winapi.CloseHandle(held)
        for name, data in before.items():
            self.assertEqual(self.read(name), data, name)
        self.assertEqual(self.leftovers(), [])
        # Once it is let go, the same start succeeds.
        launcher = self.launcher()
        launcher.wait_for_state(LATEST)
        self.assertEqual(self.read("README.txt"), b"second")
        self.assertEqual(launcher.finish(), 0)

    def test_closing_the_launcher_during_a_download_stops_it_cleanly(self):
        self.offer(self.bundle(**{f"{X86}/big.bin".replace("/", "__"): os.urandom(600_000)}))
        self.site.slow = 0.02
        launcher = self.launcher()
        launcher.wait_for(lambda l: any(frame[0] == 0x01 and struct.unpack_from("<Q", frame, 1)[0] > 0
                                        for frame in l.frames), 30, "download progress")
        self.assertEqual(launcher.finish(abort=True), 0)
        self.assertEqual(launcher.states()[-1], ABORTED)
        self.assertNotIn(INSTALLING, launcher.states())
        self.assertFalse(self.exists("README.txt"))
        self.assertEqual(self.leftovers(), [])

    def test_a_launcher_that_just_goes_away_ends_the_updater_too(self):
        self.install(self.bundle())
        launcher = self.launcher()
        launcher.wait_for_state(LATEST)
        _winapi.CloseHandle(launcher.pipe)      # no abort: the process was killed
        launcher.open = False
        self.assertEqual(launcher.process.wait(20), 0)

    # --- the resource step -----------------------------------------------

    # Two entries of a small stand-in for resources.db: offset and size.
    DB_SIZE = 4 << 20
    REGIONS = {"lobby_menu": (1_000_000, 1_500_000), "login_menu": (3_000_000, 300_000)}
    PLAN = f"{X86}/updater_skhid.resources"

    def make_db(self, size=None, regions=None):
        """The game's resources.db as shipped (the same bytes in every test)."""
        self.regions = regions or self.REGIONS
        self.db = random.Random(2777).randbytes(size or self.DB_SIZE)
        self.write("game/resources.db", self.db)
        # The release builder reads the game as shipped, not the player's.
        self.source = os.path.join(self.root, "source", "game")
        os.makedirs(self.source)
        with open(os.path.join(self.source, "resources.db"), "wb") as file:
            file.write(self.db)

    def target(self, name, version=1):
        offset, size = self.regions[name]
        return forged(self.db[offset:offset + size], f"{name} target {version} ".encode())

    def plan(self, version=1, names=None):
        """The plan as make_client_026g.py writes it for these regions; their
        blobs (targets and originals) are put on the site."""
        table = {name: (offset, size, sha256(self.db[offset:offset + size]))
                 for name, (offset, size) in self.regions.items()}
        chosen = []
        for name in names or self.regions:
            path = os.path.join(self.root, f"{name}-{version}.bin")
            with open(path, "wb") as file:
                file.write(self.target(name, version))
            chosen.append((name, path))
        with mock.patch.dict(builder.RESOURCE_REGIONS, table, clear=True):
            lines, blobs = builder.resource_regions(self.source, chosen)
        self.site.blobs.update(blobs)
        return builder.resources_plan(lines)

    def with_plan(self, plan, **changes):
        return self.bundle(**{self.PLAN.replace("/", "__"): plan}, **changes)

    def region(self, name):
        offset, size = self.regions[name]
        with open(self.path("game/resources.db"), "rb") as file:
            file.seek(offset)
            return file.read(size)

    def outside_regions_unchanged(self):
        data = self.read("game/resources.db")
        self.assertEqual(len(data), len(self.db))
        at = 0
        for offset, size in sorted(self.regions.values()):
            self.assertEqual(data[at:offset], self.db[at:offset])
            at = offset + size
        self.assertEqual(data[at:], self.db[at:])

    def blob_requests(self):
        return [path for path in self.site.requests if path.startswith("/resources/")]

    def assert_original(self):
        self.assertEqual(self.read("game/resources.db"), self.db)

    def assert_applied(self, version=1):
        for name in self.regions:
            self.assertEqual(self.region(name), self.target(name, version), name)
        self.outside_regions_unchanged()

    def test_resources_are_written_once_and_found_done_after(self):
        self.make_db()
        files = self.offer(self.with_plan(self.plan()))
        launcher = self.launcher()
        launcher.wait_for_state(LATEST)
        # The handle on the db is closed before the launcher may start the
        # game: the game's own open (share mode read and write) works.
        held = _winapi.CreateFile(self.path("game/resources.db"), _winapi.GENERIC_READ, 0,
                                  _winapi.NULL, _winapi.OPEN_EXISTING, 0, _winapi.NULL)
        _winapi.CloseHandle(held)
        self.assertEqual(launcher.states(), [CHECKING, DOWNLOADING, DOWNLOADED, INSTALLING,
                                             DOWNLOADING, INSTALLING, DOWNLOADING, INSTALLING, CHECKING, LATEST])
        self.assertEqual([text for text in launcher.texts(0x07) if text.endswith("_menu")],
                         ["lobby_menu", "login_menu"])
        self.assertEqual(launcher.texts(), [], "nothing for the player to read")
        self.assertEqual(launcher.finish(), 0)
        self.assert_applied()
        self.assertEqual(self.read(self.PLAN), files[self.PLAN])
        # Each target fetched once; the originals are kept beside the updater.
        self.assertEqual(sorted(self.blob_requests()),
                         sorted(f"/resources/{sha256(self.target(name))}" for name in self.regions))
        for name, (offset, size) in self.regions.items():
            original = self.db[offset:offset + size]
            self.assertEqual(self.read(f"{X86}/updater_skhid.backup/{sha256(original)}"), original)
        self.assertFalse(self.exists(f"{X86}/updater_skhid.cache"), "the cache goes once used")
        self.assertFalse(self.exists(f"{X86}/updater_skhid.resources.journal"))
        record = self.read(f"{X86}/updater_skhid.resources.state").decode()
        self.assertTrue(record.startswith("skhid-resources-state 1\r\nregion game/resources.db ") and
                        record.endswith("\r\nend\r\n"), record)
        # The overlay's own state knows nothing of it.
        self.assertNotIn("resources.db", self.read(f"{X86}/updater_skhid.state").decode())

        # The next start: the regions are hashed, nothing else.
        self.site.requests.clear()
        launcher = self.install(files)
        self.assertEqual(launcher.said, [CHECKING, LATEST])
        self.assertEqual(self.site.requests, ["/api/client-files"])
        self.assert_applied()
        self.assertEqual(self.leftovers(), [])

    def test_a_damaged_region_is_written_again(self):
        self.make_db()
        files = self.with_plan(self.plan())
        self.install(files)
        offset, _ = self.regions["lobby_menu"]
        with open(self.path("game/resources.db"), "r+b") as file:
            file.seek(offset + 1000)
            file.write(b"damaged by a disk or a program")
        self.site.requests.clear()
        launcher = self.install(files)
        self.assertEqual(launcher.said, [CHECKING, DOWNLOADING, INSTALLING, CHECKING, LATEST])
        self.assert_applied()
        self.assertEqual(self.blob_requests(), [f"/resources/{sha256(self.target('lobby_menu'))}"])
        self.assertIn("lobby_menu: holds neither its original nor its target: the target is written over it",
                      self.log())

    def test_a_write_that_was_cut_short_is_repaired(self):
        self.make_db()
        files = self.with_plan(self.plan())
        self.install(files)
        record = self.read(f"{X86}/updater_skhid.resources.state").decode()
        lobby_line = next(line for line in record.split("\r\n") if line.endswith(" lobby_menu"))
        # The PC went off half way through the write: half target, half
        # original, and the journal of it.
        offset, size = self.regions["lobby_menu"]
        with open(self.path("game/resources.db"), "r+b") as file:
            file.seek(offset + size // 2)
            file.write(self.db[offset + size // 2:offset + size])
        self.write(f"{X86}/updater_skhid.resources.journal",
                   f"skhid-resources-state 1\r\n{lobby_line}\r\nend\r\n".encode())
        launcher = self.install(files)
        self.assertIn(INSTALLING, launcher.said)
        self.assert_applied()
        self.assertFalse(self.exists(f"{X86}/updater_skhid.resources.journal"))
        log = self.log()
        self.assertIn("a write of lobby_menu was cut short", log)
        self.assertIn("(a write of it was cut short)", log)

    def test_a_running_game_holds_the_db_and_the_regions_wait(self):
        self.make_db()
        files = self.with_plan(self.plan())
        # The game opens resources.db for reading with share mode read and
        # write, and keeps it open while it runs.
        held = _winapi.CreateFile(self.path("game/resources.db"), _winapi.GENERIC_READ, 3,
                                  _winapi.NULL, _winapi.OPEN_EXISTING, 0, _winapi.NULL)
        try:
            launcher = self.install(files)
        finally:
            _winapi.CloseHandle(held)
        # The overlay is installed; no region is changed, nothing fetched.
        self.assertEqual(launcher.said, [CHECKING, DOWNLOADING, DOWNLOADED, INSTALLING, CHECKING, LATEST])
        self.assertEqual(self.read("README.txt"), files["README.txt"])
        self.assert_original()
        self.assertEqual(self.blob_requests(), [])
        self.assertIn("the game is running", self.log())
        # The game has ended: the next start does it.
        launcher = self.install(files)
        self.assertEqual(launcher.said, [CHECKING, DOWNLOADING, INSTALLING, DOWNLOADING, INSTALLING, CHECKING, LATEST])
        self.assert_applied()

    def test_a_region_the_plan_drops_gets_its_original_back(self):
        self.make_db()
        self.install(self.with_plan(self.plan()))
        self.site.requests.clear()
        # The next archive has no plan: its file is removed with the
        # overlay, and the originals come back from the copies kept.
        launcher = self.install(self.bundle(README__txt=b"no plan"))
        self.assertIn(INSTALLING, launcher.said)
        self.assertFalse(self.exists(self.PLAN))
        self.assert_original()
        self.assertEqual(self.blob_requests(), [])
        self.assertFalse(self.exists(f"{X86}/updater_skhid.resources.state"))
        self.assertFalse(self.exists(f"{X86}/updater_skhid.backup"))
        self.assertIn("lobby_menu: its original is back (from the copy kept)", self.log())
        # Nothing more to do after.
        self.assertEqual(self.install(self.bundle(README__txt=b"no plan")).said, [CHECKING, LATEST])

    def test_without_the_copy_kept_the_original_comes_from_the_site(self):
        self.make_db()
        self.install(self.with_plan(self.plan()))
        shutil.rmtree(self.path(f"{X86}/updater_skhid.backup"))
        self.site.requests.clear()
        self.install(self.bundle(README__txt=b"no plan"))
        self.assert_original()
        self.assertEqual(sorted(self.blob_requests()), sorted(
            f"/resources/{sha256(self.db[offset:offset + size])}" for offset, size in self.regions.values()))
        self.assertIn("login_menu: its original is back (from the server)", self.log())
        self.assertFalse(self.exists(f"{X86}/updater_skhid.cache"))

    def test_a_new_target_replaces_the_old_and_the_original_stays_kept(self):
        self.make_db()
        self.install(self.with_plan(self.plan(1)))
        launcher = self.install(self.with_plan(self.plan(2)))
        self.assertIn(INSTALLING, launcher.said)
        self.assert_applied(2)
        self.assertIn("lobby_menu: the plan has a new target for it", self.log())
        # The copy of the original made before the first write is still the
        # way back.
        self.site.requests.clear()
        self.install(self.bundle(README__txt=b"no plan"))
        self.assert_original()
        self.assertEqual(self.blob_requests(), [])

    def test_a_file_of_another_build_or_none_is_left_alone(self):
        self.make_db()
        plan = self.plan()
        # Another build of the game: one byte more.
        with open(self.path("game/resources.db"), "ab") as file:
            file.write(b"!")
        # And a region in a file that is not there.
        missing = plan.replace(b"end\r\n", b"region game/resources/extra.db 100 0 10 " + sha256(b"a").encode()
                               + b" " + sha256(b"b").encode() + b" extra\r\nend\r\n")
        launcher = self.install(self.with_plan(missing))
        self.assertEqual(launcher.said, [CHECKING, DOWNLOADING, DOWNLOADED, INSTALLING, CHECKING, LATEST])
        self.assertEqual(self.read("game/resources.db"), self.db + b"!")
        self.assertEqual(self.blob_requests(), [])
        log = self.log()
        self.assertIn(f"has {self.DB_SIZE + 1} bytes, not {self.DB_SIZE} (another build of the game): skipped", log)
        self.assertIn("extra.db is not there: skipped", log)

    def test_a_blob_the_site_lacks_or_spoils_changes_nothing(self):
        self.make_db()
        files = self.with_plan(self.plan())
        lobby, login = (sha256(self.target(name)) for name in ("lobby_menu", "login_menu"))
        self.site.blob_statuses[lobby] = 404
        self.site.blob_bodies[login] = self.target("login_menu")[:-1] + b"?"
        launcher = self.install(files)
        # The overlay's result stands: installed, "latest", no box, exit 0.
        self.assertEqual(launcher.said[:4], [CHECKING, DOWNLOADING, DOWNLOADED, INSTALLING])
        self.assertEqual(launcher.said[-2:], [CHECKING, LATEST])
        self.assertEqual(launcher.texts(), [])
        self.assertEqual(self.read("README.txt"), files["README.txt"])
        self.assert_original()
        self.assertEqual(self.blob_requests(), [f"/resources/{lobby}"] * 2 + [f"/resources/{login}"] * 2)
        log = self.log()
        self.assertIn(f"answered /resources/{lobby} with error 404", log)
        self.assertIn("the server's blob is not 300000 bytes with that SHA-256", log)
        cache = self.path(f"{X86}/updater_skhid.cache")
        self.assertEqual(os.listdir(cache) if os.path.exists(cache) else [], [])
        self.assertEqual(self.leftovers(), [])
        # Once the site has them, the next start writes them.
        self.site.blob_statuses.clear()
        self.site.blob_bodies.clear()
        self.install(files)
        self.assert_applied()

    def test_a_plan_of_a_newer_format_or_cut_short_changes_nothing(self):
        self.make_db()
        plan = self.plan()
        for spoilt, said in [(plan.replace(b"skhid-resources 1", b"skhid-resources 2"), "of a newer format"),
                             (plan[:-5], "cannot be used (CutShort)"),
                             (plan.replace(b"game/resources.db", b"game/../resources.db", 1), "cannot be used (BadLine(2))")]:
            with self.subTest(said=said):
                launcher = self.install(self.with_plan(spoilt))
                self.assertEqual(launcher.said[-2:], [CHECKING, LATEST])
                self.assert_original()
                self.assertIn(said, self.log())
        self.assertEqual(self.blob_requests(), [])

    def test_the_updater_that_replaced_itself_leaves_the_step_to_the_new_one(self):
        self.make_db()
        self.install(self.bundle())
        new_updater = self.updater_bytes + b"\0new build"
        self.offer(self.with_plan(self.plan(), **{
            f"{X86}/updater_skhid.exe".replace("/", "__"): new_updater,
            f"{X86}/launcher_skhid.exe".replace("/", "__"): self.fake,
        }))
        launcher = self.launcher()
        mutex = kernel32.CreateMutexW(None, False, f"{launcher.product}_or_its_installer_is_already_running")
        try:
            launcher.wait_for_state(ABORTED)
            self.assertEqual(launcher.states(), [CHECKING, DOWNLOADING, DOWNLOADED, INSTALLING, ABORTED])
        finally:
            kernel32.CloseHandle(mutex)
        self.assertEqual(launcher.process.wait(20), 0)
        _winapi.CloseHandle(launcher.pipe)
        self.assert_original()
        self.assertIn("the resource step is done by the new one", self.log())
        # The new updater, which the new launcher starts at once.
        launcher = self.launcher()
        launcher.wait_for_state(LATEST)
        self.assertEqual(launcher.states(), [CHECKING, DOWNLOADING, INSTALLING, DOWNLOADING, INSTALLING,
                                             CHECKING, LATEST])
        self.assertEqual(launcher.finish(), 0)
        self.assert_applied()

    def test_hashing_a_region_of_the_lobby_movies_size_is_fast(self):
        # The size of build 2777's lobby_menu.swf, hashed at every start.
        size = 38_985_769
        self.make_db(size + (2 << 20), {"lobby_menu": (1 << 20, size)})
        files = self.with_plan(self.plan())
        self.install(files)
        self.assert_applied()
        started = time.time()
        launcher = self.install(files)
        whole = time.time() - started
        self.assertEqual(launcher.said, [CHECKING, LATEST])
        hashed = [int(line.rsplit(" in ", 1)[1].split()[0]) for line in self.log().splitlines()
                  if "resources: lobby_menu hashed in" in line]
        print(f"\n  39 MB region hashed in {hashed[-1]} ms; the whole start took {whole:.2f} s", file=sys.stderr)
        self.assertLess(hashed[-1], 3000)

    # --- the other ways it is started -----------------------------------

    def test_copy_is_done_at_once(self):
        launcher = self.launcher("copy")
        launcher.wait_for_state(COPIED, 10)
        self.assertEqual(launcher.states(), [COPIED])
        self.assertEqual(launcher.finish(), 0)

    def test_by_hand_it_updates_without_a_launcher(self):
        files = self.offer(self.bundle())
        exe = os.path.join(self.x86, "updater_skhid.exe")
        self.assertEqual(subprocess.run([exe, "check", "quiet"], cwd=self.root, timeout=120).returncode, 0)
        self.assertEqual(self.read("README.txt"), files["README.txt"])
        self.site.manifest_status = 500
        os.remove(self.path(f"{X86}/survarium_skhid.exe"))
        self.assertEqual(subprocess.run([exe, "check", "quiet"], timeout=120).returncode, 1)
        self.assertIn("FAILED: x86/survarium_skhid.exe is missing or damaged and cannot be repaired now: "
                      "127.0.0.1 answered /api/client-files with error 500", self.log())
        # Outside game\binaries\x86 it does nothing.
        stray = os.path.join(self.root, "updater_skhid.exe")
        shutil.copy(exe, stray)
        self.assertEqual(subprocess.run([stray, "check", "quiet"], timeout=30).returncode, 1)
        self.assertEqual(os.listdir(self.root).count("updater_skhid.log"), 0)

    def test_a_bad_server_address_is_refused(self):
        self.write(f"{X86}/updater_skhid.url", b"http://example.com")   # plain HTTP to another host
        launcher = self.launcher()
        launcher.wait_for(lambda l: l.texts(), 30, "error text")
        self.assertIn('The update server address "http://example.com" (updater_skhid.url) is not valid',
                      launcher.texts()[0])
        self.assertEqual(self.site.requests, [])
        self.assertEqual(launcher.finish(), 1)


if __name__ == "__main__":
    unittest.main()
