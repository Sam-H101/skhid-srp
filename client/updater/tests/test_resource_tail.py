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
import io
import os
import random
import shutil
import subprocess
import sys
import tempfile
import time
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
# A throwaway key standing for Skhid's: the builder takes a head only when
# its file table is signed by Skhid's key.
KEYS = tempfile.TemporaryDirectory(prefix="skhid-key-")
KEY, PUBLIC = os.path.join(KEYS.name, "key.pem"), os.path.join(KEYS.name, "key.der")


def tearDownModule():
    KEYS.cleanup()


def setUpModule():
    global PREFIX
    builder.S.genkey(KEY, PUBLIC)
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

    def run_ready(self):
        """`updater_skhid.exe ready quiet` (what play_skhid.sh runs): its
        exit code."""
        exe = os.path.join(self.x86, "updater_skhid.exe")
        command = [exe, "ready", "quiet"] if WINDOWS else [WINE, exe, "ready", "quiet"]
        return subprocess.run(command, cwd=self.x86, env=None if WINDOWS else wine_env(), timeout=300,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode

    def run_bat(self):
        """The players' play_skhid.bat, as make_client_026g.py writes it, run
        by cmd; whether it started the game (the stand-in writes down that
        it was started)."""
        started = os.path.join(self.x86, "survarium_skhid.exe.started.txt")
        if os.path.exists(started):
            os.remove(started)
        self.write(f"{X86}/play_skhid.bat", builder.bat("srp.example:1234", "english", True))
        command = ["cmd", "/c", "play_skhid.bat"] if WINDOWS else [WINE, "cmd", "/c", "play_skhid.bat"]
        code = subprocess.run(command, cwd=self.x86, env=None if WINDOWS else wine_env(), timeout=120,
                              stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        # `start` does not wait for the game.
        for _ in range(100):
            if os.path.exists(started):
                break
            time.sleep(0.1)
        return code.returncode, os.path.exists(started), code.stdout.decode("ascii", "replace")

    def make_db(self):
        self.db = random.Random(2777).randbytes(self.DB_SIZE)
        self.write("game/resources.db", self.db)
        self.source = os.path.join(self.root, "source", "game")
        os.makedirs(self.source)
        with open(os.path.join(self.source, "resources.db"), "wb") as file:
            file.write(self.db)

    def release(self, version=1, lobby=True, tail_size=None, tail=True):
        """The plan make_client_026g.py writes for a release's final db (a
        signed head, the lobby's region, a tail; without the tail, a
        `required` line), its blobs on the site, and that db."""
        offset, size = self.HEAD
        # The db's header, a new file table and its signature by "Skhid's" key.
        table = (f"table {version} ".encode() + self.db)[:size // 2]
        head = (b"little-endian\0\0\0" + (1).to_bytes(4, "little") + len(table).to_bytes(4, "little")
                + table + bytes(builder.S.SIGNATURE_SIZE))
        head += self.db[offset + len(head):offset + size]
        grows = tail
        tail = random.Random(version).randbytes(tail_size or self.TAIL_SIZE) if grows else b""
        folder = os.path.join(self.root, f"release-{version}")
        os.makedirs(folder, exist_ok=True)
        with open(os.path.join(folder, "head.bin"), "wb") as file:
            file.write(head)
        builder.S.sign_file(os.path.join(folder, "head.bin"), KEY)
        with open(os.path.join(folder, "head.bin"), "rb") as file:
            head = file.read()
        with open(os.path.join(folder, "tail.bin"), "wb") as file:
            file.write(tail)
        # The plan `srp-game-data plan` gives for the final db: the head, the
        # lobby's region, the tail.
        with mock.patch.object(builder.S, "OWN_KEY", PUBLIC):
            self.assertEqual(builder.S.which_key(*builder.S.read_table(io.BytesIO(head))[1:]), "skhid")
        line, blobs = harness.region_line(self.db, offset, head, "resources_head")
        lines = [line]
        lobby_target = None
        if lobby:
            at, length = self.LOBBY
            lobby_target = harness.forged(self.db[at:at + length], f"lobby {version} ".encode())
            line, lobby_blobs = harness.region_line(self.db, at, lobby_target, "lobby_menu")
            lines.append(line)
            blobs.update(lobby_blobs)
        if grows:
            lines.append(f"tail game/resources.db {len(self.db)} {len(self.db) + len(tail)} {sha256(tail)} "
                         "resources_tail resources_head")
            blobs[sha256(tail)] = tail
        else:
            lines.append("required game/resources.db")
        self.site.blobs.update(blobs)
        expected = bytearray(self.db)
        expected[offset:offset + size] = head
        if lobby:
            at, length = self.LOBBY
            expected[at:at + length] = lobby_target
        self.parts = {"head": head, "tail": tail, "lobby": lobby_target}
        plan = builder.resources_plan(lines)
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
        self.assertIn(" resources_tail resources_head\r\n", record)
        # The table's original is kept; the cache is emptied.
        offset, size = self.HEAD
        original = self.db[offset:offset + size]
        self.assertEqual(self.read(f"{X86}/updater_skhid.backup/{sha256(original)}"), original)
        self.assertFalse(self.exists(f"{X86}/updater_skhid.cache"))
        self.assertFalse(self.exists(f"{X86}/updater_skhid.resources.journal"))
        log = self.log()
        self.assertIn("resources_tail: appended and checked", log)
        self.assertIn("the required plan is in place, checked as a whole", log)
        # The tail before the table that points into it.
        self.assertLess(log.index("resources_tail: appended"), log.index("resources_head: written and checked"))
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
        self.assertIn("resources_tail is in place", self.log())
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
        self.assertIn("resources_tail is no longer planned: it is cut off", log)
        # The table goes back before the tail is cut.
        self.assertLess(log.index("resources_head: its original is back"), log.index("resources_tail: cut off"))

    def test_a_new_release_replaces_the_tail_and_the_table(self):
        self.make_db()
        plan, _ = self.release(1)
        self.assertEqual(self.run_updater(self.with_plan(plan)), 0, self.log())
        plan, expected = self.release(2, tail_size=self.TAIL_SIZE + 12_345)
        self.assertEqual(self.run_updater(self.with_plan(plan)), 0, self.log())
        self.assert_grown(expected)
        self.assertIn("resources_head: the plan has a new target for it", self.log())
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
        self.assertIn("resources_head waits: its tail resources_tail is not in place", log)
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
        self.assertIn("resources_head: its original is back", log)
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
        self.assertIn("an append of resources_tail was cut short", log)
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
        for spoilt, code in [(plan.replace(b"skhid-resources 2", b"skhid-resources 4"), 1),
                             (plan[:-5], 1),
                             (plan.replace(b" resources_tail resources_head", b" resources_tail nothing"), 1)]:
            with self.subTest(plan=spoilt[:40]):
                self.assertEqual(self.run_updater(self.with_plan(spoilt)), code)
                self.assert_shipped()
        self.assertEqual(self.blob_requests(), [])
        self.assertIn("resources: BLOCKED:", self.log())

    # --- a new table and no tail: required all the same ------------------

    def test_a_new_table_without_a_tail_is_required_and_checked_whole(self):
        self.make_db()
        plan, expected = self.release(tail=False)
        self.assertTrue(plan.startswith(b"skhid-resources 3\r\n"), plan[:40])
        self.assertTrue(plan.endswith(b"required game/resources.db\r\nend\r\n"))
        self.assertEqual(len(expected), self.DB_SIZE)
        files = self.with_plan(plan)
        self.assertEqual(self.run_updater(files), 0, self.log())
        self.assert_grown(expected)
        self.assertEqual(self.status(), "ready\r\n")
        log = self.log()
        self.assertIn("(required)", log)
        self.assertIn("the required plan is in place, checked as a whole", log)
        self.assertEqual(self.run_ready(), 0, self.log())
        # The next archive without it puts the db back as shipped.
        self.assertEqual(self.run_updater(self.bundle(README__txt=b"no plan")), 0, self.log())
        self.assert_shipped()
        self.assertIsNone(self.status())

    def test_a_damaged_new_table_keeps_play_off_and_puts_the_db_back(self):
        self.make_db()
        plan, expected = self.release(tail=False)
        files = self.with_plan(plan)
        self.site.blob_bodies[sha256(self.parts["head"])] = b"not the table"
        self.assertEqual(self.run_updater(files), 1)
        self.assert_shipped()
        self.assertTrue(self.status().startswith("blocked "), self.status())
        log = self.log()
        self.assertIn("resources: BLOCKED:", log)
        self.assertIn("resources_head does not hold its target", log)
        self.assertIn("goes back to its bytes as shipped", log)
        self.assertEqual(self.run_ready(), 1)
        self.assertIn("ready: NOT READY:", self.log())
        # The site has it right now: the next start does it all.
        self.site.blob_bodies.clear()
        self.assertEqual(self.run_updater(files), 0, self.log())
        self.assert_grown(expected)
        self.assertEqual(self.run_ready(), 0, self.log())

    def test_a_region_missing_beside_a_new_table_puts_the_table_back_too(self):
        self.make_db()
        plan, expected = self.release(tail=False)
        files = self.with_plan(plan)
        self.site.blob_statuses[sha256(self.parts["lobby"])] = 404
        self.assertEqual(self.run_updater(files), 1)
        self.assert_shipped()
        self.assertIn("resources_head: its original is back", self.log())
        self.site.blob_statuses.clear()
        self.assertEqual(self.run_updater(files), 0, self.log())
        self.assert_grown(expected)

    def test_an_updater_of_format_2_would_keep_play_off_for_a_new_table(self):
        """What an updater that reads format 2 does with the plan: it is of a
        newer format (`required`), so it keeps PLAY off and changes nothing;
        the ZIP brings the updater that reads it. Here: format 3 with the
        `required` line spelt as format 2 is refused the same way."""
        self.make_db()
        plan, _ = self.release(tail=False)
        spoilt = plan.replace(b"skhid-resources 3", b"skhid-resources 2")
        self.assertEqual(self.run_updater(self.with_plan(spoilt)), 1)
        self.assert_shipped()
        self.assertEqual(self.blob_requests(), [])
        self.assertIn("resources: BLOCKED:", self.log())

    def test_a_plan_without_a_tail_stays_best_effort(self):
        self.make_db()
        at, length = self.LOBBY
        target = harness.forged(self.db[at:at + length], b"lobby ")
        line, blobs = harness.region_line(self.db, at, target, "lobby_menu")
        plan = builder.resources_plan([line])
        self.assertTrue(plan.startswith(b"skhid-resources 1\r\n"))
        self.site.blobs.update(blobs)
        self.site.blob_statuses[sha256(target)] = 404
        self.assertEqual(self.run_updater(self.with_plan(plan)), 0, self.log())
        self.assert_shipped()
        self.assertIsNone(self.status())
        self.assertNotIn("BLOCKED", self.log())


    # --- what play_skhid.bat and play_skhid.sh check before they start
    # the game without the launcher: `updater_skhid.exe ready` ----------

    def test_ready_says_whether_a_grown_db_is_in_place(self):
        self.make_db()
        plan, expected = self.release()
        files = self.with_plan(plan)
        self.site.blob_statuses[sha256(self.parts["tail"])] = 404
        self.assertEqual(self.run_updater(files), 1)
        self.assertEqual(self.run_ready(), 1)
        self.assertIn("ready: NOT READY:", self.log())
        self.site.blob_statuses.clear()
        self.assertEqual(self.run_updater(files), 0, self.log())
        self.assertEqual(self.run_ready(), 0, self.log())
        # Damaged after (another program, a disk): not ready again.
        with open(self.path("game/resources.db"), "r+b") as file:
            file.seek(len(self.db) + 10)
            file.write(b"damaged")
        self.assertEqual(self.run_ready(), 1)
        self.assertEqual(self.read("game/resources.db")[len(self.db) + 10:len(self.db) + 17], b"damaged",
                         "ready only reads")

    def test_ready_without_a_required_plan_lets_the_game_start(self):
        self.make_db()
        self.assertEqual(self.run_ready(), 0, "no plan at all")
        self.write(self.PLAN, b"skhid-resources 1\r\nend\r\n")
        self.assertEqual(self.run_ready(), 0, "a plan without a tail")
        self.write(self.PLAN, b"skhid-resources 1\r\nregion cut short")
        self.assertEqual(self.run_ready(), 0, "a format 1 plan that cannot be read")
        self.write(self.PLAN, b"skhid-resources 2\r\nregion cut short")
        self.assertEqual(self.run_ready(), 1, "a format 2 plan that cannot be read")

    def test_play_skhid_bat_starts_the_game_only_when_ready(self):
        self.make_db()
        plan, _ = self.release()
        files = self.with_plan(plan)
        self.site.blob_statuses[sha256(self.parts["tail"])] = 404
        self.assertEqual(self.run_updater(files), 1)
        code, started, said = self.run_bat()
        self.assertEqual((code, started), (1, False), said)
        self.assertIn("Skhid has not finished updating the game's files", said)
        self.site.blob_statuses.clear()
        self.assertEqual(self.run_updater(files), 0, self.log())
        code, started, said = self.run_bat()
        self.assertEqual((code, started), (0, True), said)
        # Without the updater beside it (an archive without the launcher):
        # no check, the game starts as before.
        os.remove(os.path.join(self.x86, "updater_skhid.exe"))
        with open(self.path("game/resources.db"), "r+b") as file:
            file.truncate(len(self.db))
        code, started, said = self.run_bat()
        self.assertEqual((code, started), (0, True), said)


if __name__ == "__main__":
    unittest.main()
