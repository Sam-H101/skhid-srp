"""Run the real native updater against a local HTTP server; no Wine/game needed."""
import hashlib
import http.server
import io
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
import zipfile

HERE = Path(__file__).resolve().parents[1]
EXE = Path(os.environ.get('SKHID_NATIVE_EXE', HERE / ('target/debug/skhid-launcher.exe' if os.name == 'nt' else 'target/debug/skhid-launcher')))


def archive(files):
    data = io.BytesIO()
    with zipfile.ZipFile(data, 'w', zipfile.ZIP_DEFLATED) as z:
        for name, content in files.items():
            z.writestr(name, content)
    return data.getvalue()


class NativeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='skhid native test ')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.game = self.root / 'game'
        self.bin = self.game / 'binaries/x86'
        self.bin.mkdir(parents=True)
        self.files = {'game/binaries/x86/survarium_skhid.exe': b'original test binary',
                      'game/binaries/x86/play_skhid.sh': b'#!/bin/sh\necho fixture\n',
                      'game/resources/ssl/test.crt': b'certificate fixture', 'README.txt': b'instructions'}
        self.body = archive(self.files)
        self.manifest = None
        self.redirect = False
        self.requests = []
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                owner.requests.append(self.path)
                if owner.redirect:
                    self.send_response(302)
                    self.send_header('Location', '/forbidden')
                    self.end_headers()
                    return
                if self.path == '/api/client-files':
                    body = owner.manifest or (f'skhid-client-files 1\nsha256 {hashlib.sha256(owner.body).hexdigest()}\nsize {len(owner.body)}\n').encode()
                elif self.path == '/download':
                    body = owner.body
                else:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)
        self.url = f'http://127.0.0.1:{self.server.server_port}'

    def close_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def run_update(self, success=True):
        result = subprocess.run([str(EXE), '--game-dir', str(self.root), '--site', self.url, '--check'],
                                text=True, capture_output=True, timeout=30)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def test_install_noop_repair_and_preserve_edited_script(self):
        self.run_update()
        for name, content in self.files.items():
            self.assertEqual((self.root / name).read_bytes(), content)
        self.requests.clear()
        self.run_update()
        self.assertEqual(self.requests, ['/api/client-files'])
        (self.bin / 'play_skhid.sh').write_text('edited script')
        (self.bin / 'survarium_skhid.exe').write_bytes(b'damaged')
        self.run_update()
        self.assertEqual((self.bin / 'survarium_skhid.exe').read_bytes(), self.files['game/binaries/x86/survarium_skhid.exe'])
        self.assertEqual((self.bin / 'play_skhid.sh').read_text(), 'edited script')
        if os.name != 'nt':
            self.assertTrue((self.bin / 'play_skhid.sh').stat().st_mode & 0o111)

    def test_new_archive_removes_only_unmodified_obsolete_files(self):
        self.run_update()
        (self.root / 'README.txt').write_text('keep my notes')
        self.body = archive({'game/binaries/x86/survarium_skhid.exe': b'new binary'})
        self.run_update()
        self.assertEqual((self.bin / 'survarium_skhid.exe').read_bytes(), b'new binary')
        self.assertFalse((self.bin / 'play_skhid.sh').exists())
        self.assertEqual((self.root / 'README.txt').read_text(), 'keep my notes')

    def test_wrong_hash_changes_nothing(self):
        self.manifest = f'skhid-client-files 1\nsha256 {"0" * 64}\nsize {len(self.body)}\n'.encode()
        self.assertIn('SHA-256', self.run_update(False).stderr)
        self.assertFalse((self.bin / 'survarium_skhid.exe').exists())

    def test_explicit_site_overrides_invalid_sidecar(self):
        (self.bin / 'updater_skhid.url').write_text('invalid URL')
        self.run_update()

    @unittest.skipIf(os.name == 'nt', 'Linux script uses flock')
    def test_game_script_lock_prevents_native_update(self):
        import fcntl
        with (self.bin / '.skhid-launcher.lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertIn('another launcher', self.run_update(False).stderr)
        self.assertEqual(self.requests, [])

    def test_path_traversal_is_rejected_before_any_write(self):
        self.body = archive({'game/binaries/x86/survarium_skhid.exe': b'new', '../outside': b'bad'})
        self.run_update(False)
        self.assertFalse((self.bin / 'survarium_skhid.exe').exists())

    def test_redirect_is_not_followed(self):
        self.redirect = True
        self.run_update(False)
        self.assertEqual(self.requests, ['/api/client-files'])

    def test_reserved_state_path_cannot_be_installed(self):
        self.body = archive({'game/binaries/x86/.skhid-native.state': b'bad'})
        self.run_update(False)
        self.assertFalse((self.bin / '.skhid-native.state').exists())

    @unittest.skipIf(os.name == 'nt', 'Unix symlink creation')
    def test_symlink_directory_is_rejected(self):
        outside = self.root / 'outside'
        outside.mkdir()
        (self.game / 'resources').symlink_to(outside, target_is_directory=True)
        self.run_update(False)
        self.assertFalse((outside / 'ssl').exists())
        self.assertFalse((self.bin / 'survarium_skhid.exe').exists())

    def test_network_failure_preserves_installed_game(self):
        self.run_update()
        self.url = 'http://127.0.0.1:1'
        self.run_update(False)
        self.assertEqual((self.bin / 'survarium_skhid.exe').read_bytes(), self.files['game/binaries/x86/survarium_skhid.exe'])


if __name__ == '__main__':
    unittest.main()
