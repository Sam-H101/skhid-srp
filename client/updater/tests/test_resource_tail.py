"""End-to-end tests of a plan that grows resources.db (a `tail` line, plan
format 2): the real updater_skhid.exe, `check quiet` (no launcher, no box:
the exit code, the log and the files tell what it did), a web server on
127.0.0.1 in the place of the srp one, and a small stand-in for resources.db.

    cd client/updater
    ./build.sh
    python tests/test_resource_tail.py [-v] [TestName ...]

On Windows the exe runs as it is; elsewhere under Wine (`wine` on the PATH,
or SKHID_TEST_WINE; a prefix of its own in a temporary folder unless
SKHID_TEST_WINEPREFIX names one). Nothing outside the temporary folders is
touched; no game file is needed. What needs another program to hold the db
(a running game) is in test_updater.py, on Windows.
"""

import hashlib
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

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

builder = harness.builder
X86 = harness.X86
sha256 = harness.sha256
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
        PREFIX = tempfile.mkdtemp(prefix="skhid-tail-wine-")
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


class TailTest(unittest.TestCase):
    # The stand-in db: its file table (the head), the lobby's entry, and the
    # new entries appended.
    DB_SIZE = 4 << 20
    HEAD = (0, 65_536)
    LOBBY = (1_000_000, 300_000)
    TAIL_SIZE = 1 << 20
    PLAN = f"{X86}/updater_skhid.resources"

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

    def run_updater(self, files=None):
        """One `check quiet` run against the archive offered; its exit code."""
        if files is not None:
            self.offer(files)
        exe = os.path.join(self.x86, "updater_skhid.exe")
        if WINDOWS:
            command, env = [exe, "check", "quiet"], None
        else:
            command, env = [WINE, exe, "check", "quiet"], wine_env()
        return subprocess.run(command, cwd=self.x86, env=env, timeout=300,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode

    def make_db(self):
        self.db = random.Random(2777).randbytes(self.DB_SIZE)
        self.write("game/resources.db", self.db)
        self.source = os.path.join(self.root, "source", "game")
        os.makedirs(self.source)
        with open(os.path.join(self.source, "resources.db"), "wb") as file:
            file.write(self.db)

    def release(self, version=1, lobby=True, tail_size=None):
        """The plan make_client_026g.py writes for a grow manifest (and the
        lobby's region), its blobs on the site, and the db it makes."""
        offset, size = self.HEAD
        head = (f"table {version} ".encode() + self.db[offset:offset + size])[:size]
        tail = random.Random(version).randbytes(tail_size or self.TAIL_SIZE)
        folder = os.path.join(self.root, f"release-{version}")
        os.makedirs(folder, exist_ok=True)
        with open(os.path.join(folder, "head.bin"), "wb") as file:
            file.write(head)
        with open(os.path.join(folder, "tail.bin"), "wb") as file:
            file.write(tail)
        manifest = os.path.join(folder, "grow.json")
        with open(manifest, "w") as file:
            json.dump({"format": "skhid-db-grow 1", "path": "game/resources.db",
                       "original_size": len(self.db), "new_size": len(self.db) + len(tail),
                       "head": {"name": "file_table", "offset": offset, "size": size, "file": "head.bin",
                                "original_sha256": sha256(self.db[offset:offset + size]),
                                "target_sha256": sha256(head)},
                       "tail": {"name": "new_items", "file": "tail.bin", "sha256": sha256(tail)}}, file)
        lines, blobs = [], {}
        lobby_target = None
        if lobby:
            at, length = self.LOBBY
            lobby_target = harness.forged(self.db[at:at + length], f"lobby {version} ".encode())
            path = os.path.join(folder, "lobby.bin")
            with open(path, "wb") as file:
                file.write(lobby_target)
            table = {"lobby_menu": (at, length, sha256(self.db[at:at + length]))}
            with mock.patch.dict(builder.RESOURCE_REGIONS, table, clear=True):
                lines, blobs = builder.resource_regions(self.source, [("lobby_menu", path)])
        grow_lines, grow_blobs = builder.resource_grow(self.source, manifest, lines)
        blobs.update(grow_blobs)
        self.site.blobs.update(blobs)
        expected = bytearray(self.db)
        expected[offset:offset + size] = head
        if lobby:
            at, length = self.LOBBY
            expected[at:at + length] = lobby_target
        self.parts = {"head": head, "tail": tail, "lobby": lobby_target}
        plan = builder.resources_plan(lines + grow_lines)
        return plan, bytes(expected) + tail

    def with_plan(self, plan, **changes):
        return self.bundle(**{self.PLAN.replace("/", "__"): plan}, **changes)

    def blob_requests(self):
        return [path for path in self.site.requests if path.startswith("/resources/")]

    def db_now(self):
        return self.read("game/resources.db")

    def status(self):
        name = f"{X86}/updater_skhid.resources.status"
        return self.read(name).decode() if self.exists(name) else None

    def assert_shipped(self):
        """The db byte for byte as shipped (compared without printing 4 MB)."""
        now = self.db_now()
        self.assertEqual((len(now), sha256(now)), (len(self.db), sha256(self.db)))

    def assert_grown(self, expected):
        now = self.db_now()
        self.assertEqual((len(now), sha256(now)), (len(expected), sha256(expected)))

    # --- the normal runs -------------------------------------------------

    def test_a_grown_db_is_appended_written_checked_and_found_done_after(self):
        self.make_db()
        plan, expected = self.release()
        self.assertTrue(plan.startswith(b"skhid-resources 2\r\n"))
        files = self.with_plan(plan)
        self.assertEqual(self.run_updater(files), 0, self.log())
        self.assert_grown(expected)
        self.assertEqual(self.status(), "ready\r\n")
        record = self.read(f"{X86}/updater_skhid.resources.state").decode()
        self.assertTrue(record.startswith("skhid-resources-state 2\r\n"), record)
        self.assertIn(" new_items file_table\r\n", record)
        # The table's original is kept; the cache is emptied.
        offset, size = self.HEAD
        original = self.db[offset:offset + size]
        self.assertEqual(self.read(f"{X86}/updater_skhid.backup/{sha256(original)}"), original)
        self.assertFalse(self.exists(f"{X86}/updater_skhid.cache"))
        self.assertFalse(self.exists(f"{X86}/updater_skhid.resources.journal"))
        log = self.log()
        self.assertIn("new_items: appended and checked", log)
        self.assertIn("the required plan is in place, checked as a whole", log)
        # The tail before the table that points into it.
        self.assertLess(log.index("new_items: appended"), log.index("file_table: written and checked"))
        # The next start: hashed, nothing fetched or written.
        self.site.requests.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.blob_requests(), [])
        self.assert_grown(expected)
        self.assertEqual(self.leftovers(), [])

    def test_a_db_grown_already_is_found_done(self):
        self.make_db()
        plan, expected = self.release()
        # The record is lost, the db already is what the plan makes.
        self.write("game/resources.db", expected)
        self.assertEqual(self.run_updater(self.with_plan(plan)), 0, self.log())
        self.assertEqual(self.blob_requests(), [])
        self.assert_grown(expected)
        self.assertIn("new_items is in place", self.log())
        self.assertEqual(self.status(), "ready\r\n")

    def test_a_plan_the_next_archive_drops_cuts_the_db_back(self):
        self.make_db()
        plan, expected = self.release()
        self.assertEqual(self.run_updater(self.with_plan(plan)), 0, self.log())
        self.assert_grown(expected)
        self.site.requests.clear()
        self.assertEqual(self.run_updater(self.bundle(README__txt=b"no plan")), 0, self.log())
        self.assert_shipped()
        self.assertEqual(self.blob_requests(), [], "the originals come from the copies kept")
        self.assertFalse(self.exists(f"{X86}/updater_skhid.resources.state"))
        self.assertFalse(self.exists(f"{X86}/updater_skhid.backup"))
        self.assertIsNone(self.status())
        log = self.log()
        self.assertIn("new_items is no longer planned: it is cut off", log)
        # The table goes back before the tail is cut.
        self.assertLess(log.index("file_table: its original is back"), log.index("new_items: cut off"))

    def test_a_new_release_replaces_the_tail_and_the_table(self):
        self.make_db()
        plan, _ = self.release(1)
        self.assertEqual(self.run_updater(self.with_plan(plan)), 0, self.log())
        plan, expected = self.release(2, tail_size=self.TAIL_SIZE + 12_345)
        self.assertEqual(self.run_updater(self.with_plan(plan)), 0, self.log())
        self.assert_grown(expected)
        self.assertIn("file_table: the plan has a new target for it", self.log())
        self.assertEqual(self.run_updater(self.bundle(README__txt=b"no plan")), 0, self.log())
        self.assert_shipped()

    # --- what keeps PLAY off, and is repaired --------------------------------

    def test_a_tail_the_site_lacks_keeps_play_off_and_changes_nothing(self):
        self.make_db()
        plan, expected = self.release()
        files = self.with_plan(plan)
        self.site.blob_statuses[sha256(self.parts["tail"])] = 404
        self.assertEqual(self.run_updater(files), 1)
        self.assert_shipped()
        self.assertTrue(self.status().startswith("blocked "), self.status())
        log = self.log()
        self.assertIn("resources: BLOCKED:", log)
        self.assertIn("PLAY stays off until this is done", log)
        self.assertIn("file_table waits: its tail new_items is not in place", log)
        # The site has it now: the next start does it all.
        self.site.blob_statuses.clear()
        self.assertEqual(self.run_updater(files), 0, self.log())
        self.assert_grown(expected)

    def test_a_table_that_cannot_be_written_cuts_the_tail_back(self):
        self.make_db()
        plan, expected = self.release()
        files = self.with_plan(plan)
        self.site.blob_statuses[sha256(self.parts["head"])] = 404
        self.assertEqual(self.run_updater(files), 1)
        self.assert_shipped()
        self.assertIn("goes back to its bytes as shipped", self.log())
        # The tail's blob stays for the next run: not fetched again.
        self.assertTrue(self.exists(f"{X86}/updater_skhid.cache/{sha256(self.parts['tail'])}"))
        self.site.blob_statuses.clear()
        self.site.requests.clear()
        self.assertEqual(self.run_updater(files), 0, self.log())
        self.assert_grown(expected)
        self.assertNotIn(f"/resources/{sha256(self.parts['tail'])}", self.blob_requests())

    def test_any_region_missing_keeps_play_off_and_puts_the_db_back(self):
        self.make_db()
        plan, expected = self.release()
        files = self.with_plan(plan)
        self.site.blob_bodies[sha256(self.parts["lobby"])] = b"not the lobby"
        self.assertEqual(self.run_updater(files), 1)
        self.assert_shipped()
        log = self.log()
        self.assertIn("lobby_menu does not hold its target", log)
        self.assertIn("file_table: its original is back", log)
        self.site.blob_bodies.clear()
        self.assertEqual(self.run_updater(files), 0, self.log())
        self.assert_grown(expected)

    def test_a_db_of_another_size_keeps_play_off_and_is_left_alone(self):
        self.make_db()
        plan, _ = self.release()
        changed = self.db + b"!"
        self.write("game/resources.db", changed)
        self.assertEqual(self.run_updater(self.with_plan(plan)), 1)
        self.assertEqual(sha256(self.db_now()), sha256(changed))
        self.assertEqual(self.blob_requests(), [])
        log = self.log()
        self.assertIn(f"has {self.DB_SIZE + 1} bytes, neither {self.DB_SIZE} as shipped", log)
        self.assertIn("is not the game file this Skhid release is for", log)
        self.assertIn("Delete game\\resources.db and start the launcher again", log)
        self.assertTrue(self.status().startswith("blocked "))

    def test_an_append_cut_short_is_repaired(self):
        self.make_db()
        plan, expected = self.release()
        tail_line = next(line for line in plan.decode().split("\r\n") if line.startswith("tail "))
        state = f"skhid-resources-state 2\r\n{tail_line}\r\nend\r\n".encode()
        # The PC went off half way through the append: recorded, journalled.
        self.write("game/resources.db", self.db + self.parts["tail"][:12_345])
        self.write(f"{X86}/updater_skhid.resources.state", state)
        self.write(f"{X86}/updater_skhid.resources.journal", state)
        self.assertEqual(self.run_updater(self.with_plan(plan)), 0, self.log())
        self.assert_grown(expected)
        log = self.log()
        self.assertIn("an append of new_items was cut short", log)
        self.assertFalse(self.exists(f"{X86}/updater_skhid.resources.journal"))

    def test_a_table_without_its_tail_is_repaired(self):
        self.make_db()
        plan, expected = self.release()
        lines = plan.decode().split("\r\n")
        recorded = [line for line in lines if line.startswith(("region ", "tail "))]
        state = "".join(line + "\r\n" for line in ["skhid-resources-state 2", *recorded, "end"]).encode()
        # The table and the lobby are written, the tail is gone (a cut that
        # was not followed by the table going back).
        offset, size = self.HEAD
        self.write("game/resources.db", expected[:len(self.db)])
        self.write(f"{X86}/updater_skhid.resources.state", state)
        self.assertEqual(self.run_updater(self.with_plan(plan)), 0, self.log())
        self.assert_grown(expected)

    def test_a_plan_the_updater_cannot_read_keeps_play_off_when_it_may_grow_the_db(self):
        self.make_db()
        plan, _ = self.release()
        for spoilt, code in [(plan.replace(b"skhid-resources 2", b"skhid-resources 3"), 1),
                             (plan[:-5], 1),
                             (plan.replace(b" new_items file_table", b" new_items nothing"), 1)]:
            with self.subTest(plan=spoilt[:40]):
                self.assertEqual(self.run_updater(self.with_plan(spoilt)), code)
                self.assert_shipped()
        self.assertEqual(self.blob_requests(), [])
        self.assertIn("resources: BLOCKED:", self.log())

    def test_a_plan_without_a_tail_stays_best_effort(self):
        self.make_db()
        at, length = self.LOBBY
        target = harness.forged(self.db[at:at + length], b"lobby ")
        path = os.path.join(self.root, "lobby.bin")
        with open(path, "wb") as file:
            file.write(target)
        table = {"lobby_menu": (at, length, sha256(self.db[at:at + length]))}
        with mock.patch.dict(builder.RESOURCE_REGIONS, table, clear=True):
            lines, blobs = builder.resource_regions(self.source, [("lobby_menu", path)])
        plan = builder.resources_plan(lines)
        self.assertTrue(plan.startswith(b"skhid-resources 1\r\n"))
        self.site.blobs.update(blobs)
        self.site.blob_statuses[sha256(target)] = 404
        self.assertEqual(self.run_updater(self.with_plan(plan)), 0, self.log())
        self.assert_shipped()
        self.assertIsNone(self.status())
        self.assertNotIn("BLOCKED", self.log())


if __name__ == "__main__":
    unittest.main()
