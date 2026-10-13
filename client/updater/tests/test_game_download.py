"""Real updater download tests on Windows or Wine, with a loopback mirror.

Build ../build.sh first. On Linux set WINEPREFIX to a disposable prefix.
SKHID_TEST_GAME_ARCHIVE and SKHID_TEST_EXTRACTOR enable the full 2.9 GB
installation test using the pinned release files; no internet is needed.
"""
import hashlib
import http.server
import io
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import unittest
import zipfile

UPDATER = Path(__file__).resolve().parents[1] / "exe/target/i686-pc-windows-msvc/release/updater_skhid.exe"
GAME_SIZE = 2906544103
EXE_HASH = "ac33d9937015c2299bf62cc6b127cb308b48a3475b30fff2ddd65479ae3bf153"


class GameDownloadTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="skhid-game-download-")
        self.addCleanup(self.temp.cleanup)
        self.game = Path(self.temp.name) / "game"
        self.x86 = self.game / "binaries/x86"
        self.x86.mkdir(parents=True)
        self.exe = self.x86 / "updater_skhid.exe"
        shutil.copyfile(UPDATER, self.exe)
        self.old = b"existing patched game; must survive a failed download"
        (self.x86 / "survarium_skhid.exe").write_bytes(self.old)
        self.overlay = b"verified Skhid overlay"
        bundle = io.BytesIO()
        with zipfile.ZipFile(bundle, "w", zipfile.ZIP_DEFLATED) as z:
            z.writestr("game/binaries/x86/survarium_skhid.exe", self.overlay)
        self.bundle = bundle.getvalue()
        self.routes = {
            "/game": (200, b"<html>Sign in to Google</html>", {}),
            "/extractor": (200, b"not an extractor", {}),
        }
        self.requests = []
        test = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                test.requests.append(self.path)
                if self.path == "/api/client-files":
                    body = f"skhid-client-files 1\nsha256 {hashlib.sha256(test.bundle).hexdigest()}\nsize {len(test.bundle)}\n".encode()
                    status, headers = 200, {}
                elif self.path == "/download":
                    status, body, headers = 200, test.bundle, {}
                else:
                    status, body, headers = test.routes.get(self.path, (404, b"", {}))
                self.send_response(status)
                size = body.stat().st_size if isinstance(body, Path) else len(body)
                self.send_header("Content-Length", str(headers.get("Content-Length", size)))
                for key, value in headers.items():
                    if key != "Content-Length":
                        self.send_header(key, str(value))
                self.end_headers()
                try:
                    if isinstance(body, Path):
                        with body.open("rb") as f:
                            shutil.copyfileobj(f, self.wfile, 1 << 20)
                    else:
                        self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        (self.x86 / "updater_skhid.url").write_text(self.url)
        (self.x86 / "updater_skhid.game-url").write_text(self.url + "/game")
        (self.x86 / "updater_skhid.extractor-url").write_text(self.url + "/extractor")

    def run_updater(self, timeout=60):
        command = [str(self.exe), "check", "quiet"]
        if os.name != "nt":
            command.insert(0, "wine")
        # Wine's background services can inherit pipes beyond updater exit.
        # The updater's own log is the diagnostic source on both platforms.
        result = subprocess.run(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=timeout)
        self.log = (self.x86 / "updater_skhid.log").read_text()
        return result.returncode

    def assert_rejected(self, reason):
        self.assertEqual(self.run_updater(), 1, self.log)
        self.assertIn(reason, self.log)
        self.assertEqual((self.x86 / "survarium_skhid.exe").read_bytes(), self.old)
        self.assertFalse((self.x86 / "survarium.exe").exists())
        self.assertFalse((self.game / "resources.db").exists())
        self.assertFalse((self.game / ".skhid-game.7z").exists())
        self.assertNotIn("/download", self.requests)
        self.assertNotIn("/extractor", self.requests)

    def test_html_response_is_not_extracted_or_installed(self):
        self.assert_rejected("SHA-256 mismatch")

    def test_truncated_download_is_deleted(self):
        self.routes["/game"] = (200, b"partial", {"Content-Length": GAME_SIZE})
        self.assert_rejected("download failed")

    def test_oversized_download_is_rejected_before_writing(self):
        self.routes["/game"] = (200, b"", {"Content-Length": GAME_SIZE + 1})
        self.assert_rejected("too large")

    def test_redirect_is_not_followed(self):
        self.routes["/game"] = (302, b"", {"Location": self.url + "/sign-in"})
        self.assert_rejected("error 302")
        self.assertNotIn("/sign-in", self.requests)

    def test_http_error_does_not_install(self):
        self.routes["/game"] = (403, b"quota exceeded", {})
        self.assert_rejected("error 403")

    def test_remote_plain_http_is_rejected(self):
        (self.x86 / "updater_skhid.game-url").write_text("http://example.com/game.7z")
        self.assert_rejected("URL is invalid")
        self.assertNotIn("/game", self.requests)

    def test_unverified_cached_archive_is_not_trusted(self):
        (self.game / ".skhid-game.7z").write_bytes(b"untrusted cache")
        self.assert_rejected("SHA-256 mismatch")
        self.assertIn("/game", self.requests)

    def test_existing_game_only_downloads_overlay(self):
        (self.game / "resources.db").write_bytes(b"existing resources")
        (self.x86 / "survarium.exe").write_bytes(b"existing original")
        self.assertEqual(self.run_updater(), 0, self.log)
        self.assertEqual((self.x86 / "survarium_skhid.exe").read_bytes(), self.overlay)
        self.assertNotIn("/game", self.requests)
        self.requests.clear()
        self.assertEqual(self.run_updater(), 0, self.log)
        self.assertEqual(self.requests, ["/api/client-files"])

    @unittest.skipUnless(os.environ.get("SKHID_TEST_GAME_ARCHIVE"), "needs the pinned archive fixture")
    def test_same_size_corrupt_extractor_is_never_run(self):
        archive = Path(os.environ["SKHID_TEST_GAME_ARCHIVE"])
        cached = self.game / ".skhid-game.7z"
        try:
            os.link(archive, cached)
        except OSError:
            shutil.copyfile(archive, cached)
        self.routes["/extractor"] = (200, b"\0" * 602624, {})
        self.assertEqual(self.run_updater(timeout=180), 1, self.log)
        self.assertIn("SHA-256 mismatch", self.log)
        self.assertNotIn("/game", self.requests)
        self.assertIn("/extractor", self.requests)
        self.assertFalse((self.game / ".skhid-7zr.exe").exists())
        self.assertFalse(list(self.game.glob(".skhid-extract-*")))
        self.assertEqual((self.x86 / "survarium_skhid.exe").read_bytes(), self.old)

    @unittest.skipUnless(os.environ.get("SKHID_TEST_GAME_ARCHIVE"), "needs the full-download test environment")
    def test_same_size_corrupt_cached_archive_is_rehashed(self):
        # A sparse cache has the exact expected length but wrong contents.
        with (self.game / ".skhid-game.7z").open("wb") as f:
            f.truncate(GAME_SIZE)
        self.assert_rejected("SHA-256 mismatch")
        self.assertIn("/game", self.requests)

    @unittest.skipUnless(os.environ.get("SKHID_TEST_GAME_ARCHIVE") and os.environ.get("SKHID_TEST_EXTRACTOR"),
                         "set SKHID_TEST_GAME_ARCHIVE and SKHID_TEST_EXTRACTOR for the full install")
    def test_full_verified_download_extract_and_overlay(self):
        self.routes["/game"] = (200, Path(os.environ["SKHID_TEST_GAME_ARCHIVE"]), {})
        self.routes["/extractor"] = (200, Path(os.environ["SKHID_TEST_EXTRACTOR"]), {})
        self.assertEqual(self.run_updater(timeout=900), 0, self.log)
        self.assertEqual(hashlib.sha256((self.x86 / "survarium.exe").read_bytes()).hexdigest(), EXE_HASH)
        self.assertEqual((self.game / "resources.db").stat().st_size, 6285533906)
        self.assertEqual((self.x86 / "survarium_skhid.exe").read_bytes(), self.overlay)
        self.assertIn("/game", self.requests)
        self.assertIn("/extractor", self.requests)
        self.assertFalse((self.game / ".skhid-game.7z").exists())
        self.assertFalse((self.game / ".skhid-7zr.exe").exists())
        self.assertFalse(list(self.game.glob(".skhid-extract-*")))


if __name__ == "__main__":
    unittest.main()
