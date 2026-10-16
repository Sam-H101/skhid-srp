"""End-to-end tests of the patch step and of the restore of resources.db: the
real updater_skhid.exe, `check quiet` (no launcher, no box: the exit code,
the log and the files tell what it did) or started by a stand-in for the
launcher on its pipe, a web server on 127.0.0.1 in the place of the srp one
that lists resources.db's patches at `/patches` with the address of each
file (on itself, or on a second server), and `ready` and play_skhid.bat
after it.

    cd client/updater
    ./build.sh
    python tests/test_db_patches.py [-v] [TestName ...]

On Windows the exe runs as it is; elsewhere under Wine (`wine` on the PATH,
or SKHID_TEST_WINE; a prefix of its own in a temporary folder unless
SKHID_TEST_WINEPREFIX names one). What Python cannot do in Windows' place,
hold a file as the game or an antivirus does and be the launcher's end of
its pipe, the tests' stand-in program does (tests/fake_exe). Nothing
outside the temporary folders is touched; no game file is needed.
"""

import json
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import types
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
WINDOWS = sys.platform == "win32"
if not WINDOWS:
    # test_updater.py's harness, without its Windows-only parts (the
    # launcher's pipe): its site and its archive.
    import ctypes
    sys.modules.setdefault("_winapi", types.ModuleType("_winapi"))
    ctypes.WinDLL = lambda *args, **kwargs: types.SimpleNamespace(
        CreateMutexW=types.SimpleNamespace(), CloseHandle=types.SimpleNamespace())
import test_updater as harness  # noqa: E402
# The players' play_skhid.bat, as the release builder writes it.
sys.path.insert(0, os.path.join(HERE, "..", "..", "..", "srp-026g", "client-patches"))
import make_client_026g as builder  # noqa: E402

X86 = harness.X86
sha256 = harness.sha256
CHECKING, DOWNLOADING, LATEST, ABORTED, ERROR = (harness.CHECKING, harness.DOWNLOADING, harness.LATEST,
                                                 harness.ABORTED, harness.ERROR)
WINE = os.environ.get("SKHID_TEST_WINE") or shutil.which("wine")
PREFIX = None


def setUpModule():
    global PREFIX
    if WINDOWS:
        return
    if not WINE:
        raise unittest.SkipTest("no wine: set SKHID_TEST_WINE")
    PREFIX = os.environ.get("SKHID_TEST_WINEPREFIX")
    if not PREFIX:
        PREFIX = tempfile.mkdtemp(prefix="skhid-patches-wine-")
        subprocess.run([WINE, "wineboot", "-i"], env=wine_env(), check=True, timeout=300,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def tearDownModule():
    if PREFIX and not os.environ.get("SKHID_TEST_WINEPREFIX"):
        subprocess.run([str(WINE) + "server", "-k"], env=wine_env(), check=False)
        shutil.rmtree(PREFIX, ignore_errors=True)


def wine_env():
    env = dict(os.environ, WINEPREFIX=PREFIX or "", WINEDEBUG="-all",
               WINEDLLOVERRIDES="mscoree,mshtml=;winemenubuilder.exe=d")
    # No window of any kind.
    for name in ("DISPLAY", "WAYLAND_DISPLAY"):
        env.pop(name, None)
    return env


def windows(command):
    """A command line for a Windows program: as it is on Windows, by Wine
    elsewhere; and its environment."""
    return (command, None) if WINDOWS else ([WINE, *command], wine_env())


def windows_path(path):
    """`path` as the Windows programs see it (Wine's Z: is /)."""
    path = os.path.abspath(path)
    return path if WINDOWS else "Z:" + path.replace("/", "\\")


def frames(data):
    """The updater's messages on the pipe: their bytes, the id first."""
    out = []
    while len(data) >= 2:
        (length,) = struct.unpack_from("<H", data)
        if len(data) < 2 + length:
            break
        out.append(data[2:2 + length])
        data = data[2 + length:]
    return out


class Holder:
    """Another Windows program that holds a file of the game: the game its
    resources.db, an antivirus a file it scans. It may read and write the
    file but not delete or rename it, as theirs."""

    counter = 0

    def __init__(self, test, name, wait=True):
        Holder.counter += 1
        self.held = os.path.join(test.root, f"held-{Holder.counter}")
        self.let_go = os.path.join(test.root, f"release-{Holder.counter}")
        command, env = windows([harness.FAKE_EXE, "hold", windows_path(test.path(name)),
                                windows_path(self.held), windows_path(self.let_go)])
        self.process = subprocess.Popen(command, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        test.addCleanup(self.release)
        if wait:
            self.wait()

    def wait(self, seconds=60):
        until = time.time() + seconds
        while not os.path.exists(self.held):
            if time.time() > until or self.process.poll() is not None:
                raise AssertionError("the stand-in did not get hold of the file")
            time.sleep(0.02)

    def holding(self):
        return os.path.exists(self.held)

    def release(self):
        if self.process.poll() is None:
            open(self.let_go, "w").close()
            try:
                self.process.wait(30)
            except subprocess.TimeoutExpired:
                self.process.kill()


class WineTest(unittest.TestCase):
    setUp = harness.UpdaterTest.setUp
    path = harness.UpdaterTest.path
    write = harness.UpdaterTest.write
    read = harness.UpdaterTest.read
    exists = harness.UpdaterTest.exists
    bundle = harness.UpdaterTest.bundle
    offer = harness.UpdaterTest.offer
    log = harness.UpdaterTest.log

    def tearDown(self):
        for site in getattr(self, "others", []):
            site.close()
        harness.UpdaterTest.tearDown(self)

    def exe(self):
        return os.path.join(self.x86, "updater_skhid.exe")

    def run_updater(self, files=None):
        """One `check quiet` run against the archive offered (by default the
        harness's overlay); its exit code."""
        if files is not None:
            self.offer(files)
        elif not self.site.archive:
            self.offer(self.bundle())
        command, env = windows([self.exe(), "check", "quiet"])
        return subprocess.run(command, cwd=self.x86, env=env, timeout=300,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode

    def start_updater(self):
        """`check quiet`, not waited for."""
        if not self.site.archive:
            self.offer(self.bundle())
        command, env = windows([self.exe(), "check", "quiet"])
        return subprocess.Popen(command, cwd=self.x86, env=env,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def run_launched(self, stop_when=None, seconds=300):
        """One run as the launcher starts it: the stand-in makes the pipe,
        starts the updater and reads it. It says stop (the launcher's abort)
        when `stop_when()` is true, or once the updater says "latest" or
        "error". Returns the exit code and the updater's messages."""
        if not self.site.archive:
            self.offer(self.bundle())
        out = os.path.join(self.root, "pipe.bin")
        stop = os.path.join(self.root, "stop")
        for name in (out, stop):
            if os.path.exists(name):
                os.remove(name)
        Holder.counter += 1
        command, env = windows([harness.FAKE_EXE, "launcher", f"SkhidTest{os.getpid()}x{Holder.counter}",
                                windows_path(self.exe()), windows_path(out), windows_path(stop)])
        process = subprocess.Popen(command, cwd=self.x86, env=env,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        until = time.time() + seconds
        said = []
        while process.poll() is None:
            if time.time() > until:
                process.kill()
                self.fail(f"the run did not end; it said {said}")
            said = self.said(out)
            states = [frame[1] for frame in said if frame[0] == 0x02]
            if not os.path.exists(stop) and (LATEST in states or ERROR in states or (stop_when and stop_when())):
                open(stop, "w").close()
            time.sleep(0.02)
        said = self.said(out)
        return process.returncode, said

    @staticmethod
    def said(out):
        if not os.path.exists(out):
            return []
        with open(out, "rb") as file:
            return frames(file.read())

    def run_ready(self):
        """`updater_skhid.exe ready quiet` (what play_skhid.sh runs): its
        exit code."""
        command, env = windows([self.exe(), "ready", "quiet"])
        return subprocess.run(command, cwd=self.x86, env=env, timeout=300,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode

    def run_bat(self):
        """The players' play_skhid.bat, as make_client_026g.py writes it, run
        by cmd; whether it started the game (the stand-in writes down that
        it was started)."""
        started = os.path.join(self.x86, "survarium_skhid.exe.started.txt")
        if os.path.exists(started):
            os.remove(started)
        self.write(f"{X86}/play_skhid.bat", builder.bat("srp.example:1234", "english", True))
        command, env = windows(["cmd", "/c", "play_skhid.bat"])
        code = subprocess.run(command, cwd=self.x86, env=env, timeout=120,
                              stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        # `start` does not wait for the game.
        for _ in range(100):
            if os.path.exists(started):
                break
            time.sleep(0.1)
        return code.returncode, os.path.exists(started), code.stdout.decode("ascii", "replace")

    def log_since(self, mark):
        return self.log()[mark:]

    def log_size(self):
        return len(self.log()) if self.exists(f"{X86}/updater_skhid.log") else 0


class PatchTest(WineTest):
    def make_patches(self, *numbers, size=200_000):
        patches = {n: random.Random(n).randbytes(size + n) for n in numbers}
        self.site.patches.update(patches)
        return patches

    def patch_name(self, n):
        return f"game/resources.db.patch_{n}"

    def patch_files(self):
        """The patch files in the game folder: name -> bytes."""
        return {name: self.read(f"game/{name}")
                for name in sorted(os.listdir(self.path("game"))) if name.lower().startswith("resources.db.patch_")}

    def assert_patches(self, patches):
        self.assertEqual({name: sha256(data) for name, data in self.patch_files().items()},
                         {f"resources.db.patch_{n}": sha256(data) for n, data in patches.items()})

    def store_requests(self, site=None):
        return [path for path in (site or self.site).requests if path.startswith("/store/")]

    def status(self):
        name = f"{X86}/updater_skhid.patches.status"
        return self.read(name).decode() if self.exists(name) else None

    def assert_blocked(self, *said, status="blocked"):
        """PLAY is off, the player told `said`. `status`: what the status
        file says now: `blocked <why>` (the folder was to change), or as it
        was before (the run changed nothing)."""
        if status == "blocked":
            self.assertTrue((self.status() or "").startswith("blocked "), self.status())
        else:
            self.assertEqual(self.status(), status)
        log = self.log()
        self.assertIn("patches: BLOCKED:", log)
        self.assertIn("PLAY stays off until this is done", log)
        for words in said:
            self.assertIn(words, log)
        self.assertEqual([name for name in os.listdir(self.path("game")) if name.endswith(".skhid-part")], [])

    # --- the normal runs -------------------------------------------------

    def test_a_fresh_install_downloads_every_patch_gaps_and_all_and_then_nothing(self):
        patches = self.make_patches(1, 14, 17, 255)
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)
        self.assertEqual(sorted(self.store_requests()), sorted(f"/store/patch_{n}" for n in patches))
        self.assertEqual(self.status(), "ready\r\n")
        kept = json.loads(self.read(f"{X86}/updater_skhid.patches"))
        self.assertEqual(sorted(kept), ["patch_1", "patch_14", "patch_17", "patch_255"])
        self.assertEqual(kept["patch_14"]["sha256"], sha256(patches[14]))
        self.assertIn("patches: 4 patch(es) in place, checked", self.log())
        self.assertEqual(self.run_ready(), 0, self.log())
        # The next start: every file hashed once, nothing downloaded or
        # written.
        self.site.requests.clear()
        mark = self.log_size()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.store_requests(), [])
        run = self.log_since(mark)
        self.assertIn("0 to download, 0 to delete", run)
        self.assertEqual(run.count("patches: 4 file(s), "), 1, run)
        self.assert_patches(patches)

    def test_the_launcher_is_told_while_the_patches_are_checked(self):
        patches = self.make_patches(14, 17, size=3_000_000)
        self.assertEqual(self.run_updater(), 0, self.log())
        code, said = self.run_launched()
        self.assertEqual(code, 0, self.log())
        states = [frame[1] for frame in said if frame[0] == 0x02]
        self.assertEqual(states[-3:], [CHECKING, LATEST, ABORTED], states)
        self.assertGreaterEqual(states.count(CHECKING), 2, "the overlay's question, then the patches")
        # The bar over the bytes of every patch, to the end.
        total = sum(len(data) for data in patches.values())
        progress = [struct.unpack_from("<QQ", frame, 1) for frame in said if frame[0] == 0x01]
        self.assertIn((total, total), progress)

    def test_an_empty_list_has_nothing_to_do(self):
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.patch_files(), {})
        self.assertEqual(self.status(), "ready\r\n")
        self.assertEqual(self.run_ready(), 0, self.log())

    def test_the_files_may_be_on_another_server(self):
        files = harness.Site()
        self.others = [files]
        patches = self.make_patches(14, 20)
        files.patches.update(patches)
        self.site.patches.clear()
        self.site.patch_list = json.dumps({f"patch_{n}": {"sha256": sha256(data), "url": f"{files.url}/store/patch_{n}"}
                                           for n, data in patches.items()}).encode()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)
        self.assertEqual(self.store_requests(), [])
        self.assertEqual(sorted(self.store_requests(files)), ["/store/patch_14", "/store/patch_20"])

    def test_one_missing_is_downloaded_alone(self):
        patches = self.make_patches(14, 17, 20)
        self.assertEqual(self.run_updater(), 0, self.log())
        os.remove(self.path(self.patch_name(17)))
        self.assertEqual(self.run_ready(), 1, "a patch is missing")
        self.site.requests.clear()
        mark = self.log_size()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.store_requests(), ["/store/patch_17"])
        self.assert_patches(patches)
        # Hashed once before, and only the new file after.
        self.assertEqual(self.log_since(mark).count("patches: 3 file(s), "), 1)

    def test_one_damaged_here_is_downloaded_again(self):
        patches = self.make_patches(14, 17)
        self.assertEqual(self.run_updater(), 0, self.log())
        with open(self.path(self.patch_name(14)), "r+b") as file:
            file.seek(1000)
            file.write(b"damaged")
        self.assertEqual(self.run_ready(), 1, "a patch is damaged")
        self.site.requests.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.store_requests(), ["/store/patch_14"])
        self.assertIn("resources.db.patch_14 does not have the list's checksum: downloaded again", self.log())
        self.assert_patches(patches)

    def test_a_patch_the_list_has_not_is_deleted(self):
        patches = self.make_patches(14, 15)
        self.assertEqual(self.run_updater(), 0, self.log())
        # Names the game does not take, a download left over, and a patch
        # the list never had.
        for name in ("resources.db.patch_014", "resources.db.patch_0", "resources.db.patch_300",
                     "resources.db.patch_14.skhid-part", "resources.db.patch_16"):
            self.write(f"game/{name}", b"stray")
        self.write("game/resources.db.skhid-keep", b"not a patch file")
        self.assertEqual(self.run_ready(), 1, "extra patch files")
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)
        self.assertIn("patches: deleted resources.db.patch_16", self.log())
        self.assertTrue(self.exists("game/resources.db.skhid-keep"))
        self.assertEqual(self.run_ready(), 0)
        # A rollback: the server drops patch 15.
        del self.site.patches[15]
        self.site.requests.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches({14: patches[14]})
        self.assertEqual(self.store_requests(), [])
        self.assertEqual(self.read("game/resources.db"), b"existing game resources")

    # --- what keeps PLAY off, and is repaired --------------------------------

    def test_a_server_file_that_is_wrong_keeps_play_off(self):
        patches = self.make_patches(1, 14)
        self.site.patch_bodies[14] = b"not patch 14"
        self.assertEqual(self.run_updater(), 1)
        # Tried twice, never put in place; the good one is.
        self.assertEqual(self.store_requests().count("/store/patch_14"), 2)
        self.assertFalse(self.exists(self.patch_name(14)))
        self.assertEqual(self.read(self.patch_name(1)), patches[1])
        self.assert_blocked("sent game\\resources.db.patch_14 with another checksum than its list says",
                            "(12 bytes) has SHA-256")
        self.assertEqual(self.run_ready(), 1)
        # The server has it right now: the next start does it.
        self.site.patch_bodies.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)
        self.assertEqual(self.run_ready(), 0)

    def test_a_redirect_is_not_followed(self):
        patches = self.make_patches(14)
        elsewhere = harness.Site()
        self.others = [elsewhere]
        elsewhere.patches.update(patches)
        self.site.patch_redirects[14] = f"{elsewhere.url}/store/patch_14"
        self.assertEqual(self.run_updater(), 1)
        self.assertEqual(elsewhere.requests, [])
        self.assertFalse(self.exists(self.patch_name(14)))
        self.assert_blocked("game\\resources.db.patch_14 could not be downloaded. "
                            "127.0.0.1 answered /store/patch_14 with error 302")
        self.site.patch_redirects.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    def test_a_file_cut_short_is_never_put_in_place(self):
        patches = self.make_patches(14)
        self.site.patch_cut[14] = 50_000
        self.assertEqual(self.run_updater(), 1)
        self.assertEqual(self.store_requests(), ["/store/patch_14"] * 2)
        self.assertFalse(self.exists(self.patch_name(14)))
        self.assert_blocked("game\\resources.db.patch_14 could not be downloaded.",
                            f"broke off (50000 of {len(patches[14])} bytes)")
        self.assertEqual(self.run_ready(), 1)
        self.site.patch_cut.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    def test_a_damaged_file_the_server_spoils_too_is_not_left_in_place(self):
        patches = self.make_patches(14)
        self.assertEqual(self.run_updater(), 0, self.log())
        self.write(self.patch_name(14), b"damaged")
        self.site.patch_bodies[14] = b"also wrong"
        self.assertEqual(self.run_updater(), 1)
        self.assertFalse(self.exists(self.patch_name(14)), "the damaged file is gone, the wrong one never came")
        self.assert_blocked()
        self.site.patch_bodies.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    def test_a_list_that_cannot_be_read_changes_nothing_and_keeps_play_off(self):
        patches = self.make_patches(14)
        self.assertEqual(self.run_updater(), 0, self.log())
        self.write("game/resources.db.patch_16", b"stray")
        good = sha256(patches[14])
        for body, said in [
            (b"<html>Sign in to the Wi-Fi</html>", "(not JSON at byte 0)"),
            (b'{"patch_14": "' + good.encode() + b'"}', "(bad entry patch_14)"),
            (json.dumps({"patch_014": {"sha256": good, "url": f"{self.site.url}/store/patch_14"}}).encode(),
             "(bad name patch_014)"),
            (json.dumps({"patch_256": {"sha256": good, "url": f"{self.site.url}/store/patch_14"}}).encode(),
             "(bad name patch_256)"),
            (json.dumps({"patch_14": {"sha256": good[:-1], "url": f"{self.site.url}/store/patch_14"}}).encode(),
             "(bad checksum of patch_14)"),
            (json.dumps({"patch_14": {"sha256": good, "url": "ftp://127.0.0.1/patch_14"}}).encode(),
             "(bad address of patch_14)"),
            (json.dumps({"patch_14": {"sha256": good, "url": "file:///C:/patch_14"}}).encode(),
             "(bad address of patch_14)"),
            (json.dumps({"patch_14": {"sha256": good, "url": "../patch_14"}}).encode(),
             "(bad address of patch_14)"),
            (('{"patch_14": {"sha256": "%s", "url": "%s/store/patch_14"}, "patch_14": {"sha256": "%s", '
              '"url": "%s/store/patch_14"}}' % (good, self.site.url, good, self.site.url)).encode(),
             "(patch_14 twice)"),
        ]:
            with self.subTest(said=said):
                self.site.patch_list = body
                self.site.requests.clear()
                self.assertEqual(self.run_updater(), 1)
                self.assertEqual(self.store_requests(), [])
                self.assertEqual(self.read("game/resources.db.patch_16"), b"stray", "nothing deleted")
                self.assertEqual(self.read(self.patch_name(14)), patches[14])
                # Nothing was changed: the last run's word stands.
                self.assert_blocked("The list of the game's patches from 127.0.0.1 cannot be used " + said,
                                    status="ready\r\n")
        self.site.patch_list = None
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    def test_a_server_without_the_list_keeps_play_off(self):
        self.site.patch_list_status = 404
        self.assertEqual(self.run_updater(), 1)
        # The first start: the overlay was installed, nothing checked.
        self.assert_blocked("127.0.0.1 offers no list of the game's patches", status="blocked overlay changed\r\n")
        self.assertEqual(self.run_ready(), 1)

    def test_a_list_that_cannot_be_fetched_keeps_play_off_and_is_asked_for_next_time(self):
        patches = self.make_patches(14)
        self.site.patch_list_status = 500
        self.assertEqual(self.run_updater(), 1)
        self.assertEqual(self.site.requests.count("/patches"), 3)
        self.assert_blocked("The list of the game's patches could not be fetched. "
                            "127.0.0.1 answered /patches with error 500", status="blocked overlay changed\r\n")
        self.site.patch_list_status = 200
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    def test_a_list_that_cannot_be_fetched_after_a_checked_run_is_as_a_server_that_is_down(self):
        patches = self.make_patches(14)
        self.assertEqual(self.run_updater(), 0, self.log())
        # A moment's 500 of the server: the patches checked before may be
        # played, and the status is not touched.
        self.site.patch_list_status = 500
        self.assertEqual(self.run_updater(), 0, self.log())
        log = self.log()
        self.assertIn("patches: the list could not be fetched: 127.0.0.1 answered /patches with error 500", log)
        self.assertIn("patches: the server could not be asked; the patches kept are in place", log)
        self.assertEqual(self.status(), "ready\r\n")
        self.assertEqual(self.run_ready(), 0)
        # One damaged meanwhile: not, and the status still says what the
        # last checked run found.
        self.write(self.patch_name(14), b"damaged")
        self.assertEqual(self.run_updater(), 1)
        self.assert_blocked("The list of the game's patches could not be fetched.", status="ready\r\n")
        self.assertEqual(self.run_ready(), 1)
        self.site.patch_list_status = 200
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    def test_new_skhid_files_are_not_played_with_the_patches_checked_for_the_old(self):
        patches = self.make_patches(14)
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.status(), "ready\r\n")
        # A release replaces the updater: the new one is to do the rest, at
        # the restart the launcher does. Until it has, `ready` refuses...
        new_updater = {f"{X86}/updater_skhid.exe".replace("/", "__"): self.updater_bytes + b"\0new build"}
        self.offer(self.bundle(**new_updater))
        self.site.requests.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertIn("the rest of the update is done by the new one", self.log())
        self.assertNotIn("/patches", self.site.requests)
        self.assertEqual(self.status(), "blocked overlay changed\r\n")
        self.assertEqual(self.run_ready(), 1)
        # ... and so does a start without the server.
        self.site.close()
        self.assertEqual(self.run_updater(), 1)
        self.assertIn("the last run did not finish: blocked overlay changed", self.log())
        self.site = harness.Site()
        self.site.patches.update(patches)
        self.write(f"{X86}/updater_skhid.url", self.site.url.encode())
        # A release that replaces neither: the same, when its list cannot
        # be fetched.
        self.offer(self.bundle(**{"README.txt": b"a new readme"}, **new_updater))
        self.site.patch_list_status = 500
        self.assertEqual(self.run_updater(), 1)
        self.assertEqual(self.read("README.txt"), b"a new readme")
        self.assert_blocked("The list of the game's patches could not be fetched.",
                            status="blocked overlay changed\r\n")
        # The list: checked, ready.
        self.site.patch_list_status = 200
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.status(), "ready\r\n")
        self.assertEqual(self.run_ready(), 0)
        self.assert_patches(patches)

    def test_a_file_server_that_cannot_be_reached_keeps_play_off(self):
        patches = self.make_patches(14)
        down = harness.Site()
        dead = down.url
        down.close()
        self.site.patch_base = dead
        self.assertEqual(self.run_updater(), 1)
        self.assertFalse(self.exists(self.patch_name(14)))
        self.assert_blocked("game\\resources.db.patch_14 could not be downloaded. Cannot connect to 127.0.0.1")
        self.site.patch_base = None
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    def test_a_server_that_is_down_lets_play_only_with_the_patches_checked_before(self):
        patches = self.make_patches(14)
        # The first start, with the server down: never checked.
        archive = self.bundle()
        self.offer(archive)
        self.site.close()
        self.assertEqual(self.run_updater(), 1)
        self.assertIn("The game's patches could not be checked: Cannot connect to 127.0.0.1", self.log())
        self.assertEqual(self.run_ready(), 1)
        self.assertEqual(self.patch_files(), {})
        # Up again: installed and checked.
        self.site = harness.Site()
        self.site.patches.update(patches)
        self.write(f"{X86}/updater_skhid.url", self.site.url.encode())
        self.offer(archive)
        self.assertEqual(self.run_updater(), 0, self.log())
        # Down: the patches kept are in place, so the game may be played.
        self.site.close()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertIn("patches: the server could not be asked; the patches kept are in place", self.log())
        # Down, and one damaged meanwhile: not.
        self.write(self.patch_name(14), b"damaged")
        self.assertEqual(self.run_updater(), 1)
        self.assertIn("resources.db.patch_14 does not have the list's checksum", self.log())
        self.site = harness.Site()

    def test_not_enough_disk_downloads_nothing(self):
        self.make_patches(14)
        free = shutil.disk_usage(self.path("game")).free
        self.site.patch_lengths[14] = free + (1 << 30)
        if self.site.patch_lengths[14] >= 1 << 40:
            self.skipTest("more than 1 TB free: larger than the largest patch taken")
        self.assertEqual(self.run_updater(), 1)
        self.assertFalse(self.exists(self.patch_name(14)))
        self.assertEqual(self.store_requests(), ["/store/patch_14"], "tried once: the disk does not change")
        needed = ((free + (1 << 30) + (64 << 20)) + (1 << 20) - 1) >> 20
        self.assert_blocked("Not enough free disk space to update the game's files in",
                            f": {needed} MB needed,", "bytes free in")

    def test_the_overlay_may_not_bring_a_patch_file(self):
        for name in ("game/resources.db.patch_14", f"{X86}/resources.db.patch_3"):
            with self.subTest(name=name):
                self.offer(self.bundle(**{name.replace("/", "__"): b"from the zip"}))
                self.assertEqual(self.run_updater(), 1)
                self.assert_blocked(f"the overlay has {name.replace(os.sep, '/')}")
                self.assertEqual(self.run_ready(), 1)

    # --- the launcher says stop ------------------------------------------

    def test_a_download_stopped_half_way_leaves_no_file_and_no_ready(self):
        patches = self.make_patches(14, size=4_000_000)
        self.site.patch_slow = 0.05
        code, said = self.run_launched(stop_when=lambda: "/store/patch_14" in self.site.requests)
        self.assertEqual(code, 0, self.log())
        states = [frame[1] for frame in said if frame[0] == 0x02]
        self.assertEqual(states[-1], ABORTED)
        self.assertIn(DOWNLOADING, states)
        self.assertNotIn(LATEST, states)
        self.assertEqual(self.patch_files(), {})
        self.assertIn("patches: stopped; the rest next time", self.log())
        self.assertEqual(self.status(), "blocked stopped\r\n")
        self.assertEqual(self.run_ready(), 1)
        self.site.patch_slow = 0
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    def test_a_stop_while_the_list_is_asked_again_keeps_the_last_word(self):
        patches = self.make_patches(14)
        self.assertEqual(self.run_updater(), 0, self.log())
        self.site.patch_list_status = 503
        self.site.requests.clear()
        code, said = self.run_launched(stop_when=lambda: "/patches" in self.site.requests)
        self.assertEqual(code, 0, self.log())
        self.assertEqual(self.site.requests.count("/patches"), 1, "stopped in the pause before the next try")
        self.assertEqual([frame[1] for frame in said if frame[0] == 0x02][-1], ABORTED)
        self.assertEqual(self.status(), "ready\r\n")
        self.assertEqual(self.run_ready(), 0)
        self.assert_patches(patches)

    # --- other programs hold files ---------------------------------------

    def test_a_running_game_keeps_the_patches_as_they_are(self):
        patches = self.make_patches(14)
        self.assertEqual(self.run_updater(), 0, self.log())
        self.write("game/resources.db.patch_16", b"stray")
        game = Holder(self, "game/resources.db")
        self.assertEqual(self.run_updater(), 1)
        self.assertTrue(self.exists("game/resources.db.patch_16"), "nothing deleted while the game runs")
        self.assert_blocked("The game is running, so Skhid could not update the game's patches",
                            "resources.db is open in another program")
        # With nothing to change, a game that runs is no reason to stop.
        os.remove(self.path("game/resources.db.patch_16"))
        self.assertEqual(self.run_updater(), 0, self.log())
        self.write("game/resources.db.patch_16", b"stray")
        game.release()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    def test_a_patch_file_another_program_holds_a_moment_is_tried_again(self):
        # An antivirus that scans the new file before it is renamed into
        # place: the rename waits for it.
        patches = self.make_patches(14, size=32 << 20)
        scanner = Holder(self, "game/resources.db.patch_14.skhid-part", wait=False)
        updater = self.start_updater()
        until = time.time() + 120
        while updater.poll() is None and time.time() < until:
            if self.exists(f"{X86}/updater_skhid.log") and "cannot be renamed now" in self.log():
                scanner.release()
            time.sleep(0.01)
        self.assertEqual(updater.wait(120), 0, self.log())
        self.assertTrue(scanner.holding(), "the stand-in never got hold of the download")
        log = self.log()
        self.assertIn("resources.db.patch_14.skhid-part cannot be renamed now", log)
        self.assertIn("try 1 of 6: another program holds it", log)
        self.assert_patches(patches)
        self.assertEqual(self.status(), "ready\r\n")

    def test_a_patch_file_another_program_keeps_holding_keeps_play_off(self):
        patches = self.make_patches(14)
        self.assertEqual(self.run_updater(), 0, self.log())
        self.write("game/resources.db.patch_16", b"stray")
        scanner = Holder(self, "game/resources.db.patch_16")
        self.assertEqual(self.run_updater(), 1)
        self.assertIn("try 5 of 6", self.log())
        self.assert_blocked("Another program holds game\\resources.db.patch_16")
        self.assertNotIn("The game is running", self.log())
        scanner.release()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    # --- what play_skhid.bat checks before it starts the game: `ready` ----

    def test_play_skhid_bat_starts_the_game_only_when_the_patches_are_checked(self):
        patches = self.make_patches(14)
        code, started, said = self.run_bat()
        self.assertEqual((code, started), (1, False), said)
        self.assertIn("Skhid has not finished updating the game's files", said)
        self.assertIn("ready: NOT READY: patches:", self.log())
        self.site.patch_bodies[14] = b"wrong"
        self.assertEqual(self.run_updater(), 1)
        code, started, said = self.run_bat()
        self.assertEqual((code, started), (1, False), said)
        self.site.patch_bodies.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        code, started, said = self.run_bat()
        self.assertEqual((code, started), (0, True), said)
        # An extra patch file after (copied by hand): not started.
        self.write("game/resources.db.patch_200", b"from somewhere")
        code, started, said = self.run_bat()
        self.assertEqual((code, started), (1, False), said)
        self.assertTrue(self.exists("game/resources.db.patch_200"), "ready only reads")
        self.assert_patches({**patches, 200: b"from somewhere"})


class RestoreTest(WineTest):
    """The one-time restore: a resources.db whose regions an earlier updater
    rewrote in place, with the record and the copies it kept, as the
    players' folders are."""

    DB_SIZE = 4 << 20
    REGIONS = {"main_menu": (900_000, 100_000), "lobby_menu": (1_000_000, 1_500_000),
               "login_menu": (3_000_000, 300_000)}
    RECORD = f"{X86}/updater_skhid.resources.state"
    BACKUP = f"{X86}/updater_skhid.backup"

    def make_db(self, kept=True):
        """The db as shipped, then as the earlier updater left it."""
        self.db = random.Random(2777).randbytes(self.DB_SIZE)
        changed = bytearray(self.db)
        lines = []
        for name, (offset, size) in self.REGIONS.items():
            original = self.db[offset:offset + size]
            target = random.Random(name).randbytes(size)
            changed[offset:offset + size] = target
            lines.append(f"region game/resources.db {self.DB_SIZE} {offset} {size} "
                         f"{sha256(original)} {sha256(target)} {name}")
            self.site.blobs[sha256(original)] = original
            if kept:
                self.write(f"{self.BACKUP}/{sha256(original)}", original)
        self.write("game/resources.db", bytes(changed))
        self.record = "".join(line + "\r\n" for line in ["skhid-resources-state 1", *lines, "end"]).encode()
        self.write(self.RECORD, self.record)
        # The earlier updaters' plan (the ZIP's) and the status of a plan
        # that grew the db.
        self.write(f"{X86}/updater_skhid.resources", b"skhid-resources 1\r\nend\r\n")
        self.write(f"{X86}/updater_skhid.resources.status", b"ready\r\n")
        return bytes(changed)

    def blob_requests(self):
        return [path for path in self.site.requests if path.startswith("/resources/")]

    def assert_stock(self):
        now = self.read("game/resources.db")
        self.assertEqual((len(now), sha256(now)), (len(self.db), sha256(self.db)))

    def assert_nothing_left(self):
        for name in (self.RECORD, self.BACKUP, f"{X86}/updater_skhid.cache", f"{X86}/updater_skhid.resources",
                     f"{X86}/updater_skhid.resources.status", f"{X86}/updater_skhid.resources.journal"):
            self.assertFalse(self.exists(name), name)

    def test_every_region_gets_its_stock_bytes_back_once(self):
        self.make_db()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_stock()
        self.assert_nothing_left()
        self.assertEqual(self.blob_requests(), [])
        log = self.log()
        self.assertIn("resources: 3 region(s) of resources.db changed by an earlier updater", log)
        for name in self.REGIONS:
            self.assertIn(f"resources: {name}: its original is back (from the copy kept)", log)
        # And never again.
        mark = self.log_size()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertNotIn("resources:", self.log_since(mark))
        self.assert_stock()

    def test_without_the_copies_the_stock_bytes_come_from_the_server(self):
        self.make_db(kept=False)
        lobby = sha256(self.db[1_000_000:2_500_000])
        self.site.blob_bodies[lobby] = b"x" * 1_500_000
        self.assertEqual(self.run_updater(), 0, self.log())
        # The two the server has right are back; the lobby stays recorded.
        self.assertIn("the server's blob is not 1500000 bytes with that SHA-256", self.log())
        self.assertIn("resources: 1 region(s) not restored yet (lobby_menu): next time", self.log())
        self.assertIn("lobby_menu", self.read(self.RECORD).decode())
        self.assertNotEqual(sha256(self.read("game/resources.db")), sha256(self.db))
        self.site.blob_bodies.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertIn("resources: lobby_menu: its original is back (from the server)", self.log())
        self.assert_stock()
        self.assert_nothing_left()

    def test_a_running_game_holds_the_db_and_the_restore_waits(self):
        changed = self.make_db()
        game = Holder(self, "game/resources.db")
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(sha256(self.read("game/resources.db")), sha256(changed))
        self.assertEqual(self.read(self.RECORD), self.record)
        self.assertIn("resources: the game is running", self.log())
        game.release()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_stock()
        self.assert_nothing_left()

    def test_a_write_that_was_cut_short_is_repaired(self):
        self.make_db()
        # The PC went off half way through putting the lobby back: half
        # stock, half not, the journal of it, and the record without it.
        offset, size = self.REGIONS["lobby_menu"]
        lines = self.record.decode().split("\r\n")
        lobby = next(line for line in lines if line.endswith(" lobby_menu"))
        self.write(self.RECORD, "\r\n".join(line for line in lines if line != lobby).encode())
        self.write(f"{X86}/updater_skhid.resources.journal",
                   f"skhid-resources-state 1\r\n{lobby}\r\nend\r\n".encode())
        with open(self.path("game/resources.db"), "r+b") as file:
            file.seek(offset)
            file.write(self.db[offset:offset + size // 2])
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertIn("a write of lobby_menu was cut short", self.log())
        self.assert_stock()
        self.assert_nothing_left()

    def test_the_copies_kept_are_enough_without_the_server(self):
        self.make_db()
        self.offer(self.bundle())
        self.site.close()
        # The patches were never checked, so PLAY stays off; the db is
        # given back all the same.
        self.assertEqual(self.run_updater(), 1)
        self.assert_stock()
        self.assert_nothing_left()
        self.site = harness.Site()

    def test_a_db_of_another_build_is_left_alone(self):
        changed = self.make_db()
        with open(self.path("game/resources.db"), "ab") as file:
            file.write(b"!")
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.read("game/resources.db"), changed + b"!")
        self.assertFalse(self.exists(self.RECORD))
        self.assertIn("lobby_menu holds its stock bytes (or the db is gone or of another build): forgotten",
                      self.log())


if __name__ == "__main__":
    unittest.main()
