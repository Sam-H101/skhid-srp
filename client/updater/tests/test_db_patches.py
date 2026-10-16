"""End-to-end tests of the patch step: the real updater_skhid.exe, `check
quiet` (no launcher, no box: the exit code, the log and the files tell what
it did), a web server on 127.0.0.1 in the place of the srp one that lists
resources.db's patches at `/patches` with the address of each file (on
itself, or on a second server), and `ready` and play_skhid.bat after it.

    cd client/updater
    ./build.sh
    python tests/test_db_patches.py [-v] [TestName ...]

On Windows the exe runs as it is; elsewhere under Wine, as
test_resource_tail.py (whose harness this uses). Nothing outside the
temporary folders is touched; no game file is needed.
"""

import json
import os
import random
import shutil
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import test_resource_tail as tail  # noqa: E402

harness = tail.harness
X86 = tail.X86
sha256 = tail.sha256
setUpModule = tail.setUpModule
tearDownModule = tail.tearDownModule


class PatchTest(unittest.TestCase):
    setUp = harness.UpdaterTest.setUp
    path = harness.UpdaterTest.path
    write = harness.UpdaterTest.write
    read = harness.UpdaterTest.read
    exists = harness.UpdaterTest.exists
    bundle = harness.UpdaterTest.bundle
    offer = harness.UpdaterTest.offer
    log = harness.UpdaterTest.log
    run_ready = tail.TailTest.run_ready
    run_bat = tail.TailTest.run_bat

    def tearDown(self):
        for site in getattr(self, "others", []):
            site.close()
        harness.UpdaterTest.tearDown(self)

    def run_updater(self):
        """One `check quiet` run against the archive offered (the overlay of
        the harness, no resource plan); its exit code."""
        if not self.site.archive:
            self.offer(self.bundle())
        return tail.TailTest.run_updater(self)

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

    def assert_blocked(self, *said):
        self.assertTrue((self.status() or "").startswith("blocked "), self.status())
        log = self.log()
        self.assertIn("patches: BLOCKED:", log)
        self.assertIn("PLAY stays off until this is done", log)
        for words in said:
            self.assertIn(words, log)
        self.assertEqual(self.run_ready(), 1, "ready refuses while blocked")
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
        # The next start: every file hashed, nothing downloaded or written.
        self.site.requests.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.store_requests(), [])
        self.assertIn("0 to download, 0 to delete", self.log())
        self.assert_patches(patches)

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
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assertEqual(self.store_requests(), ["/store/patch_17"])
        self.assert_patches(patches)

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
        # The server has it right now: the next start does it.
        self.site.patch_bodies.clear()
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)
        self.assertEqual(self.run_ready(), 0)

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
                self.assert_blocked("The list of the game's patches from 127.0.0.1 cannot be used " + said)
        self.site.patch_list = None
        self.assertEqual(self.run_updater(), 0, self.log())
        self.assert_patches(patches)

    def test_a_server_without_the_list_keeps_play_off(self):
        self.site.patch_list_status = 404
        self.assertEqual(self.run_updater(), 1)
        self.assert_blocked("127.0.0.1 offers no list of the game's patches")

    def test_a_list_that_cannot_be_fetched_keeps_play_off_and_is_asked_for_next_time(self):
        patches = self.make_patches(14)
        self.site.patch_list_status = 500
        self.assertEqual(self.run_updater(), 1)
        self.assertEqual(self.site.requests.count("/patches"), 3)
        self.assert_blocked("The list of the game's patches could not be fetched. "
                            "127.0.0.1 answered /patches with error 500")
        self.site.patch_list_status = 200
        self.assertEqual(self.run_updater(), 0, self.log())
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
        self.assert_blocked("Not enough free disk space to update the game's files in", "bytes free in")

    def test_the_overlay_may_not_bring_a_patch_file(self):
        self.offer(self.bundle(**{"game__resources.db.patch_14": b"from the zip"}))
        self.assertEqual(self.run_updater(), 1)
        self.assert_blocked("the overlay has game/resources.db.patch_14")

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


if __name__ == "__main__":
    unittest.main()
