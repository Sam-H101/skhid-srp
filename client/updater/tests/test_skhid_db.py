"""End-to-end tests of the game's database (`resources_skhid.db`, built from
the untouched `resources.db` and the release's patches): the real
updater_skhid.exe, `check quiet` (no launcher, no box: the exit code, the log
and the files tell what it did), a web server on 127.0.0.1 in the place of
the srp one, and a 4 MB stand-in for the stock resources.db.

    cd client/updater
    ./build.sh
    python tests/test_skhid_db.py [-v] [TestName ...]

On Windows the exe runs as it is; elsewhere under Wine (`wine` on the PATH,
or SKHID_TEST_WINE; a prefix of its own in a temporary folder unless
SKHID_TEST_WINEPREFIX names one). The blobs are compressed with the `zstd`
tool. Nothing outside the temporary folders is touched; no game file is
needed. What needs another program to hold a file (a running game) is in
test_updater.py, on Windows.
"""

import hashlib
import os
import random
import shutil
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
    # launcher's pipe): its site, its archive and the release builder.
    import ctypes
    sys.modules.setdefault("_winapi", types.ModuleType("_winapi"))
    ctypes.WinDLL = lambda *args, **kwargs: types.SimpleNamespace(
        CreateMutexW=types.SimpleNamespace(), CloseHandle=types.SimpleNamespace())
import test_updater as harness  # noqa: E402
import db_patches as D  # noqa: E402  (srp-026g/client-patches, on the path through the harness)

builder = harness.builder
X86 = harness.X86
sha256 = harness.sha256
WINE = os.environ.get("SKHID_TEST_WINE") or shutil.which("wine")
ZSTD = shutil.which("zstd")
PREFIX = None
LIST = f"{X86}/updater_skhid.patches"
SKHID = "game/resources_skhid.db"


def setUpModule():
    global PREFIX
    if not ZSTD:
        raise unittest.SkipTest("no zstd tool")
    if WINDOWS:
        return
    if not WINE:
        raise unittest.SkipTest("no wine: set SKHID_TEST_WINE")
    PREFIX = os.environ.get("SKHID_TEST_WINEPREFIX")
    if not PREFIX:
        PREFIX = tempfile.mkdtemp(prefix="skhid-db-wine-")
        subprocess.run([WINE, "wineboot", "-i"], env=wine_env(), check=True, timeout=300,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def tearDownModule():
    if PREFIX and not os.environ.get("SKHID_TEST_WINEPREFIX"):
        subprocess.run([str(WINE) + "server", "-k"], env=wine_env(), check=False)
        shutil.rmtree(PREFIX, ignore_errors=True)


def wine_env():
    env = dict(os.environ, WINEPREFIX=PREFIX or "", WINEDEBUG="-all",
               WINEDLLOVERRIDES="mscoree,mshtml=;winemenubuilder.exe=d")
    for name in ("DISPLAY", "WAYLAND_DISPLAY"):
        env.pop(name, None)
    return env


def zstd(data):
    return subprocess.run([ZSTD, "-3", "-q", "-c"], input=data, stdout=subprocess.PIPE, check=True).stdout


def make_patch(revision, before, writes=(), append=b"", **changes):
    """A patch as the release's tools make it: `writes` [(offset, bytes)]
    over `before`, `append` added. (description, blob, the db after)."""
    after = bytearray(before)
    ops, data = [], b""
    for offset, chunk in writes:
        after[offset:offset + len(chunk)] = chunk
        ops.append(("write", offset, len(chunk)))
        data += chunk
    if append:
        after += append
        ops.append(("append", len(append)))
        data += append
    blob = zstd(data)
    patch = {"revision": revision, "from": (sha256(before), len(before)),
             "to": (sha256(bytes(after)), len(after)), "blob": (sha256(blob), len(blob)), "ops": ops}
    patch.update(changes)
    return patch, blob, bytes(after)


class SkhidDbTest(unittest.TestCase):
    DB_SIZE = 4 << 20
    # Where the three menus are in the stand-in, as in build 2777's db.
    MENUS = {"main_menu": (300_000, 50_000), "lobby_menu": (350_000, 900_000), "login_menu": (1_400_000, 80_000)}

    setUp = harness.UpdaterTest.setUp
    tearDown = harness.UpdaterTest.tearDown
    path = harness.UpdaterTest.path
    write = harness.UpdaterTest.write
    read = harness.UpdaterTest.read
    exists = harness.UpdaterTest.exists
    bundle = harness.UpdaterTest.bundle
    offer = harness.UpdaterTest.offer
    leftovers = harness.UpdaterTest.leftovers
    log = harness.UpdaterTest.log

    def run_exe(self, *arguments):
        exe = os.path.join(self.x86, "updater_skhid.exe")
        command = [exe, *arguments] if WINDOWS else [WINE, exe, *arguments]
        return subprocess.run(command, cwd=self.x86, env=None if WINDOWS else wine_env(), timeout=300,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode

    def run_updater(self, files=None):
        """One `check quiet` run against the archive offered; its exit code."""
        if files is not None:
            self.offer(files)
        return self.run_exe("check", "quiet")

    def run_ready(self):
        return self.run_exe("ready", "quiet")

    def make_db(self):
        """The stand-in stock resources.db, and the list's original made it."""
        self.db = random.Random(2777).randbytes(self.DB_SIZE)
        self.write("game/resources.db", self.db)
        self.original = (sha256(self.db), len(self.db))
        self.patches = []
        self.parts = {}

    def menus(self, version=1):
        return [(offset, random.Random(f"{name} {version}").randbytes(size))
                for name, (offset, size) in self.MENUS.items()]

    def add(self, revision, writes=(), append=b"", **changes):
        """Patch `revision` after the last one; its blob on the site."""
        before = self.parts[self.patches[-1]["revision"]] if self.patches else self.db
        patch, blob, after = make_patch(revision, before, writes, append, **changes)
        self.patches.append(patch)
        self.parts[revision] = after
        self.site.blobs[patch["blob"][0]] = blob
        return patch

    def first(self):
        """Revision 14: the three menus, written in place."""
        return self.add(14, self.menus())

    def second(self):
        """Revision 15: a new file table and new entries appended."""
        return self.add(15, [(0, b"table 15 " * 1000)], random.Random(15).randbytes(300_000))

    def release(self, patches=None):
        listing = D.write_list(self.original, self.patches if patches is None else patches)
        return self.bundle(**{LIST.replace("/", "__"): listing})

    def expected(self):
        return self.parts[self.patches[-1]["revision"]] if self.patches else self.db

    def blob_requests(self):
        return [path for path in self.site.requests if path.startswith("/resources/")]

    def assert_built(self, expected=None):
        expected = self.expected() if expected is None else expected
        now = self.read(SKHID)
        self.assertEqual((len(now), sha256(now)), (len(expected), sha256(expected)))
        self.assertEqual(sha256(self.read("game/resources.db")), sha256(self.db), "the original is not changed")

    def status(self):
        name = f"{X86}/updater_skhid.patches.status"
        return self.read(name).decode() if self.exists(name) else None

    def assert_blocked(self, said):
        self.assertTrue(self.status().startswith("blocked "), self.status())
        log = self.log()
        self.assertIn("db: BLOCKED: ", log)
        self.assertIn(". The player is told: ", log)
        self.assertIn(said, log)
        self.assertIn("PLAY stays off until this is done", log)

    # --- the normal runs -------------------------------------------------

    def test_a_fresh_build_is_checked_whole_and_found_done_after(self):
        self.make_db()
        self.first()
        self.second()
        files = self.release()
        self.assertEqual(self.run_updater(files), 0, self.log())
        self.assert_built()
        self.assertEqual(self.status(), "ready\r\n")
        state = D._lines(self.read(f"{X86}/updater_skhid.patches.state"))
        self.assertEqual(state[:2], ["skhid-db-state 1", "revision 15"])
        self.assertFalse(self.exists(f"{X86}/updater_skhid.cache"), "the blobs go once applied")
        log = self.log()
        self.assertIn("db: building resources_skhid.db (revision 15), attempt 1", log)
        self.assertLess(log.index("patch 14 applied"), log.index("patch 15 applied"))
        self.assertIn("resources_skhid.db built, revision 15, checked whole", log)
        # The next start: a quick look, nothing fetched, copied or hashed whole.
        self.site.requests.clear()
        before = len(log)
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.blob_requests(), [])
        self.assertNotIn("hashed whole", self.log()[before:])
        self.assert_built()
        self.assertEqual(self.run_ready(), 0)
        self.assertEqual(self.leftovers(), [])

    def test_a_release_with_a_new_patch_applies_only_that_one(self):
        self.make_db()
        self.first()
        self.assertEqual(self.run_updater(self.release()), 0, self.log())
        self.assert_built()
        self.second()
        self.site.requests.clear()
        before = len(self.log())
        self.assertEqual(self.run_updater(self.release()), 0, self.log())
        self.assert_built()
        log = self.log()[before:]
        self.assertEqual(self.blob_requests(), [f"/resources/{self.patches[1]['blob'][0]}"])
        self.assertIn("db: resources_skhid.db from revision 14 to 15", log)
        self.assertNotIn("building", log)
        self.assertIn("resources_skhid.db is at revision 15, checked whole", log)

    def test_no_patch_makes_a_copy_of_the_original(self):
        self.make_db()
        self.assertEqual(self.run_updater(self.release()), 0, self.log())
        self.assert_built(self.db)
        self.assertEqual(self.blob_requests(), [])

    def test_a_database_that_is_a_revision_of_the_list_is_taken_as_it_is(self):
        self.make_db()
        self.first()
        self.second()
        # Built before, its state lost: hashed whole and found at 14.
        self.write(SKHID, self.parts[14])
        self.assertEqual(self.run_updater(self.release()), 0, self.log())
        self.assert_built()
        log = self.log()
        self.assertIn("db: resources_skhid.db is at revision 14", log)
        self.assertIn("from revision 14 to 15", log)
        self.assertNotIn("building", log)

    # --- what is repaired -------------------------------------------------

    def test_a_damaged_database_is_built_anew(self):
        self.make_db()
        self.first()
        files = self.release()
        self.assertEqual(self.run_updater(files), 0, self.log())
        with open(self.path(SKHID), "r+b") as file:
            file.seek(2_000_000)
            file.write(b"damaged")
        self.assertEqual(self.run_ready(), 1)
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_built()
        self.assertIn("resources_skhid.db is no revision of the list: it is built anew", self.log())
        self.assertEqual(self.run_ready(), 0)

    def test_a_build_cut_short_is_removed_and_done_again(self):
        self.make_db()
        self.first()
        # The PC went off during a build, and during an update in place (its
        # state is removed before the first byte is written).
        self.write(SKHID + ".skhid-build", self.db[:1_000_000])
        self.assertEqual(self.run_updater(self.release()), 0, self.log())
        self.assert_built()
        self.assertIn("resources_skhid.db.skhid-build, left by a run that was cut short", self.log())
        self.second()
        os.remove(self.path(f"{X86}/updater_skhid.patches.state"))
        half = bytearray(self.parts[14])
        half[0:5000] = self.parts[15][0:5000]
        self.write(SKHID, bytes(half) + self.parts[15][len(half):len(half) + 1000])
        self.assertEqual(self.run_updater(self.release()), 0, self.log())
        self.assert_built()
        self.assertFalse(self.exists(SKHID + ".skhid-build"))

    def test_a_patch_whose_result_is_not_its_own_keeps_play_off(self):
        self.make_db()
        self.add(14, self.menus(), to=(sha256(b"another result"), self.DB_SIZE))
        self.assertEqual(self.run_updater(self.release()), 1)
        self.assert_blocked("Skhid could not prepare the game's database")
        log = self.log()
        self.assertIn("attempt 2", log)
        self.assertIn("not the release's", log)
        self.assertFalse(self.exists(SKHID))
        self.assertFalse(self.exists(SKHID + ".skhid-build"))
        self.assertEqual(self.run_ready(), 1)

    def test_a_blob_the_site_lacks_or_spoils_keeps_play_off_until_it_is_there(self):
        self.make_db()
        patch = self.first()
        files = self.release()
        for status, body in [(404, None), (200, b"not the blob")]:
            with self.subTest(status=status):
                self.site.blob_statuses = {patch["blob"][0]: status} if status != 200 else {}
                self.site.blob_bodies = {patch["blob"][0]: body} if body else {}
                self.assertEqual(self.run_updater(files), 1)
                self.assert_blocked("Skhid could not prepare the game's database")
                self.assertFalse(self.exists(SKHID))
        self.site.blob_statuses, self.site.blob_bodies = {}, {}
        self.assertEqual(self.run_updater(files), 0, self.log())
        self.assert_built()

    def test_not_enough_disk_is_said_before_anything_is_fetched(self):
        self.make_db()
        # A patch that would grow the db by 64 TB.
        huge = 64 << 40
        patch = self.add(14, self.menus())
        patch["ops"].append(("append", huge))
        patch["to"] = (sha256(b"huge"), self.DB_SIZE + huge)
        self.assertEqual(self.run_updater(self.release()), 1)
        self.assert_blocked("Not enough free disk space to prepare the game's database in")
        self.assertEqual(self.blob_requests(), [])

    def test_a_list_that_cannot_be_read_keeps_play_off(self):
        self.make_db()
        self.first()
        good = D.write_list(self.original, self.patches)
        for spoilt in [good.replace(b"skhid-db-patches 1", b"skhid-db-patches 2"), good[:-5],
                       good.replace(b"revision 14", b"revision 0")]:
            with self.subTest(spoilt=spoilt[:30]):
                self.assertEqual(self.run_updater(self.bundle(**{LIST.replace("/", "__"): spoilt})), 1)
                self.assert_blocked("Skhid could not prepare the game's database")
                self.assertEqual(self.run_ready(), 1)
        self.assertFalse(self.exists(SKHID))
        # An archive without the list (made without the launcher): nothing.
        self.assertEqual(self.run_updater(self.bundle()), 0, self.log())
        self.assertFalse(self.exists(SKHID))
        self.assertEqual(self.run_ready(), 0)

    # --- the original ----------------------------------------------------

    def pristine_on_site(self, data):
        """A pristine resources.db (zstd) on the site, named in updater_skhid.db-url."""
        self.site.blobs["pristine"] = zstd(data)
        self.write(f"{X86}/updater_skhid.db-url", f"{self.site.url}/resources/pristine".encode())

    def test_an_original_that_is_not_the_original_is_downloaded_again(self):
        self.make_db()
        self.first()
        self.write("game/resources.db", self.db[:100] + b"changed" + self.db[107:])
        self.pristine_on_site(self.db)
        self.assertEqual(self.run_updater(self.release()), 0, self.log())
        self.assert_built()
        log = self.log()
        self.assertIn("is not the original", log)
        self.assertIn("resources.db: the pristine copy is in place, checked", log)
        self.assertFalse(self.exists("game/.skhid-resources.db.download"))
        self.assertFalse(self.exists("game/resources.db.skhid-new"))

    def test_a_download_that_is_not_the_original_either_keeps_play_off(self):
        self.make_db()
        self.first()
        changed = self.db[:100] + b"changed" + self.db[107:]
        self.write("game/resources.db", changed)
        self.pristine_on_site(changed)
        self.assertEqual(self.run_updater(self.release()), 1)
        self.assert_blocked("game\\resources.db is missing or not the original game file")
        self.assertIn("is not the original resources.db", self.log())
        self.assertFalse(self.exists(SKHID))
        # No address: the game's own archive, which needs its extractor (not
        # in this folder): said, PLAY off.
        os.remove(self.path(f"{X86}/updater_skhid.db-url"))
        self.write("game/resources.db", changed)
        self.assertEqual(self.run_updater(), 1)
        self.assert_blocked("could not be downloaded again: The original game cannot be unpacked")

    # --- players whose resources.db earlier updaters changed in place ----

    def legacy(self, kept=("main_menu", "lobby_menu")):
        """resources.db as the live players have it: the three menus written
        in place by the updater of before, its record, and the copies of the
        originals it kept (`kept`); the others' originals are on the site."""
        lines = ["skhid-resources-state 1"]
        db = bytearray(self.db)
        for name, (offset, size) in self.MENUS.items():
            original = self.db[offset:offset + size]
            target = random.Random(f"live {name}").randbytes(size)
            db[offset:offset + size] = target
            lines.append(f"region game/resources.db {len(self.db)} {offset} {size} "
                         f"{sha256(original)} {sha256(target)} {name}")
            if name in kept:
                self.write(f"{X86}/updater_skhid.backup/{sha256(original)}", original)
            else:
                self.site.blobs[sha256(original)] = original
        self.write("game/resources.db", bytes(db))
        record = "".join(line + "\r\n" for line in [*lines, "end"]).encode()
        self.write(f"{X86}/updater_skhid.resources.state", record)
        self.write(f"{X86}/updater_skhid.resources.status", b"ready\r\n")
        return bytes(db)

    def test_a_db_changed_in_place_is_put_back_to_stock_first(self):
        self.make_db()
        self.first()
        self.legacy()
        self.assertEqual(self.run_updater(self.release()), 0, self.log())
        self.assert_built()
        log = self.log()
        self.assertIn("changed in place by an earlier updater (3 region(s)): put back", log)
        for name in self.MENUS:
            self.assertIn(f"db: {name}: its original is back in resources.db", log)
        self.assertEqual(self.blob_requests().count(f"/resources/{sha256(self.db[1_400_000:1_480_000])}"), 1)
        self.assertIn("resources.db is the original again; the earlier updater's copies are removed", log)
        for name in ("updater_skhid.resources.state", "updater_skhid.resources.status", "updater_skhid.backup"):
            self.assertFalse(self.exists(f"{X86}/{name}"), name)

    def test_a_db_changed_in_place_that_cannot_be_put_back_is_downloaded_again(self):
        self.make_db()
        self.first()
        self.legacy(kept=())
        # The site has lost the login menu's original.
        del self.site.blobs[sha256(self.db[1_400_000:1_480_000])]
        self.pristine_on_site(self.db)
        self.assertEqual(self.run_updater(self.release()), 0, self.log())
        self.assert_built()
        log = self.log()
        self.assertIn("login_menu: no copy of its original", log)
        self.assertIn("the pristine copy is in place, checked", log)
        self.assertFalse(self.exists(f"{X86}/updater_skhid.resources.state"))

    def test_a_db_changed_in_place_beside_a_ready_database_is_checked_whole_too(self):
        self.make_db()
        self.first()
        self.assertEqual(self.run_updater(self.release()), 0, self.log())
        self.legacy()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertIn("db: resources.db hashed whole", self.log())
        self.assertEqual(sha256(self.read("game/resources.db")), sha256(self.db))
        self.assertFalse(self.exists(f"{X86}/updater_skhid.backup"))

    # --- what play_skhid.bat checks before it starts the game ------------

    def test_play_skhid_bat_starts_the_game_only_when_the_database_is_ready(self):
        self.make_db()
        patch = self.first()
        files = self.release()
        self.site.blob_statuses = {patch["blob"][0]: 404}
        self.assertEqual(self.run_updater(files), 1)
        code, started, said = self.run_bat()
        self.assertEqual((code, started), (1, False), said)
        self.assertIn("Skhid has not finished preparing the game's database", said)
        self.site.blob_statuses = {}
        self.assertEqual(self.run_updater(files), 0, self.log())
        code, started, said = self.run_bat()
        self.assertEqual((code, started), (0, True), said)

    def run_bat(self):
        started = os.path.join(self.x86, "survarium_skhid.exe.started.txt")
        if os.path.exists(started):
            os.remove(started)
        self.write(f"{X86}/play_skhid.bat", builder.bat("srp.example:1234", "english", True))
        command = ["cmd", "/c", "play_skhid.bat"] if WINDOWS else [WINE, "cmd", "/c", "play_skhid.bat"]
        code = subprocess.run(command, cwd=self.x86, env=None if WINDOWS else wine_env(), timeout=120,
                              stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        for _ in range(100):
            if os.path.exists(started):
                break
            time.sleep(0.1)
        return code.returncode, os.path.exists(started), code.stdout.decode("ascii", "replace")


if __name__ == "__main__":
    unittest.main()
