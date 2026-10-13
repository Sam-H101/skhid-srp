"""Local tests for the per-user Nix game import; no real game files needed."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('prepare-game.py')

class PrepareTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / 'store game'
        (self.source / 'binaries/x86').mkdir(parents=True)
        (self.source / 'binaries/x86/survarium_launcher.exe').write_bytes(b'fixture')
        (self.source / 'config').write_text('original')
        self.env = dict(os.environ, XDG_DATA_HOME=str(self.root / 'user data'))
        self.command = [sys.executable, str(SCRIPT), str(self.source)]
        self.game = self.root / 'user data/skhid/0.26g/game'

    def run_prepare(self):
        return subprocess.run(self.command, env=self.env, text=True, capture_output=True)

    def test_first_run_and_rerun_preserve_user_files(self):
        first = self.run_prepare()
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(Path(first.stdout.strip()), self.game)
        (self.game / 'config').write_text('player settings')
        (self.source / 'config').write_text('changed input')
        self.assertEqual(self.run_prepare().returncode, 0)
        self.assertEqual((self.game / 'config').read_text(), 'player settings')

    def test_concurrent_starters_share_one_install(self):
        children = [subprocess.Popen(self.command, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE) for _ in range(2)]
        for child in children:
            out, err = child.communicate(timeout=20)
            self.assertEqual(child.returncode, 0, err)
            self.assertEqual(Path(out.decode().strip()), self.game)

    def test_incomplete_existing_game_is_left_untouched(self):
        self.game.mkdir(parents=True)
        (self.game / 'keep').write_text('keep')
        result = self.run_prepare()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list(p.name for p in self.game.iterdir()), ['keep'])

    def test_interrupted_copy_can_retry(self):
        staging = self.game.parent / '.game-staging'
        staging.mkdir(parents=True)
        (staging / 'partial').write_text('partial')
        self.assertEqual(self.run_prepare().returncode, 0)
        self.assertFalse((self.game / 'partial').exists())

if __name__ == '__main__':
    unittest.main()
