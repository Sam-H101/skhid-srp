"""Archive compatibility checks without requiring the proprietary game assets."""
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import zlib

import login_hint_026g as hint


class LoginHintTests(unittest.TestCase):
    def test_crc_trailer_keeps_compressed_movie_and_archive_hash(self):
        # Minimal SWF: zero-bit RECT, frame rate/count, End tag.
        header = b'\x00\x00\x18\x01\x00'
        movie = hint.pack(10, header, [(0, b'\x00\x00')])
        original = bytes(range(256)) * 4
        padded = movie + bytes(len(original) - len(movie))
        result = hint.preserve_crc(padded, original)
        self.assertEqual(len(result), len(original))
        self.assertEqual(result[:-4], padded[:-4])
        self.assertEqual(hint.unpack(result), hint.unpack(movie))
        # Engine CRC has different initial/final bits from zlib. Matching both
        # starting states verifies the equal-length correction works for both.
        for initial in (0, 0xffffffff):
            self.assertEqual(zlib.crc32(result, initial), zlib.crc32(original, initial))

    def test_unknown_movie_is_rejected_before_invoking_compiler(self):
        with mock.patch.object(hint.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'original build 2777'):
                hint.build_movie(b'unknown movie', None, None, None, None)
            run.assert_not_called()

    def test_existing_archive_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'resources.db'
            output.write_bytes(b'existing archive')
            with mock.patch('sys.argv', ['login_hint_026g.py', 'unused', str(output), '--ffdec=unused']):
                with self.assertRaises(SystemExit):
                    hint.main()
            self.assertEqual(output.read_bytes(), b'existing archive')


if __name__ == '__main__':
    unittest.main()
